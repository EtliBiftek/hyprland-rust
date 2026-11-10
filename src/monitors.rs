//! Multiple monitors. Every output shows its own workspace (`WmState::mons`);
//! one monitor is focused and `Hypr::active_workspace` is the index of the
//! workspace it shows. Backends create outputs and call `add_output` /
//! `remove_output`; placement, scale and transform come from `monitor =`
//! rules.

use smithay::{
    output::{Mode, Output, PhysicalProperties, Scale, Subpixel},
    utils::{Logical, Point, Rectangle, Transform},
};

use crate::{
    config::{Direction, MonMode, MonPos, MonitorRule},
    layout::Rect,
    selectors::Target,
    state::Hypr,
};

/// One output as the window manager sees it.
#[derive(Debug, Clone)]
pub struct Mon {
    /// Hyprland's monitor id: stable while the output exists.
    pub id: u32,
    pub name: String,
    /// Id of the workspace shown on this monitor.
    pub ws: i32,
}

pub fn transform_of(t: u8) -> Transform {
    match t {
        1 => Transform::_90,
        2 => Transform::_180,
        3 => Transform::_270,
        4 => Transform::Flipped,
        5 => Transform::Flipped90,
        6 => Transform::Flipped180,
        7 => Transform::Flipped270,
        _ => Transform::Normal,
    }
}

pub fn transform_index(t: Transform) -> u8 {
    match t {
        Transform::Normal => 0,
        Transform::_90 => 1,
        Transform::_180 => 2,
        Transform::_270 => 3,
        Transform::Flipped => 4,
        Transform::Flipped90 => 5,
        Transform::Flipped180 => 6,
        Transform::Flipped270 => 7,
    }
}

/// Auto placement: next to the existing outputs in `dir`.
fn auto_position(
    existing: &[Rectangle<i32, Logical>],
    size: (i32, i32),
    dir: Direction,
) -> (i32, i32) {
    if existing.is_empty() {
        return (0, 0);
    }
    let max_x = existing
        .iter()
        .map(|g| g.loc.x + g.size.w)
        .max()
        .unwrap_or(0);
    let min_x = existing.iter().map(|g| g.loc.x).min().unwrap_or(0);
    let max_y = existing
        .iter()
        .map(|g| g.loc.y + g.size.h)
        .max()
        .unwrap_or(0);
    let min_y = existing.iter().map(|g| g.loc.y).min().unwrap_or(0);
    match dir {
        Direction::Right => (max_x, 0),
        Direction::Left => (min_x - size.0, 0),
        Direction::Down => (0, max_y),
        Direction::Up => (0, min_y - size.1),
    }
}

impl Hypr {
    pub fn output_named(&self, name: &str) -> Option<Output> {
        self.space.outputs().find(|o| o.name() == name).cloned()
    }

    pub fn mon_index(&self, name: &str) -> Option<usize> {
        self.wm.mons.iter().position(|m| m.name == name)
    }

    /// Name of the focused monitor (empty before the first output).
    pub fn focused_mon_name(&self) -> String {
        self.wm
            .mons
            .get(self.wm.focused_mon)
            .map(|m| m.name.clone())
            .unwrap_or_default()
    }

    pub fn focused_output(&self) -> Option<Output> {
        self.output_named(&self.focused_mon_name())
            .or_else(|| self.space.outputs().next().cloned())
    }

    pub fn mon_geometry(&self, name: &str) -> Option<Rectangle<i32, Logical>> {
        let o = self.output_named(name)?;
        self.space.output_geometry(&o)
    }

    /// The output minus the exclusive zones of layer surfaces (bars, docks).
    pub fn usable_geometry_of(&self, name: &str) -> Option<Rectangle<i32, Logical>> {
        self.usable_of(&self.output_named(name)?)
    }

    /// `o` minus layer-shell exclusive zones and `monitor = ..., addreserved`.
    pub fn usable_of(&self, o: &Output) -> Option<Rectangle<i32, Logical>> {
        let geo = self.space.output_geometry(o)?;
        let zone = smithay::desktop::layer_map_for_output(o).non_exclusive_zone();
        let [t, b, l, r] = self.config.reserved_for(&o.name(), &o.description());
        Some(Rectangle::new(
            (geo.loc.x + zone.loc.x + l, geo.loc.y + zone.loc.y + t).into(),
            (zone.size.w - l - r, zone.size.h - t - b).into(),
        ))
    }

    /// Tiling area of monitor `name` (see `tiling_area`).
    pub fn tiling_area_of(&self, name: &str) -> Rect {
        let g = &self.config.general;
        let out = self
            .usable_geometry_of(name)
            .or_else(|| self.usable_geometry())
            .map(|r| Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h))
            .unwrap_or(Rect::new(0, 0, 1280, 800));
        out.shrink((g.gaps_out - g.gaps_in).max(0))
    }

    /// Monitor a workspace lives on (the focused one if unassigned or gone).
    pub fn ws_monitor(&self, idx: usize) -> String {
        let m = &self.workspaces[idx].monitor;
        if self.mon_index(m).is_some() {
            m.clone()
        } else {
            self.focused_mon_name()
        }
    }

    /// Monitor whose regular workspace is `idx`.
    pub fn mon_showing(&self, idx: usize) -> Option<usize> {
        let id = self.workspaces.get(idx)?.id;
        self.wm.mons.iter().position(|m| m.ws == id)
    }

    /// Indices of the regular workspaces shown on all monitors.
    pub fn shown_ws(&self) -> Vec<usize> {
        self.wm
            .mons
            .iter()
            .filter_map(|m| self.ws_index(m.ws))
            .collect()
    }

    /// Keeps `active_workspace` on the focused monitor's workspace.
    pub fn sync_active(&mut self) {
        if let Some(i) = self
            .wm
            .mons
            .get(self.wm.focused_mon)
            .and_then(|m| self.ws_index(m.ws))
        {
            self.active_workspace = i;
        }
    }

    /// Makes monitor `m` the focused one (no window focus change).
    pub fn set_focused_mon(&mut self, m: usize) {
        if m >= self.wm.mons.len() {
            return;
        }
        let changed = m != self.wm.focused_mon;
        self.wm.focused_mon = m;
        self.sync_active();
        if changed {
            self.emit_focused_mon();
        }
    }

    /// Monitor under a point of the global space.
    pub fn mon_at(&self, p: Point<f64, Logical>) -> Option<usize> {
        let o = self.space.output_under(p).next()?.clone();
        self.mon_index(&o.name())
    }

    /// Keeps a pointer position inside the outputs: a position outside all of
    /// them is clamped to the output that contains `from`.
    pub fn clamp_to_outputs(
        &self,
        from: Point<f64, Logical>,
        to: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let p = self.clamp_to_output(from, to);
        // `cursor:hotspot_padding` (0-100): distance kept to the screen edges.
        let pad = match crate::options::get_option(&self.config, "cursor:hotspot_padding") {
            Some(crate::options::OptValue::Int(n)) => n.clamp(0, 100) as f64,
            _ => 0.0,
        };
        if pad == 0.0 {
            return p;
        }
        let rects: Vec<_> = self
            .space
            .outputs()
            .filter_map(|o| self.space.output_geometry(o))
            .map(|g| {
                (
                    g.loc.x as f64,
                    g.loc.y as f64,
                    g.size.w as f64,
                    g.size.h as f64,
                )
            })
            .collect();
        crate::layout::pad_hotspot((p.x, p.y), &rects, pad).into()
    }

    fn clamp_to_output(
        &self,
        from: Point<f64, Logical>,
        to: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        if self.space.output_under(to).next().is_some() {
            return to;
        }
        let o = self
            .space
            .output_under(from)
            .next()
            .cloned()
            .or_else(|| self.focused_output());
        let Some(geo) = o.and_then(|o| self.space.output_geometry(&o)) else {
            return to;
        };
        let geo = geo.to_f64();
        (
            to.x.clamp(geo.loc.x, geo.loc.x + geo.size.w - 1.0),
            to.y.clamp(geo.loc.y, geo.loc.y + geo.size.h - 1.0),
        )
            .into()
    }

    /// Pointer moved: the monitor under it becomes the focused one.
    pub fn pointer_on_monitor(&mut self, p: Point<f64, Logical>) {
        if let Some(m) = self.mon_at(p) {
            if m != self.wm.focused_mon {
                self.set_focused_mon(m);
            }
        }
    }

    /// Workspace shown on a newly added monitor: a `default:true` workspace
    /// rule for it, a hidden workspace already assigned to it, the startup
    /// workspace for the first monitor, else the lowest free id.
    fn initial_ws_for(&mut self, name: &str) -> usize {
        let free = |h: &Hypr, i: usize| h.mon_showing(i).is_none();
        let rule = self
            .config
            .workspace_rules
            .iter()
            .find(|r| r.default && r.monitor.as_deref() == Some(name))
            .map(|r| r.workspace.clone());
        if let Some(ws) = rule {
            let t = match ws.parse::<i32>() {
                Ok(n) => Target::Id(n),
                Err(_) => Target::Name(ws),
            };
            if let Some(i) = self.ensure_ws(&t).filter(|&i| free(self, i)) {
                return i;
            }
        }
        if let Some(i) = (0..self.workspaces.len())
            .filter(|&i| {
                let w = &self.workspaces[i];
                w.monitor == name && !w.is_special() && free(self, i)
            })
            .min_by_key(|&i| self.workspaces[i].id)
        {
            return i;
        }
        if self.wm.mons.is_empty() {
            if let Some(i) = (0..self.workspaces.len())
                .find(|&i| !self.workspaces[i].is_special() && free(self, i))
            {
                return i;
            }
        }
        let id = (1..).find(|n| self.ws_index(*n).is_none()).unwrap_or(1);
        self.ensure_ws(&Target::Id(id)).unwrap_or(0)
    }

    /// Places, scales and registers a new output that the backend created
    /// (with its mode set). Called once per output.
    pub fn add_output(&mut self, output: &Output) {
        let name = output.name();
        if self.mon_index(&name).is_some() {
            return;
        }
        let desc = output.description();
        let rule = self.config.monitor_rule(&name, &desc);
        self.apply_monitor_rule(output, &rule);

        let id = self.wm.next_mon_id;
        self.wm.next_mon_id += 1;
        let idx = self.initial_ws_for(&name);
        self.workspaces[idx].monitor = name.clone();
        let ws = self.workspaces[idx].id;
        self.wm.mons.push(Mon {
            id,
            name: name.clone(),
            ws,
        });
        if self.wm.mons.len() == 1 {
            self.wm.focused_mon = 0;
        }
        self.sync_active();
        self.ipc.emit("monitoradded", &name);
        self.ipc
            .emit("monitoraddedv2", &format!("{id},{name},{desc}"));
        self.relayout();
    }

    /// Scale, transform and position from a `monitor =` rule, then maps the
    /// output. The backend has already picked the mode.
    pub fn apply_monitor_rule(&mut self, output: &Output, rule: &MonitorRule) {
        let scale = rule.scale.unwrap_or(1.0);
        // The winit output keeps its GL-required flip.
        let transform = (output.name() != "winit").then(|| transform_of(rule.transform));
        output.change_current_state(None, transform, Some(Scale::Fractional(scale)), None);
        let mode = output
            .current_mode()
            .map_or((1920, 1080), |m| (m.size.w, m.size.h));
        // Logical size after transform and scale.
        let (mut lw, mut lh) = (mode.0 as f64 / scale, mode.1 as f64 / scale);
        if rule.transform % 2 == 1 {
            std::mem::swap(&mut lw, &mut lh);
        }
        let others: Vec<Rectangle<i32, Logical>> = self
            .space
            .outputs()
            .filter(|o| *o != output)
            .filter_map(|o| self.space.output_geometry(o))
            .collect();
        let loc = match rule.position {
            MonPos::Exact(x, y) => (x, y),
            MonPos::Auto(dir) => auto_position(&others, (lw as i32, lh as i32), dir),
        };
        output.change_current_state(None, None, None, Some(loc.into()));
        self.space.map_output(output, loc);
    }

    /// Re-applies `monitor =` rules to every output (config reload, keyword).
    pub fn apply_monitor_rules(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for o in outputs {
            let rule = self.config.monitor_rule(&o.name(), &o.description());
            self.apply_monitor_rule(&o, &rule);
        }
        self.relayout();
    }

    /// An output went away: its workspaces move to another monitor.
    pub fn remove_output(&mut self, name: &str) {
        let Some(m) = self.mon_index(name) else {
            return;
        };
        let mon = self.wm.mons.remove(m);
        if let Some(o) = self.output_named(name) {
            self.space.unmap_output(&o);
        }
        if self.wm.focused_mon >= m && self.wm.focused_mon > 0 {
            self.wm.focused_mon -= 1;
        }
        let target = self
            .wm
            .mons
            .get(self.wm.focused_mon)
            .map(|t| t.name.clone());
        if let Some(target) = target {
            for i in 0..self.workspaces.len() {
                if self.workspaces[i].monitor == name {
                    self.workspaces[i].monitor = target.clone();
                }
            }
        }
        // Windows of the workspace it showed are hidden now.
        if let Some(i) = self.ws_index(mon.ws) {
            for w in self.workspaces[i].windows() {
                self.space.unmap_elem(&w);
            }
        }
        self.sync_active();
        self.ipc.emit("monitorremoved", name);
        self.ipc
            .emit("monitorremovedv2", &format!("{},{name},", mon.id));
        self.emit_focused_mon();
        self.relayout();
        let f = self.focus_ws();
        let next = self.workspaces[f].last_focused.clone();
        self.focus_window(next);
        self.gc_workspaces();
    }

    /// Shows workspace `idx` on monitor `m` (it leaves the monitor it was on,
    /// if hidden there). Floating windows keep their place relative to the
    /// monitor. Pinned windows stay on screen.
    pub fn show_ws_on(&mut self, m: usize, idx: usize) {
        let Some(old_id) = self.wm.mons.get(m).map(|x| x.ws) else {
            return;
        };
        let mon_name = self.wm.mons[m].name.clone();
        if let Some(old) = self.ws_index(old_id).filter(|&o| o != idx) {
            let pinned: Vec<_> = self
                .wm
                .pinned
                .iter()
                .filter(|w| self.workspaces[old].is_floating(w))
                .cloned()
                .collect();
            for w in self.workspaces[old].windows() {
                if !pinned.contains(&w) {
                    self.space.unmap_elem(&w);
                }
            }
            for w in &pinned {
                self.workspaces[old].remove(w);
                self.workspaces[idx].floating.push(w.clone());
            }
        }
        let from = self.workspaces[idx].monitor.clone();
        if from != mon_name {
            self.translate_floating(idx, &from, &mon_name);
            self.workspaces[idx].monitor = mon_name;
        }
        self.wm.mons[m].ws = self.workspaces[idx].id;
        self.sync_active();
    }

    /// Moves the floating windows of a workspace by the offset between two
    /// monitors, so they keep their place on screen.
    fn translate_floating(&mut self, idx: usize, from: &str, to: &str) {
        let (Some(a), Some(b)) = (self.mon_geometry(from), self.mon_geometry(to)) else {
            return;
        };
        let delta = b.loc - a.loc;
        for w in self.workspaces[idx].floating.clone() {
            if let Some(loc) = self.space.element_location(&w) {
                self.space.map_element(w, loc + delta, false);
            }
        }
    }

    /// Resolves a monitor argument: `l`/`r`/`u`/`d`, a name, `+N`/`-N`
    /// (cyclic), a monitor id or `current`.
    pub fn resolve_mon(&self, arg: &str) -> Option<usize> {
        let arg = arg.trim();
        let n = self.wm.mons.len();
        if n == 0 {
            return None;
        }
        let cur = self.wm.focused_mon;
        let dir = match arg {
            "l" | "left" => Some(Direction::Left),
            "r" | "right" => Some(Direction::Right),
            "u" | "up" | "t" => Some(Direction::Up),
            "d" | "down" | "b" => Some(Direction::Down),
            _ => None,
        };
        if let Some(dir) = dir {
            let geo = |i: usize| self.mon_geometry(&self.wm.mons[i].name);
            let c = geo(cur)?;
            let (cx, cy) = (c.loc.x + c.size.w / 2, c.loc.y + c.size.h / 2);
            return (0..n)
                .filter(|&i| i != cur)
                .filter_map(|i| Some((i, geo(i)?)))
                .filter(|(_, g)| {
                    let (gx, gy) = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
                    match dir {
                        Direction::Left => gx < cx,
                        Direction::Right => gx > cx,
                        Direction::Up => gy < cy,
                        Direction::Down => gy > cy,
                    }
                })
                .min_by_key(|(_, g)| {
                    let (gx, gy) = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
                    (gx - cx).abs() + (gy - cy).abs()
                })
                .map(|(i, _)| i);
        }
        if arg == "current" {
            return Some(cur);
        }
        if let Some(i) = self.mon_index(arg) {
            return Some(i);
        }
        if arg.starts_with('+') || arg.starts_with('-') {
            let d: i64 = arg.parse().ok()?;
            return Some((cur as i64 + d).rem_euclid(n as i64) as usize);
        }
        if let Ok(id) = arg.parse::<u32>() {
            return self.wm.mons.iter().position(|m| m.id == id);
        }
        let desc = arg.strip_prefix("desc:").unwrap_or(arg);
        self.wm.mons.iter().position(|m| {
            self.output_named(&m.name)
                .is_some_and(|o| o.description().starts_with(desc))
        })
    }

    /// Focus moves to monitor `m`: its workspace, its last window, and the
    /// pointer to its center (unless `cursor:no_warps`).
    pub fn focus_monitor_index(&mut self, m: usize) {
        if m == self.wm.focused_mon || m >= self.wm.mons.len() {
            return;
        }
        self.set_focused_mon(m);
        let f = self.focus_ws();
        let next = self.workspaces[f].last_focused.clone();
        self.focus_window(next);
        self.warp_to_mon(m);
        let (id, name) = (
            self.active_id(),
            self.workspaces[self.active_workspace].name.clone(),
        );
        self.ipc.emit("workspace", &name);
        self.ipc.emit("workspacev2", &format!("{id},{name}"));
    }

    pub fn warp_to_mon(&mut self, m: usize) {
        if self.config.cursor_no_warps {
            return;
        }
        let Some(g) = self.wm.mons.get(m).and_then(|x| self.mon_geometry(&x.name)) else {
            return;
        };
        let c = (
            (g.loc.x + g.size.w / 2) as f64,
            (g.loc.y + g.size.h / 2) as f64,
        );
        self.warp_pointer(c.into());
    }

    /// `movecurrentworkspacetomonitor` / `moveworkspacetomonitor`.
    pub fn move_ws_to_monitor(&mut self, idx: usize, m: usize) {
        let Some(target) = self.wm.mons.get(m).map(|x| x.name.clone()) else {
            return;
        };
        if self.ws_monitor(idx) == target && self.mon_showing(idx) == Some(m) {
            return;
        }
        // The monitor it leaves shows another workspace (or a new one).
        if let Some(src) = self.mon_showing(idx) {
            let replacement = (0..self.workspaces.len())
                .filter(|&i| {
                    i != idx
                        && self.workspaces[i].monitor == self.wm.mons[src].name
                        && !self.workspaces[i].is_special()
                        && self.mon_showing(i).is_none()
                })
                .min_by_key(|&i| self.workspaces[i].id)
                .or_else(|| {
                    let id = (1..).find(|n| self.ws_index(*n).is_none())?;
                    let i = self.ensure_ws(&Target::Id(id))?;
                    self.workspaces[i].monitor = self.wm.mons[src].name.clone();
                    Some(i)
                });
            // The new workspace may have shifted nothing: indices only grow.
            if let Some(r) = replacement {
                self.wm.mons[src].ws = self.workspaces[r].id;
            }
        }
        self.show_ws_on(m, idx);
        let (id, name) = (self.workspaces[idx].id, self.workspaces[idx].name.clone());
        self.ipc.emit("moveworkspace", &format!("{name},{target}"));
        self.ipc
            .emit("moveworkspacev2", &format!("{id},{name},{target}"));
        self.set_focused_mon(m);
        self.relayout();
        let next = self.workspaces[self.focus_ws()].last_focused.clone();
        self.focus_window(next);
        self.gc_workspaces();
    }

    /// `swapactiveworkspaces`.
    pub fn swap_active_workspaces(&mut self, a: usize, b: usize) {
        if a == b || a >= self.wm.mons.len() || b >= self.wm.mons.len() {
            return;
        }
        let (wa, wb) = (self.wm.mons[a].ws, self.wm.mons[b].ws);
        let (Some(ia), Some(ib)) = (self.ws_index(wa), self.ws_index(wb)) else {
            return;
        };
        let (na, nb) = (self.wm.mons[a].name.clone(), self.wm.mons[b].name.clone());
        self.translate_floating(ia, &na, &nb);
        self.translate_floating(ib, &nb, &na);
        self.workspaces[ia].monitor = nb;
        self.workspaces[ib].monitor = na;
        self.wm.mons[a].ws = wb;
        self.wm.mons[b].ws = wa;
        self.sync_active();
        self.relayout();
        let next = self.workspaces[self.focus_ws()].last_focused.clone();
        self.focus_window(next);
    }

    /// `focusworkspaceoncurrentmonitor`: brings the workspace here; if it is
    /// shown on another monitor, the two monitors swap.
    pub fn focus_ws_on_current(&mut self, idx: usize) {
        let cur = self.wm.focused_mon;
        match self.mon_showing(idx) {
            Some(m) if m == cur => {}
            Some(m) => self.swap_active_workspaces(cur, m),
            None => {
                let prev = self.active_id();
                self.show_ws_on(cur, idx);
                self.wm.previous_workspace = Some(prev);
                let (id, name) = (self.workspaces[idx].id, self.workspaces[idx].name.clone());
                self.ipc.emit("workspace", &name);
                self.ipc.emit("workspacev2", &format!("{id},{name}"));
                self.relayout();
                let next = self.workspaces[idx].last_focused.clone();
                self.focus_window(next);
                self.gc_workspaces();
            }
        }
    }

    /// `movewindow, mon:X`: the focused window goes to that monitor's
    /// workspace.
    pub fn move_window_to_monitor(&mut self, arg: &str) {
        let Some(m) = self.resolve_mon(arg) else {
            return;
        };
        let id = self.wm.mons[m].ws;
        self.dispatch(&crate::config::Dispatcher::MoveToWorkspace(
            crate::selectors::WsSel::Id(id),
        ));
    }

    /// A virtual output (headless backend, `hyprctl output create headless`).
    pub fn create_virtual_output(&mut self, name: Option<&str>) -> Result<String, String> {
        let name = match name {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => (1..)
                .map(|i| format!("HEADLESS-{i}"))
                .find(|n| self.output_named(n).is_none())
                .unwrap_or_default(),
        };
        if self.output_named(&name).is_some() {
            return Err(format!("output {name} already exists"));
        }
        let rule = self.config.monitor_rule(&name, "");
        if rule.disabled {
            return Err(format!("output {name} is disabled by a monitor rule"));
        }
        let mode = match rule.mode {
            MonMode::Exact { w, h, refresh } => Mode {
                size: (w, h).into(),
                refresh: refresh.map_or(60_000, |r| (r * 1000.0) as i32),
            },
            _ => Mode {
                size: crate::headless::size_from_env().into(),
                refresh: 60_000,
            },
        };
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "hyprland-rust".into(),
                model: "Headless".into(),
            },
        );
        let global = output.create_global::<Hypr>(&self.display_handle);
        self.wm.output_globals.push((name.clone(), global));
        output.change_current_state(Some(mode), Some(Transform::Normal), None, None);
        output.set_preferred(mode);
        self.add_output(&output);
        Ok(name)
    }

    /// `hyprctl output remove NAME` (virtual outputs only).
    pub fn remove_virtual_output(&mut self, name: &str) -> Result<(), String> {
        let Some(pos) = self.wm.output_globals.iter().position(|(n, _)| n == name) else {
            return Err(format!("no virtual output {name}"));
        };
        let (_, global) = self.wm.output_globals.remove(pos);
        self.remove_output(name);
        self.display_handle.remove_global::<Hypr>(global);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_position_places_next_to_existing() {
        let r = |x, y, w, h| Rectangle::new((x, y).into(), (w, h).into());
        assert_eq!(auto_position(&[], (100, 100), Direction::Right), (0, 0));
        let ex = [r(0, 0, 1920, 1080), r(1920, 0, 1280, 1024)];
        assert_eq!(auto_position(&ex, (800, 600), Direction::Right), (3200, 0));
        assert_eq!(auto_position(&ex, (800, 600), Direction::Left), (-800, 0));
        assert_eq!(auto_position(&ex, (800, 600), Direction::Down), (0, 1080));
        assert_eq!(auto_position(&ex, (800, 600), Direction::Up), (0, -600));
    }
}
