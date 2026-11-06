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

    fn center(self) -> (i32, i32) {
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

/// Decides orientation for a split node occupying `area`.
fn orientation(side_by_side: bool, locked: bool, preserve: bool, area: Rect) -> bool {
    if locked || preserve {
        side_by_side
    } else {
        area.w >= area.h
    }
}

#[derive(Debug, Clone)]
pub struct Dwindle<T> {
    root: Option<Node<T>>,
    pub preserve_split: bool,
    pub split_ratio: f32,
}

impl<T> Default for Dwindle<T> {
    fn default() -> Self {
        Self {
            root: None,
            preserve_split: false,
            split_ratio: 1.0,
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
        let preserve = self.preserve_split;
        let ratio = self.split_ratio;

        fn go<T: PartialEq + Clone>(
            node: Node<T>,
            area: Rect,
            target: &T,
            new: &T,
            preserve: bool,
            ratio: f32,
        ) -> Node<T> {
            match node {
                Node::Leaf(l) if &l == target => Node::Split {
                    side_by_side: area.w >= area.h,
                    locked: false,
                    ratio,
                    children: Box::new([Node::Leaf(l), Node::Leaf(new.clone())]),
                },
                leaf @ Node::Leaf(_) => leaf,
                Node::Split {
                    side_by_side,
                    locked,
                    ratio: r,
                    children,
                } => {
                    let o = orientation(side_by_side, locked, preserve, area);
                    let (a1, a2) = area.split(o, r);
                    let [c1, c2] = *children;
                    let (c1, c2) = if c1.contains(target) {
                        (go(c1, a1, target, new, preserve, ratio), c2)
                    } else {
                        (c1, go(c2, a2, target, new, preserve, ratio))
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

        self.root = Some(go(root, area, &target, &new, preserve, ratio));
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

    /// Flips the orientation of the split containing `w` (Hyprland's `togglesplit`).
    pub fn toggle_split(&mut self, w: &T, area: Rect) {
        let preserve = self.preserve_split;
        fn go<T: PartialEq + Clone>(node: &mut Node<T>, area: Rect, w: &T, preserve: bool) -> bool {
            let Node::Split {
                side_by_side,
                locked,
                ratio,
                children,
            } = node
            else {
                return false;
            };
            let o = orientation(*side_by_side, *locked, preserve, area);
            let (a1, a2) = area.split(o, *ratio);
            let direct_child = children
                .iter()
                .any(|c| matches!(c, Node::Leaf(l) if l == w));
            if direct_child {
                *side_by_side = !o;
                *locked = true;
                return true;
            }
            go(&mut children[0], a1, w, preserve) || go(&mut children[1], a2, w, preserve)
        }
        if let Some(root) = &mut self.root {
            go(root, area, w, preserve);
        }
    }

    /// Computes each window's rectangle. `area` should already exclude the
    /// outer gaps; every window is shrunk by `gaps_in` on each side.
    pub fn layout(&self, area: Rect, gaps_in: i32) -> Vec<(T, Rect)> {
        let mut out = Vec::new();
        fn go<T: Clone>(
            n: &Node<T>,
            area: Rect,
            gaps_in: i32,
            preserve: bool,
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
            go(root, area, gaps_in, self.preserve_split, &mut out);
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
