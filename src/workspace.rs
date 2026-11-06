//! Workspaces: each one owns a dwindle tree of tiled windows plus a list of
//! floating windows.

use crate::layout::Dwindle;

pub const MAX_WORKSPACES: usize = 10;

#[derive(Debug, Clone)]
pub struct Workspace<W> {
    pub tiled: Dwindle<W>,
    /// Floating windows, bottom to top.
    pub floating: Vec<W>,
    pub fullscreen: Option<W>,
    pub last_focused: Option<W>,
}

impl<W> Default for Workspace<W> {
    fn default() -> Self {
        Self {
            tiled: Dwindle::default(),
            floating: Vec::new(),
            fullscreen: None,
            last_focused: None,
        }
    }
}

impl<W: PartialEq + Clone> Workspace<W> {
    pub fn contains(&self, w: &W) -> bool {
        self.tiled.contains(w) || self.floating.contains(w)
    }

    pub fn is_floating(&self, w: &W) -> bool {
        self.floating.contains(w)
    }

    /// All windows, tiled first, then floating bottom to top.
    pub fn windows(&self) -> Vec<W> {
        let mut v = self.tiled.windows();
        v.extend(self.floating.iter().cloned());
        v
    }

    /// Removes a window from the workspace, fixing up focus/fullscreen state.
    pub fn remove(&mut self, w: &W) -> bool {
        let removed = self.tiled.remove(w) || {
            let before = self.floating.len();
            self.floating.retain(|f| f != w);
            before != self.floating.len()
        };
        if self.fullscreen.as_ref() == Some(w) {
            self.fullscreen = None;
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
}
