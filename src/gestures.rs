//! Touchpad gestures (`gesture = FINGERS, DIRECTION, [mod:M] [scale:S],
//! ACTION, ARGS`): pure recognition of libinput swipe/pinch sequences into
//! the configured action. `input.rs` feeds the events and runs the result.

use crate::config::{Gesture, Mods};

/// `gestures { workspace_swipe_* }` options used here.
#[derive(Debug, Clone, PartialEq)]
pub struct SwipeOpts {
    /// `gestures:workspace_swipe_distance` (px of finger travel per switch).
    pub distance: f64,
    /// `gestures:workspace_swipe_invert`: swiping left goes to the next one.
    pub invert: bool,
    /// `gestures:workspace_swipe_cancel_ratio`: fraction of `distance` needed.
    pub cancel_ratio: f64,
}

impl Default for SwipeOpts {
    fn default() -> Self {
        SwipeOpts {
            distance: 300.0,
            invert: true,
            cancel_ratio: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Swipe,
    Pinch,
}

/// One gesture in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracker {
    pub kind: Kind,
    pub fingers: u32,
    pub mods: Mods,
    pub dx: f64,
    pub dy: f64,
    /// Pinch scale (1.0 = unchanged).
    pub scale: f64,
}

/// What a finished (or ongoing, for `move`/`resize`) gesture does.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Hyprland dispatcher `name, args`.
    Dispatch(String, String),
    /// Nothing matched: the gesture belongs to the client.
    Unhandled,
}

impl Tracker {
    pub fn new(kind: Kind, fingers: u32, mods: Mods) -> Self {
        Tracker {
            kind,
            fingers,
            mods,
            dx: 0.0,
            dy: 0.0,
            scale: 1.0,
        }
    }

    /// The direction names this gesture satisfies, most specific first.
    fn directions(&self) -> Vec<&'static str> {
        match self.kind {
            Kind::Pinch => {
                let dir = if self.scale < 1.0 {
                    "pinchin"
                } else {
                    "pinchout"
                };
                vec![dir, "pinch"]
            }
            Kind::Swipe => {
                let horizontal = self.dx.abs() >= self.dy.abs();
                let exact = match (horizontal, self.dx < 0.0, self.dy < 0.0) {
                    (true, true, _) => "left",
                    (true, false, _) => "right",
                    (false, _, true) => "up",
                    (false, _, false) => "down",
                };
                let axis = if horizontal { "horizontal" } else { "vertical" };
                vec![exact, axis, "swipe"]
            }
        }
    }

    /// The configured gesture this one matches (last definition wins, like
    /// binds).
    pub fn matching<'a>(&self, gestures: &'a [Gesture]) -> Option<&'a Gesture> {
        let dirs = self.directions();
        dirs.iter().find_map(|d| {
            gestures.iter().rev().find(|g| {
                g.fingers == self.fingers
                    && g.direction == *d
                    && g.mods == self.mods
                    && g.action != "unset"
            })
        })
    }

    /// Is this a swipe that moves a window live (`move`/`resize` actions)?
    pub fn live_action<'a>(&self, gestures: &'a [Gesture]) -> Option<&'a Gesture> {
        // Live actions are matched on finger count alone: the direction is
        // only known at the end.
        gestures.iter().rev().find(|g| {
            self.kind == Kind::Swipe
                && g.fingers == self.fingers
                && g.mods == self.mods
                && matches!(g.action.as_str(), "move" | "resize")
        })
    }

    /// The action of a finished gesture.
    pub fn finish(&self, gestures: &[Gesture], opts: &SwipeOpts) -> Outcome {
        let Some(g) = self.matching(gestures) else {
            return Outcome::Unhandled;
        };
        let arg = g.args.join(", ");
        let d = |n: &str, a: &str| Outcome::Dispatch(n.into(), a.into());
        match g.action.as_str() {
            "workspace" => {
                let travel = if g.direction == "vertical"
                    || matches!(g.direction.as_str(), "up" | "down")
                    || (g.direction == "swipe" && self.dy.abs() > self.dx.abs())
                {
                    self.dy
                } else {
                    self.dx
                } * g.scale as f64;
                if travel.abs() < opts.distance * opts.cancel_ratio {
                    return Outcome::Dispatch("noop".into(), String::new());
                }
                // Fingers moving left/up: next workspace (with `invert`).
                let next = (travel < 0.0) == opts.invert;
                d("workspace", if next { "r+1" } else { "r-1" })
            }
            "dispatcher" => match g.args.split_first() {
                Some((name, rest)) => d(name.trim(), &rest.join(",")),
                None => Outcome::Unhandled,
            },
            "special" => d("togglespecialworkspace", &arg),
            "close" => d("killactive", ""),
            "fullscreen" => d(
                "fullscreen",
                if arg.trim() == "maximize" { "1" } else { "0" },
            ),
            "float" => match arg.trim() {
                "float" => d("setfloating", ""),
                "tile" => d("settiled", ""),
                _ => d("togglefloating", ""),
            },
            // Done live while the fingers move.
            "move" | "resize" => d("noop", ""),
            // shortcut: no cursor zoom and no scrolling layout yet.
            _ => Outcome::Dispatch("noop".into(), String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn gestures(src: &str) -> Vec<Gesture> {
        let (cfg, errors) = Config::parse_str(src);
        assert!(errors.is_empty(), "{errors:?}");
        cfg.gestures
    }

    fn swipe(fingers: u32, dx: f64, dy: f64) -> Tracker {
        let mut t = Tracker::new(Kind::Swipe, fingers, Mods::default());
        t.dx = dx;
        t.dy = dy;
        t
    }

    #[test]
    fn workspace_swipe_direction_and_threshold() {
        let g = gestures("gesture = 3, horizontal, workspace");
        let o = SwipeOpts::default();
        let ws = |a: &str| Outcome::Dispatch("workspace".into(), a.into());
        assert_eq!(swipe(3, -400.0, 10.0).finish(&g, &o), ws("r+1"));
        assert_eq!(swipe(3, 400.0, -10.0).finish(&g, &o), ws("r-1"));
        // Too short: cancelled.
        assert_eq!(
            swipe(3, -100.0, 0.0).finish(&g, &o),
            Outcome::Dispatch("noop".into(), String::new())
        );
        // Wrong finger count or a vertical swipe: not ours.
        assert_eq!(swipe(4, -400.0, 0.0).finish(&g, &o), Outcome::Unhandled);
        assert_eq!(swipe(3, 0.0, -400.0).finish(&g, &o), Outcome::Unhandled);
        let not_inverted = SwipeOpts {
            invert: false,
            ..Default::default()
        };
        assert_eq!(swipe(3, -400.0, 0.0).finish(&g, &not_inverted), ws("r-1"));
    }

    #[test]
    fn specific_directions_dispatchers_and_mods() {
        let g = gestures(
            "gesture = 4, up, dispatcher, exec, kitty\ngesture = 4, down, close\ngesture = 3, swipe, mod:SUPER, special, magic\ngesture = 4, pinchout, fullscreen",
        );
        let o = SwipeOpts::default();
        assert_eq!(
            swipe(4, 0.0, -50.0).finish(&g, &o),
            Outcome::Dispatch("exec".into(), "kitty".into())
        );
        assert_eq!(
            swipe(4, 5.0, 80.0).finish(&g, &o),
            Outcome::Dispatch("killactive".into(), String::new())
        );
        assert_eq!(swipe(3, 100.0, 0.0).finish(&g, &o), Outcome::Unhandled);
        let mut t = swipe(3, 100.0, 0.0);
        t.mods.logo = true;
        assert_eq!(
            t.finish(&g, &o),
            Outcome::Dispatch("togglespecialworkspace".into(), "magic".into())
        );
        let mut p = Tracker::new(Kind::Pinch, 4, Mods::default());
        p.scale = 1.6;
        assert_eq!(
            p.finish(&g, &o),
            Outcome::Dispatch("fullscreen".into(), "0".into())
        );
        p.scale = 0.5;
        assert_eq!(p.finish(&g, &o), Outcome::Unhandled);
    }

    #[test]
    fn lua_callback_gesture_and_live_move() {
        let g = gestures("gesture = 4, up, dispatcher, __lua, 3\ngesture = 3, swipe, move");
        let o = SwipeOpts::default();
        assert_eq!(
            swipe(4, 0.0, -90.0).finish(&g, &o),
            Outcome::Dispatch("__lua".into(), "3".into())
        );
        assert!(swipe(3, 1.0, 1.0).live_action(&g).is_some());
        assert!(swipe(4, 1.0, 1.0).live_action(&g).is_none());
    }
}
