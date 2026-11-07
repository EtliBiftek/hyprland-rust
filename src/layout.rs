//! Hyprland's "dwindle" tiling layout: a binary tree where every new window
//! splits the focused window's area in two, along its longer side.
//!
//! The tree is generic over the window type so it can be unit-tested without
//! a running compositor.

use crate::config::Direction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    /// Shrinks the rectangle by `by` pixels on every side (never below 1x1).
    pub fn shrink(self, by: i32) -> Self {
        let w = (self.w - 2 * by).max(1);
        let h = (self.h - 2 * by).max(1);
        Rect::new(self.x + by, self.y + by, w, h)
    }

    pub fn center(self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    /// Splits into (first, second). `side_by_side` splits along the x axis.
    /// `ratio` is in 0.1..1.9, where 1.0 means two equal halves (like Hyprland).
    fn split(self, side_by_side: bool, ratio: f32) -> (Rect, Rect) {
        let ratio = ratio.clamp(0.1, 1.9) / 2.0;
        if side_by_side {
            let w1 = (self.w as f32 * ratio).round() as i32;
            (
                Rect::new(self.x, self.y, w1, self.h),
                Rect::new(self.x + w1, self.y, self.w - w1, self.h),
            )
        } else {
            let h1 = (self.h as f32 * ratio).round() as i32;
            (
                Rect::new(self.x, self.y, self.w, h1),
                Rect::new(self.x, self.y + h1, self.w, self.h - h1),
            )
        }
    }
}

#[derive(Debug, Clone)]
enum Node<T> {
    Leaf(T),
    Split {
        /// Orientation chosen when the split was created (or by `togglesplit`).
        side_by_side: bool,
        /// Set by `togglesplit`: keep `side_by_side` even without `preserve_split`.
        locked: bool,
        ratio: f32,
        children: Box<[Node<T>; 2]>,
    },
}

impl<T: PartialEq + Clone> Node<T> {
    fn contains(&self, w: &T) -> bool {
        match self {
            Node::Leaf(l) => l == w,
            Node::Split { children, .. } => children.iter().any(|c| c.contains(w)),
        }
    }

    fn last_leaf(&self) -> &T {
        match self {
            Node::Leaf(l) => l,
            Node::Split { children, .. } => children[1].last_leaf(),
        }
    }
}

/// Per-layout switches that decide a split's orientation.
#[derive(Debug, Clone, Copy)]
pub struct Opt {
    preserve: bool,
    /// `split_width_multiplier`: width is multiplied before comparing to height.
    mult: f32,
}

/// Decides orientation for a split node occupying `area`.
fn orientation(side_by_side: bool, locked: bool, opt: Opt, area: Rect) -> bool {
    if locked || opt.preserve {
        side_by_side
    } else {
        area.w as f32 * opt.mult >= area.h as f32
    }
}

struct Ins<'a, T> {
    target: &'a T,
    new: &'a T,
    opt: Opt,
    ratio: f32,
    pre: Option<Direction>,
    smart: bool,
    force: i32,
    cursor: Option<(i32, i32)>,
}

#[derive(Debug, Clone)]
pub struct Dwindle<T> {
    root: Option<Node<T>>,
    pub preserve_split: bool,
    pub split_ratio: f32,
    /// `force_split`: 0 follows the cursor, 1 new window first, 2 second.
    pub force_split: i32,
    /// `smart_split`: the cursor's triangle in the target picks the side.
    pub smart_split: bool,
    pub split_width_multiplier: f32,
    pub permanent_direction_override: bool,
    /// `layoutmsg preselect`: side for the next opened window.
    pub preselect: Option<Direction>,
    /// Cursor position used by `force_split = 0` and `smart_split`.
    pub cursor: Option<(i32, i32)>,
}

impl<T> Default for Dwindle<T> {
    fn default() -> Self {
        Self {
            root: None,
            preserve_split: false,
            split_ratio: 1.0,
            force_split: 0,
            smart_split: false,
            split_width_multiplier: 1.0,
            permanent_direction_override: false,
            preselect: None,
            cursor: None,
        }
    }
}

impl<T: PartialEq + Clone> Dwindle<T> {
    pub fn is_empty(&self) -> bool {
        self.root.is_none()
    }

    pub fn contains(&self, w: &T) -> bool {
        self.root.as_ref().is_some_and(|r| r.contains(w))
    }

    pub fn windows(&self) -> Vec<T> {
        let mut out = Vec::new();
        fn walk<T: Clone>(n: &Node<T>, out: &mut Vec<T>) {
            match n {
                Node::Leaf(l) => out.push(l.clone()),
                Node::Split { children, .. } => children.iter().for_each(|c| walk(c, out)),
            }
        }
        if let Some(r) = &self.root {
            walk(r, &mut out);
        }
        out
    }

    fn opt(&self) -> Opt {
        Opt {
            preserve: self.preserve_split || self.smart_split,
            mult: self.split_width_multiplier,
        }
    }

    /// Inserts `new` by splitting `target` (or the most recently added window
    /// if `target` is `None` or not in the tree). `area` is the space the
    /// layout covers, needed to pick the split orientation.
    pub fn insert(&mut self, new: T, target: Option<&T>, area: Rect) {
        if self.contains(&new) {
            return;
        }
        let Some(root) = self.root.take() else {
            self.root = Some(Node::Leaf(new));
            return;
        };
        let target = match target {
            Some(t) if root.contains(t) => t.clone(),
            _ => root.last_leaf().clone(),
        };
        let pre = self.preselect;
        if !self.permanent_direction_override {
            self.preselect = None;
        }
        let ins = Ins {
            target: &target,
            new: &new,
            opt: self.opt(),
            ratio: self.split_ratio,
            pre,
            smart: self.smart_split,
            force: self.force_split,
            cursor: self.cursor,
        };

        fn go<T: PartialEq + Clone>(node: Node<T>, area: Rect, i: &Ins<T>) -> Node<T> {
            match node {
                Node::Leaf(l) if &l == i.target => {
                    let inside = i.cursor.filter(|&(cx, cy)| {
                        cx >= area.x && cx < area.x + area.w && cy >= area.y && cy < area.y + area.h
                    });
                    let mut side = area.w as f32 * i.opt.mult >= area.h as f32;
                    let mut locked = false;
                    let new_first;
                    if let Some(d) = i.pre {
                        side = matches!(d, Direction::Left | Direction::Right);
                        new_first = matches!(d, Direction::Left | Direction::Up);
                        locked = true;
                    } else if let Some((cx, cy)) = inside.filter(|_| i.smart) {
                        let (mx, my) = area.center();
                        let (dx, dy) = ((cx - mx) as f32, (cy - my) as f32);
                        side = dy.abs() * (area.w as f32) < dx.abs() * (area.h as f32);
                        new_first = if side { dx <= 0.0 } else { dy <= 0.0 };
                        locked = true;
                    } else {
                        // shortcut: the cursor only counts while it is inside the
                        // target (Hyprland compares even outside); fine until pointer
                        // warping on focus exists.
                        new_first = match (i.force, inside) {
                            (1, _) => true,
                            (0, Some((cx, cy))) => {
                                if side {
                                    cx < area.x + area.w / 2
                                } else {
                                    cy < area.y + area.h / 2
                                }
                            }
                            _ => false,
                        };
                    }
                    let (a, b) = (Node::Leaf(l), Node::Leaf(i.new.clone()));
                    Node::Split {
                        side_by_side: side,
                        locked,
                        ratio: i.ratio,
                        children: Box::new(if new_first { [b, a] } else { [a, b] }),
                    }
                }
                leaf @ Node::Leaf(_) => leaf,
                Node::Split {
                    side_by_side,
                    locked,
                    ratio: r,
                    children,
                } => {
                    let o = orientation(side_by_side, locked, i.opt, area);
                    let (a1, a2) = area.split(o, r);
                    let [c1, c2] = *children;
                    let (c1, c2) = if c1.contains(i.target) {
                        (go(c1, a1, i), c2)
                    } else {
                        (c1, go(c2, a2, i))
                    };
                    Node::Split {
                        side_by_side,
                        locked,
                        ratio: r,
                        children: Box::new([c1, c2]),
                    }
                }
            }
        }

        self.root = Some(go(root, area, &ins));
    }

    /// Removes a window; its sibling takes over the freed space.
    pub fn remove(&mut self, w: &T) -> bool {
        let Some(root) = self.root.take() else {
            return false;
        };
        if !root.contains(w) {
            self.root = Some(root);
            return false;
        }

        fn go<T: PartialEq + Clone>(node: Node<T>, w: &T) -> Option<Node<T>> {
            match node {
                Node::Leaf(l) if &l == w => None,
                leaf @ Node::Leaf(_) => Some(leaf),
                Node::Split {
                    side_by_side,
                    locked,
                    ratio,
                    children,
                } => {
                    let [c1, c2] = *children;
                    match (go(c1, w), go(c2, w)) {
                        (Some(a), Some(b)) => Some(Node::Split {
                            side_by_side,
                            locked,
                            ratio,
                            children: Box::new([a, b]),
                        }),
                        (Some(only), None) | (None, Some(only)) => Some(only),
                        (None, None) => None,
                    }
                }
            }
        }

        self.root = go(root, w);
        true
    }

    /// Runs `f(node, side_by_side)` on the split that directly contains leaf
    /// `w`; `side_by_side` is the split's current orientation for `area`.
    fn with_parent(
        &mut self,
        w: &T,
        area: Rect,
        f: &mut dyn FnMut(&mut Node<T>, bool) -> bool,
    ) -> bool {
        let opt = self.opt();
        fn go<T: PartialEq + Clone>(
            node: &mut Node<T>,
            area: Rect,
            w: &T,
            opt: Opt,
            f: &mut dyn FnMut(&mut Node<T>, bool) -> bool,
        ) -> bool {
            let Node::Split {
                side_by_side,
                locked,
                ratio,
                children,
            } = node
            else {
                return false;
            };
            let o = orientation(*side_by_side, *locked, opt, area);
            let (a1, a2) = area.split(o, *ratio);
            if children
                .iter()
                .any(|c| matches!(c, Node::Leaf(l) if l == w))
            {
                return f(node, o);
            }
            go(&mut children[0], a1, w, opt, f) || go(&mut children[1], a2, w, opt, f)
        }
        self.root.as_mut().is_some_and(|r| go(r, area, w, opt, f))
    }

    /// Flips the orientation of the split containing `w` (Hyprland's `togglesplit`).
    pub fn toggle_split(&mut self, w: &T, area: Rect) -> bool {
        self.with_parent(w, area, &mut |n, o| {
            if let Node::Split {
                side_by_side,
                locked,
                ..
            } = n
            {
                *side_by_side = !o;
                *locked = true;
            }
            true
        })
    }

    /// `swapsplit`: swaps the two halves of the split containing `w`.
    pub fn swap_split(&mut self, w: &T, area: Rect) -> bool {
        self.with_parent(w, area, &mut |n, _| {
            if let Node::Split { children, .. } = n {
                children.swap(0, 1);
            }
            true
        })
    }

    /// `rotatesplit`: rotates the split containing `w` by a multiple of 90 degrees.
    pub fn rotate_split(&mut self, w: &T, angle: i32, area: Rect) -> bool {
        let turns = (angle / 90).rem_euclid(4);
        self.with_parent(w, area, &mut |n, o| {
            if let Node::Split {
                side_by_side,
                locked,
                children,
                ..
            } = n
            {
                // `o` true means side by side, i.e. not Hyprland's `splitTop`.
                let swap = match turns {
                    1 => !o,
                    2 => true,
                    3 => o,
                    _ => false,
                };
                if turns == 1 || turns == 3 {
                    *side_by_side = !o;
                    *locked = true;
                }
                if swap {
                    children.swap(0, 1);
                }
            }
            true
        })
    }

    /// `movetoroot`: makes `w` a direct child of the root. `stable` keeps it
    /// on the same side of the screen. False if it already is one (or alone).
    pub fn move_to_root(&mut self, w: &T, stable: bool) -> bool {
        fn slot<'a, T: PartialEq>(n: &'a mut Node<T>, w: &T) -> Option<&'a mut Node<T>> {
            match n {
                Node::Leaf(l) if l == w => Some(n),
                Node::Leaf(_) => None,
                Node::Split { children, .. } => {
                    let [a, b] = &mut **children;
                    slot(a, w).or_else(|| slot(b, w))
                }
            }
        }
        let Some(Node::Split { children, .. }) = &mut self.root else {
            return false;
        };
        if children
            .iter()
            .any(|c| matches!(c, Node::Leaf(l) if l == w))
        {
            return false;
        }
        let idx = if children[0].contains(w) { 0 } else { 1 };
        if !children[idx].contains(w) {
            return false;
        }
        let (a, b) = children.split_at_mut(1);
        let (anc, other) = if idx == 0 {
            (&mut a[0], &mut b[0])
        } else {
            (&mut b[0], &mut a[0])
        };
        if let Some(s) = slot(anc, w) {
            std::mem::swap(s, other);
        }
        if stable {
            children.swap(0, 1);
        }
        true
    }

    /// Puts `new` where `old` is (`old` must be in the tree, `new` not).
    pub fn replace(&mut self, old: &T, new: &T) -> bool {
        fn go<T: PartialEq + Clone>(n: &mut Node<T>, old: &T, new: &T) -> bool {
            match n {
                Node::Leaf(l) if l == old => {
                    *l = new.clone();
                    true
                }
                Node::Leaf(_) => false,
                Node::Split { children, .. } => children.iter_mut().any(|c| go(c, old, new)),
            }
        }
        self.root.as_mut().is_some_and(|r| go(r, old, new))
    }

    /// Swaps the positions of two windows in the tree.
    pub fn swap(&mut self, a: &T, b: &T) -> bool {
        if a == b || !self.contains(a) || !self.contains(b) {
            return false;
        }
        fn go<T: PartialEq + Clone>(n: &mut Node<T>, a: &T, b: &T) {
            match n {
                Node::Leaf(l) if *l == *a => *l = b.clone(),
                Node::Leaf(l) if *l == *b => *l = a.clone(),
                Node::Leaf(_) => {}
                Node::Split { children, .. } => children.iter_mut().for_each(|c| go(c, a, b)),
            }
        }
        if let Some(root) = &mut self.root {
            go(root, a, b);
        }
        true
    }

    /// Changes the ratio of the split directly containing `w` (Hyprland's
    /// `splitratio`): `f` maps the old ratio (0.1-1.9) to the new one.
    pub fn update_ratio(&mut self, w: &T, f: impl Fn(f32) -> f32) -> bool {
        fn go<T: PartialEq>(n: &mut Node<T>, w: &T, f: &dyn Fn(f32) -> f32) -> bool {
            let Node::Split {
                ratio, children, ..
            } = n
            else {
                return false;
            };
            if children
                .iter()
                .any(|c| matches!(c, Node::Leaf(l) if l == w))
            {
                *ratio = f(*ratio).clamp(0.1, 1.9);
                return true;
            }
            children.iter_mut().any(|c| go(c, w, f))
        }
        self.root.as_mut().is_some_and(|r| go(r, w, &f))
    }

    /// Resizes tiled window `w` by `(dx, dy)` pixels (grow if positive) by
    /// moving the edge of the split that directly contains it. Only the axis
    /// of that split has an effect. `area` is the area passed to `layout`.
    pub fn resize(&mut self, w: &T, dx: i32, dy: i32, area: Rect) -> bool {
        let preserve = self.opt();
        fn go<T: PartialEq>(
            n: &mut Node<T>,
            area: Rect,
            w: &T,
            d: (i32, i32),
            preserve: Opt,
        ) -> bool {
            let Node::Split {
                side_by_side,
                locked,
                ratio,
                children,
            } = n
            else {
                return false;
            };
            let o = orientation(*side_by_side, *locked, preserve, area);
            if let Some(first) = children
                .iter()
                .position(|c| matches!(c, Node::Leaf(l) if l == w))
            {
                let (delta, extent) = if o { (d.0, area.w) } else { (d.1, area.h) };
                if extent > 0 {
                    // Growing the first child grows the ratio, the second shrinks it.
                    let sign = if first == 0 { 1.0 } else { -1.0 };
                    *ratio = (*ratio + sign * 2.0 * delta as f32 / extent as f32).clamp(0.1, 1.9);
                }
                return true;
            }
            let (a1, a2) = area.split(o, *ratio);
            let [c1, c2] = &mut **children;
            go(c1, a1, w, d, preserve) || go(c2, a2, w, d, preserve)
        }
        self.root
            .as_mut()
            .is_some_and(|r| go(r, area, w, (dx, dy), preserve))
    }

    /// Computes each window's rectangle. `area` should already exclude the
    /// outer gaps; every window is shrunk by `gaps_in` on each side.
    pub fn layout(&self, area: Rect, gaps_in: i32) -> Vec<(T, Rect)> {
        let mut out = Vec::new();
        fn go<T: Clone>(
            n: &Node<T>,
            area: Rect,
            gaps_in: i32,
            preserve: Opt,
            out: &mut Vec<(T, Rect)>,
        ) {
            match n {
                Node::Leaf(l) => out.push((l.clone(), area.shrink(gaps_in))),
                Node::Split {
                    side_by_side,
                    locked,
                    ratio,
                    children,
                } => {
                    let o = orientation(*side_by_side, *locked, preserve, area);
                    let (a1, a2) = area.split(o, *ratio);
                    go(&children[0], a1, gaps_in, preserve, out);
                    go(&children[1], a2, gaps_in, preserve, out);
                }
            }
        }
        if let Some(root) = &self.root {
            go(root, area, gaps_in, self.opt(), &mut out);
        }
        out
    }
}

/// Finds the window closest to `from` in direction `dir`, given a computed layout.
pub fn find_in_direction<T: PartialEq + Clone>(
    rects: &[(T, Rect)],
    from: &T,
    dir: Direction,
) -> Option<T> {
    let (_, src) = rects.iter().find(|(w, _)| w == from)?;
    let (sx, sy) = src.center();
    rects
        .iter()
        .filter(|(w, _)| w != from)
        .filter(|(_, r)| match dir {
            Direction::Left => r.x + r.w <= src.x,
            Direction::Right => r.x >= src.x + src.w,
            Direction::Up => r.y + r.h <= src.y,
            Direction::Down => r.y >= src.y + src.h,
        })
        .min_by_key(|(_, r)| {
            let (cx, cy) = r.center();
            let (dx, dy) = ((cx - sx) as i64, (cy - sy) as i64);
            dx * dx + dy * dy
        })
        .map(|(w, _)| w.clone())
}

/// The four border rectangles (top, bottom, left, right) of width `b` drawn
/// outside `r`; the top and bottom ones cover the corners.
#[allow(dead_code)]
pub fn border_sides(r: Rect, b: i32) -> [Rect; 4] {
    [
        Rect::new(r.x - b, r.y - b, r.w + 2 * b, b),
        Rect::new(r.x - b, r.y + r.h, r.w + 2 * b, b),
        Rect::new(r.x - b, r.y, b, r.h),
        Rect::new(r.x + r.w, r.y, b, r.h),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect::new(0, 0, 1000, 500);

    fn rect_of(d: &Dwindle<u32>, w: u32) -> Rect {
        d.layout(AREA, 0)
            .into_iter()
            .find(|(x, _)| *x == w)
            .unwrap()
            .1
    }

    #[test]
    fn single_window_fills_area() {
        let mut d = Dwindle::default();
        d.insert(1, None, AREA);
        assert_eq!(d.layout(AREA, 0), vec![(1, AREA)]);
        assert_eq!(d.layout(AREA, 5), vec![(1, Rect::new(5, 5, 990, 490))]);
    }

    #[test]
    fn dwindle_spiral() {
        let mut d = Dwindle::default();
        d.insert(1, None, AREA);
        d.insert(2, Some(&1), AREA); // wide -> side by side
        assert_eq!(rect_of(&d, 1), Rect::new(0, 0, 500, 500));
        assert_eq!(rect_of(&d, 2), Rect::new(500, 0, 500, 500));
        d.insert(3, Some(&2), AREA); // 500x500 -> side by side (w >= h)
        assert_eq!(rect_of(&d, 3), Rect::new(750, 0, 250, 500));
        d.insert(4, Some(&3), AREA); // 250x500 tall -> stacked
        assert_eq!(rect_of(&d, 3), Rect::new(750, 0, 250, 250));
        assert_eq!(rect_of(&d, 4), Rect::new(750, 250, 250, 250));
    }

    #[test]
    fn remove_gives_space_to_sibling() {
        let mut d = Dwindle::default();
        for (w, t) in [(1, None), (2, Some(1)), (3, Some(2))] {
            d.insert(w, t.as_ref(), AREA);
        }
        assert!(d.remove(&2));
        assert_eq!(rect_of(&d, 3), Rect::new(500, 0, 500, 500));
        assert!(d.remove(&1));
        assert_eq!(rect_of(&d, 3), AREA);
        assert!(d.remove(&3));
        assert!(d.is_empty());
        assert!(!d.remove(&3));
    }

    #[test]
    fn toggle_split_flips_orientation() {
        let mut d = Dwindle::default();
        d.insert(1, None, AREA);
        d.insert(2, None, AREA);
        d.toggle_split(&2, AREA);
        assert_eq!(rect_of(&d, 1), Rect::new(0, 0, 1000, 250));
        assert_eq!(rect_of(&d, 2), Rect::new(0, 250, 1000, 250));
    }

    #[test]
    fn orientation_follows_resize_without_preserve_split() {
        let mut d = Dwindle::default();
        d.insert(1, None, AREA);
        d.insert(2, None, AREA);
        let tall = Rect::new(0, 0, 500, 1000);
        let l = d.layout(tall, 0);
        assert_eq!(l[1].1, Rect::new(0, 500, 500, 500));

        d.preserve_split = true;
        let l = d.layout(tall, 0);
        assert_eq!(l[1].1, Rect::new(250, 0, 250, 1000));
    }

    #[test]
    fn swap_ratio_and_resize() {
        let mut d = Dwindle::default();
        d.insert(1, None, AREA);
        d.insert(2, None, AREA);
        assert!(d.swap(&1, &2));
        assert_eq!(rect_of(&d, 1), Rect::new(500, 0, 500, 500));
        assert!(!d.swap(&1, &9));
        assert!(d.swap(&1, &2));

        assert!(d.update_ratio(&1, |r| r + 0.2));
        assert_eq!(rect_of(&d, 1), Rect::new(0, 0, 600, 500));
        assert!(d.update_ratio(&2, |_| 5.0));
        assert_eq!(rect_of(&d, 1).w, 950);
        assert!(!d.update_ratio(&9, |r| r));

        assert!(d.update_ratio(&1, |_| 1.0));
        // Growing the second window by 100px moves the edge left by 100px.
        assert!(d.resize(&2, 100, 0, AREA));
        assert_eq!(rect_of(&d, 2).w, 600);
        // Vertical delta has no effect on a side-by-side split.
        assert!(d.resize(&2, 0, 100, AREA));
        assert_eq!(rect_of(&d, 2).w, 600);
        assert!(!d.resize(&9, 1, 1, AREA));
    }

    #[test]
    fn direction_lookup() {
        let mut d = Dwindle::default();
        d.insert(1, None, AREA);
        d.insert(2, Some(&1), AREA);
        d.insert(3, Some(&2), Rect::new(0, 0, 1000, 1000)); // ensure stacked split of 2
        let rects = d.layout(Rect::new(0, 0, 1000, 1000), 0);
        assert_eq!(find_in_direction(&rects, &1, Direction::Right), Some(2));
        assert_eq!(find_in_direction(&rects, &2, Direction::Down), Some(3));
        assert_eq!(find_in_direction(&rects, &3, Direction::Up), Some(2));
        assert_eq!(find_in_direction(&rects, &1, Direction::Left), None);
    }

    #[test]
    fn border_sides_surround_rect() {
        let [t, b, l, r] = border_sides(Rect::new(10, 20, 100, 50), 2);
        assert_eq!(t, Rect::new(8, 18, 104, 2));
        assert_eq!(b, Rect::new(8, 70, 104, 2));
        assert_eq!(l, Rect::new(8, 20, 2, 50));
        assert_eq!(r, Rect::new(110, 20, 2, 50));
    }
}
