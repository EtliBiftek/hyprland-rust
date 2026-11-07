//! hyprland-focus-grab-v1 (quickshell and ags popups, menus): while a committed
//! grab is active, a click outside its whitelisted surfaces sends `cleared`.
//! shortcuts: pointer focus is not restricted to the whitelist (only the clear on
//! an outside click and keyboard focus on commit are implemented); a whitelisted
//! surface matches by its root (subsurface parent), popups must be listed.

use smithay::{
    input::keyboard::KeyboardHandle,
    reexports::wayland_server::{
        protocol::wl_surface::WlSurface, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch,
        New, Resource,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::compositor::get_parent,
};

use super::hyprland_protocols::focus_grab::server::{
    hyprland_focus_grab_manager_v1::{self, HyprlandFocusGrabManagerV1},
    hyprland_focus_grab_v1::{self, HyprlandFocusGrabV1},
};
use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandFocusGrabManagerV1, _>(1, ());
}

struct Grab {
    resource: HyprlandFocusGrabV1,
    surfaces: Vec<WlSurface>,
    committed: bool,
}

#[derive(Default)]
pub struct FocusGrabState {
    grabs: Vec<Grab>,
}

fn root(s: &WlSurface) -> WlSurface {
    let mut s = s.clone();
    while let Some(p) = get_parent(&s) {
        s = p;
    }
    s
}

impl GlobalDispatch<HyprlandFocusGrabManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandFocusGrabManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<HyprlandFocusGrabManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &HyprlandFocusGrabManagerV1,
        request: hyprland_focus_grab_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let hyprland_focus_grab_manager_v1::Request::CreateGrab { grab } = request {
            let resource = data_init.init(grab, ());
            state.extra.focus_grab.grabs.push(Grab {
                resource,
                surfaces: Vec::new(),
                committed: false,
            });
        }
    }
}

impl Dispatch<HyprlandFocusGrabV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &HyprlandFocusGrabV1,
        request: hyprland_focus_grab_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use hyprland_focus_grab_v1::Request as R;
        let Some(grab) = state
            .extra
            .focus_grab
            .grabs
            .iter_mut()
            .find(|g| &g.resource == resource)
        else {
            return;
        };
        match request {
            R::AddSurface { surface } => {
                if !grab.surfaces.contains(&surface) {
                    grab.surfaces.push(surface);
                }
            }
            R::RemoveSurface { surface } => grab.surfaces.retain(|s| s != &surface),
            R::Commit => {
                grab.committed = true;
                let first = grab.surfaces.first().cloned();
                let kb: Option<KeyboardHandle<Hypr>> = state.seat.get_keyboard();
                if let (Some(kb), Some(s)) = (kb, first) {
                    if kb.current_focus().as_ref() != Some(&s) {
                        kb.set_focus(state, Some(s), SERIAL_COUNTER.next_serial());
                    }
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &HyprlandFocusGrabV1,
        _data: &(),
    ) {
        state
            .extra
            .focus_grab
            .grabs
            .retain(|g| &g.resource != resource);
    }
}

impl Hypr {
    /// Called on every button press: a press outside the whitelist clears active grabs.
    pub fn focus_grab_button(&mut self, pos: Point<f64, Logical>) {
        if self.extra.focus_grab.grabs.iter().all(|g| !g.committed) {
            return;
        }
        let under = self.surface_under(pos).map(|(s, _)| root(&s));
        for g in self
            .extra
            .focus_grab
            .grabs
            .iter_mut()
            .filter(|g| g.committed)
        {
            let inside = under
                .as_ref()
                .is_some_and(|u| g.surfaces.iter().any(|s| &root(s) == u));
            if !inside {
                g.committed = false;
                g.surfaces.clear();
                g.resource.cleared();
            }
        }
    }
}
