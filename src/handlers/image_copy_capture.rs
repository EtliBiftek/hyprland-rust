//! ext-image-capture-source-v1 and ext-image-copy-capture-v1 (portals, OBS, grim
//! 1.5+, wf-recorder): output and foreign-toplevel sources copied into shm
//! buffers, or rendered into dmabufs (Xrgb8888, `dmabuf_device` only when EGL
//! names a render node), through the capture queue in `capture.rs`.
//! shortcuts: every frame is a full
//! copy reported as fully damaged; `paint_cursors` is ignored; cursor sessions
//! exist but never get position or capture data (their frames fail);
//! a size change after session creation sends new constraints and fails the frame.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use smithay::{
    backend::allocator::Fourcc,
    output::Output,
    reexports::{
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_foreign_toplevel_image_capture_source_manager_v1::{
                    self, ExtForeignToplevelImageCaptureSourceManagerV1,
                },
                ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
                ext_output_image_capture_source_manager_v1::{
                    self, ExtOutputImageCaptureSourceManagerV1,
                },
            },
            image_copy_capture::v1::server::{
                ext_image_copy_capture_cursor_session_v1::{
                    self, ExtImageCopyCaptureCursorSessionV1,
                },
                ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
                ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
                ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
            },
        },
        wayland_server::{
            backend::ClientId,
            protocol::{wl_buffer::WlBuffer, wl_output, wl_shm},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
};

use super::{
    capture::Target,
    dmabuf::DmabufHolder,
    screencopy::{dmabuf_matches, shm_matches},
};
use crate::{ipc::window_addr, Hypr};

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, ExtOutputImageCaptureSourceManagerV1, _>(1, ());
    dh.create_global::<Hypr, ExtForeignToplevelImageCaptureSourceManagerV1, _>(1, ());
    dh.create_global::<Hypr, ExtImageCopyCaptureManagerV1, _>(1, ());
}

pub enum Source {
    Output(Output),
    /// Window address; `None` when the handle did not name a live window.
    Toplevel(Option<String>),
    Cursor,
}

pub struct SessionData {
    source: Source,
    /// Buffer size announced to the client.
    size: Mutex<(i32, i32)>,
    /// A frame object exists that has not been destroyed yet.
    frame_live: AtomicBool,
}

pub struct FrameData {
    session: ExtImageCopyCaptureSessionV1,
    buffer: Mutex<Option<WlBuffer>>,
    captured: AtomicBool,
}

macro_rules! plain_global {
    ($iface:ty) => {
        impl GlobalDispatch<$iface, ()> for Hypr {
            fn bind(
                _state: &mut Self,
                _handle: &DisplayHandle,
                _client: &Client,
                resource: New<$iface>,
                _global_data: &(),
                data_init: &mut DataInit<'_, Self>,
            ) {
                data_init.init(resource, ());
            }
        }
    };
}
plain_global!(ExtOutputImageCaptureSourceManagerV1);
plain_global!(ExtForeignToplevelImageCaptureSourceManagerV1);
plain_global!(ExtImageCopyCaptureManagerV1);

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ExtOutputImageCaptureSourceManagerV1,
        request: ext_output_image_capture_source_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource {
            source,
            output,
        } = request
        {
            // An output that is not ours gets a source that stops its sessions at once.
            let s = match Output::from_resource(&output) {
                Some(o) => Source::Output(o),
                None => Source::Toplevel(None),
            };
            data_init.init(source, s);
        }
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
            source,
            toplevel_handle,
        } = request
        {
            let addr = state
                .window_for_ext_handle(&toplevel_handle)
                .map(|w| window_addr(&w));
            data_init.init(source, Source::Toplevel(addr));
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, Source> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ExtImageCaptureSourceV1,
        _request: ext_image_capture_source_v1::Request,
        _data: &Source,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Only `destroy`.
    }
}

impl Source {
    fn target(&self, state: &Hypr) -> Option<Target> {
        match self {
            Source::Output(o) => Some(Target::Output(o.clone())),
            Source::Toplevel(Some(addr)) => {
                let a = u64::from_str_radix(addr.trim_start_matches("0x"), 16).ok()?;
                state.window_by_addr(a, false).map(Target::Window)
            }
            _ => None,
        }
    }
}

/// Creates a session object for `source` and announces its constraints.
fn new_session(
    state: &Hypr,
    data_init: &mut DataInit<'_, Hypr>,
    session: New<ExtImageCopyCaptureSessionV1>,
    source: Source,
) {
    let size = match &source {
        Source::Cursor => Some((64, 64)),
        s => s.target(state).and_then(|t| state.capture_size(&t)),
    };
    let sess = data_init.init(
        session,
        SessionData {
            source,
            size: Mutex::new(size.unwrap_or((0, 0))),
            frame_live: AtomicBool::new(false),
        },
    );
    match size {
        Some((w, h)) => send_constraints(&sess, w, h, &state.extra.dmabuf),
        None => sess.stopped(),
    }
}

fn send_constraints(session: &ExtImageCopyCaptureSessionV1, w: i32, h: i32, dma: &DmabufHolder) {
    session.buffer_size(w as u32, h as u32);
    session.shm_format(wl_shm::Format::Xrgb8888);
    if let Some((mods, device)) = dma.capture() {
        let mods = mods.iter().flat_map(|m| m.to_ne_bytes()).collect();
        session.dmabuf_format(Fourcc::Xrgb8888 as u32, mods);
        if let Some(dev) = device {
            session.dmabuf_device(dev.to_ne_bytes().to_vec());
        }
    }
    session.done();
}

/// Clone of the `Source` held in a capture source object's user data.
fn source_data(source: &ExtImageCaptureSourceV1) -> Option<Source> {
    match source.data::<Source>()? {
        Source::Output(o) => Some(Source::Output(o.clone())),
        Source::Toplevel(a) => Some(Source::Toplevel(a.clone())),
        Source::Cursor => Some(Source::Cursor),
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use ext_image_copy_capture_manager_v1::Request as R;
        match request {
            R::CreateSession {
                session, source, ..
            } => {
                let src = source_data(&source).unwrap_or(Source::Toplevel(None));
                new_session(state, data_init, session, src);
            }
            R::CreatePointerCursorSession { session, .. } => {
                data_init.init(session, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } =
            request
        {
            new_session(state, data_init, session, Source::Cursor);
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, SessionData> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        data: &SessionData,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            if data.frame_live.swap(true, Ordering::Relaxed) {
                resource.post_error(
                    ext_image_copy_capture_session_v1::Error::DuplicateFrame,
                    "the previous frame was not destroyed",
                );
                return;
            }
            data_init.init(
                frame,
                FrameData {
                    session: resource.clone(),
                    buffer: Mutex::new(None),
                    captured: AtomicBool::new(false),
                },
            );
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &FrameData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use ext_image_copy_capture_frame_v1::{Error, FailureReason, Request as R};
        match request {
            R::AttachBuffer { buffer } => {
                if data.captured.load(Ordering::Relaxed) {
                    resource.post_error(Error::AlreadyCaptured, "frame was already captured");
                    return;
                }
                *data.buffer.lock().unwrap() = Some(buffer);
            }
            R::Capture => {
                if data.captured.swap(true, Ordering::Relaxed) {
                    resource.post_error(Error::AlreadyCaptured, "frame was already captured");
                    return;
                }
                let Some(buffer) = data.buffer.lock().unwrap().clone() else {
                    resource.post_error(Error::NoBuffer, "no buffer attached");
                    return;
                };
                let Some(sess) = data.session.data::<SessionData>() else {
                    return;
                };
                let Some(target) = sess.source.target(state) else {
                    resource.failed(FailureReason::Unknown);
                    if !matches!(sess.source, Source::Cursor) {
                        data.session.stopped();
                    }
                    return;
                };
                let Some((w, h)) = state.capture_size(&target) else {
                    resource.failed(FailureReason::Stopped);
                    data.session.stopped();
                    return;
                };
                {
                    let mut announced = sess.size.lock().unwrap();
                    if *announced != (w, h) {
                        *announced = (w, h);
                        send_constraints(&data.session, w, h, &state.extra.dmabuf);
                        resource.failed(FailureReason::BufferConstraints);
                        return;
                    }
                }
                if !shm_matches(&buffer, w, h)
                    && !(state.extra.dmabuf.can_export() && dmabuf_matches(&buffer, w, h))
                {
                    resource.failed(FailureReason::BufferConstraints);
                    return;
                }
                let frame = resource.clone();
                // Output frames are in scanout orientation, like wlr-screencopy.
                let transform = match &target {
                    Target::Output(o) => o.current_transform().into(),
                    _ => wl_output::Transform::Normal,
                };
                state.queue_capture(
                    target,
                    buffer,
                    Box::new(move |ok, time| {
                        if !ok {
                            frame.failed(FailureReason::Unknown);
                            return;
                        }
                        frame.transform(transform);
                        frame.damage(0, 0, w, h);
                        let secs = time.as_secs();
                        frame.presentation_time(
                            (secs >> 32) as u32,
                            secs as u32,
                            time.subsec_nanos(),
                        );
                        frame.ready();
                    }),
                );
            }
            _ => {}
        }
    }

    fn destroyed(
        _state: &mut Self,
        _client: ClientId,
        _resource: &ExtImageCopyCaptureFrameV1,
        data: &FrameData,
    ) {
        if let Some(s) = data.session.data::<SessionData>() {
            s.frame_live.store(false, Ordering::Relaxed);
        }
    }
}
