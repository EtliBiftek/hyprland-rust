use smithay::{
    delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, PopupKind, PopupManager, Window,
    },
    input::{
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Logical, Point, Rectangle, Serial},
    wayland::shell::xdg::{
        PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    },
};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    Hypr,
};

impl XdgShellHandler for Hypr {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let window = Window::new_wayland_window(surface);
        self.add_window(window);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.remove_window(surface.wl_surface());
    }

    fn fullscreen_request(
        &mut self,
        surface: ToplevelSurface,
        _output: Option<smithay::reexports::wayland_server::protocol::wl_output::WlOutput>,
    ) {
        if let Some((idx, window)) = self.window_for_surface(surface.wl_surface()) {
            self.workspaces[idx].fullscreen = Some(window);
            if self.visible_ws().contains(&idx) {
                self.relayout();
            }
            self.ipc.emit("fullscreen", "1");
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some((idx, window)) = self.window_for_surface(surface.wl_surface()) {
            if self.workspaces[idx].fullscreen.as_ref() == Some(&window) {
                self.workspaces[idx].fullscreen = None;
                if self.visible_ws().contains(&idx) {
                    self.relayout();
                }
                self.ipc.emit("fullscreen", "0");
            }
        }
    }

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some((_, w)) = self.window_for_surface(surface.wl_surface()) {
            self.rules_on_title(&w);
        }
        self.on_title_changed(surface.wl_surface());
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();

        // Tiled windows are positioned by the layout; only floating ones can be dragged.
        let floating = self
            .window_for_surface(wl_surface)
            .is_some_and(|(idx, w)| self.workspaces[idx].is_floating(&w));
        if !floating {
            return;
        }

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            // Only a window that is on screen can be dragged; a client may
            // ask for one on a hidden workspace with another window's serial.
            let Some((window, initial_window_location)) = self.on_screen(wl_surface) else {
                return;
            };

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();

        let wl_surface = surface.wl_surface();

        // Tiled windows are positioned by the layout; only floating ones can be dragged.
        let floating = self
            .window_for_surface(wl_surface)
            .is_some_and(|(idx, w)| self.workspaces[idx].is_floating(&w));
        if !floating {
            return;
        }

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();

            // Only a window that is on screen can be dragged; a client may
            // ask for one on a hidden workspace with another window's serial.
            let Some((window, initial_window_location)) = self.on_screen(wl_surface) else {
                return;
            };
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });

            surface.send_pending_configure();

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );

            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // TODO popup grabs
    }
}

// Xdg Shell
delegate_xdg_shell!(Hypr);

fn check_grab(
    seat: &Seat<Hypr>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Hypr>> {
    let pointer = seat.get_pointer()?;

    // Check that this surface has a click grab.
    if !pointer.has_grab(serial) {
        return None;
    }

    let start_data = pointer.grab_start_data()?;

    let (focus, _) = start_data.focus.as_ref()?;
    // If the focus was for a different surface, ignore the request.
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }

    Some(start_data)
}

/// Should be called on `WlSurface::commit`
/// `toplevel` is the toplevel whose own surface is `surface`, on any
/// workspace (a window moved off-screen by a rule before its first configure
/// would otherwise never get one and never map).
pub fn handle_commit(
    popups: &mut PopupManager,
    toplevel: Option<&ToplevelSurface>,
    surface: &WlSurface,
) {
    if let Some(t) = toplevel {
        if !t.is_initial_configure_sent() {
            t.send_configure();
        }
    }

    // Handle popup commits.
    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    // NOTE: This should never fail as the initial configure is always
                    // allowed.
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Hypr {
    /// The mapped window of a toplevel surface and its location.
    fn on_screen(&self, surface: &WlSurface) -> Option<(Window, Point<i32, Logical>)> {
        let window = self
            .space
            .elements()
            .find(|w| w.toplevel().map(|t| t.wl_surface()) == Some(surface))?
            .clone();
        let loc = self.space.element_location(&window)?;
        Some((window, loc))
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &root))
        else {
            return;
        };

        let Some(output_geo) = self
            .space
            .outputs()
            .next()
            .and_then(|o| self.space.output_geometry(o))
        else {
            return;
        };
        let Some(window_geo) = self.space.element_geometry(window) else {
            return;
        };

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
