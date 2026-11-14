//! hyprland-toplevel-export-v1 (xdg-desktop-portal-hyprland window sharing and
//! previews): renders one toplevel into the client's shm buffer or dmabuf
//! (Xrgb8888) through the capture queue in `capture.rs`.
//! shortcuts: `overlay_cursor` is ignored, the whole window is reported as damaged.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::{
    backend::allocator::Fourcc,
    desktop::Window,
    reexports::wayland_server::{
        protocol::wl_shm, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
};

use super::{
    capture::Target,
    hyprland_protocols::toplevel_export::server::{
        hyprland_toplevel_export_frame_v1::{self, HyprlandToplevelExportFrameV1},
        hyprland_toplevel_export_manager_v1::{self, HyprlandToplevelExportManagerV1},
    },
    screencopy::{dmabuf_matches, shm_matches},
};
use crate::{ipc::window_addr, Hypr};

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandToplevelExportManagerV1, _>(2, ());
}

pub struct FrameData {
    /// Window address, resolved again at `copy` (the window may be gone by then).
    addr: Option<String>,
    used: AtomicBool,
}

impl GlobalDispatch<HyprlandToplevelExportManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandToplevelExportManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

fn new_frame(
    state: &Hypr,
    data_init: &mut DataInit<'_, Hypr>,
    frame: New<HyprlandToplevelExportFrameV1>,
    window: Option<Window>,
) {
    let size = window
        .as_ref()
        .and_then(|w| state.capture_size(&Target::Window(w.clone())));
    let frame = data_init.init(
        frame,
        FrameData {
            addr: window.as_ref().map(window_addr),
            used: AtomicBool::new(false),
        },
    );
    match size {
        Some((w, h)) => {
            frame.buffer(wl_shm::Format::Xrgb8888, w as u32, h as u32, w as u32 * 4);
            if state.extra.dmabuf.can_export() {
                frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, w as u32, h as u32);
            }
            if frame.version() >= 2 {
                frame.buffer_done();
            }
        }
        None => frame.failed(),
    }
}

impl Dispatch<HyprlandToplevelExportManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &HyprlandToplevelExportManagerV1,
        request: hyprland_toplevel_export_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use hyprland_toplevel_export_manager_v1::Request as R;
        match request {
            R::CaptureToplevel { frame, handle, .. } => {
                let w = state.window_by_addr(handle as u64, true);
                new_frame(state, data_init, frame, w);
            }
            R::CaptureToplevelWithWlrToplevelHandle { frame, handle, .. } => {
                let w = state.window_for_wlr_handle(&handle);
                new_frame(state, data_init, frame, w);
            }
            _ => {}
        }
    }
}

impl Dispatch<HyprlandToplevelExportFrameV1, FrameData> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &HyprlandToplevelExportFrameV1,
        request: hyprland_toplevel_export_frame_v1::Request,
        data: &FrameData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let hyprland_toplevel_export_frame_v1::Request::Copy { buffer, .. } = request else {
            return;
        };
        let Some(addr) = data.addr.as_ref() else {
            return; // `failed` was already sent
        };
        if data.used.swap(true, Ordering::Relaxed) {
            resource.post_error(
                hyprland_toplevel_export_frame_v1::Error::AlreadyUsed,
                "frame was already copied",
            );
            return;
        }
        let window = u64::from_str_radix(addr.trim_start_matches("0x"), 16)
            .ok()
            .and_then(|a| state.window_by_addr(a, false));
        let Some(target) = window.map(Target::Window) else {
            resource.failed();
            return;
        };
        let Some((w, h)) = state.capture_size(&target) else {
            resource.failed();
            return;
        };
        if !shm_matches(&buffer, w, h)
            && !(state.extra.dmabuf.can_export() && dmabuf_matches(&buffer, w, h))
        {
            resource.post_error(
                hyprland_toplevel_export_frame_v1::Error::InvalidBuffer,
                "buffer must match an advertised format and size",
            );
            return;
        }
        let frame = resource.clone();
        state.queue_capture(
            target,
            buffer,
            Box::new(move |ok, time| {
                if !ok {
                    frame.failed();
                    return;
                }
                frame.flags(hyprland_toplevel_export_frame_v1::Flags::empty());
                frame.damage(0, 0, w as u32, h as u32);
                let secs = time.as_secs();
                frame.ready((secs >> 32) as u32, secs as u32, time.subsec_nanos());
            }),
        );
    }
}
