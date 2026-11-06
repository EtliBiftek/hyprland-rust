//! Workspaces: each one owns a dwindle tree of tiled windows plus a list of
//! floating windows.

use crate::{groups::Groups, tiling::Tiling};

/// Id of the default special workspace (`special`); named ones count down.
pub const SPECIAL_ID: i32 = -98;
/// Id of the first named (`name:foo`) regular workspace; further ones count down.
pub const FIRST_NAMED_ID: i32 = -1337;

#[derive(Debug, Clone)]
pub struct Workspace<W> {
    pub id: i32,
    /// `1`, `foo` for `name:foo`, `special` / `special:foo` for special ones.
    pub name: String,
    pub tiled: Tiling<W>,
    /// Groups; only the active member of each is in `tiled`/`floating`.
    pub groups: Groups<W>,
    /// Floating windows, bottom to top.
    pub floating: Vec<W>,
    pub fullscreen: Option<W>,
    /// `fullscreen, 1`: window fills the tiling area.
    pub maximized: Option<W>,
    pub last_focused: Option<W>,
}

impl<W> Default for Workspace<W> {
    fn default() -> Self {
        Self::new(1, "1".into())
    }
}

impl<W> Workspace<W> {
    pub fn new(id: i32, name: String) -> Self {
        Self {
            id,
            name,
            tiled: Tiling::default(),
            groups: Groups::default(),
            floating: Vec::new(),
            fullscreen: None,
            maximized: None,
            last_focused: None,
        }
    }

    pub fn is_special(&self) -> bool {
        self.name == "special" || self.name.starts_with("special:")
    }
}

/// Id for a new special workspace called `name` (empty = default one),
/// given the ids of the existing special workspaces.
pub fn new_special_id(name: &str, existing: impl Iterator<Item = i32>) -> i32 {
    if name.is_empty() {
        SPECIAL_ID
    } else {
        existing.min().unwrap_or(SPECIAL_ID).min(SPECIAL_ID) - 1
    }
}

/// Id for a new `name:foo` workspace, given the ids of existing named ones.
pub fn new_named_id(existing: impl Iterator<Item = i32>) -> i32 {
    existing
        .min()
        .map_or(FIRST_NAMED_ID, |m| m.min(FIRST_NAMED_ID) - 1)
}

impl<W: PartialEq + Clone> Workspace<W> {
    pub fn contains(&self, w: &W) -> bool {
        self.tiled.contains(w) || self.floating.contains(w) || self.groups.is_hidden(w)
    }

    /// A hidden group member counts as floating if its group is.
    pub fn is_floating(&self, w: &W) -> bool {
        self.floating.contains(self.groups.rep(w))
    }

    /// Windows that take part in the layout: tiled first, then floating
    /// bottom to top. One window per group.
    pub fn visible_windows(&self) -> Vec<W> {
        let mut v = self.tiled.windows();
        v.extend(self.floating.iter().cloned());
        v
    }

    /// All windows, including hidden group members.
    pub fn windows(&self) -> Vec<W> {
        let mut v = self.visible_windows();
        v.extend(self.groups.hidden());
        v
    }

    /// Puts `new` in the layout slot (and focus/fullscreen role) of `old`.
    pub fn replace_in_layout(&mut self, old: &W, new: &W) {
        if !self.tiled.replace(old, new) {
            for f in self.floating.iter_mut().filter(|f| *f == old) {
                *f = new.clone();
            }
        }
        for slot in [
            &mut self.fullscreen,
            &mut self.maximized,
            &mut self.last_focused,
        ] {
            if slot.as_ref() == Some(old) {
                *slot = Some(new.clone());
            }
        }
    }

    /// Removes a window from the workspace, fixing up focus/fullscreen state.
    /// A group member leaves its group; the group's next member takes the slot.
    pub fn remove(&mut self, w: &W) -> bool {
        if let Some(gi) = self.groups.find(w) {
            let succ = self.groups.list[gi].remove(w);
            if self.groups.list[gi].members.is_empty() {
                self.groups.list.remove(gi);
            } else {
                let to = succ.unwrap_or_else(|| self.groups.list[gi].active().clone());
                self.replace_in_layout(w, &to);
                return true;
            }
        }
        let removed = self.tiled.remove(w) || {
            let before = self.floating.len();
            self.floating.retain(|f| f != w);
            before != self.floating.len()
        };
        if self.fullscreen.as_ref() == Some(w) {
            self.fullscreen = None;
        }
        if self.maximized.as_ref() == Some(w) {
            self.maximized = None;
        }
        if self.last_focused.as_ref() == Some(w) {
            self.last_focused = self.windows().last().cloned();
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;

    #[test]
    fn remove_updates_focus_and_fullscreen() {
        let mut ws: Workspace<u32> = Workspace::default();
        let area = Rect::new(0, 0, 100, 100);
        ws.tiled.insert(1, None, area);
        ws.tiled.insert(2, None, area);
        ws.floating.push(3);
        ws.fullscreen = Some(3);
        ws.last_focused = Some(3);

        assert!(ws.remove(&3));
        assert_eq!(ws.fullscreen, None);
        assert_eq!(ws.last_focused, Some(2));
        assert_eq!(ws.windows(), vec![1, 2]);
        assert!(!ws.remove(&3));
    }

    #[test]
    fn special_and_named_ids() {
        assert_eq!(new_special_id("", [].into_iter()), SPECIAL_ID);
        assert_eq!(new_special_id("a", [].into_iter()), -99);
        assert_eq!(new_special_id("b", [-98, -99].into_iter()), -100);
        assert_eq!(new_named_id([].into_iter()), FIRST_NAMED_ID);
        assert_eq!(new_named_id([-1337].into_iter()), -1338);
        let s: Workspace<u32> = Workspace::new(-98, "special".into());
        assert!(s.is_special());
        assert!(Workspace::<u32>::new(-99, "special:x".into()).is_special());
        assert!(!Workspace::<u32>::new(-1337, "web".into()).is_special());
    }
}
