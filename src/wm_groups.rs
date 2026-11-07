//! Groups (tabbed windows), `layoutmsg` and the groupbar. Group membership
//! itself lives in `groups.rs`; this file moves windows between layout slots.

use std::cell::RefCell;

use smithay::{
    backend::renderer::element::{
        solid::{SolidColorBuffer, SolidColorRenderElement},
        Kind,
    },
    desktop::Window,
    utils::{Logical, Point},
};

use crate::{
    config::{Direction, GroupStep, Toggle},
    groups::Group,
    ipc::window_addr,
    layout::{find_in_direction, Rect},
    selectors::{RatioArg, WinSel},
    state::Hypr,
    tiling::LayoutKind,
};

/// One tab of a groupbar.
pub struct Tab {
    pub window: Window,
    pub rect: Rect,
    pub active: bool,
    pub locked: bool,
}

/// Persistent buffer of a window's tab, so unchanged tabs are not redrawn.
struct TabBuf(RefCell<SolidColorBuffer>);

fn short_addr(w: &Window) -> String {
    window_addr(w).trim_start_matches("0x").to_string()
}

impl Hypr {
    /// (workspace index, group index) of the group `w` is in.
    pub fn group_of(&self, w: &Window) -> Option<(usize, usize)> {
        let i = self.ws_of(w)?;
        Some((i, self.workspaces[i].groups.find(w)?))
    }

    fn lock_blocks(&self) -> bool {
        self.wm.groups_locked && !self.config.ignore_group_lock
    }

    /// Shows `new` instead of `old` in the layout slot of a group on workspace `i`.
    fn show_member(&mut self, i: usize, old: &Window, new: &Window) {
        let floating = self.workspaces[i].floating.contains(old);
        let geo = self.space.element_geometry(old);
        self.workspaces[i].replace_in_layout(old, new);
        self.space.unmap_elem(old);
        if let (true, Some(g)) = (floating, geo) {
            if let Some(t) = new.toplevel() {
                t.with_pending_state(|s| s.size = Some(g.size));
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
            self.space.map_element(new.clone(), g.loc, false);
        }
    }

    /// Makes a hidden group member the visible one. False if `w` is not a
    /// hidden member.
    pub fn group_activate(&mut self, w: &Window) -> bool {
        let Some((i, gi)) = self.group_of(w) else {
            return false;
        };
        let g = &mut self.workspaces[i].groups.list[gi];
        let Some(idx) = g.members.iter().position(|m| m == w) else {
            return false;
        };
        let Some(old) = g.set_current(idx) else {
            return false;
        };
        self.show_member(i, &old, w);
        true
    }

    /// Joins the new `window` to the focused unlocked group (`group:auto_group`).
    /// `idx` is the workspace receiving the window. False if no group applies.
    pub fn try_auto_group(&mut self, idx: usize, window: &Window) -> bool {
        if !self.config.group.auto_group || self.wm.groups_locked {
            return false;
        }
        let Some(f) = self.focused_window() else {
            return false;
        };
        let Some(gi) = self.workspaces[idx].groups.find(&f) else {
            return false;
        };
        let after = self.config.group.insert_after_current;
        let g = &mut self.workspaces[idx].groups.list[gi];
        if g.locked || g.denied {
            return false;
        }
        let old = g.active().clone();
        g.add(window.clone(), after);
        self.show_member(idx, &old, window);
        true
    }

    pub fn toggle_group(&mut self) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some(i) = self.ws_of(&w) else {
            return;
        };
        let area = self.tiling_area();
        let ws = &mut self.workspaces[i];
        let Some(gi) = ws.groups.find(&w) else {
            ws.groups.list.push(Group::new(w.clone()));
            self.ipc
                .emit("togglegroup", &format!("1,{}", short_addr(&w)));
            return;
        };
        let g = ws.groups.list.remove(gi);
        let floating = ws.floating.contains(g.active());
        let mut anchor = g.active().clone();
        for m in g.members.iter().filter(|m| *m != g.active()) {
            if floating {
                ws.floating.push(m.clone());
            } else {
                ws.tiled.insert(m.clone(), Some(&anchor), area);
                anchor = m.clone();
            }
        }
        self.ipc
            .emit("togglegroup", &format!("0,{}", short_addr(&g.members[0])));
        self.relayout();
        self.focus_window(Some(w));
    }

    pub fn change_group_active(&mut self, step: GroupStep) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some((i, gi)) = self.group_of(&w) else {
            return;
        };
        let g = &mut self.workspaces[i].groups.list[gi];
        let len = g.members.len();
        if len < 2 {
            return;
        }
        let old = match step {
            GroupStep::Forward => g.step(true),
            GroupStep::Backward => g.step(false),
            GroupStep::Index(n) if n <= 0 => g.set_current(len - 1),
            GroupStep::Index(n) if n as usize > len => None,
            GroupStep::Index(n) => g.set_current(n as usize - 1),
        };
        let Some(old) = old else {
            return;
        };
        let new = g.active().clone();
        self.show_member(i, &old, &new);
        self.relayout();
        self.focus_window(Some(new));
    }

    /// Neighbour of the focused tiled window in `dir` (an active group member
    /// or a plain window).
    fn tiled_neighbour(&self, w: &Window, dir: Direction) -> Option<Window> {
        let (_, rects) = self.tiled_rects(w)?;
        find_in_direction(&rects, w, dir)
    }

    /// Moves `w` into the group of `to` and shows it there.
    fn join_group(&mut self, i: usize, w: &Window, to: &Window) {
        self.workspaces[i].remove(w);
        let after = self.config.group.insert_after_current;
        let ws = &mut self.workspaces[i];
        let Some(gi) = ws.groups.find(to) else {
            return;
        };
        let old = ws.groups.list[gi].active().clone();
        ws.groups.list[gi].add(w.clone(), after);
        ws.replace_in_layout(&old, w);
        self.space.unmap_elem(&old);
        self.relayout();
        self.focus_window(Some(w.clone()));
        self.ipc.emit("moveintogroup", &short_addr(w));
    }

    pub fn move_into_group(&mut self, dir: Direction) {
        if self.lock_blocks() {
            return;
        }
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some(to) = self.tiled_neighbour(&w, dir) else {
            return;
        };
        let Some((i, tg)) = self.group_of(&to) else {
            return;
        };
        let ignore = self.config.ignore_group_lock;
        let target = &self.workspaces[i].groups.list[tg];
        let own = self.workspaces[i].groups.group_of(&w);
        if target.denied || (!ignore && (target.locked || own.is_some_and(|g| g.locked))) {
            return;
        }
        self.join_group(i, &w, &to);
    }

    pub fn move_out_of_group(&mut self, dir: Option<Direction>) {
        if self.lock_blocks() {
            return;
        }
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some((i, gi)) = self.group_of(&w) else {
            return;
        };
        let area = self.tiling_area();
        let focus_removed = self.config.group.focus_removed_window;
        let ws = &mut self.workspaces[i];
        let g = &mut ws.groups.list[gi];
        let anchor = if g.members.len() == 1 {
            // A group of one just dissolves; the window keeps its slot.
            ws.groups.list.remove(gi);
            None
        } else {
            let floating = ws.floating.contains(g.active());
            let was_active = g.active() == &w;
            g.remove(&w);
            let anchor = g.active().clone();
            if was_active {
                ws.replace_in_layout(&w, &anchor);
            }
            if floating {
                ws.floating.push(w.clone());
            } else {
                ws.tiled.dwindle.preselect = dir;
                ws.tiled.insert(w.clone(), Some(&anchor), area);
                ws.tiled.dwindle.preselect = None;
            }
            Some(anchor)
        };
        self.relayout();
        let focus = match anchor {
            Some(a) if !focus_removed => a,
            _ => w.clone(),
        };
        self.focus_window(Some(focus));
        self.ipc.emit("moveoutofgroup", &short_addr(&w));
    }

    pub fn move_window_or_group(&mut self, dir: Direction) {
        let Some(w) = self.focused_window() else {
            return;
        };
        let Some(i) = self.ws_of(&w) else {
            return;
        };
        if self.lock_blocks() {
            self.swap_dir(dir);
            return;
        }
        let ignore = self.config.ignore_group_lock;
        let own = self.workspaces[i].groups.group_of(&w).cloned();
        let own_locked = own.as_ref().is_some_and(|g| g.locked);
        let own_denied = own.as_ref().is_some_and(|g| g.denied);
        match self.tiled_neighbour(&w, dir) {
            Some(n) => {
                if let Some(ng) = self.workspaces[i].groups.group_of(&n) {
                    if !ignore && (ng.locked || own_locked || own_denied) {
                        self.swap_dir(dir);
                    } else {
                        self.join_group(i, &w, &n);
                    }
                } else if (!ignore && own_locked) || own.is_none() {
                    self.swap_dir(dir);
                } else {
                    self.move_out_of_group(Some(dir));
                }
            }
            None => {
                if own.is_some() && (ignore || !own_locked) {
                    self.move_out_of_group(Some(dir));
                }
            }
        }
    }

    pub fn move_group_window(&mut self, back: bool) {
        if let Some((i, gi)) = self.focused_window().and_then(|w| self.group_of(&w)) {
            self.workspaces[i].groups.list[gi].move_active(!back);
        }
    }

    pub fn lock_groups(&mut self, t: Toggle) {
        self.wm.groups_locked = t.apply(self.wm.groups_locked);
        let v = self.wm.groups_locked as u8;
        self.ipc.emit("lockgroups", &v.to_string());
    }

    /// `lockactivegroup` (`lock == true`) and `denywindowfromgroup`.
    pub fn set_group_flag(&mut self, t: Toggle, lock: bool) {
        if let Some((i, gi)) = self.focused_window().and_then(|w| self.group_of(&w)) {
            let g = &mut self.workspaces[i].groups.list[gi];
            let flag = if lock { &mut g.locked } else { &mut g.denied };
            *flag = t.apply(*flag);
        }
    }

    // ------------------------------------------------------------ groupbar

    /// Tabs of every visible group, above the group's active window. Titles
    /// are not drawn (shortcut: no text rendering yet; each tab is a solid rect).
    pub fn groupbar_tabs(&self) -> Vec<Tab> {
        let gb = &self.config.group.groupbar;
        let mut out = Vec::new();
        if !gb.enabled || gb.height <= 0 {
            return out;
        }
        let b = self.config.general.border_size.max(0);
        for i in self.visible_ws() {
            let ws = &self.workspaces[i];
            for g in &ws.groups.list {
                let a = g.active();
                if (gb.disable_when_only && g.members.len() == 1)
                    || ws.fullscreen.as_ref() == Some(a)
                {
                    continue;
                }
                let Some(loc) = self.space.element_location(a) else {
                    continue;
                };
                let size = a.geometry().size;
                let n = g.members.len() as i32;
                let total = size.w + 2 * b;
                let tab_w = ((total - gb.gaps_in * (n - 1)) / n).max(1);
                let y = (loc.y - b - gb.height).max(0);
                for (k, m) in g.members.iter().enumerate() {
                    let x = loc.x - b + k as i32 * (tab_w + gb.gaps_in);
                    let w = if k as i32 == n - 1 {
                        (loc.x - b + total - x).max(1)
                    } else {
                        tab_w
                    };
                    out.push(Tab {
                        window: m.clone(),
                        rect: Rect::new(x, y, w, gb.height),
                        active: k == g.current,
                        locked: g.locked,
                    });
                }
            }
        }
        out
    }

    /// The group member whose tab is under `pos`.
    pub fn groupbar_tab_at(&self, pos: Point<f64, Logical>) -> Option<Window> {
        let (px, py) = (pos.x as i32, pos.y as i32);
        self.groupbar_tabs()
            .into_iter()
            .find(|t| {
                let r = t.rect;
                px >= r.x && px < r.x + r.w && py >= r.y && py < r.y + r.h
            })
            .map(|t| t.window)
    }

    pub fn groupbar_elements(&self, origin: Point<i32, Logical>) -> Vec<SolidColorRenderElement> {
        let gb = &self.config.group.groupbar;
        self.groupbar_tabs()
            .into_iter()
            .map(|t| {
                let color = match (t.active, t.locked) {
                    (true, false) => gb.col_active,
                    (false, false) => gb.col_inactive,
                    (true, true) => gb.col_locked_active,
                    (false, true) => gb.col_locked_inactive,
                };
                let buf = t
                    .window
                    .user_data()
                    .get_or_insert(|| TabBuf(RefCell::new(SolidColorBuffer::default())));
                let mut buf = buf.0.borrow_mut();
                buf.update((t.rect.w, t.rect.h), color);
                SolidColorRenderElement::from_buffer(
                    &buf,
                    (t.rect.x - origin.x, t.rect.y - origin.y),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )
            })
            .collect()
    }

    // ------------------------------------------------------------ layoutmsg

    /// `layoutmsg <message> [args]` for the layout of the focused workspace.
    pub fn layout_msg(&mut self, msg: &str) {
        let mut it = msg.split_whitespace();
        let Some(cmd) = it.next() else {
            return;
        };
        let args: Vec<&str> = it.collect();
        let w = self.focused_window();
        let i = w
            .as_ref()
            .and_then(|w| self.ws_of(w))
            .unwrap_or(self.focus_ws());
        let r = match self.workspaces[i].tiled.kind {
            LayoutKind::Dwindle => self.dwindle_msg(i, cmd, &args, w),
            LayoutKind::Master => self.master_msg(i, cmd, &args, w),
        };
        if let Err(e) = r {
            tracing::warn!("layoutmsg `{msg}`: {e}");
        }
    }

    fn dwindle_msg(
        &mut self,
        i: usize,
        cmd: &str,
        args: &[&str],
        w: Option<Window>,
    ) -> Result<(), String> {
        let area = self.tiling_area();
        let tiled = |s: &Self, w: &Window| !s.workspaces[i].is_floating(w);
        let need = || w.clone().ok_or("no focused window".to_string());
        let d = &mut self.workspaces[i].tiled.dwindle;
        let ok = match cmd {
            "togglesplit" => d.toggle_split(&need()?, area),
            "swapsplit" => d.swap_split(&need()?, area),
            "rotatesplit" => {
                let angle = match args.first() {
                    Some(a) => a.parse().map_err(|_| "invalid angle".to_string())?,
                    None => 90,
                };
                d.rotate_split(&need()?, angle, area)
            }
            "movetoroot" => {
                let stable = args.get(1) != Some(&"unstable");
                let target = match args.first() {
                    Some(s) if *s != "active" && *s != "unstable" => {
                        let sel: WinSel = s.parse()?;
                        self.find_window(&sel)
                            .map(|(_, w)| w)
                            .ok_or("no such window")?
                    }
                    _ => need()?,
                };
                let ok = tiled(self, &target)
                    && self.workspaces[i]
                        .tiled
                        .dwindle
                        .move_to_root(&target, stable);
                if !ok {
                    return Err("can't movetoroot in the current workspace".into());
                }
                self.relayout();
                return Ok(());
            }
            "preselect" => {
                let c = args.first().and_then(|a| a.chars().next());
                d.preselect = match c {
                    Some('u' | 't') => Some(Direction::Up),
                    Some('d' | 'b') => Some(Direction::Down),
                    Some('l') => Some(Direction::Left),
                    Some('r') => Some(Direction::Right),
                    Some(_) => None,
                    None => return Err("No direction for preselect".into()),
                };
                return Ok(());
            }
            "splitratio" => {
                let v = args.first().ok_or("splitratio requires an arg")?;
                let exact = args.get(1).is_some_and(|a| a.starts_with("exact"));
                let ra: RatioArg = format!("{}{v}", if exact { "exact " } else { "" }).parse()?;
                d.update_ratio(&need()?, |r| ra.apply(r))
            }
            other => return Err(format!("Unknown dwindle layoutmsg: {other}")),
        };
        if !ok {
            return Err(format!("can't {cmd} in the current workspace"));
        }
        self.relayout();
        Ok(())
    }

    fn master_msg(
        &mut self,
        i: usize,
        cmd: &str,
        args: &[&str],
        w: Option<Window>,
    ) -> Result<(), String> {
        let w = w.ok_or("no focused window")?;
        if self.workspaces[i].is_floating(&w) {
            return Err("window is floating".into());
        }
        let focus = self.workspaces[i].tiled.master.msg(cmd, args, &w)?;
        self.relayout();
        if let Some(f) = focus {
            self.focus_window(Some(f));
        }
        Ok(())
    }
}
