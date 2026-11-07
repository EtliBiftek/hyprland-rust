//! xdg-system-bell-v1: `ring` from clients (terminals).
//! shortcuts: only logged, no sound or visual bell.

use smithay::{
    delegate_xdg_system_bell,
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    wayland::xdg_system_bell::{XdgSystemBellHandler, XdgSystemBellState},
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) -> XdgSystemBellState {
    XdgSystemBellState::new::<Hypr>(dh)
}

impl XdgSystemBellHandler for Hypr {
    fn ring(&mut self, surface: Option<WlSurface>) {
        tracing::debug!("system bell (surface: {})", surface.is_some());
    }
}

delegate_xdg_system_bell!(Hypr);
