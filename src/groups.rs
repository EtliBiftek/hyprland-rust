//! Window groups (tabbed containers). A group takes one slot in the layout:
//! only its active member is in the layout tree (and mapped), the others are
//! hidden. Membership logic follows `CGroup` of Hyprland v0.57.1 and is kept
//! free of Smithay types.

#[derive(Debug, Clone, PartialEq)]
pub struct Group<W> {
    pub members: Vec<W>,
    /// Index of the visible member.
    pub current: usize,
    /// `lockactivegroup`: no windows may enter or leave via dispatchers.
    pub locked: bool,
    /// `denywindowfromgroup`: windows may not be added to this group.
    pub denied: bool,
}

impl<W: PartialEq + Clone> Group<W> {
    pub fn new(w: W) -> Self {
        Self {
            members: vec![w],
            current: 0,
            locked: false,
            denied: false,
        }
    }

    pub fn active(&self) -> &W {
        &self.members[self.current]
    }

    pub fn contains(&self, w: &W) -> bool {
        self.members.contains(w)
    }

    /// Adds `w` and makes it the active member: after the current one
    /// (`group:insert_after_current`) or at the tail.
    pub fn add(&mut self, w: W, after_current: bool) {
        if self.contains(&w) {
            return;
        }
        if after_current {
            self.members.insert(self.current + 1, w);
            self.current += 1;
        } else {
            self.members.push(w);
            self.current = self.members.len() - 1;
        }
    }

    /// Removes `w`. Returns the new active member if `w` was the active one
    /// and others remain. Keeps `current` on the same window otherwise, and
    /// moves to the previous member when the active one goes.
    pub fn remove(&mut self, w: &W) -> Option<W> {
        let idx = self.members.iter().position(|m| m == w)?;
        let was_active = idx == self.current;
        let len = self.members.len();
        if (self.current >= idx && idx != 0) || (self.current >= len - 1 && self.current > 0) {
            self.current -= 1;
        }
        self.members.remove(idx);
        (was_active && !self.members.is_empty()).then(|| self.active().clone())
    }

    /// Makes member `idx` active; returns the previous active member if changed.
    pub fn set_current(&mut self, idx: usize) -> Option<W> {
        if idx >= self.members.len() || idx == self.current {
            return None;
        }
        let old = self.active().clone();
        self.current = idx;
        Some(old)
    }

    /// `changegroupactive`: next or previous member, wrapping.
    pub fn step(&mut self, forward: bool) -> Option<W> {
        let n = self.members.len();
        let idx = if forward {
            (self.current + 1) % n
        } else {
            (self.current + n - 1) % n
        };
        self.set_current(idx)
    }

    /// `movegroupwindow`: moves the active member one place (wrapping).
    pub fn move_active(&mut self, forward: bool) {
        let n = self.members.len();
        if n < 2 {
            return;
        }
        let j = if forward {
            (self.current + 1) % n
        } else {
            (self.current + n - 1) % n
        };
        self.members.swap(self.current, j);
        self.current = j;
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Groups<W> {
    pub list: Vec<Group<W>>,
}

impl<W> Default for Groups<W> {
    fn default() -> Self {
        Self { list: Vec::new() }
    }
}

impl<W: PartialEq + Clone> Groups<W> {
    /// Index of the group `w` belongs to.
    pub fn find(&self, w: &W) -> Option<usize> {
        self.list.iter().position(|g| g.contains(w))
    }

    pub fn group_of(&self, w: &W) -> Option<&Group<W>> {
        self.find(w).map(|i| &self.list[i])
    }

    /// Members that are not the visible one of their group.
    pub fn hidden(&self) -> Vec<W> {
        self.list
            .iter()
            .flat_map(|g| g.members.iter().filter(|m| *m != g.active()).cloned())
            .collect()
    }

    pub fn is_hidden(&self, w: &W) -> bool {
        self.group_of(w).is_some_and(|g| g.active() != w)
    }

    /// The window that stands for `w` in the layout (its group's active one).
    pub fn rep<'a>(&'a self, w: &'a W) -> &'a W {
        self.group_of(w).map_or(w, |g| g.active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(ms: &[u32], cur: usize) -> Group<u32> {
        Group {
            members: ms.to_vec(),
            current: cur,
            locked: false,
            denied: false,
        }
    }

    #[test]
    fn add_after_current_or_tail() {
        let mut a = Group::new(1);
        a.add(2, true);
        assert_eq!((a.members.clone(), *a.active()), (vec![1, 2], 2));
        a.add(3, true);
        a.set_current(0);
        a.add(4, true);
        assert_eq!(a.members, vec![1, 4, 2, 3]);
        assert_eq!(*a.active(), 4);
        a.add(5, false);
        assert_eq!(a.members, vec![1, 4, 2, 3, 5]);
        assert_eq!(*a.active(), 5);
        a.add(5, false);
        assert_eq!(a.members.len(), 5);
    }

    #[test]
    fn remove_adjusts_current() {
        // removing the active member goes to the previous one
        let mut a = g(&[1, 2, 3], 1);
        assert_eq!(a.remove(&2), Some(1));
        // removing the first active member goes to what slid into its place
        let mut a = g(&[1, 2, 3], 0);
        assert_eq!(a.remove(&1), Some(2));
        // removing a hidden member keeps the active window
        let mut a = g(&[1, 2, 3], 2);
        assert_eq!(a.remove(&1), None);
        assert_eq!(*a.active(), 3);
        let mut a = g(&[1, 2, 3], 1);
        assert_eq!(a.remove(&3), None);
        assert_eq!(*a.active(), 2);
        // the last member leaves an empty group
        let mut a = Group::new(7);
        assert_eq!(a.remove(&7), None);
        assert!(a.members.is_empty());
        assert_eq!(a.remove(&7), None);
    }

    #[test]
    fn stepping_and_index() {
        let mut a = g(&[1, 2, 3], 2);
        assert_eq!(a.step(true), Some(3));
        assert_eq!(*a.active(), 1);
        assert_eq!(a.step(false), Some(1));
        assert_eq!(*a.active(), 3);
        assert_eq!(a.set_current(1), Some(3));
        assert_eq!(a.set_current(1), None);
        assert_eq!(a.set_current(9), None);
        assert_eq!(Group::new(1).step(true), None);
    }

    #[test]
    fn move_active_swaps_with_neighbour() {
        let mut a = g(&[1, 2, 3], 0);
        a.move_active(true);
        assert_eq!(a.members, vec![2, 1, 3]);
        assert_eq!(*a.active(), 1);
        a.move_active(false);
        a.move_active(false);
        assert_eq!(a.members, vec![3, 2, 1]);
        assert_eq!(*a.active(), 1);
    }

    #[test]
    fn groups_lookup() {
        let gs = Groups {
            list: vec![g(&[1, 2], 0), g(&[3], 0)],
        };
        assert_eq!(gs.find(&2), Some(0));
        assert_eq!(gs.find(&9), None);
        assert_eq!(gs.hidden(), vec![2]);
        assert!(gs.is_hidden(&2) && !gs.is_hidden(&1) && !gs.is_hidden(&9));
        assert_eq!(*gs.rep(&2), 1);
        assert_eq!(*gs.rep(&9), 9);
    }
}
