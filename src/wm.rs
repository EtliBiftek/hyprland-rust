//! Window management: workspaces, tiling, focus and dispatchers.

use smithay::backend::input::KeyState;
use smithay::{
    desktop::{layer_map_for_output, Window},
    input::pointer::{Focus, GrabStartData as PointerGrabStartData, MotionEvent},
    reexports::{
        calloop::{LoopHandle, RegistrationToken},
        input,
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{protocol::wl_surface::WlSurface, Resource},
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use crate::{
    config::{Direction, Dispatcher, Mods},
    grabs::{resize_grab::ResizeEdge, MoveSurfaceGrab, ResizeSurfaceGrab},
    ipc::{class_title, window_addr},
    layout::{find_in_direction, Rect},
    selectors::{resolve_ws, Target, Vec2Arg, WinInfo, WinSel, WsInfo, WsSel},
    state::Hypr,
    tiling::LayoutKind,
    workspace::{new_named_id, new_special_id, Workspace, FIRST_NAMED_ID},
    CalloopData,
};

mod window_rules;

/// Window-management state that is not part of a workspace.
pub struct WmState {
    pub loop_handle: LoopHandle<'static, CalloopData>,
    /// Window with keyboard focus (if it is on a visible workspace).
    pub focused: Option<Window>,
    /// The window focused before `focused`, for `focuscurrentorlast`.
    pub prev_focus: Option<Window>,
    /// Id of the workspace we came from, for `workspace, previous`.
    pub previous_workspace: Option<i32>,
    /// Id of the special workspace shown on top of the active one.
    pub special_shown: Option<i32>,
    /// Pinned floating windows; they follow the active workspace.
    pub pinned: Vec<Window>,
    /// Active submap; empty is the global one.
    pub submap: String,
    /// Keycode and timer of the `binde` bind currently repeating.
    pub repeat: Option<(u32, RegistrationToken)>,
    /// Keycodes whose press was consumed by a bind (their release is too).
    pub consumed_keys: Vec<u32>,
    /// Modifiers in effect when each held key was pressed, so a release bind
    /// still matches if the modifier is let go first.
    pub key_mods: Vec<(u32, Mods)>,
    /// Keys forwarded by `pass`: the release goes to the same surface.
    pub passed_keys: Vec<(u32, WlSurface)>,
    /// libinput devices seen so far, to re-apply options on config reload.
    pub libinput_devices: Vec<input::Device>,
    /// `lockgroups`: no window may enter or leave groups.
    pub groups_locked: bool,
    /// `togglepseudo`: tiled windows that keep their own size.
    pub pseudo: Vec<Window>,
    /// `setprop`: (window, prop, value), applied over the rule effects by
    /// `window_dynp` (`rules::apply_setprops`).
    pub props: Vec<(Window, String, String)>,
    /// `dpms`: false while the output is switched off.
    pub dpms_on: bool,
    /// Monitors in the order they were added (see `monitors.rs`).
    pub mons: Vec<crate::monitors::Mon>,
    /// Index into `mons` of the focused monitor.
    pub focused_mon: usize,
    pub next_mon_id: u32,
    /// Globals of virtual outputs, removed by `hyprctl output remove`.
    pub output_globals: Vec<(
        String,
        smithay::reexports::wayland_server::backend::GlobalId,
    )>,
    /// Touchpad gesture in progress and whether a `gesture =` owns it.
    pub gesture: Option<(crate::gestures::Tracker, bool)>,
    /// Key holding a keyboard `bindm` grab.
    pub key_grab: Option<u32>,
    /// Toplevels that committed a null buffer (out of every workspace).
    pub unmapped: Vec<Window>,
    /// Windows whose client-side fullscreen state is forced on.
    pub client_fs: Vec<Window>,
    /// `o` bind waiting for its key to be held long enough.
    pub long_press: Option<(u32, RegistrationToken)>,
}

impl WmState {
    pub fn new(loop_handle: LoopHandle<'static, CalloopData>) -> Self {
        Self {
            loop_handle,
            focused: None,
            prev_focus: None,
            previous_workspace: None,
            special_shown: None,
            pinned: Vec::new(),
            submap: String::new(),
            repeat: None,
            consumed_keys: Vec::new(),
            key_mods: Vec::new(),
            passed_keys: Vec::new(),
            libinput_devices: Vec::new(),
            groups_locked: false,
            pseudo: Vec::new(),
            props: Vec::new(),
            dpms_on: true,
            mons: Vec::new(),
            focused_mon: 0,
            next_mon_id: 0,
            output_globals: Vec::new(),
            gesture: None,
            key_grab: None,
            unmapped: Vec::new(),
            client_fs: Vec::new(),
            long_press: None,
        }
    }
}

fn to_rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

fn from_rect(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

fn wl_surface(w: &Window) -> Option<WlSurface> {
    crate::xwayland::wl_surface_of(w)
}

fn special_name(n: &str) -> String {
    if n.is_empty() {
        "special".into()
    } else {
        format!("special:{n}")
    }
}

impl Hypr {
    /// Geometry of the focused output.
    pub fn output_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.focused_output()?;
        self.space.output_geometry(&output)
    }

    /// The focused output minus the exclusive zones of layer surfaces.
    pub fn usable_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.focused_output()?;
        let geo = self.space.output_geometry(&output)?;
        let zone = layer_map_for_output(&output).non_exclusive_zone();
        Some(Rectangle::new(geo.loc + zone.loc, zone.size))
    }

    /// The area tiled windows of the focused monitor may use: the usable area
    /// minus the outer gaps. Window rects are later shrunk by `gaps_in`, so
    /// subtract that here to make the distance to the screen edge exactly
    /// `gaps_out`.
    pub fn tiling_area(&self) -> Rect {
        let g = &self.config.general;
        let out = self
            .usable_geometry()
            .map(to_rect)
            .unwrap_or(Rect::new(0, 0, 1280, 800));
        out.shrink((g.gaps_out - g.gaps_in).max(0))
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<(usize, Window)> {
        self.workspaces.iter().enumerate().find_map(|(i, ws)| {
            ws.windows()
                .into_iter()
                .find(|w| wl_surface(w).as_ref() == Some(surface))
                .map(|w| (i, w))
        })
    }

    // ------------------------------------------------------------ workspaces

    pub fn ws_index(&self, id: i32) -> Option<usize> {
        self.workspaces.iter().position(|w| w.id == id)
    }

    pub fn active_id(&self) -> i32 {
        self.workspaces[self.active_workspace].id
    }

    /// Index of the shown special workspace, if any.
    pub(crate) fn special_index(&self) -> Option<usize> {
        self.wm.special_shown.and_then(|id| self.ws_index(id))
    }

    /// Workspace that receives new windows and focus: the special workspace
    /// while one is shown, else the active one.
    pub fn focus_ws(&self) -> usize {
        self.special_index().unwrap_or(self.active_workspace)
    }

    /// Indices of the workspaces whose windows are on screen.
    pub(crate) fn visible_ws(&self) -> Vec<usize> {
        let mut v = self.shown_ws();
        if !v.contains(&self.active_workspace) {
            v.insert(0, self.active_workspace);
        }
        v.extend(self.special_index());
        v
    }

    pub(crate) fn ws_of(&self, w: &Window) -> Option<usize> {
        self.workspaces.iter().position(|ws| ws.contains(w))
    }

    fn mon_name(&self) -> String {
        self.monitor_name()
    }

    fn ws_infos(&self) -> Vec<WsInfo> {
        self.workspaces
            .iter()
            .filter(|w| w.id >= 1)
            .map(|w| WsInfo {
                id: w.id,
                windows: w.windows().len(),
            })
            .collect()
    }

    fn create_ws(&mut self, id: i32, name: String) -> usize {
        self.ipc.emit("createworkspace", &name);
        self.ipc.emit("createworkspacev2", &format!("{id},{name}"));
        // A `workspace = N, monitor:X` rule places it, else the focused monitor.
        let ruled = self
            .config
            .workspace_rules
            .iter()
            .find(|r| r.workspace == name || r.workspace == id.to_string())
            .and_then(|r| r.monitor.clone())
            .filter(|m| self.mon_index(m).is_some());
        let mut ws = Workspace::new(id, name);
        ws.monitor = ruled.unwrap_or_else(|| self.focused_mon_name());
        self.workspaces.push(ws);
        self.workspaces.len() - 1
    }

    /// Finds or creates the workspace a selector resolved to.
    pub(crate) fn ensure_ws(&mut self, t: &Target) -> Option<usize> {
        match t {
            Target::Id(n) => match self.ws_index(*n) {
                Some(i) => Some(i),
                None if *n >= 1 => Some(self.create_ws(*n, n.to_string())),
                None => None,
            },
            Target::Name(name) => match self.workspaces.iter().position(|w| &w.name == name) {
                Some(i) => Some(i),
                None => {
                    let ids = self.workspaces.iter().map(|w| w.id);
                    let id = new_named_id(ids.filter(|&i| i <= FIRST_NAMED_ID));
                    Some(self.create_ws(id, name.clone()))
                }
            },
            Target::Special(n) => {
                let name = special_name(n);
                match self.workspaces.iter().position(|w| w.name == name) {
                    Some(i) => Some(i),
                    None => {
                        let ids = self
                            .workspaces
                            .iter()
                            .filter(|w| w.is_special())
                            .map(|w| w.id);
                        let id = new_special_id(n, ids);
                        Some(self.create_ws(id, name))
                    }
                }
            }
        }
    }

    /// Destroys empty workspaces that are neither active nor shown.
    pub fn gc_workspaces(&mut self) {
        let shown: Vec<i32> = self.wm.mons.iter().map(|m| m.ws).collect();
        let mut i = 0;
        while i < self.workspaces.len() {
            let ws = &self.workspaces[i];
            let keep = i == self.active_workspace
                || shown.contains(&ws.id)
                || Some(ws.id) == self.wm.special_shown
                || !ws.windows().is_empty();
            if keep {
                i += 1;
                continue;
            }
            let (id, name) = (ws.id, ws.name.clone());
            self.workspaces.remove(i);
            if i < self.active_workspace {
                self.active_workspace -= 1;
            }
            self.ipc.emit("destroyworkspace", &name);
            self.ipc.emit("destroyworkspacev2", &format!("{id},{name}"));
        }
    }

    /// `workspace, <selector>`.
    pub fn select_workspace(&mut self, sel: &WsSel) {
        let mut infos = self.ws_infos();
        // `m+1`: only the workspaces of the focused monitor count.
        if matches!(sel, WsSel::OnMonitor(_)) {
            let mon = self.focused_mon_name();
            infos.retain(|i| {
                self.ws_index(i.id)
                    .is_some_and(|x| self.ws_monitor(x) == mon)
            });
        }
        let Some(target) = resolve_ws(sel, self.active_id(), self.wm.previous_workspace, &infos)
        else {
            return;
        };
        match &target {
            Target::Special(n) => self.toggle_special(n),
            t => {
                if let Some(i) = self.ensure_ws(t) {
                    self.switch_to_index(i);
                }
            }
        }
    }

    fn hide_special(&mut self) {
        let Some(id) = self.wm.special_shown.take() else {
            return;
        };
        if let Some(i) = self.ws_index(id) {
            for w in self.workspaces[i].windows() {
                self.space.unmap_elem(&w);
            }
        }
        let mon = self.mon_name();
        self.ipc.emit("activespecial", &format!(",{mon}"));
        self.ipc.emit("activespecialv2", &format!(",,{mon}"));
    }

    /// `togglespecialworkspace`: shows the special workspace on top of the
    /// active one, or hides it if it is the one shown.
    pub fn toggle_special(&mut self, name: &str) {
        let full = special_name(name);
        let shown = self
            .special_index()
            .map(|i| self.workspaces[i].name.clone());
        let hide_only = shown.as_deref() == Some(full.as_str());
        self.hide_special();
        if !hide_only {
            let Some(i) = self.ensure_ws(&Target::Special(name.to_string())) else {
                return;
            };
            self.wm.special_shown = Some(self.workspaces[i].id);
            // Shown on the focused monitor.
            self.workspaces[i].monitor = self.focused_mon_name();
            let mon = self.mon_name();
            self.ipc.emit("activespecial", &format!("{full},{mon}"));
            let id = self.workspaces[i].id;
            self.ipc
                .emit("activespecialv2", &format!("{id},{full},{mon}"));
        }
        self.relayout();
        let f = self.focus_ws();
        let next = self.workspaces[f].last_focused.clone();
        self.focus_window(next);
        self.gc_workspaces();
    }

    /// Makes workspace `idx` the active one.
    pub fn switch_to_index(&mut self, idx: usize) {
        let had_special = self.wm.special_shown.is_some();
        self.hide_special();
        if idx == self.active_workspace {
            if had_special {
                self.relayout();
                let next = self.workspaces[idx].last_focused.clone();
                self.focus_window(next);
                self.gc_workspaces();
            }
            return;
        }
        let old = self.active_workspace;
        self.wm.previous_workspace = Some(self.workspaces[old].id);
        let cur = self.wm.focused_mon;
        match self.mon_showing(idx) {
            // Shown on another monitor: focus moves there.
            Some(m) => {
                self.set_focused_mon(m);
                self.warp_to_mon(m);
            }
            None if self.wm.mons.is_empty() => {
                for w in self.workspaces[old].windows() {
                    self.space.unmap_elem(&w);
                }
                self.active_workspace = idx;
            }
            None => {
                // A workspace bound to another monitor opens there.
                let m = self.mon_index(&self.workspaces[idx].monitor).unwrap_or(cur);
                self.show_ws_on(m, idx);
                self.set_focused_mon(m);
                if m != cur {
                    self.warp_to_mon(m);
                }
            }
        }
        let (id, name) = (self.workspaces[idx].id, self.workspaces[idx].name.clone());
        self.ipc.emit("workspace", &name);
        self.ipc.emit("workspacev2", &format!("{id},{name}"));
        self.relayout();
        let focus = self.workspaces[idx].last_focused.clone();
        self.focus_window(focus);
        self.gc_workspaces();
    }

    // ---------------------------------------------------------------- windows

    pub fn focused_window(&self) -> Option<Window> {
        let f = self.wm.focused.clone()?;
        self.visible_ws()
            .into_iter()
            .any(|i| self.workspaces[i].contains(&f))
            .then_some(f)
    }

    /// Called when a client creates a new toplevel window.
    pub fn add_window(&mut self, window: Window) {
        let idx = self.focus_ws();
        tracing::info!("window mapped on workspace {}", self.workspaces[idx].name);
        self.sync_layouts();
        if self.try_auto_group(idx, &window) {
            self.relayout();
            self.focus_window(Some(window));
            return;
        }
        let area = self.tiling_area();
        let cursor = self.seat.get_pointer().map(|p| p.current_location());
        let focused = self
            .focused_window()
            .filter(|w| self.workspaces[idx].contains(w));
        // `use_active_for_splits = false`: split the window under the pointer.
        let under = cursor
            .filter(|_| !self.config.dwindle.use_active_for_splits)
            .and_then(|p| self.window_under(p))
            .filter(|w| self.workspaces[idx].contains(w) && !self.workspaces[idx].is_floating(w));
        let target = under.or(focused);
        let ws = &mut self.workspaces[idx];
        ws.tiled.dwindle.cursor = cursor.map(|p| (p.x as i32, p.y as i32));
        ws.tiled.insert(window.clone(), target.as_ref(), area);
        ws.tiled.dwindle.cursor = None;
        self.relayout();
        self.focus_window(Some(window));
    }

    /// Applies layout choice (`general:layout`, workspace rules) and options
    /// to every workspace. Switching layout keeps the windows.
    pub fn sync_layouts(&mut self) {
        let area = self.tiling_area();
        for ws in &mut self.workspaces {
            ws.tiled.configure(&self.config);
            let rule = self
                .config
                .workspace_rules
                .iter()
                .find(|r| r.workspace == ws.name);
            let kind = LayoutKind::parse(
                rule.and_then(|r| r.layout.as_deref())
                    .unwrap_or(&self.config.general.layout),
            );
            ws.tiled.set_kind(kind, area);
            ws.tiled.master.rule_orientation = rule.and_then(|r| r.orientation);
        }
    }

    /// Called when a client destroys a toplevel window.
    pub fn remove_window(&mut self, surface: &WlSurface) {
        self.wm
            .unmapped
            .retain(|w| w.toplevel().is_none_or(|t| t.wl_surface() != surface));
        if let Some((_, window)) = self.window_for_surface(surface) {
            self.remove_window_obj(&window);
        }
    }

    /// Like `remove_window`, for windows without a surface (X11).
    pub fn remove_window_obj(&mut self, window: &Window) {
        let Some(idx) = self.ws_of(window) else {
            return;
        };
        let window = window.clone();
        if crate::ipc::is_mapped(&window) {
            self.ipc.emit(
                "closewindow",
                crate::ipc::window_addr(&window).trim_start_matches("0x"),
            );
        }
        self.workspaces[idx].remove(&window);
        self.wm.pinned.retain(|w| w != &window);
        self.space.unmap_elem(&window);
        if self.visible_ws().contains(&idx) {
            // An emptied special workspace closes itself.
            if self.workspaces[idx].is_special() && self.workspaces[idx].windows().is_empty() {
                self.hide_special();
            }
            self.relayout();
            let f = self.focus_ws();
            let ws = &self.workspaces[f];
            let master = if ws.tiled.kind == LayoutKind::Master
                && self.config.master.focus_master_on_close
            {
                ws.tiled.master.master_window().cloned()
            } else {
                None
            };
            let next = master.or_else(|| ws.last_focused.clone());
            self.focus_window(next);
        }
        self.gc_workspaces();
    }

    /// Positions every visible window and sends configures.
    pub fn relayout(&mut self) {
        self.sync_layouts();
        let open: Vec<Window> = self.workspaces.iter().flat_map(|ws| ws.windows()).collect();
        self.wm.pseudo.retain(|p| open.contains(p));
        for i in self.visible_ws() {
            self.layout_ws(i);
        }
        self.raise_override_redirect();
    }

    fn layout_ws(&mut self, idx: usize) {
        let mon = self.ws_monitor(idx);
        let area = self.tiling_area_of(&mon);
        let gaps_in = self.config.general.gaps_in;
        let output_geo = self.mon_geometry(&mon).or_else(|| self.output_geometry());
        let ws = &mut self.workspaces[idx];
        let is_special = ws.is_special();
        // `special_scale_factor` shrinks tiled windows of special workspaces
        // towards the center of the tiling area.
        let scale = match ws.tiled.kind {
            LayoutKind::Dwindle => self.config.dwindle.special_scale_factor,
            LayoutKind::Master => self.config.master.special_scale_factor,
        };
        let (cx, cy) = area.center();
        let scaled = |r: Rect| {
            if !is_special || scale >= 1.0 {
                return r;
            }
            let f = |v: i32, c: i32| c + ((v - c) as f32 * scale).round() as i32;
            let (x0, y0) = (f(r.x, cx), f(r.y, cy));
            Rect::new(x0, y0, f(r.x + r.w, cx) - x0, f(r.y + r.h, cy) - y0)
        };
        let pseudo = self.wm.pseudo.clone();

        let mut placements: Vec<(Window, Rectangle<i32, Logical>, bool)> = ws
            .tiled
            .layout(area, gaps_in)
            .into_iter()
            .map(|(w, r)| {
                let mut r = scaled(r);
                // Pseudotiled: keep the client's own size, centered in its slot.
                if pseudo.contains(&w) {
                    if let Some(g) = self.space.element_geometry(&w) {
                        let (x, y) = (r.x + (r.w - g.size.w) / 2, r.y + (r.h - g.size.h) / 2);
                        r = Rect::new(x, y, g.size.w, g.size.h);
                    }
                }
                (w, from_rect(r), false)
            })
            .collect();

        for w in &ws.floating {
            let geo = self.space.element_geometry(w);
            let rect = match geo {
                Some(g) => g,
                None => {
                    // First time floating: center at 60% of the output (X11
                    // windows keep the size they asked for).
                    let out = output_geo.unwrap_or(Rectangle::from_size((1280, 800).into()));
                    let own = w
                        .x11_surface()
                        .map(|x| x.geometry().size)
                        .filter(|s| s.w > 0 && s.h > 0);
                    let size = own.map_or((out.size.w * 3 / 5, out.size.h * 3 / 5), |s| (s.w, s.h));
                    let loc = (
                        out.loc.x + (out.size.w - size.0) / 2,
                        out.loc.y + (out.size.h - size.1) / 2,
                    );
                    Rectangle::new(loc.into(), size.into())
                }
            };
            placements.push((w.clone(), rect, false));
        }

        if let Some(mx) = ws.maximized.clone() {
            for p in placements.iter_mut() {
                if p.0 == mx {
                    p.1 = from_rect(area.shrink(gaps_in));
                }
            }
        }

        if let (Some(fs), Some(out)) = (ws.fullscreen.clone(), output_geo) {
            for p in placements.iter_mut() {
                if p.0 == fs {
                    *p = (fs.clone(), out, true);
                }
            }
        }

        // Hidden group members get the size of the visible one but stay unmapped.
        let mut hidden = Vec::new();
        for g in &ws.groups.list {
            let active = g.active();
            let Some(p) = placements.iter().find(|p| &p.0 == active).cloned() else {
                continue;
            };
            for m in g.members.iter().filter(|m| *m != active) {
                placements.push((m.clone(), p.1, p.2));
                hidden.push(m.clone());
            }
        }

        let fullscreen = ws.fullscreen.clone();
        let maximized = ws.maximized.clone();
        let floating = ws.floating.clone();
        let all = ws.windows();
        for (window, rect, is_fullscreen) in placements {
            if let Some(toplevel) = window.toplevel() {
                let is_floating = floating.contains(&window);
                // `fullscreenstate _ 2`: the client is told it is fullscreen.
                let client_fs = self.wm.client_fs.contains(&window);
                // Not inside `with_pending_state`: it holds the surface lock that
                // `element_geometry` takes (deadlock otherwise).
                let keep_size = (is_floating || pseudo.contains(&window))
                    && self.space.element_geometry(&window).is_some();
                toplevel.with_pending_state(|state| {
                    // Floating windows pick their own size after the first configure.
                    if !keep_size {
                        state.size = Some(rect.size);
                    }
                    if is_fullscreen || client_fs {
                        state.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        state.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                    for s in [
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        if is_floating {
                            state.states.unset(s);
                        } else {
                            state.states.set(s);
                        }
                    }
                });
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            } else if let Some(x) = window.x11_surface() {
                let is_floating = floating.contains(&window);
                let keep_size = (is_floating || pseudo.contains(&window))
                    && self.space.element_geometry(&window).is_some();
                let size = if keep_size {
                    x.geometry().size
                } else {
                    rect.size
                };
                let _ = x.set_fullscreen(is_fullscreen);
                let _ = x.configure(Rectangle::new(rect.loc, size));
            }
            // `Space` positions the window's geometry (it applies the client-side
            // decoration offset, e.g. shadows, itself).
            if hidden.contains(&window) {
                self.space.unmap_elem(&window);
            } else {
                self.space.map_element(window.clone(), rect.loc, false);
            }
        }

        // A special workspace is drawn above the regular one.
        if is_special {
            for w in &all {
                self.space.raise_element(w, false);
            }
        }
        // Keep floating windows above tiled ones, fullscreen above everything.
        for w in floating
            .iter()
            .chain(maximized.iter())
            .chain(fullscreen.iter())
        {
            self.space.raise_element(w, false);
        }
    }

    /// Gives keyboard focus to `window` (or clears focus) and raises it.
    pub fn focus_window(&mut self, window: Option<Window>) {
        // Locked: keyboard focus stays on the lock surface.
        if self.session_locked {
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();

        // Focusing a hidden group member brings its tab to the front.
        if let Some(w) = &window {
            if self.group_activate(w) {
                self.relayout();
            }
        }
        // Focusing a window on another monitor focuses that monitor.
        if let Some(m) = window
            .as_ref()
            .and_then(|w| self.ws_of(w))
            .and_then(|i| self.mon_showing(i))
        {
            if m != self.wm.focused_mon {
                self.set_focused_mon(m);
            }
        }
        if let Some(w) = &window {
            if let Some(i) = self.ws_of(w) {
                let ws = &mut self.workspaces[i];
                ws.last_focused = Some(w.clone());
                if ws.is_floating(w) {
                    // Move to top of the floating stack.
                    ws.floating.retain(|f| f != w);
                    ws.floating.push(w.clone());
                    self.space.raise_element(w, true);
                }
            }
        }
        let old = self.focused_window();
        if old != window && old.is_some() {
            self.wm.prev_focus = old;
        }
        self.wm.focused = window.clone();
        for w in self.space.elements() {
            if crate::xwayland::is_override_redirect(w) {
                continue;
            }
            w.set_activated(Some(w) == window.as_ref());
            if let Some(t) = w.toplevel() {
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
        }
        // An exclusive-keyboard layer surface (launcher, lock screen) keeps focus.
        self.layer_focus = self.exclusive_layer();
        let target = self
            .layer_focus
            .clone()
            .or_else(|| window.as_ref().and_then(wl_surface));
        keyboard.set_focus(self, target, serial);
        self.ipc.emit_active(window.as_ref());
        self.refresh_rules_on_focus();
    }

    /// `movetoworkspace` (follows the window) and `movetoworkspacesilent`.
    pub fn move_to_workspace(&mut self, sel: &WsSel, silent: bool) {
        let Some(window) = self.focused_window() else {
            return;
        };
        let Some(src) = self.ws_of(&window) else {
            return;
        };
        let infos = self.ws_infos();
        let Some(target) = resolve_ws(sel, self.active_id(), self.wm.previous_workspace, &infos)
        else {
            return;
        };
        let Some(dst) = self.ensure_ws(&target) else {
            return;
        };
        if dst == src {
            return;
        }
        self.sync_layouts();
        let area = self.tiling_area();
        let was_floating = self.workspaces[src].is_floating(&window);
        // A grouped window takes its whole group along.
        let ws_src = &mut self.workspaces[src];
        let group = ws_src
            .groups
            .find(&window)
            .map(|g| ws_src.groups.list.remove(g));
        ws_src.remove(&window);
        for m in group.iter().flat_map(|g| g.members.iter()) {
            self.space.unmap_elem(m);
        }
        self.space.unmap_elem(&window);

        let (id, name) = (self.workspaces[dst].id, self.workspaces[dst].name.clone());
        let addr = window_addr(&window);
        let addr = addr.trim_start_matches("0x");
        self.ipc.emit("movewindow", &format!("{addr},{name}"));
        self.ipc
            .emit("movewindowv2", &format!("{addr},{id},{name}"));
        let target_ws = &mut self.workspaces[dst];
        if was_floating {
            target_ws.floating.push(window.clone());
        } else {
            let anchor = target_ws.last_focused.clone();
            target_ws
                .tiled
                .insert(window.clone(), anchor.as_ref(), area);
        }
        target_ws.last_focused = Some(window);
        target_ws.groups.list.extend(group);

        self.relayout();
        let f = self.focus_ws();
        let next = self.workspaces[f].last_focused.clone();
        self.focus_window(next);
        if !silent && !matches!(target, Target::Special(_)) {
            self.switch_to_index(dst);
        }
        self.gc_workspaces();
    }

    /// Sets (`Some`) or toggles (`None`) the floating state of `window`.
    pub(crate) fn set_floating(&mut self, window: &Window, want: Option<bool>) {
        let Some(i) = self.ws_of(window) else {
            return;
        };
        self.sync_layouts();
        let area = self.tiling_area();
        let ws = &mut self.workspaces[i];
        let floating = ws.is_floating(window);
        if want.unwrap_or(!floating) == floating {
            return;
        }
        if floating {
            ws.floating.retain(|w| w != window);
            ws.tiled.insert(window.clone(), None, area);
            self.wm.pinned.retain(|w| w != window);
        } else {
            ws.tiled.remove(window);
            ws.floating.push(window.clone());
            // Forget the tiled position so relayout centers it.
            self.space.unmap_elem(window);
        }
        self.relayout();
        let floating = self.workspaces[i].is_floating(window);
        let addr = window_addr(window);
        self.ipc.emit(
            "changefloatingmode",
            &format!("{},{}", addr.trim_start_matches("0x"), floating as u8),
        );
        self.focus_window(Some(window.clone()));
    }

    pub fn toggle_floating(&mut self) {
        if let Some(w) = self.focused_window() {
            self.set_floating(&w, None);
        }
    }

    fn toggle_fullscreen(&mut self, maximize: bool) {
        let Some(window) = self.focused_window() else {
            return;
        };
        let Some(i) = self.ws_of(&window) else {
            return;
        };
        let ws = &mut self.workspaces[i];
        let slot = if maximize {
            &mut ws.maximized
        } else {
            &mut ws.fullscreen
        };
        *slot = if slot.as_ref() == Some(&window) {
            None
        } else {
            Some(window)
        };
        self.relayout();
        if !maximize {
            let fs = self.workspaces[i].fullscreen.is_some();
            self.ipc.emit("fullscreen", &(fs as u8).to_string());
        }
    }

    /// `fullscreenstate INTERNAL CLIENT [toggle]`: internal 0 none,
    /// 1 maximized, 2/3 fullscreen; client 2/3 tells the client it is
    /// fullscreen without changing its size; -1 keeps the current state.
    fn fullscreen_state(&mut self, internal: i8, client: i8, toggle: bool) {
        let Some(window) = self.focused_window() else {
            return;
        };
        let Some(i) = self.ws_of(&window) else {
            return;
        };
        let ws = &mut self.workspaces[i];
        let cur = if ws.fullscreen.as_ref() == Some(&window) {
            2
        } else if ws.maximized.as_ref() == Some(&window) {
            1
        } else {
            0
        };
        let mut want = if internal < 0 { cur } else { internal.min(2) };
        if toggle && want == cur && internal >= 0 {
            want = 0;
        }
        ws.fullscreen = if want == 2 {
            Some(window.clone())
        } else {
            ws.fullscreen.take().filter(|w| w != &window)
        };
        ws.maximized = if want == 1 {
            Some(window.clone())
        } else {
            ws.maximized.take().filter(|w| w != &window)
        };
        if client >= 0 {
            let on = client >= 2 && !(toggle && self.wm.client_fs.contains(&window));
            self.wm.client_fs.retain(|w| w != &window);
            if on {
                self.wm.client_fs.push(window.clone());
            }
        }
        self.relayout();
        if want != cur {
            self.ipc
                .emit("fullscreen", &((want == 2) as u8).to_string());
        }
    }

    /// `sendshortcut` / `sendkeystate`: key events (with modifier keys) to a
    /// window, without changing the keyboard focus for good.
    fn send_key(&mut self, mods: Mods, key: &str, sel: Option<&WinSel>, states: &[&str]) {
        let Some(window) = self.target_window(sel) else {
            return;
        };
        let Some(surface) = crate::xwayland::wl_surface_of(&window) else {
            return;
        };
        let Some(code) = self.keycode_for(key) else {
            tracing::warn!("sendshortcut: no key `{key}` in the keymap");
            return;
        };
        // evdev + 8: Super_L, Shift_L, Control_L, Alt_L.
        let mod_codes: Vec<u32> = [
            (mods.logo, 133),
            (mods.shift, 50),
            (mods.ctrl, 37),
            (mods.alt, 64),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, c)| *c)
        .collect();
        let kb = self.seat.get_keyboard().unwrap();
        let prev = kb.current_focus();
        let time = self.start_time.elapsed().as_millis() as u32;
        kb.set_focus(self, Some(surface), SERIAL_COUNTER.next_serial());
        let send = |h: &mut Hypr, c: u32, s: KeyState| {
            kb.input_forward(h, c.into(), s, SERIAL_COUNTER.next_serial(), time, true);
        };
        for st in states {
            match *st {
                "down" | "repeat" => {
                    for c in &mod_codes {
                        send(self, *c, KeyState::Pressed);
                    }
                    send(self, code, KeyState::Pressed);
                }
                _ => {
                    send(self, code, KeyState::Released);
                    for c in mod_codes.iter().rev() {
                        send(self, *c, KeyState::Released);
                    }
                }
            }
        }
        kb.set_focus(self, prev, SERIAL_COUNTER.next_serial());
    }

    /// Keycode (xkb, evdev + 8) that produces the keysym `name` at level 0.
    fn keycode_for(&mut self, name: &str) -> Option<u32> {
        use smithay::input::keyboard::xkb;
        if let Some(n) = name.strip_prefix("code:") {
            return n.parse().ok();
        }
        let sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE);
        if sym.raw() == 0 {
            return None;
        }
        // The configured layout, compiled again (the seat's keymap is not
        // reachable from here).
        let i = &self.config.input;
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &ctx,
            &i.kb_rules,
            &i.kb_model,
            &i.kb_layout,
            &i.kb_variant,
            (!i.kb_options.is_empty()).then(|| i.kb_options.clone()),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )?;
        (keymap.min_keycode().raw()..=keymap.max_keycode().raw())
            .find(|&kc| keymap.key_get_syms_by_level(kc.into(), 0, 0).contains(&sym))
    }

    /// Rects of the tiled windows of the workspace holding `w`, plus its index.
    pub(crate) fn tiled_rects(&self, w: &Window) -> Option<(usize, Vec<(Window, Rect)>)> {
        let i = self.ws_of(w)?;
        let ws = &self.workspaces[i];
        (!ws.is_floating(w)).then(|| (i, ws.tiled.layout(self.tiling_area(), 0)))
    }

    fn move_focus(&mut self, dir: Direction) {
        let Some(from) = self.focused_window() else {
            return;
        };
        let Some((_, rects)) = self.tiled_rects(&from) else {
            return;
        };
        if let Some(to) = find_in_direction(&rects, &from, dir) {
            self.focus_window(Some(to));
        }
    }

    /// `movewindow` / `swapwindow`: swap with the tiled neighbour in `dir`.
    // shortcut: movewindow swaps instead of re-parenting in the tree like
    // Hyprland; implement tree moves when master layout/groups land.
    pub(crate) fn swap_dir(&mut self, dir: Direction) {
        let Some(from) = self.focused_window() else {
            return;
        };
        let Some((i, rects)) = self.tiled_rects(&from) else {
            return;
        };
        if let Some(to) = find_in_direction(&rects, &from, dir) {
            self.workspaces[i].tiled.swap(&from, &to);
            self.relayout();
        }
    }

    fn swap_next(&mut self, prev: bool) {
        let Some(from) = self.focused_window() else {
            return;
        };
        let Some(i) = self.ws_of(&from) else {
            return;
        };
        let wins = self.workspaces[i].tiled.windows();
        let Some(pos) = wins.iter().position(|w| w == &from) else {
            return;
        };
        let n = wins.len();
        let to = wins[(pos + if prev { n - 1 } else { 1 }) % n].clone();
        self.workspaces[i].tiled.swap(&from, &to);
        self.relayout();
    }

    fn cycle_next(&mut self) {
        let windows = self.workspaces[self.focus_ws()].visible_windows();
        if windows.is_empty() {
            return;
        }
        let next = match self
            .focused_window()
            .and_then(|f| windows.iter().position(|w| *w == f))
        {
            Some(i) => windows[(i + 1) % windows.len()].clone(),
            None => windows[0].clone(),
        };
        self.focus_window(Some(next));
    }

    fn out_size(&self) -> (i32, i32) {
        self.output_geometry()
            .map_or((1280, 800), |g| (g.size.w, g.size.h))
    }

    fn resize_active(&mut self, arg: &Vec2Arg) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some(i) = self.ws_of(&w) else {
            return;
        };
        let size = w.geometry().size;
        let cur = (size.w, size.h);
        if self.workspaces[i].is_floating(&w) {
            let (nw, nh) = arg.resolve(cur, cur, self.out_size());
            let loc = self.space.element_location(&w).unwrap_or((0, 0).into());
            crate::xwayland::request_size(&w, loc, (nw.max(20), nh.max(20)).into());
        } else if !arg.exact {
            // shortcut: tiled windows resize by moving their split edge; exact
            // sizes are ignored for them.
            let (dx, dy) = arg.delta(cur);
            let area = self.tiling_area();
            if self.workspaces[i].tiled.resize(&w, dx, dy, area) {
                self.relayout();
            }
        }
    }

    fn move_active(&mut self, arg: &Vec2Arg) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let floating = self
            .ws_of(&w)
            .is_some_and(|i| self.workspaces[i].is_floating(&w));
        let Some(loc) = self.space.element_location(&w).filter(|_| floating) else {
            return;
        };
        let out = self.out_size();
        let (x, y) = arg.resolve((loc.x, loc.y), out, out);
        self.space.map_element(w, (x, y), false);
    }

    fn center_window(&mut self) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let floating = self
            .ws_of(&w)
            .is_some_and(|i| self.workspaces[i].is_floating(&w));
        let Some(out) = self.output_geometry().filter(|_| floating) else {
            return;
        };
        let size = w.geometry().size;
        let loc = (
            out.loc.x + (out.size.w - size.w) / 2,
            out.loc.y + (out.size.h - size.h) / 2,
        );
        self.space.map_element(w, loc, false);
    }

    fn pin_active(&mut self) {
        let Some(w) = self.focused_window() else {
            return;
        };
        // Only floating windows can be pinned.
        if !self
            .ws_of(&w)
            .is_some_and(|i| self.workspaces[i].is_floating(&w))
        {
            return;
        }
        let pinned = if self.wm.pinned.contains(&w) {
            self.wm.pinned.retain(|p| p != &w);
            false
        } else {
            self.wm.pinned.push(w.clone());
            true
        };
        let addr = window_addr(&w);
        self.ipc.emit(
            "pin",
            &format!("{},{}", addr.trim_start_matches("0x"), pinned as u8),
        );
    }

    pub fn window_pid(&self, w: &Window) -> i32 {
        if let Some(x) = w.x11_surface() {
            return crate::xwayland::x11_pid(x);
        }
        w.toplevel()
            .and_then(|t| t.wl_surface().client())
            .and_then(|c| c.get_credentials(&self.display_handle).ok())
            .map_or(-1, |c| c.pid)
    }

    /// First window (on any workspace) matching a selector.
    pub fn find_window(&self, sel: &WinSel) -> Option<(usize, Window)> {
        self.workspaces.iter().enumerate().find_map(|(i, ws)| {
            ws.windows().into_iter().find_map(|w| {
                let (class, title) = class_title(&w);
                let info = WinInfo {
                    class,
                    title,
                    address: window_addr(&w),
                    pid: self.window_pid(&w),
                    floating: ws.is_floating(&w),
                };
                sel.matches(&info).then_some((i, w))
            })
        })
    }

    /// `focuswindow`: switches to the window's workspace if needed.
    fn focus_window_sel(&mut self, sel: &WinSel) {
        let Some((i, w)) = self.find_window(sel) else {
            return;
        };
        let ws = &self.workspaces[i];
        if ws.is_special() {
            if self.wm.special_shown != Some(ws.id) {
                let name = ws.name.strip_prefix("special:").unwrap_or("").to_string();
                self.toggle_special(&name);
            }
        } else if i != self.active_workspace || self.wm.special_shown.is_some() {
            self.switch_to_index(i);
        }
        self.focus_window(Some(w));
    }

    fn kill_window(&mut self, sel: Option<&WinSel>, force: bool) {
        let window = match sel {
            Some(s) => self.find_window(s).map(|(_, w)| w),
            None => self.focused_window(),
        };
        let Some(window) = window else {
            return;
        };
        if force {
            // shortcut: shells out to kill(1); use libc::kill if a libc dep
            // is added for something else.
            let pid = self.window_pid(&window);
            if pid > 0 {
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid.to_string()])
                    .status();
            }
        } else if let Some(t) = window.toplevel() {
            t.send_close();
        } else if let Some(x) = window.x11_surface() {
            let _ = x.close();
        }
    }

    /// The window a dispatcher acts on: the selector's match, else the focused one.
    fn target_window(&self, sel: Option<&WinSel>) -> Option<Window> {
        match sel {
            Some(s) => self.find_window(s).map(|(_, w)| w),
            None => self.focused_window(),
        }
    }

    fn rename_workspace(&mut self, id: i32, name: &str) {
        let Some(i) = self.ws_index(id) else {
            return;
        };
        self.workspaces[i].name = name.to_string();
        self.ipc.emit("renameworkspace", &format!("{id},{name}"));
    }

    /// `alterzorder`. shortcut: smithay 0.7 has no `lower_element`, so bottom
    /// raises every other window; relayout re-raises floating windows.
    fn alter_zorder(&mut self, top: bool, sel: Option<&WinSel>) {
        let Some(w) = self.target_window(sel) else {
            return;
        };
        if top {
            self.space.raise_element(&w, false);
            return;
        }
        let others: Vec<Window> = self
            .space
            .elements()
            .filter(|e| *e != &w)
            .cloned()
            .collect();
        for o in &others {
            self.space.raise_element(o, false);
        }
    }

    /// `movecursortocorner`: pointer to a corner of the focused window.
    fn move_cursor_to_corner(&mut self, corner: u8) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some(g) = self.space.element_geometry(&w) else {
            return;
        };
        let (l, t) = (g.loc.x as f64, g.loc.y as f64);
        let (r, b) = (
            (g.loc.x + g.size.w - 1) as f64,
            (g.loc.y + g.size.h - 1) as f64,
        );
        let pos = match corner {
            0 => (l, b),
            1 => (r, b),
            2 => (r, t),
            _ => (l, t),
        };
        self.warp_pointer(pos.into());
    }

    /// Moves the pointer with a real motion event, so enter and leave are sent.
    pub(crate) fn warp_pointer(&mut self, pos: Point<f64, Logical>) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let under = self.surface_under(pos);
        let time = self.start_time.elapsed().as_millis() as u32;
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    /// `signal`. shortcut: shells out to kill(1), like `kill_window`.
    fn signal_window(&self, sig: i32, sel: Option<&WinSel>) {
        let Some(w) = self.target_window(sel) else {
            return;
        };
        let pid = self.window_pid(&w);
        if pid > 0 {
            let _ = std::process::Command::new("kill")
                .args([format!("-{sig}"), pid.to_string()])
                .status();
        }
    }

    /// `setprop`: stored per window, read by `window_dynp`.
    fn set_prop(&mut self, sel: Option<&WinSel>, prop: &str, value: &str) {
        let Some(w) = self.target_window(sel) else {
            return;
        };
        self.wm.props.retain(|(x, p, _)| !(x == &w && p == prop));
        self.wm.props.push((w, prop.to_string(), value.to_string()));
    }

    /// Starts an interactive move/resize of the window under the pointer
    /// (`bindm`). Dragging a tiled window with `movewindow` makes it floating
    /// first; `resizewindow` on a tiled window is ignored.
    pub fn start_mouse_grab(&mut self, resize: bool, button: u32) -> bool {
        let pointer = self.seat.get_pointer().unwrap();
        let pos = pointer.current_location();
        let Some(window) = self.window_under(pos) else {
            return false;
        };
        self.focus_window(Some(window.clone()));
        let Some(i) = self.ws_of(&window) else {
            return false;
        };
        if !self.workspaces[i].is_floating(&window) {
            if resize {
                return false;
            }
            self.set_floating(&window, Some(true));
        }
        let Some(loc) = self.space.element_location(&window) else {
            return false;
        };
        let start_data = PointerGrabStartData {
            focus: None,
            button,
            location: pos,
        };
        let serial = SERIAL_COUNTER.next_serial();
        if resize {
            let size = window.geometry().size;
            // Resize from the corner nearest to the pointer.
            let rel = pos - loc.to_f64();
            let mut edges = if rel.x < size.w as f64 / 2.0 {
                ResizeEdge::LEFT
            } else {
                ResizeEdge::RIGHT
            };
            edges |= if rel.y < size.h as f64 / 2.0 {
                ResizeEdge::TOP
            } else {
                ResizeEdge::BOTTOM
            };
            if let Some(t) = window.toplevel() {
                t.with_pending_state(|s| s.states.set(xdg_toplevel::State::Resizing));
                t.send_pending_configure();
            }
            let grab =
                ResizeSurfaceGrab::start(start_data, window, edges, Rectangle::new(loc, size));
            pointer.set_grab(self, grab, serial, Focus::Clear);
        } else {
            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location: loc,
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
        true
    }

    pub fn dispatch(&mut self, action: &Dispatcher) {
        tracing::debug!(?action, "dispatch");
        match action {
            Dispatcher::Exec(cmd) => spawn(cmd),
            Dispatcher::KillActive => self.kill_window(None, false),
            Dispatcher::ForceKillActive => self.kill_window(None, true),
            Dispatcher::Exit => {
                self.lua_fire("hyprland.shutdown", &[]);
                self.loop_signal.stop()
            }
            Dispatcher::Lua(id) => self.lua_bind(*id),
            Dispatcher::Workspace(sel) => self.select_workspace(sel),
            Dispatcher::MoveToWorkspace(sel) => self.move_to_workspace(sel, false),
            Dispatcher::MoveToWorkspaceSilent(sel) => self.move_to_workspace(sel, true),
            Dispatcher::ToggleFloating => self.toggle_floating(),
            Dispatcher::SetFloating | Dispatcher::SetTiled => {
                if let Some(w) = self.focused_window() {
                    self.set_floating(&w, Some(*action == Dispatcher::SetFloating));
                }
            }
            Dispatcher::Fullscreen => self.toggle_fullscreen(false),
            Dispatcher::Maximize => self.toggle_fullscreen(true),
            Dispatcher::MoveFocus(dir) => self.move_focus(*dir),
            Dispatcher::CycleNext => self.cycle_next(),
            Dispatcher::ToggleSplit => self.layout_msg("togglesplit"),
            Dispatcher::LayoutMsg(msg) => self.layout_msg(msg),
            Dispatcher::ToggleGroup => self.toggle_group(),
            Dispatcher::ChangeGroupActive(step) => self.change_group_active(*step),
            Dispatcher::MoveIntoGroup(dir) => self.move_into_group(*dir),
            Dispatcher::MoveOutOfGroup(dir) => self.move_out_of_group(*dir),
            Dispatcher::MoveWindowOrGroup(dir) => self.move_window_or_group(*dir),
            Dispatcher::MoveGroupWindow(back) => self.move_group_window(*back),
            Dispatcher::LockGroups(t) => self.lock_groups(*t),
            Dispatcher::LockActiveGroup(t) => self.set_group_flag(*t, true),
            Dispatcher::DenyWindowFromGroup(t) => self.set_group_flag(*t, false),
            Dispatcher::SetIgnoreGroupLock(t) => {
                self.config.ignore_group_lock = t.apply(self.config.ignore_group_lock)
            }
            Dispatcher::MoveWindow(dir) | Dispatcher::SwapWindow(dir) => self.swap_dir(*dir),
            Dispatcher::ResizeActive(arg) => self.resize_active(arg),
            Dispatcher::MoveActive(arg) => self.move_active(arg),
            Dispatcher::CenterWindow => self.center_window(),
            Dispatcher::Pin => self.pin_active(),
            Dispatcher::FocusCurrentOrLast | Dispatcher::FocusUrgentOrLast => {
                // shortcut: no urgency tracking, so focusurgentorlast behaves
                // like focuscurrentorlast.
                if let Some(p) = self.wm.prev_focus.clone() {
                    self.focus_window_sel(&WinSel::Address(window_addr(&p)));
                }
            }
            Dispatcher::SplitRatio(arg) => {
                if let Some(w) = self.focused_window() {
                    if let Some(i) = self.ws_of(&w) {
                        let arg = *arg;
                        if self.workspaces[i].tiled.update_ratio(&w, |r| arg.apply(r)) {
                            self.relayout();
                        }
                    }
                }
            }
            Dispatcher::SwapNext(prev) => self.swap_next(*prev),
            Dispatcher::CloseWindow(sel) => self.kill_window(Some(sel), false),
            Dispatcher::FocusWindow(sel) => self.focus_window_sel(sel),
            // Needs the triggering key: handled in `process_input_event`.
            Dispatcher::Pass(_) | Dispatcher::MouseMove | Dispatcher::MouseResize => {}
            Dispatcher::ToggleSpecialWorkspace(name) => self.toggle_special(name),
            Dispatcher::Submap(name) => {
                self.wm.submap = name.clone();
                self.ipc.emit("submap", name);
            }
            Dispatcher::TagWindow(tag, sel) => self.tag_window(tag, sel.as_ref()),
            Dispatcher::RenameWorkspace(id, name) => self.rename_workspace(*id, name),
            Dispatcher::BringActiveToTop => {
                if let Some(w) = self.focused_window() {
                    if self
                        .ws_of(&w)
                        .is_some_and(|i| self.workspaces[i].is_floating(&w))
                    {
                        self.space.raise_element(&w, true);
                    }
                }
            }
            Dispatcher::AlterZOrder(top, sel) => self.alter_zorder(*top, sel.as_ref()),
            Dispatcher::MoveCursorToCorner(c) => self.move_cursor_to_corner(*c),
            Dispatcher::MoveCursor(x, y) => self.warp_pointer((*x as f64, *y as f64).into()),
            Dispatcher::TogglePseudo => {
                if let Some(w) = self.focused_window() {
                    match self.wm.pseudo.iter().position(|p| p == &w) {
                        Some(i) => {
                            self.wm.pseudo.remove(i);
                        }
                        None => self.wm.pseudo.push(w),
                    }
                    self.relayout();
                }
            }
            Dispatcher::SetProp(sel, prop, value) => self.set_prop(sel.as_ref(), prop, value),
            Dispatcher::KillWindow(sel) => self.kill_window(sel.as_ref(), true),
            Dispatcher::Signal(sig, sel) => self.signal_window(*sig, sel.as_ref()),
            Dispatcher::Event(data) => self.ipc.emit("custom", data),
            Dispatcher::Dpms(t) => self.wm.dpms_on = t.apply(self.wm.dpms_on),
            Dispatcher::Global(name) => {
                self.global_shortcut(name, true);
            }
            Dispatcher::Noop(..) => {}
            Dispatcher::FocusMonitor(arg) => {
                if let Some(m) = self.resolve_mon(arg) {
                    self.focus_monitor_index(m);
                }
            }
            Dispatcher::MoveCurrentWorkspaceToMonitor(arg) => {
                if let Some(m) = self.resolve_mon(arg) {
                    self.move_ws_to_monitor(self.active_workspace, m);
                }
            }
            Dispatcher::MoveWorkspaceToMonitor(ws, mon) => {
                let idx = if ws == "current" {
                    Some(self.active_workspace)
                } else {
                    ws.parse::<WsSel>().ok().and_then(|sel| {
                        let infos = self.ws_infos();
                        let t =
                            resolve_ws(&sel, self.active_id(), self.wm.previous_workspace, &infos)?;
                        self.ensure_ws(&t)
                    })
                };
                if let (Some(idx), Some(m)) = (idx, self.resolve_mon(mon)) {
                    self.move_ws_to_monitor(idx, m);
                }
            }
            Dispatcher::SwapActiveWorkspaces(a, b) => {
                if let (Some(a), Some(b)) = (self.resolve_mon(a), self.resolve_mon(b)) {
                    self.swap_active_workspaces(a, b);
                }
            }
            Dispatcher::FocusWorkspaceOnCurrentMonitor(sel) => {
                let infos = self.ws_infos();
                if let Some(t) =
                    resolve_ws(sel, self.active_id(), self.wm.previous_workspace, &infos)
                {
                    if let Some(idx) = self.ensure_ws(&t) {
                        self.focus_ws_on_current(idx);
                    }
                }
            }
            Dispatcher::MoveWindowToMonitor(arg) => self.move_window_to_monitor(arg),
            Dispatcher::FullscreenState(internal, client, toggle) => {
                self.fullscreen_state(*internal, *client, *toggle)
            }
            Dispatcher::SendShortcut(mods, key, sel) => {
                self.send_key(*mods, key, sel.as_ref(), &["down", "up"])
            }
            Dispatcher::SendKeyState(mods, key, state, sel) => {
                self.send_key(*mods, key, sel.as_ref(), &[state.as_str()])
            }
        }
        // Floating, fullscreen, pin or tags may have changed: dynamic rules.
        self.refresh_all_rules();
    }

    /// Window under the pointer on the visible workspaces.
    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<Window> {
        let w = self.space.element_under(pos).map(|(w, _)| w.clone())?;
        // A menu or tooltip never takes focus: the focused window stays.
        if crate::xwayland::is_override_redirect(&w) {
            return self.focused_window();
        }
        Some(w)
    }
}

pub fn spawn(cmd: &str) {
    let cmd = crate::config::strip_exec_rules(cmd);
    tracing::info!("exec: {cmd}");
    // Double fork: the inner `sh` backgrounds the command and exits at once, so
    // we reap it here and the program is re-parented to init (no zombies).
    match std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("eval \"$1\" &")
        .arg("sh")
        .arg(cmd)
        .spawn()
    {
        Ok(mut child) => {
            if let Err(e) = child.wait() {
                tracing::warn!("failed to reap `{cmd}`: {e}");
            }
        }
        Err(e) => tracing::warn!("failed to spawn `{cmd}`: {e}"),
    }
}

#[cfg(test)]
mod tests {
    /// Zombie children of this process, read from /proc.
    fn zombies() -> usize {
        let me = std::process::id().to_string();
        std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("stat")).ok())
            .filter(|stat| {
                // Fields after the `(comm)`: state, ppid, ...
                let rest = stat.rsplit_once(") ").map_or("", |(_, r)| r);
                let mut fields = rest.split(' ');
                fields.next() == Some("Z") && fields.next() == Some(me.as_str())
            })
            .count()
    }

    #[test]
    fn spawn_leaves_no_zombies() {
        let marker = std::env::temp_dir().join(format!("hypr-spawn-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        for _ in 0..3 {
            super::spawn("true");
        }
        super::spawn(&format!("touch '{}'", marker.display()));
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "spawned command did not run");
        let _ = std::fs::remove_file(&marker);
        assert_eq!(zombies(), 0);
    }
}
