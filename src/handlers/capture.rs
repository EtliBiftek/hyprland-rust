//! Capture queue shared by hyprland-toplevel-export and ext-image-copy-capture.
//!
//! A protocol handler validates the client's shm buffer and queues a `Job`; the
//! backends' render loop reaches `capture_process` through `screencopy_process`,
//! which renders the target offscreen (like wlr-screencopy), writes the pixels
//! into the shm buffer and runs the job's completion closure. A dmabuf buffer is
//! the render target itself (no readback).
//! shortcuts: every job renders its target again (no damage tracking, no frame
//! sharing), headless fails all jobs.

use std::time::Duration;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{surface::WaylandSurfaceRenderElement, AsRenderElements},
            gles::{GlesRenderer, GlesTarget, GlesTexture},
            sync::SyncPoint,
            Bind, ExportMem, Offscreen,
        },
    },
    desktop::Window,
    output::Output,
    reexports::wayland_server::protocol::wl_buffer::WlBuffer,
    utils::{Rectangle, Scale, Transform},
    wayland::dmabuf::get_dmabuf,
};

use super::screencopy::{copy_shot_to_shm, shm_matches, Shot};
use crate::{ipc::window_addr, Hypr};

pub enum Target {
    Output(Output),
    Window(Window),
}

/// Completion callback: `true` when the pixels were written.
pub type Done = Box<dyn FnOnce(bool, Duration)>;

pub(crate) struct Job {
    target: Target,
    buffer: WlBuffer,
    done: Done,
}

#[derive(Default)]
pub struct CaptureState {
    pub(crate) jobs: Vec<Job>,
}

impl Hypr {
    /// Pixel size of a capture target (output mode, or window geometry).
    pub fn capture_size(&self, target: &Target) -> Option<(i32, i32)> {
        let (w, h) = match target {
            Target::Output(o) => {
                let s = o.current_mode()?.size;
                (s.w, s.h)
            }
            Target::Window(w) => {
                let s = w.geometry().size;
                (s.w, s.h)
            }
        };
        (w > 0 && h > 0).then_some((w, h))
    }

    pub fn queue_capture(&mut self, target: Target, buffer: WlBuffer, done: Done) {
        self.extra.capture.jobs.push(Job {
            target,
            buffer,
            done,
        });
    }

    pub fn capture_fail_all(&mut self) {
        for job in std::mem::take(&mut self.extra.capture.jobs) {
            (job.done)(false, Duration::ZERO);
        }
    }

    /// Window whose Hyprland address is `addr`; with `low32` only the low 32 bits
    /// are compared (hyprland-toplevel-export passes a 32-bit handle).
    pub fn window_by_addr(&self, addr: u64, low32: bool) -> Option<Window> {
        self.workspaces
            .iter()
            .flat_map(|ws| ws.windows())
            .find(|w| {
                let a =
                    u64::from_str_radix(window_addr(w).trim_start_matches("0x"), 16).unwrap_or(0);
                if low32 {
                    a as u32 == addr as u32
                } else {
                    a == addr
                }
            })
    }

    /// Serves the queued jobs. Output jobs only run on their own output; window
    /// jobs run on the first output that renders.
    pub fn capture_process(&mut self, renderer: &mut GlesRenderer, output: &Output) {
        let jobs = std::mem::take(&mut self.extra.capture.jobs);
        if jobs.is_empty() {
            return;
        }
        let time = self.start_time.elapsed();
        for job in jobs {
            if matches!(&job.target, Target::Output(o) if o != output) {
                self.extra.capture.jobs.push(job);
                continue;
            }
            if let Ok(mut dmabuf) = get_dmabuf(&job.buffer).cloned() {
                let res = match &job.target {
                    Target::Output(o) => self.render_shot_dmabuf(renderer, o, false, &mut dmabuf),
                    Target::Window(w) => renderer
                        .bind(&mut dmabuf)
                        .map_err(|e| format!("bind: {e:?}"))
                        .and_then(|mut target| render_window_into(renderer, &mut target, w))
                        // The client reads the buffer as soon as `ready` arrives.
                        .map(|sync| {
                            let _ = sync.wait();
                        }),
                };
                if let Err(e) = &res {
                    tracing::warn!("capture into a dmabuf failed: {e}");
                }
                (job.done)(res.is_ok(), time);
                continue;
            }
            let shot = match &job.target {
                Target::Output(o) => self.render_shot(renderer, o, false),
                Target::Window(w) => render_window_shot(renderer, w),
            };
            let ok = match shot {
                Ok(shot) => {
                    let full = Rectangle::from_size((shot.w as i32, shot.h as i32).into());
                    shm_matches(&job.buffer, shot.w as i32, shot.h as i32)
                        && copy_shot_to_shm(&job.buffer, &shot, full)
                }
                Err(e) => {
                    tracing::warn!("capture failed: {e}");
                    false
                }
            };
            (job.done)(ok, time);
        }
    }
}

/// Draws one window (its geometry box, with popups) into `target`.
fn render_window_into(
    renderer: &mut GlesRenderer,
    target: &mut GlesTarget<'_>,
    window: &Window,
) -> Result<SyncPoint, String> {
    let geo = window.geometry();
    let size = geo.size;
    if size.w <= 0 || size.h <= 0 {
        return Err("window has no size".into());
    }
    // The element location is the surface origin; shift it so the geometry starts at 0,0.
    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> = window.render_elements(
        renderer,
        (-geo.loc.x, -geo.loc.y).into(),
        Scale::from(1.0),
        1.0,
    );
    let mut tracker = OutputDamageTracker::new((size.w, size.h), 1.0, Transform::Normal);
    let res = tracker
        .render_output(renderer, target, 0, &elements, [0.0, 0.0, 0.0, 0.0])
        .map_err(|e| format!("render: {e:?}"))?;
    Ok(res.sync)
}

/// Renders one window into an RGBA readback.
fn render_window_shot(renderer: &mut GlesRenderer, window: &Window) -> Result<Shot, String> {
    let size = window.geometry().size;
    if size.w <= 0 || size.h <= 0 {
        return Err("window has no size".into());
    }
    let mut tex: GlesTexture = renderer
        .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
        .map_err(|e| format!("create_buffer: {e:?}"))?;
    let mut target = renderer
        .bind(&mut tex)
        .map_err(|e| format!("bind: {e:?}"))?;
    let _ = render_window_into(renderer, &mut target, window)?;
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
