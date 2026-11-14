//! wlr-screencopy-unstable-v1 (grim, slurp+grim, hyprshot, hyprpicker, wf-recorder,
//! hyprlock screenshot background).
//!
//! `copy` queues the frame; the backend render loop calls `screencopy_process`
//! which renders the whole output into an offscreen texture, reads it back and
//! writes the requested region into the client's shm buffer.
//! Whole-output frames also offer `linux_dmabuf` (Xrgb8888): the output is
//! rendered straight into the client's dmabuf, no readback.
//! shortcuts: region frames are shm only (render with an offset to add dmabuf);
//! `overlay_cursor` is drawn at scale 1; a full-output
//! readback is cropped on the CPU; `copy_with_damage` reports the whole region
//! as damaged and completes on the next frame; the headless backend has no
//! renderer and answers `failed`. Upgrade with damage tracking.

use std::{
    slice,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, Buffer, Fourcc},
        renderer::{
            damage::OutputDamageTracker,
            element::surface::WaylandSurfaceRenderElement,
            gles::{GlesRenderer, GlesTarget, GlesTexture},
            sync::SyncPoint,
            Bind, ExportMem, Offscreen,
        },
    },
    desktop::{space::render_output, Space, Window},
    output::Output,
    reexports::{
        wayland_protocols_wlr::screencopy::v1::server::{
            zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
            zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
        },
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Logical, Rectangle, Transform},
    wayland::{dmabuf::get_dmabuf, shm::with_buffer_contents_mut},
};

use crate::Hypr;

pub struct FrameData {
    output: Option<Output>,
    /// Region in output pixels; `None` when the request was invalid.
    rect: Option<Rectangle<i32, smithay::utils::Physical>>,
    copied: AtomicBool,
    overlay_cursor: bool,
    /// `linux_dmabuf` was offered for this frame.
    dmabuf: bool,
}

/// True for an (X|A)rgb8888 dmabuf of exactly `w` x `h`.
fn dmabuf_matches(buffer: &WlBuffer, w: i32, h: i32) -> bool {
    get_dmabuf(buffer).is_ok_and(|d| {
        matches!(d.format().code, Fourcc::Xrgb8888 | Fourcc::Argb8888)
            && d.size().w == w
            && d.size().h == h
    })
}

struct Pending {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    output: Output,
    rect: Rectangle<i32, smithay::utils::Physical>,
    with_damage: bool,
    overlay_cursor: bool,
}

impl ScreencopyState {
    /// A client is waiting for a frame.
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
    }
}

#[derive(Default)]
pub struct ScreencopyState {
    pending: Vec<Pending>,
}

pub(crate) struct Shot {
    pub rgba: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

/// True for an Xrgb8888 shm buffer of exactly `w` x `h` that fits its pool.
pub(crate) fn shm_matches(buffer: &WlBuffer, w: i32, h: i32) -> bool {
    with_buffer_contents_mut(buffer, |_, len, d| {
        d.format == wl_shm::Format::Xrgb8888
            && d.width == w
            && d.height == h
            && d.stride >= w * 4
            && d.offset >= 0
            && d.offset as usize + d.stride as usize * h as usize <= len
    })
    .unwrap_or(false)
}

/// Writes `rect` of `shot` into a validated shm buffer (Xrgb8888). False if the pool is gone.
pub(crate) fn copy_shot_to_shm(
    buffer: &WlBuffer,
    shot: &Shot,
    rect: Rectangle<i32, smithay::utils::Physical>,
) -> bool {
    let r = rect;
    with_buffer_contents_mut(buffer, |ptr, _, d| {
        for row in 0..r.size.h as usize {
            let y = r.loc.y as usize + row;
            let start = (y * shot.w + r.loc.x as usize) * 4;
            let src = &shot.rgba[start..start + r.size.w as usize * 4];
            // Safety: bounds were validated by the caller against the pool length.
            let dst = unsafe {
                slice::from_raw_parts_mut(
                    ptr.add(d.offset as usize + row * d.stride as usize),
                    r.size.w as usize * 4,
                )
            };
            // GL gives R,G,B,A; wl_shm Xrgb8888 is B,G,R,X in memory.
            for (o, i) in dst
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(src.as_chunks::<4>().0)
            {
                *o = [i[2], i[1], i[0], 255];
            }
        }
    })
    .is_ok()
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

fn new_frame(
    data_init: &mut DataInit<'_, Hypr>,
    frame: New<ZwlrScreencopyFrameV1>,
    wl_output: &WlOutput,
    region: Option<(i32, i32, i32, i32)>,
    overlay_cursor: bool,
    can_export: bool,
) {
    let output = Output::from_resource(wl_output);
    let size = output
        .as_ref()
        .and_then(|o| o.current_mode())
        .map(|m| m.size);
    let scale = output
        .as_ref()
        .map_or(1.0, |o| o.current_scale().fractional_scale());
    let transform = output
        .as_ref()
        .map_or(Transform::Normal, |o| o.current_transform());
    let rect = size.and_then(|s| {
        // The region is in output-local logical coordinates; the buffer is
        // physical and in the mode's orientation (before the output transform).
        let shown = transform.transform_size(s);
        let full = Rectangle::<i32, smithay::utils::Physical>::from_size(shown);
        let r = match region {
            Some((x, y, w, h)) => Rectangle::<i32, Logical>::new((x, y).into(), (w, h).into())
                .to_physical_precise_round(scale),
            None => full,
        };
        r.intersection(full)
            .filter(|r| !r.is_empty())
            .map(|r| transform.transform_rect_in(r, &shown))
    });
    // Only whole-output frames: the dmabuf is the render target.
    let dmabuf = can_export && rect.is_some_and(|r| Some(r.size) == size && r.loc == (0, 0).into());
    let frame = data_init.init(
        frame,
        FrameData {
            output,
            rect,
            copied: AtomicBool::new(false),
            overlay_cursor,
            dmabuf,
        },
    );
    match rect {
        Some(r) => {
            frame.buffer(
                wl_shm::Format::Xrgb8888,
                r.size.w as u32,
                r.size.h as u32,
                r.size.w as u32 * 4,
            );
            if dmabuf {
                frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, r.size.w as u32, r.size.h as u32);
            }
            if frame.version() >= 3 {
                frame.buffer_done();
            }
        }
        None => frame.failed(),
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // `linux_dmabuf` is a version 3 event; a frame has its manager's version.
        let can_export = state.extra.dmabuf.can_export() && resource.version() >= 3;
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                output,
                overlay_cursor,
            } => new_frame(
                data_init,
                frame,
                &output,
                None,
                overlay_cursor != 0,
                can_export,
            ),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                output,
                x,
                y,
                width,
                height,
                overlay_cursor,
            } => new_frame(
                data_init,
                frame,
                &output,
                Some((x, y, width, height)),
                overlay_cursor != 0,
                can_export,
            ),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, FrameData> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &FrameData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let (buffer, with_damage) = match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => (buffer, false),
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        let (Some(rect), Some(output)) = (data.rect, data.output.clone()) else {
            return; // `failed` was already sent
        };
        if data.copied.swap(true, Ordering::Relaxed) {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::AlreadyUsed,
                "frame was already copied",
            );
            return;
        }
        if !shm_matches(&buffer, rect.size.w, rect.size.h)
            && !(data.dmabuf && dmabuf_matches(&buffer, rect.size.w, rect.size.h))
        {
            resource.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "buffer must match an advertised format and size",
            );
            return;
        }
        state.screencopy.pending.push(Pending {
            frame: resource.clone(),
            buffer,
            output,
            rect,
            with_damage,
            overlay_cursor: data.overlay_cursor,
        });
    }
}

impl Pending {
    fn finish(self, shot: &Shot, time: Duration) {
        let r = self.rect;
        let ok = copy_shot_to_shm(&self.buffer, shot, r);
        if !ok {
            self.frame.failed();
            return;
        }
        self.ready(time);
    }

    fn ready(self, time: Duration) {
        let r = self.rect;
        self.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
        if self.with_damage {
            self.frame.damage(0, 0, r.size.w as u32, r.size.h as u32);
        }
        let secs = time.as_secs();
        self.frame
            .ready((secs >> 32) as u32, secs as u32, time.subsec_nanos());
    }
}

impl Hypr {
    /// Answers every queued capture with `failed` (backends without a renderer).
    pub fn screencopy_fail_all(&mut self) {
        for p in self.screencopy.pending.drain(..) {
            p.frame.failed();
        }
        self.capture_fail_all();
    }

    /// Serves the captures queued for `output`. Called by the backends after
    /// each frame, with the renderer that draws the output.
    pub fn screencopy_process(&mut self, renderer: &mut GlesRenderer, output: &Output) {
        self.capture_process(renderer, output);
        let all = std::mem::take(&mut self.screencopy.pending);
        let (mine, rest): (Vec<_>, Vec<_>) = all.into_iter().partition(|p| &p.output == output);
        self.screencopy.pending = rest;
        let mine: Vec<Pending> = mine.into_iter().filter(|p| p.frame.is_alive()).collect();
        if mine.is_empty() {
            return;
        }
        let time = self.start_time.elapsed();
        let (mut plain, mut with_cursor) = (None, None);
        for p in mine {
            if let Ok(mut dmabuf) = get_dmabuf(&p.buffer).cloned() {
                match self.render_shot_dmabuf(renderer, output, p.overlay_cursor, &mut dmabuf) {
                    Ok(()) => p.ready(time),
                    Err(e) => {
                        tracing::warn!("screencopy into a dmabuf failed: {e}");
                        p.frame.failed();
                    }
                }
                continue;
            }
            let slot = if p.overlay_cursor {
                &mut with_cursor
            } else {
                &mut plain
            };
            match slot.get_or_insert_with(|| self.render_shot(renderer, output, p.overlay_cursor)) {
                Ok(shot) => p.finish(shot, time),
                Err(e) => {
                    tracing::warn!("screencopy failed: {e}");
                    p.frame.failed();
                }
            }
        }
    }

    /// Renders the whole output into a texture and reads it back (RGBA).
    pub(crate) fn render_shot(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        overlay_cursor: bool,
    ) -> Result<Shot, String> {
        let size = output.current_mode().ok_or("output has no mode")?.size;
        let mut tex: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
            .map_err(|e| format!("create_buffer: {e:?}"))?;
        let mut target = renderer
            .bind(&mut tex)
            .map_err(|e| format!("bind: {e:?}"))?;
        let _ = self.render_output_into(renderer, &mut target, output, overlay_cursor)?;
        let mapping = renderer
            .copy_framebuffer(
                &target,
                Rectangle::from_size((size.w, size.h).into()),
                Fourcc::Abgr8888,
            )
            .map_err(|e| format!("copy_framebuffer: {e:?}"))?;
        let rgba = renderer
            .map_texture(&mapping)
            .map_err(|e| format!("map_texture: {e:?}"))?
            .to_vec();
        Ok(Shot {
            rgba,
            w: size.w as usize,
            h: size.h as usize,
        })
    }

    /// Renders the whole output into the client's dmabuf.
    fn render_shot_dmabuf(
        &mut self,
        renderer: &mut GlesRenderer,
        output: &Output,
        overlay_cursor: bool,
        dmabuf: &mut Dmabuf,
    ) -> Result<(), String> {
        let mut target = renderer.bind(dmabuf).map_err(|e| format!("bind: {e:?}"))?;
        let sync = self.render_output_into(renderer, &mut target, output, overlay_cursor)?;
        // The client reads the buffer as soon as `ready` arrives.
        let _ = sync.wait();
        Ok(())
    }

    /// Draws the output (or the lock screen) into `target`, which has the mode's size.
    fn render_output_into(
        &mut self,
        renderer: &mut GlesRenderer,
        target: &mut GlesTarget<'_>,
        output: &Output,
        overlay_cursor: bool,
    ) -> Result<SyncPoint, String> {
        let size = output.current_mode().ok_or("output has no mode")?.size;
        let scale = output.current_scale().fractional_scale();
        // Drawn like the scanout buffer, output transform applied: clients undo
        // the transform they know from wl_output (the winit output is
        // `Flipped180`, a rotated tty output 90/270). The readback is top row first.
        let mut tracker = OutputDamageTracker::new(size, scale, output.current_transform());
        let clear = self.clear_color();
        let sync = if self.session_locked {
            let lock = self.lock_elements(renderer, output);
            render_output::<_, WaylandSurfaceRenderElement<GlesRenderer>, _, _>(
                output,
                renderer,
                target,
                1.0,
                0,
                std::iter::empty::<&Space<Window>>(),
                &lock,
                &mut tracker,
                clear,
            )
            .map_err(|e| format!("render: {e:?}"))?
            .sync
        } else {
            let mut elements = crate::render::output_elements(self, renderer, output);
            if overlay_cursor {
                elements.splice(0..0, self.cursor_fx_elements(renderer, output));
            }
            tracker
                .render_output(renderer, target, 0, &elements, clear)
                .map_err(|e| format!("render: {e:?}"))?
                .sync
        };
        Ok(sync)
    }
}
