//! hyprland-surface-v1: per-surface opacity (quickshell) and visible region.
//! shortcuts: the opacity is validated and stored in the object but is not
//! applied when rendering; the visible region is ignored.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

use smithay::{
    reexports::wayland_server::{
        protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch,
        New, Resource, Weak,
    },
    wayland::compositor::with_states,
};

use super::hyprland_protocols::surface::server::{
    hyprland_surface_manager_v1::{self, HyprlandSurfaceManagerV1},
    hyprland_surface_v1::{self, HyprlandSurfaceV1},
};
use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandSurfaceManagerV1, _>(2, ());
}

/// Marks a `wl_surface` that already has a `hyprland_surface_v1`.
struct Taken(AtomicBool);

pub struct SurfaceData {
    surface: Weak<WlSurface>,
    #[allow(dead_code)]
    opacity: Mutex<f64>,
}

impl GlobalDispatch<HyprlandSurfaceManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandSurfaceManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<HyprlandSurfaceManagerV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &HyprlandSurfaceManagerV1,
        request: hyprland_surface_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let hyprland_surface_manager_v1::Request::GetHyprlandSurface { id, surface } = request {
            let taken = with_states(&surface, |s| {
                s.data_map
                    .get_or_insert_threadsafe(|| Taken(AtomicBool::new(false)))
                    .0
                    .swap(true, Ordering::Relaxed)
            });
            if taken {
                resource.post_error(
                    hyprland_surface_manager_v1::Error::AlreadyConstructed,
                    "surface already has a hyprland_surface_v1",
                );
                return;
            }
            data_init.init(
                id,
                SurfaceData {
                    surface: surface.downgrade(),
                    opacity: Mutex::new(1.0),
                },
            );
        }
    }
}

impl Dispatch<HyprlandSurfaceV1, SurfaceData> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &HyprlandSurfaceV1,
        request: hyprland_surface_v1::Request,
        data: &SurfaceData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let hyprland_surface_v1::Request::SetOpacity { opacity } = request {
            if data.surface.upgrade().is_err() {
                resource.post_error(hyprland_surface_v1::Error::NoSurface, "surface is gone");
            } else if !(0.0..=1.0).contains(&opacity) {
                resource.post_error(
                    hyprland_surface_v1::Error::OutOfRange,
                    "opacity out of range",
                );
            } else {
                *data.opacity.lock().unwrap() = opacity;
            }
        }
    }

    fn destroyed(
        _state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        _resource: &HyprlandSurfaceV1,
        data: &SurfaceData,
    ) {
        if let Ok(s) = data.surface.upgrade() {
            with_states(&s, |st| {
                if let Some(t) = st.data_map.get::<Taken>() {
                    t.0.store(false, Ordering::Relaxed);
                }
            });
        }
    }
}
