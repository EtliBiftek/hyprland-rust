//! xdg-toplevel-icon-v1: icons are stored in the surface state by Smithay.
//! shortcuts: no icon is drawn (no titlebars) and none is sent through IPC.

use smithay::{
    delegate_xdg_toplevel_icon,
    reexports::wayland_server::DisplayHandle,
    wayland::xdg_toplevel_icon::{XdgToplevelIconHandler, XdgToplevelIconManager},
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) -> XdgToplevelIconManager {
    let mut m = XdgToplevelIconManager::new::<Hypr>(dh);
    for size in [16, 24, 32, 48, 64] {
        m.add_icon_size(size);
    }
    m
}

impl XdgToplevelIconHandler for Hypr {}

delegate_xdg_toplevel_icon!(Hypr);
