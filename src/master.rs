//! Hyprland's "master" layout: one or more master windows on one side and a
//! stack of slaves on the other. Follows `CMasterAlgorithm` of v0.57.1 for one
//! monitor, generic over the window type so it can be unit-tested.
//!
//! shortcut: every window has the same weight (no per-window `percSize`), so
//! `resizeactive` only moves the master/stack split (`mfact`); upgrade by
//! adding a weight per node when per-window resizing is needed.

use crate::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl Orientation {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim() {
            "left" => Orientation::Left,
            "right" => Orientation::Right,
            "top" => Orientation::Top,
            "bottom" => Orientation::Bottom,
            "center" => Orientation::Center,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Orientation::Left => "left",
            Orientation::Right => "right",
            Orientation::Top => "top",
            Orientation::Bottom => "bottom",
            Orientation::Center => "center",
        }
    }

    /// Clockwise order used by `orientationnext`.
    const CYCLE: [Orientation; 5] = [
        Orientation::Left,
        Orientation::Top,
        Orientation::Right,
        Orientation::Bottom,
        Orientation::Center,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewStatus {
    Master,
    #[default]
    Slave,
    Inherit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewOnActive {
    #[default]
    None,
    Before,
    After,
}

/// The `master { }` config category.
#[derive(Debug, Clone, PartialEq)]
pub struct MasterCfg {
    pub mfact: f32,
    pub orientation: Orientation,
    pub new_status: NewStatus,
    pub new_on_top: bool,
    pub new_on_active: NewOnActive,
    pub slave_count_for_center_master: i32,
    pub center_master_fallback: Orientation,
    pub always_keep_position: bool,
    pub allow_small_split: bool,
    pub focus_master_on_close: bool,
    pub special_scale_factor: f32,
    pub smart_resizing: bool,
    pub drop_at_cursor: bool,
}

impl Default for MasterCfg {
    fn default() -> Self {
        Self {
            mfact: 0.55,
            orientation: Orientation::Left,
            new_status: NewStatus::Slave,
            new_on_top: false,
            new_on_active: NewOnActive::None,
            slave_count_for_center_master: 2,
            center_master_fallback: Orientation::Left,
            always_keep_position: false,
            allow_small_split: false,
            focus_master_on_close: false,
            special_scale_factor: 1.0,
            smart_resizing: true,
            drop_at_cursor: true,
        }
    }
}

#[derive(Debug, Clone)]
struct MNode<T> {
    w: T,
    master: bool,
}

#[derive(Debug, Clone)]
pub struct Master<T> {
    nodes: Vec<MNode<T>>,
    pub cfg: MasterCfg,
    /// Set by `layoutmsg orientation*`; wins over the workspace rule and config.
    pub orientation: Option<Orientation>,
    /// Orientation from the workspace rule `layoutopt:orientation:`.
    pub rule_orientation: Option<Orientation>,
    /// Set by `layoutmsg mfact`/`resizeactive`; reset when the config value changes.
    mfact: Option<f32>,
    /// Window remembered by `focusmaster previous`.
    focus_prev: Option<T>,
}

impl<T> Default for Master<T> {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            cfg: MasterCfg::default(),
            orientation: None,
            rule_orientation: None,
            mfact: None,
            focus_prev: None,
        }
    }
}

/// Splits `len` into `n` consecutive spans whose edges are rounded, so no
/// pixel is lost or doubled.
fn spans(start: i32, len: i32, n: usize) -> Vec<(i32, i32)> {
    let edge = |i: usize| start + (len as i64 * i as i64 / n as i64) as i32;
    (0..n).map(|i| (edge(i), edge(i + 1) - edge(i))).collect()
}

impl<T: PartialEq + Clone> Master<T> {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, w: &T) -> bool {
        self.pos(w).is_some()
    }

    pub fn windows(&self) -> Vec<T> {
        self.nodes.iter().map(|n| n.w.clone()).collect()
    }

    fn pos(&self, w: &T) -> Option<usize> {
        self.nodes.iter().position(|n| &n.w == w)
    }

    pub fn masters(&self) -> usize {
        self.nodes.iter().filter(|n| n.master).count()
    }

    /// The first master window.
    pub fn master_window(&self) -> Option<&T> {
        self.nodes.iter().find(|n| n.master).map(|n| &n.w)
    }

    pub fn is_master(&self, w: &T) -> bool {
        self.pos(w).is_some_and(|i| self.nodes[i].master)
    }

    /// Applies the config; a changed `mfact` drops a `layoutmsg mfact` override.
    pub fn configure(&mut self, cfg: &MasterCfg) {
        if cfg.mfact != self.cfg.mfact {
            self.mfact = None;
        }
        self.cfg = cfg.clone();
    }

    pub fn mfact(&self) -> f32 {
        self.mfact.unwrap_or(self.cfg.mfact).clamp(0.05, 0.95)
    }

    /// Orientation chosen by `layoutmsg`, workspace rule or config.
    pub fn base_orientation(&self) -> Orientation {
        self.orientation
            .or(self.rule_orientation)
            .unwrap_or(self.cfg.orientation)
    }

    /// Orientation after the `center_master_fallback`, and whether the master
    /// is really centered (needs enough slaves).
    pub fn effective_orientation(&self) -> (Orientation, bool) {
        let o = self.base_orientation();
        if o != Orientation::Center {
            return (o, false);
        }
        let slaves = self.nodes.len() - self.masters();
        if slaves as i32 >= self.cfg.slave_count_for_center_master {
            (o, true)
        } else {
            let fb = self.cfg.center_master_fallback;
            (
                if fb == Orientation::Center {
                    Orientation::Left
                } else {
                    fb
                },
                false,
            )
        }
    }

    /// Adds `new`. `target` is the focused tiled window of the workspace.
    pub fn insert(&mut self, new: T, target: Option<&T>) {
        if self.contains(&new) {
            return;
        }
        let c = self.cfg.clone();
        let new_is_master = c.new_status == NewStatus::Master;
        let t = target.and_then(|t| self.pos(t));
        let before = c.new_on_active == NewOnActive::Before;
        let at = match t {
            Some(i)
                if c.new_on_active != NewOnActive::None
                    && !new_is_master
                    && !(self.nodes[i].master
                        && (self.masters() == 1 || c.new_status == NewStatus::Slave)) =>
            {
                if before {
                    i
                } else {
                    i + 1
                }
            }
            _ if c.new_on_top => 0,
            _ => self.nodes.len(),
        };
        // The node the window opens on; the master if the focus is elsewhere.
        let opening_master = t.is_none_or(|i| self.nodes[i].master);
        let total = self.nodes.len() + 1;
        let becomes_master =
            new_is_master || total == 1 || (c.new_status == NewStatus::Inherit && opening_master);
        if becomes_master {
            let old = if before {
                self.nodes.iter_mut().rev().find(|n| n.master)
            } else {
                self.nodes.iter_mut().find(|n| n.master)
            };
            if let Some(n) = old {
                n.master = false;
            }
        }
        self.nodes.insert(
            at,
            MNode {
                w: new,
                master: becomes_master,
            },
        );
    }

    pub fn remove(&mut self, w: &T) -> bool {
        let Some(i) = self.pos(w) else {
            return false;
        };
        let masters_left = self.masters();
        if self.nodes[i].master && (masters_left <= 1 || self.cfg.allow_small_split) {
            if let Some(n) = self.nodes.iter_mut().find(|n| !n.master) {
                n.master = true;
            }
        }
        self.nodes.remove(i);
        if !self.nodes.is_empty() && self.masters() == self.nodes.len() && masters_left > 1 {
            if let Some(n) = self.nodes.last_mut() {
                n.master = false;
            }
        }
        if self.nodes.len() == 1 {
            self.nodes[0].master = true;
        }
        true
    }

    /// Swaps two windows, keeping the slots (and master flags) in place.
    pub fn swap(&mut self, a: &T, b: &T) -> bool {
        match (self.pos(a), self.pos(b)) {
            (Some(i), Some(j)) if i != j => {
                let (wa, wb) = (self.nodes[i].w.clone(), self.nodes[j].w.clone());
                self.nodes[i].w = wb;
                self.nodes[j].w = wa;
                true
            }
            _ => false,
        }
    }

    pub fn replace(&mut self, old: &T, new: &T) -> bool {
        match self.pos(old) {
            Some(i) => {
                self.nodes[i].w = new.clone();
                true
            }
            None => false,
        }
    }

    /// Rectangles of all windows; every one is shrunk by `gaps_in`.
    pub fn layout(&self, area: Rect, gaps_in: i32) -> Vec<(T, Rect)> {
        let n = self.nodes.len();
        if n == 0 {
            return Vec::new();
        }
        let (orient, centered) = self.effective_orientation();
        let masters: Vec<&T> = self
            .nodes
            .iter()
            .filter(|x| x.master)
            .map(|x| &x.w)
            .collect();
        let slaves: Vec<&T> = self
            .nodes
            .iter()
            .filter(|x| !x.master)
            .map(|x| &x.w)
            .collect();
        let mfact = self.mfact();
        let mut out: Vec<(T, Rect)> = Vec::new();
        let mut put = |w: &T, r: Rect| out.push((w.clone(), r.shrink(gaps_in)));

        if n == 1 && !centered {
            let w = masters[0];
            if self.cfg.always_keep_position {
                let ww = (area.w as f32 * mfact).round() as i32;
                let x = match orient {
                    Orientation::Right => area.w - ww,
                    Orientation::Center => (area.w - ww) / 2,
                    _ => 0,
                };
                put(w, Rect::new(area.x + x, area.y, ww, area.h));
            } else {
                put(w, area);
            }
            return out;
        }

        let stack = !slaves.is_empty();
        match orient {
            Orientation::Top | Orientation::Bottom => {
                let mh = if stack {
                    (area.h as f32 * mfact).round() as i32
                } else {
                    area.h
                };
                let (my, sy) = if orient == Orientation::Bottom {
                    (area.h - mh, 0)
                } else {
                    (0, mh)
                };
                for (w, (x, l)) in masters
                    .iter()
                    .copied()
                    .zip(spans(area.x, area.w, masters.len()))
                {
                    put(w, Rect::new(x, area.y + my, l, mh));
                }
                for (w, (x, l)) in slaves
                    .iter()
                    .copied()
                    .zip(spans(area.x, area.w, slaves.len()))
                {
                    put(w, Rect::new(x, area.y + sy, l, area.h - mh));
                }
            }
            _ => {
                let mw = if stack || centered {
                    (area.w as f32 * mfact).round() as i32
                } else {
                    area.w
                };
                let mx = match orient {
                    Orientation::Right => area.w - mw,
                    Orientation::Center => (area.w - mw) / 2,
                    _ => 0,
                };
                for (w, (y, l)) in masters
                    .iter()
                    .copied()
                    .zip(spans(area.y, area.h, masters.len()))
                {
                    put(w, Rect::new(area.x + mx, y, mw, l));
                }
                match orient {
                    Orientation::Left | Orientation::Right => {
                        let sx = if orient == Orientation::Left { mw } else { 0 };
                        for (w, (y, l)) in
                            slaves
                                .iter()
                                .copied()
                                .zip(spans(area.y, area.h, slaves.len()))
                        {
                            put(w, Rect::new(area.x + sx, y, area.w - mw, l));
                        }
                    }
                    _ => {
                        // Centered: slaves alternate between the left and right column.
                        let side = (area.w - mw) / 2;
                        let first_right = self.cfg.center_master_fallback == Orientation::Right;
                        let mut cols: [Vec<&T>; 2] = [Vec::new(), Vec::new()];
                        for (i, w) in slaves.iter().enumerate() {
                            let right = (i % 2 == 0) == first_right;
                            cols[right as usize].push(*w);
                        }
                        for (right, col) in cols.iter().enumerate() {
                            let x = if right == 1 { side + mw } else { 0 };
                            let wd = if right == 1 { area.w - side - mw } else { side };
                            for (w, (y, l)) in
                                col.iter().copied().zip(spans(area.y, area.h, col.len()))
                            {
                                put(w, Rect::new(area.x + x, y, wd, l));
                            }
                        }
                        // Keep the node order stable for callers.
                        let order = |w: &T| self.pos(w).unwrap_or(0);
                        out.sort_by_key(|(w, _)| order(w));
                    }
                }
            }
        }
        out
    }

    /// Next (`next`) or previous window in master order: masters first, then
    /// the stack, as `getNextTarget` does.
    pub fn next_target(&self, w: &T, next: bool, wrap: bool) -> Option<T> {
        let i = self.pos(w)?;
        let is_m = self.nodes[i].master;
        let order: Vec<&MNode<T>> = if next {
            self.nodes.iter().collect()
        } else {
            self.nodes.iter().rev().collect()
        };
        let at = order.iter().position(|n| &n.w == w)?;
        let cand = order[at..]
            .iter()
            .find(|n| &n.w != w && n.master == is_m)
            .or_else(|| order.iter().find(|n| &n.w != w && n.master != is_m))?;
        if !wrap && ((cand.master && next) || (!cand.master && is_m && !next)) {
            return None;
        }
        Some(cand.w.clone())
    }

    /// `resizeactive` on a tiled window: moves the master/stack split.
    pub fn resize(&mut self, w: &T, dx: i32, dy: i32, area: Rect) -> bool {
        let Some(i) = self.pos(w) else {
            return false;
        };
        let (orient, centered) = self.effective_orientation();
        if self.nodes.len() == 1 && !centered {
            return false;
        }
        let master = self.nodes[i].master;
        let sign = if master { 1.0 } else { -1.0 };
        let d = match orient {
            Orientation::Top | Orientation::Bottom => dy as f32 / area.h.max(1) as f32,
            Orientation::Center if !master => 2.0 * dx as f32 / area.w.max(1) as f32,
            _ => dx as f32 / area.w.max(1) as f32,
        };
        self.mfact = Some((self.mfact() + sign * d).clamp(0.05, 0.95));
        true
    }

    /// Relative (`exact` false) or absolute change of the split ratio.
    pub fn set_mfact(&mut self, v: f32, exact: bool) {
        let m = if exact { v } else { self.mfact() + v };
        self.mfact = Some(m.clamp(0.05, 0.95));
    }

    /// Handles `layoutmsg <cmd> [args]` for the focused window `f`. `Ok(Some(w))`
    /// asks the caller to focus `w`.
    pub fn msg(&mut self, cmd: &str, args: &[&str], f: &T) -> Result<Option<T>, String> {
        let arg = args.first().copied().unwrap_or("");
        let fi = self.pos(f).ok_or("focused window isn't tiled")?;
        match cmd {
            "swapwithmaster" => {
                let m = self.master_window().cloned().ok_or("no master node")?;
                if &m != f {
                    self.swap(f, &m);
                    return Ok(Some(if arg == "child" { m } else { f.clone() }));
                }
                if args.contains(&"ignoremaster") {
                    return Ok(None);
                }
                let Some(n) = self.nodes.iter().find(|n| !n.master).map(|n| n.w.clone()) else {
                    return Ok(None);
                };
                self.swap(&n, &m);
                Ok(Some(if arg == "master" { n } else { m }))
            }
            "focusmaster" => {
                let m = self.master_window().cloned().ok_or("no master")?;
                if &m != f {
                    if arg == "previous" {
                        self.focus_prev = Some(f.clone());
                    }
                    return Ok(Some(m));
                }
                let auto = |s: &Self| s.nodes.iter().find(|n| !n.master).map(|n| n.w.clone());
                match arg {
                    "master" => Ok(None),
                    "previous" => Ok(self
                        .focus_prev
                        .clone()
                        .filter(|p| p != f && self.contains(p))
                        .or_else(|| auto(self))),
                    _ => Ok(auto(self)),
                }
            }
            "cyclenext" | "cycleprev" => {
                Ok(self.next_target(f, cmd == "cyclenext", arg != "noloop"))
            }
            "swapnext" | "swapprev" => {
                let Some(o) = self.next_target(f, cmd == "swapnext", arg != "noloop") else {
                    return Ok(None);
                };
                self.swap(f, &o);
                Ok(Some(f.clone()))
            }
            "addmaster" => {
                let (windows, masters) = (self.nodes.len(), self.masters());
                if masters + 2 > windows && !self.cfg.allow_small_split {
                    return Err("nothing to do".into());
                }
                if self.nodes[fi].master {
                    if let Some(n) = self.nodes.iter_mut().find(|n| !n.master) {
                        n.master = true;
                    }
                } else {
                    self.nodes[fi].master = true;
                }
                Ok(None)
            }
            "removemaster" => {
                if self.nodes.len() < 2 || self.masters() < 2 {
                    return Err("nothing to do".into());
                }
                if self.nodes[fi].master {
                    self.nodes[fi].master = false;
                } else if let Some(n) = self.nodes.iter_mut().rev().find(|n| n.master) {
                    n.master = false;
                }
                Ok(None)
            }
            "orientationleft" | "orientationright" | "orientationtop" | "orientationbottom"
            | "orientationcenter" => {
                self.orientation = Orientation::parse(&cmd["orientation".len()..]);
                Ok(None)
            }
            "orientationnext" | "orientationprev" | "orientationcycle" => {
                let mut cycle: Vec<Orientation> = if cmd == "orientationcycle" {
                    args.iter().filter_map(|a| Orientation::parse(a)).collect()
                } else {
                    Vec::new()
                };
                if cycle.is_empty() {
                    cycle = Orientation::CYCLE.to_vec();
                }
                let cur = self.orientation.unwrap_or(self.base_orientation());
                let at = cycle.iter().position(|o| *o == cur).unwrap_or(0) as i32;
                let step = if cmd == "orientationprev" { -1 } else { 1 };
                let n = cycle.len() as i32;
                self.orientation = Some(cycle[(at + step).rem_euclid(n) as usize]);
                Ok(None)
            }
            "mfact" => {
                let exact = arg == "exact";
                let v = if exact { args.get(1) } else { args.first() };
                let v: f32 = v.and_then(|v| v.parse().ok()).ok_or("mfact: bad ratio")?;
                self.set_mfact(v, exact);
                Ok(None)
            }
            "rollnext" | "rollprev" => {
                let old = if self.nodes[fi].master {
                    fi
                } else {
                    self.nodes
                        .iter()
                        .position(|n| n.master)
                        .ok_or("no old master")?
                };
                let next = cmd == "rollnext";
                let cand = if next {
                    self.nodes.iter().position(|n| !n.master)
                } else {
                    self.nodes.iter().rposition(|n| !n.master)
                };
                let Some(j) = cand else {
                    return Ok(None);
                };
                let focus = self.nodes[j].w.clone();
                let old_w = self.nodes[old].w.clone();
                self.nodes[j].master = true;
                if j < old {
                    self.nodes[j..old].rotate_left(1);
                } else if j > old {
                    self.nodes[old..=j].rotate_right(1);
                }
                // Demote the old master and move it to the end (next) or start (prev).
                let at = self.pos(&old_w).unwrap_or(0);
                self.nodes[at].master = false;
                if next {
                    self.nodes[at..].rotate_left(1);
                } else {
                    self.nodes[..=at].rotate_right(1);
                }
                Ok(Some(focus))
            }
            _ => Err(format!("Unknown master layoutmsg: {cmd}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect::new(0, 0, 1000, 600);

    fn mk(n: u32) -> Master<u32> {
        let mut m = Master::default();
        for i in 1..=n {
            m.insert(i, None);
        }
        m
    }

    fn rects(m: &Master<u32>) -> Vec<(u32, Rect)> {
        m.layout(A, 0)
    }

    #[test]
    fn single_window_fills() {
        assert_eq!(rects(&mk(1)), vec![(1, A)]);
    }

    #[test]
    fn left_orientation_with_mfact() {
        let l = rects(&mk(3));
        // mfact 0.55 of 1000 = 550
        assert_eq!(l[0], (1, Rect::new(0, 0, 550, 600)));
        assert_eq!(l[1], (2, Rect::new(550, 0, 450, 300)));
        assert_eq!(l[2], (3, Rect::new(550, 300, 450, 300)));
    }

    #[test]
    fn each_orientation() {
        let mut m = mk(3);
        m.orientation = Some(Orientation::Right);
        let l = rects(&m);
        assert_eq!(l[0].1, Rect::new(450, 0, 550, 600));
        assert_eq!(l[1].1, Rect::new(0, 0, 450, 300));
        m.orientation = Some(Orientation::Top);
        let l = rects(&m);
        assert_eq!(l[0].1, Rect::new(0, 0, 1000, 330));
        assert_eq!(l[1].1, Rect::new(0, 330, 500, 270));
        assert_eq!(l[2].1, Rect::new(500, 330, 500, 270));
        m.orientation = Some(Orientation::Bottom);
        let l = rects(&m);
        assert_eq!(l[0].1, Rect::new(0, 270, 1000, 330));
        assert_eq!(l[1].1, Rect::new(0, 0, 500, 270));
    }

    #[test]
    fn center_orientation_and_fallback() {
        let mut m = mk(2);
        m.orientation = Some(Orientation::Center);
        // one slave < slave_count_for_center_master (2): falls back to left
        assert_eq!(rects(&m)[0].1, Rect::new(0, 0, 550, 600));
        m.insert(3, None);
        let l = rects(&m);
        assert_eq!(l[0].1, Rect::new(225, 0, 550, 600));
        assert_eq!(l[1].1, Rect::new(0, 0, 225, 600));
        assert_eq!(l[2].1, Rect::new(775, 0, 225, 600));
        m.insert(4, None);
        let l = rects(&m);
        assert_eq!(l[1].1, Rect::new(0, 0, 225, 300));
        assert_eq!(l[3].1, Rect::new(0, 300, 225, 300));
        assert_eq!(l[2].1, Rect::new(775, 0, 225, 600));
        // always center the lone master with a threshold of 0
        let mut one = mk(1);
        one.orientation = Some(Orientation::Center);
        one.cfg.slave_count_for_center_master = 0;
        assert_eq!(rects(&one)[0].1, Rect::new(225, 0, 550, 600));
    }

    #[test]
    fn multiple_masters() {
        let mut m = mk(4);
        m.msg("addmaster", &[], &1).unwrap();
        assert_eq!(m.masters(), 2);
        let l = rects(&m);
        assert_eq!(l[0].1, Rect::new(0, 0, 550, 300));
        assert_eq!(l[1].1, Rect::new(0, 300, 550, 300));
        assert_eq!(l[2].1, Rect::new(550, 0, 450, 300));
        m.msg("removemaster", &[], &1).unwrap();
        assert_eq!(m.masters(), 1);
        assert!(m.msg("removemaster", &[], &1).is_err());
    }

    #[test]
    fn mfact_messages_and_config() {
        let mut m = mk(2);
        m.msg("mfact", &["+0.2"], &1).unwrap();
        assert_eq!(rects(&m)[0].1.w, 750);
        m.msg("mfact", &["exact", "0.5"], &1).unwrap();
        assert_eq!(rects(&m)[0].1.w, 500);
        assert!(m.msg("mfact", &["x"], &1).is_err());
        let cfg = MasterCfg {
            mfact: 0.7,
            ..Default::default()
        };
        m.configure(&cfg);
        assert_eq!(rects(&m)[0].1.w, 700);
        assert!(m.resize(&1, 100, 0, A));
        assert_eq!(rects(&m)[0].1.w, 800);
        assert!(m.resize(&2, 100, 0, A));
        assert_eq!(rects(&m)[0].1.w, 700);
    }

    #[test]
    fn swap_and_focus_messages() {
        let mut m = mk(3);
        assert_eq!(m.msg("swapwithmaster", &[], &2).unwrap(), Some(2));
        assert_eq!(m.windows(), vec![2, 1, 3]);
        assert!(m.is_master(&2));
        // on master: swaps with the first child, focus stays on the old master
        assert_eq!(m.msg("swapwithmaster", &[], &2).unwrap(), Some(2));
        assert_eq!(m.windows(), vec![1, 2, 3]);
        assert_eq!(
            m.msg("swapwithmaster", &["ignoremaster"], &1).unwrap(),
            None
        );
        assert_eq!(m.msg("focusmaster", &[], &3).unwrap(), Some(1));
        assert_eq!(m.msg("focusmaster", &[], &1).unwrap(), Some(2));
        assert_eq!(m.msg("focusmaster", &["master"], &1).unwrap(), None);
        assert_eq!(m.msg("cyclenext", &[], &1).unwrap(), Some(2));
        assert_eq!(m.msg("cyclenext", &[], &3).unwrap(), Some(1));
        assert_eq!(m.msg("cyclenext", &["noloop"], &3).unwrap(), None);
        assert_eq!(m.msg("cycleprev", &[], &1).unwrap(), Some(3));
        assert_eq!(m.msg("cycleprev", &["noloop"], &1).unwrap(), None);
        assert_eq!(m.msg("swapnext", &[], &2).unwrap(), Some(2));
        assert_eq!(m.windows(), vec![1, 3, 2]);
        assert_eq!(m.msg("swapprev", &[], &2).unwrap(), Some(2));
        assert_eq!(m.windows(), vec![1, 2, 3]);
        assert!(m.msg("nope", &[], &1).is_err());
    }

    #[test]
    fn orientation_messages() {
        let mut m = mk(2);
        m.msg("orientationtop", &[], &1).unwrap();
        assert_eq!(m.base_orientation(), Orientation::Top);
        m.msg("orientationnext", &[], &1).unwrap();
        assert_eq!(m.base_orientation(), Orientation::Right);
        m.msg("orientationprev", &[], &1).unwrap();
        m.msg("orientationprev", &[], &1).unwrap();
        assert_eq!(m.base_orientation(), Orientation::Left);
        m.msg("orientationcycle", &["left", "center"], &1).unwrap();
        assert_eq!(m.base_orientation(), Orientation::Center);
        m.msg("orientationcenter", &[], &1).unwrap();
        m.msg("orientationleft", &[], &1).unwrap();
        assert_eq!(m.base_orientation(), Orientation::Left);
        // the workspace rule only applies until layoutmsg sets one
        let mut r = mk(2);
        r.rule_orientation = Some(Orientation::Bottom);
        assert_eq!(r.base_orientation(), Orientation::Bottom);
    }

    #[test]
    fn roll_messages() {
        let mut m = mk(4);
        assert_eq!(m.msg("rollnext", &[], &1).unwrap(), Some(2));
        assert_eq!(m.windows(), vec![2, 3, 4, 1]);
        assert!(m.is_master(&2));
        assert_eq!(m.msg("rollprev", &[], &2).unwrap(), Some(1));
        assert_eq!(m.windows(), vec![2, 1, 3, 4]);
        assert!(m.is_master(&1) && !m.is_master(&2));
    }

    #[test]
    fn insert_options() {
        let mut m: Master<u32> = Master::default();
        m.cfg.new_status = NewStatus::Master;
        m.insert(1, None);
        m.insert(2, Some(&1));
        assert!(m.is_master(&2) && !m.is_master(&1));

        let mut m: Master<u32> = Master::default();
        m.cfg.new_on_top = true;
        m.insert(1, None);
        m.insert(2, Some(&1));
        assert_eq!(m.windows(), vec![2, 1]);

        let mut m: Master<u32> = Master::default();
        m.cfg.new_on_active = NewOnActive::After;
        for i in 1..=3 {
            m.insert(i, None);
        }
        m.insert(4, Some(&2));
        assert_eq!(m.windows(), vec![1, 2, 4, 3]);
        m.cfg.new_on_active = NewOnActive::Before;
        m.insert(5, Some(&3));
        assert_eq!(m.windows(), vec![1, 2, 4, 5, 3]);

        let mut m: Master<u32> = Master::default();
        m.cfg.new_status = NewStatus::Inherit;
        m.insert(1, None);
        m.insert(2, Some(&1)); // opened on the master: becomes master
        assert!(m.is_master(&2));
        m.insert(3, Some(&1)); // 1 is a slave now
        assert!(!m.is_master(&3));
    }

    #[test]
    fn remove_promotes_and_keeps_invariants() {
        let mut m = mk(3);
        assert!(m.remove(&1));
        assert!(m.is_master(&2));
        assert!(m.remove(&2));
        assert!(m.is_master(&3));
        assert!(m.remove(&3));
        assert!(m.is_empty());
        assert!(!m.remove(&3));
    }

    #[test]
    fn always_keep_position_and_gaps() {
        let mut m = mk(1);
        m.orientation = Some(Orientation::Right);
        m.cfg.always_keep_position = true;
        assert_eq!(rects(&m)[0].1, Rect::new(450, 0, 550, 600));
        m.cfg.always_keep_position = false;
        assert_eq!(m.layout(A, 10)[0].1, Rect::new(10, 10, 980, 580));
    }
}
