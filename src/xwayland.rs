//! XWayland: spawns Xwayland, runs the X11 window manager (smithay's `X11Wm`)
//! and maps X11 windows into the workspaces like xdg toplevels.
//!
//! Managed windows are `Window::new_x11_window` in a workspace (tiled, or
//! floating by `should_float`). Override-redirect windows (menus, tooltips)
//! are mapped straight into the `Space` at their own position, on top, and
//! live only in `XState::or_windows`: never tiled, no borders, no focus.

use std::{os::unix::io::OwnedFd, process::Stdio};

use smithay::{
    delegate_xwayland_shell,
    desktop::Window,
    reexports::{
        calloop::LoopHandle,
        wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::{
        selection::{
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
            SelectionSource, SelectionTarget,
        },
        seat::WaylandFocus,
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge, WmWindowProperty, WmWindowType, X11Window, XwmId},
        X11Surface, X11Wm, XWayland, XWaylandClientData, XWaylandEvent, XwmHandler,
    },
};

use crate::{CalloopData, Hypr};

pub struct XState {
    pub wm: Option<X11Wm>,
    pub shell: XWaylandShellState,
    /// Override-redirect windows currently mapped, bottom to top.
    pub or_windows: Vec<Window>,
    /// X display number once Xwayland was spawned.
    pub display: Option<u32>,
}

impl XState {
    pub fn new(dh: &DisplayHandle) -> Self {
        Self {
            wm: None,
            shell: XWaylandShellState::new::<Hypr>(dh),
            or_windows: Vec::new(),
            display: None,
        }
    }
}

/// Everything `should_float` looks at.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WinHints {
    pub ty: Option<WmWindowType>,
    pub transient: bool,
    pub popup: bool,
    /// `WM_NORMAL_HINTS` min size equals max size.
    pub fixed_size: bool,
}

/// Hyprland's heuristic: dialogs, transients, fixed-size windows and the
/// utility-like window types float; everything else tiles.
pub fn should_float(h: &WinHints) -> bool {
    use WmWindowType::*;
    h.transient
        || h.popup
        || h.fixed_size
        || matches!(
            h.ty,
            Some(
                Dialog
                    | Utility
                    | Splash
                    | Toolbar
                    | Menu
                    | DropdownMenu
                    | PopupMenu
                    | Tooltip
                    | Notification
            )
        )
}

fn hints_of(x: &X11Surface) -> WinHints {
    let fixed = match (x.min_size(), x.max_size()) {
        (Some(a), Some(b)) => a.w > 0 && a.h > 0 && a == b,
        _ => false,
    };
    WinHints {
        ty: x.window_type(),
        transient: x.is_transient_for().is_some(),
        popup: x.is_popup(),
        fixed_size: fixed,
    }
}

/// The `wl_surface` of a window of either kind (X11 ones only once associated).
pub fn wl_surface_of(w: &Window) -> Option<WlSurface> {
    w.wl_surface().map(|s| s.into_owned())
}

pub fn is_override_redirect(w: &Window) -> bool {
    w.x11_surface().is_some_and(|x| x.is_override_redirect())
}

/// Asks the client for `size`; X11 windows are also (re)positioned at `loc`.
pub fn request_size(w: &Window, loc: Point<i32, Logical>, size: Size<i32, Logical>) {
    if let Some(t) = w.toplevel() {
        t.with_pending_state(|s| s.size = Some(size));
        if t.is_initial_configure_sent() {
            t.send_pending_configure();
        }
    } else if let Some(x) = w.x11_surface() {
        let _ = x.configure(Rectangle::new(loc, size));
    }
}

/// Pid of the process behind the window (-1 if unknown).
pub fn x11_pid(x: &X11Surface) -> i32 {
    // `_NET_WM_PID` if set, else ask the X server (XRes).
    x.pid()
        .or_else(|| x.get_client_pid().ok())
        .map_or(-1, |p| p as i32)
}

fn same_x11(w: &Window, x: &X11Surface) -> bool {
    // Not `==`: a destroyed surface is never equal to anything.
    w.x11_surface().map(|s| s.window_id()) == Some(x.window_id())
}

impl Hypr {
    /// Spawns Xwayland (if `xwayland:enabled`) and exports `DISPLAY`. The X
    /// sockets are listening before Xwayland is ready, so clients started
    /// right away just wait for it.
    pub fn start_xwayland(&mut self, handle: &LoopHandle<'static, CalloopData>) {
        if !self.config.xwayland.enabled {
            return;
        }
        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            self.config.xwayland.create_abstract_socket,
            Stdio::null(),
            Stdio::null(),
            |_| {},
        );
        let (xwayland, client) = match spawned {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("cannot start Xwayland: {e}");
                return;
            }
        };
        let display = xwayland.display_number();
        self.x11.display = Some(display);
        std::env::set_var("DISPLAY", format!(":{display}"));
        tracing::info!("Xwayland on DISPLAY=:{display}");
        let h = handle.clone();
        let res = handle.insert_source(xwayland, move |event, _, data| match event {
            XWaylandEvent::Ready { x11_socket, .. } => {
                match X11Wm::start_wm(h.clone(), x11_socket, client.clone()) {
                    Ok(wm) => {
                        if let Some(d) = client.get_data::<XWaylandClientData>() {
                            d.user_data().insert_if_missing(|| wm.id());
                        }
                        data.state.x11.wm = Some(wm);
                        tracing::info!("X11 window manager started");
                    }
                    Err(e) => tracing::warn!("cannot start the X11 window manager: {e}"),
                }
            }
            XWaylandEvent::Error => tracing::warn!("Xwayland failed to start"),
        });
        if let Err(e) = res {
            tracing::warn!("cannot watch Xwayland: {e}");
        }
    }

    /// Makes X11 positions match the `Space` (moves, drags, rules and
    /// dispatchers only touch the `Space`). Runs once per loop iteration.
    pub fn x11_sync(&mut self) {
        for ws in &self.workspaces {
            for w in ws.windows() {
                let Some(x) = w.x11_surface() else { continue };
                let Some(loc) = self.space.element_location(&w) else {
                    continue;
                };
                let geo = x.geometry();
                if geo.loc != loc && x.is_mapped() {
                    let _ = x.configure(Rectangle::new(loc, geo.size));
                }
            }
        }
    }

    /// Keeps override-redirect windows above everything else.
    pub fn raise_override_redirect(&mut self) {
        for w in &self.x11.or_windows {
            self.space.raise_element(w, false);
        }
    }

    fn x11_window(&self, x: &X11Surface) -> Option<(usize, Window)> {
        self.workspaces.iter().enumerate().find_map(|(i, ws)| {
            ws.windows()
                .into_iter()
                .find(|w| same_x11(w, x))
                .map(|w| (i, w))
        })
    }

    fn drop_x11(&mut self, x: &X11Surface) {
        if let Some(p) = self.x11.or_windows.iter().position(|w| same_x11(w, x)) {
            let w = self.x11.or_windows.remove(p);
            self.space.unmap_elem(&w);
        } else if let Some((_, w)) = self.x11_window(x) {
            self.remove_window_obj(&w);
        }
    }

    fn x11_fullscreen(&mut self, x: &X11Surface, on: bool) {
        let Some((i, w)) = self.x11_window(x) else {
            return;
        };
        let ws = &mut self.workspaces[i];
        if on {
            ws.fullscreen = Some(w);
        } else if ws.fullscreen.as_ref() == Some(&w) {
            ws.fullscreen = None;
        } else {
            return;
        }
        let _ = x.set_fullscreen(on);
        if i == self.active_workspace {
            self.relayout();
        }
        self.ipc.emit("fullscreen", if on { "1" } else { "0" });
    }
}

impl XwmHandler for Hypr {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.x11.wm.as_mut().expect("X11 wm not started")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, x: X11Surface) {
        if x.set_mapped(true).is_err() {
            return;
        }
        let hints = hints_of(&x);
        let own = x.geometry().size;
        let window = Window::new_x11_window(x.clone());
        self.add_window(window.clone());
        if should_float(&hints) {
            self.set_floating(&window, Some(true));
            // Floating windows keep the size they asked for, centered
            // (add_window had configured them to a tile).
            if let (true, Some(out)) = (own.w > 1 && own.h > 1, self.output_geometry()) {
                let loc = out.loc + Point::from(((out.size.w - own.w) / 2, (out.size.h - own.h) / 2));
                let _ = x.configure(Rectangle::new(loc, own));
                self.space.map_element(window.clone(), loc, false);
            }
        }
        // Class and title are known already: apply the rules now (xdg windows
        // do it at their first commit).
        self.rules_on_commit(&window);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, x: X11Surface) {
        let loc = x.geometry().loc;
        let w = Window::new_x11_window(x);
        self.space.map_element(w.clone(), loc, true);
        self.x11.or_windows.push(w);
        self.raise_override_redirect();
    }

    fn unmapped_window(&mut self, _xwm: XwmId, x: X11Surface) {
        self.drop_x11(&x);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, x: X11Surface) {
        self.drop_x11(&x);
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        x: X11Surface,
        px: Option<i32>,
        py: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // Tiled windows keep the layout's geometry; the client just gets
        // told what it is.
        if let Some((i, win)) = self.x11_window(&x) {
            if !self.workspaces[i].is_floating(&win) {
                let _ = x.configure(None);
                return;
            }
        }
        let mut geo = x.geometry();
        if let Some(px) = px {
            geo.loc.x = px;
        }
        if let Some(py) = py {
            geo.loc.y = py;
        }
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        let _ = x.configure(geo);
        if let Some((_, win)) = self.x11_window(&x) {
            self.space.map_element(win, geo.loc, false);
        }
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        x: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<X11Window>,
    ) {
        // Override-redirect windows move and resize on their own.
        if let Some(w) = self.x11.or_windows.iter().find(|w| same_x11(w, &x)).cloned() {
            self.space.map_element(w, geometry.loc, false);
            self.raise_override_redirect();
        }
    }

    fn property_notify(&mut self, _xwm: XwmId, x: X11Surface, property: WmWindowProperty) {
        let Some((_, w)) = self.x11_window(&x) else {
            return;
        };
        match property {
            WmWindowProperty::Title => {
                self.rules_on_title(&w);
                self.on_title_changed_win(&w);
            }
            WmWindowProperty::Class => self.rules_on_commit(&w),
            WmWindowProperty::Hints => {
                if x.hints().is_some_and(|h| h.urgent) && self.focused_window().as_ref() != Some(&w)
                {
                    let addr = crate::ipc::window_addr(&w);
                    self.ipc.emit("urgent", addr.trim_start_matches("0x"));
                }
            }
            _ => {}
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, x: X11Surface) {
        if let Some((i, w)) = self.x11_window(&x) {
            self.workspaces[i].maximized = Some(w);
            let _ = x.set_maximized(true);
            if i == self.active_workspace {
                self.relayout();
            }
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, x: X11Surface) {
        if let Some((i, w)) = self.x11_window(&x) {
            if self.workspaces[i].maximized.as_ref() == Some(&w) {
                self.workspaces[i].maximized = None;
            }
            let _ = x.set_maximized(false);
            if i == self.active_workspace {
                self.relayout();
            }
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, x: X11Surface) {
        self.x11_fullscreen(&x, true);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, x: X11Surface) {
        self.x11_fullscreen(&x, false);
    }

    // minimize_request / unminimize_request: ignored (default).

    fn resize_request(&mut self, _xwm: XwmId, _x: X11Surface, _button: u32, _edge: ResizeEdge) {
        // shortcut: client-initiated move/resize of X11 windows is ignored,
        // use `bindm` (the compositor grabs work on X11 windows).
    }

    fn move_request(&mut self, _xwm: XwmId, _x: X11Surface, _button: u32) {}

    fn allow_selection_access(&mut self, _xwm: XwmId, _selection: SelectionTarget) -> bool {
        true
    }

    // An X client reads a selection owned by a Wayland client.
    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let r = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd).map_err(|e| e.to_string())
            }
            SelectionTarget::Primary => {
                request_primary_client_selection(&self.seat, mime_type, fd).map_err(|e| e.to_string())
            }
        };
        if let Err(e) = r {
            tracing::warn!("cannot send the Wayland selection to X11: {e}");
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        let (dh, seat) = (&self.display_handle, &self.seat);
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(dh, seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(dh, seat, mime_types, ()),
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        let (dh, seat) = (&self.display_handle, &self.seat);
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(seat).is_some() {
                    clear_data_device_selection(dh, seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(seat).is_some() {
                    clear_primary_selection(dh, seat);
                }
            }
        }
    }

    fn disconnected(&mut self, _xwm: XwmId) {
        tracing::warn!("lost the X11 connection");
        self.x11.wm = None;
    }
}

impl XWaylandShellHandler for Hypr {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.x11.shell
    }

    fn surface_associated(&mut self, _xwm: XwmId, _surface: WlSurface, x: X11Surface) {
        // The window may have been focused before it had a surface.
        if let Some((_, w)) = self.x11_window(&x) {
            if self.focused_window().as_ref() == Some(&w) {
                self.focus_window(Some(w));
            }
        }
    }
}

delegate_xwayland_shell!(Hypr);

impl Hypr {
    /// Wayland -> X11: forwards new Wayland selections to the X server.
    pub fn x11_new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>) {
        if let Some(wm) = self.x11.wm.as_mut() {
            if let Err(e) = wm.new_selection(ty, source.map(|s| s.mime_types())) {
                tracing::warn!("cannot set the X11 selection: {e}");
            }
        }
    }

    /// A Wayland client reads a selection owned by an X client.
    pub fn x11_send_selection(&mut self, ty: SelectionTarget, mime_type: String, fd: OwnedFd) {
        let handle = self.wm.loop_handle.clone();
        if let Some(wm) = self.x11.wm.as_mut() {
            if let Err(e) = wm.send_selection(ty, mime_type, fd, handle) {
                tracing::warn!("cannot send the selection to X11: {e}");
            }
        }
    }
}

// `X11Wm::start_wm` runs on the calloop data type, which wraps `Hypr`.
impl XwmHandler for CalloopData {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        self.state.xwm_state(xwm)
    }
    fn new_window(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.new_window(xwm, window)
    }
    fn new_override_redirect_window(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.new_override_redirect_window(xwm, window)
    }
    fn map_window_request(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.map_window_request(xwm, window)
    }
    fn mapped_override_redirect_window(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.mapped_override_redirect_window(xwm, window)
    }
    fn unmapped_window(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.unmapped_window(xwm, window)
    }
    fn destroyed_window(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.destroyed_window(xwm, window)
    }
    fn configure_request(
        &mut self,
        xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        reorder: Option<Reorder>,
    ) {
        self.state.configure_request(xwm, window, x, y, w, h, reorder)
    }
    fn configure_notify(
        &mut self,
        xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        above: Option<X11Window>,
    ) {
        self.state.configure_notify(xwm, window, geometry, above)
    }
    fn property_notify(&mut self, xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        self.state.property_notify(xwm, window, property)
    }
    fn maximize_request(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.maximize_request(xwm, window)
    }
    fn unmaximize_request(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.unmaximize_request(xwm, window)
    }
    fn fullscreen_request(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.fullscreen_request(xwm, window)
    }
    fn unfullscreen_request(&mut self, xwm: XwmId, window: X11Surface) {
        self.state.unfullscreen_request(xwm, window)
    }
    fn resize_request(&mut self, xwm: XwmId, window: X11Surface, button: u32, edge: ResizeEdge) {
        self.state.resize_request(xwm, window, button, edge)
    }
    fn move_request(&mut self, xwm: XwmId, window: X11Surface, button: u32) {
        self.state.move_request(xwm, window, button)
    }
    fn allow_selection_access(&mut self, xwm: XwmId, selection: SelectionTarget) -> bool {
        self.state.allow_selection_access(xwm, selection)
    }
    fn send_selection(
        &mut self,
        xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        XwmHandler::send_selection(&mut self.state, xwm, selection, mime_type, fd)
    }
    fn new_selection(&mut self, xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        XwmHandler::new_selection(&mut self.state, xwm, selection, mime_types)
    }
    fn cleared_selection(&mut self, xwm: XwmId, selection: SelectionTarget) {
        self.state.cleared_selection(xwm, selection)
    }
    fn disconnected(&mut self, xwm: XwmId) {
        XwmHandler::disconnected(&mut self.state, xwm)
    }
}

impl XWaylandShellHandler for CalloopData {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        self.state.xwayland_shell_state()
    }
    fn surface_associated(&mut self, xwm: XwmId, surface: WlSurface, window: X11Surface) {
        self.state.surface_associated(xwm, surface, window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristics() {
        assert!(!should_float(&WinHints::default()));
        assert!(!should_float(&WinHints {
            ty: Some(WmWindowType::Normal),
            ..Default::default()
        }));
        for ty in [
            WmWindowType::Dialog,
            WmWindowType::Utility,
            WmWindowType::Splash,
            WmWindowType::Toolbar,
            WmWindowType::Menu,
        ] {
            assert!(should_float(&WinHints {
                ty: Some(ty),
                ..Default::default()
            }));
        }
        assert!(should_float(&WinHints {
            transient: true,
            ..Default::default()
        }));
        assert!(should_float(&WinHints {
            fixed_size: true,
            ..Default::default()
        }));
    }
}
