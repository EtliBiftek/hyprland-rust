//! The tiled part of a workspace: either a dwindle tree or the master layout,
//! chosen per workspace (`general:layout`, workspace rule `layout:`).

use crate::{
    config::Config,
    layout::{Dwindle, Rect},
    master::Master,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutKind {
    #[default]
    Dwindle,
    Master,
}

impl LayoutKind {
    /// Unknown names (and layouts we do not have, like `scrolling`) fall back
    /// to dwindle, as Hyprland falls back to its default algorithm.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "master" => LayoutKind::Master,
            _ => LayoutKind::Dwindle,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutKind::Dwindle => "dwindle",
            LayoutKind::Master => "master",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Tiling<W> {
    pub kind: LayoutKind,
    pub dwindle: Dwindle<W>,
    pub master: Master<W>,
}

impl<W> Default for Tiling<W> {
    fn default() -> Self {
        Self {
            kind: LayoutKind::Dwindle,
            dwindle: Dwindle::default(),
            master: Master::default(),
        }
    }
}

impl<W: PartialEq + Clone> Tiling<W> {
    /// Copies the layout options from the config.
    pub fn configure(&mut self, cfg: &Config) {
        let d = &cfg.dwindle;
        self.dwindle.preserve_split = d.preserve_split;
        self.dwindle.split_ratio = d.default_split_ratio;
        self.dwindle.force_split = d.force_split;
        self.dwindle.smart_split = d.smart_split;
        self.dwindle.split_width_multiplier = d.split_width_multiplier;
        self.dwindle.permanent_direction_override = d.permanent_direction_override;
        self.master.configure(&cfg.master);
    }

    /// Switches layout, re-inserting the windows in their current order.
    pub fn set_kind(&mut self, kind: LayoutKind, area: Rect) {
        if kind == self.kind {
            return;
        }
        let wins = self.windows();
        for w in &wins {
            self.dwindle.remove(w);
            self.master.remove(w);
        }
        self.kind = kind;
        let mut prev: Option<W> = None;
        for w in wins {
            self.insert(w.clone(), prev.as_ref(), area);
            prev = Some(w);
        }
    }

    pub fn is_empty(&self) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.is_empty(),
            LayoutKind::Master => self.master.is_empty(),
        }
    }

    pub fn contains(&self, w: &W) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.contains(w),
            LayoutKind::Master => self.master.contains(w),
        }
    }

    pub fn windows(&self) -> Vec<W> {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.windows(),
            LayoutKind::Master => self.master.windows(),
        }
    }

    /// `target` is the tiled window the new one opens on (the focused one).
    pub fn insert(&mut self, new: W, target: Option<&W>, area: Rect) {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.insert(new, target, area),
            LayoutKind::Master => self.master.insert(new, target),
        }
    }

    pub fn remove(&mut self, w: &W) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.remove(w),
            LayoutKind::Master => self.master.remove(w),
        }
    }

    pub fn swap(&mut self, a: &W, b: &W) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.swap(a, b),
            LayoutKind::Master => self.master.swap(a, b),
        }
    }

    /// Puts `new` in the slot of `old`.
    pub fn replace(&mut self, old: &W, new: &W) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.replace(old, new),
            LayoutKind::Master => self.master.replace(old, new),
        }
    }

    pub fn layout(&self, area: Rect, gaps_in: i32) -> Vec<(W, Rect)> {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.layout(area, gaps_in),
            LayoutKind::Master => self.master.layout(area, gaps_in),
        }
    }

    pub fn resize(&mut self, w: &W, dx: i32, dy: i32, area: Rect) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.resize(w, dx, dy, area),
            LayoutKind::Master => self.master.resize(w, dx, dy, area),
        }
    }

    /// `splitratio`: `f` maps the old ratio (0.1-1.9) to the new one. Under
    /// master it moves `mfact` (ratio / 2), which Hyprland's master rejects.
    pub fn update_ratio(&mut self, w: &W, f: impl Fn(f32) -> f32) -> bool {
        match self.kind {
            LayoutKind::Dwindle => self.dwindle.update_ratio(w, f),
            LayoutKind::Master if self.master.contains(w) => {
                let m = f(self.master.mfact() * 2.0) / 2.0;
                self.master.set_mfact(m, true);
                true
            }
            LayoutKind::Master => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Rect = Rect::new(0, 0, 1000, 600);

    #[test]
    fn switching_layout_keeps_windows() {
        let mut t: Tiling<u32> = Tiling::default();
        for i in 1..=3 {
            t.insert(i, t.windows().last(), A);
        }
        assert_eq!(t.windows(), vec![1, 2, 3]);
        t.set_kind(LayoutKind::Master, A);
        assert_eq!(t.windows(), vec![1, 2, 3]);
        let l = t.layout(A, 0);
        assert_eq!(l[0], (1, Rect::new(0, 0, 550, 600)));
        t.set_kind(LayoutKind::Dwindle, A);
        assert_eq!(t.windows().len(), 3);
        assert!(t.contains(&2) && !t.master.contains(&2));
        assert_eq!(t.layout(A, 0).len(), 3);
    }

    #[test]
    fn splitratio_moves_mfact_under_master() {
        let mut t: Tiling<u32> = Tiling {
            kind: LayoutKind::Master,
            ..Default::default()
        };
        t.insert(1, None, A);
        t.insert(2, Some(&1), A);
        assert!(t.update_ratio(&1, |r| r + 0.2));
        assert_eq!(t.layout(A, 0)[0].1.w, 650);
    }

    #[test]
    fn parse_names() {
        assert_eq!(LayoutKind::parse("master"), LayoutKind::Master);
        assert_eq!(LayoutKind::parse("dwindle"), LayoutKind::Dwindle);
        assert_eq!(LayoutKind::parse("scrolling"), LayoutKind::Dwindle);
    }
}
