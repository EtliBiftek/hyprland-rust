//! hyprland-toplevel-mapping-v1: maps an ext/wlr foreign toplevel handle to the
//! Hyprland window address (xdg-desktop-portal-hyprland, so a sharing picker can
//! name a window for hyprland-toplevel-export).

use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};

use super::hyprland_protocols::toplevel_mapping::server::{
    hyprland_toplevel_mapping_manager_v1::{self, HyprlandToplevelMappingManagerV1},
    hyprland_toplevel_window_mapping_handle_v1::{self, HyprlandToplevelWindowMappingHandleV1},
};
use crate::{ipc::window_addr, Hypr};

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandToplevelMappingManagerV1, _>(1, ());
}

impl GlobalDispatch<HyprlandToplevelMappingManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandToplevelMappingManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<HyprlandToplevelMappingManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &HyprlandToplevelMappingManagerV1,
        request: hyprland_toplevel_mapping_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use hyprland_toplevel_mapping_manager_v1::Request as R;
        let (handle, window) = match request {
            R::GetWindowForToplevel { handle, toplevel } => {
                (handle, state.window_for_ext_handle(&toplevel))
            }
            R::GetWindowForToplevelWlr { handle, toplevel } => {
                (handle, state.window_for_wlr_handle(&toplevel))
            }
            _ => return,
        };
        let handle = data_init.init(handle, ());
        let addr = window
            .and_then(|w| u64::from_str_radix(window_addr(&w).trim_start_matches("0x"), 16).ok());
        match addr {
            Some(a) => handle.window_address((a >> 32) as u32, a as u32),
            None => handle.failed(),
        }
    }
}

impl Dispatch<HyprlandToplevelWindowMappingHandleV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &HyprlandToplevelWindowMappingHandleV1,
        _request: hyprland_toplevel_window_mapping_handle_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Only `destroy`.
    }
}
