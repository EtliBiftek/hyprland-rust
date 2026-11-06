//! Pure parsing and resolution of Hyprland's argument syntaxes: workspace
//! selectors (`3`, `+1`, `e-1`, `name:foo`, `special:bar`), window selectors
//! (`class:^(foo)$`, `address:0x...`), and `resizeactive`/`moveactive`/
//! `splitratio` arguments. No Smithay types, so everything is unit-tested.

use std::{fmt, str::FromStr};

/// Argument of `workspace` / `movetoworkspace` / `movetoworkspacesilent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsSel {
    Id(i32),
    /// `+1` / `-1`: relative to the current workspace id.
    Rel(i32),
    /// `e+1` / `e-1`: cycle through existing (non-empty) workspaces.
    Existing(i32),
    /// `m+1` / `m-1`: relative on the current monitor.
    OnMonitor(i32),
    /// `r+1` / `r-1`: open workspaces on the current monitor.
    Open(i32),
    Previous,
    Empty,
    Name(String),
    /// `special` (empty name) or `special:name`.
    Special(String),
}

fn rel(s: &str) -> Option<i32> {
    if !(s.starts_with('+') || s.starts_with('-')) {
        return None;
    }
    s.parse().ok()
}

impl FromStr for WsSel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let bad = || format!("invalid workspace `{s}`");
        match s {
            "previous" | "previous_per_monitor" => return Ok(WsSel::Previous),
            "empty" | "emptym" | "emptyn" | "emptynm" | "emptymn" => return Ok(WsSel::Empty),
            "special" => return Ok(WsSel::Special(String::new())),
            _ => {}
        }
        if let Some(n) = s.strip_prefix("special:") {
            return Ok(WsSel::Special(n.to_string()));
        }
        if let Some(n) = s.strip_prefix("name:") {
            if n.is_empty() {
                return Err(bad());
            }
            return Ok(WsSel::Name(n.to_string()));
        }
        if let Some(d) = s.strip_prefix('e').and_then(rel) {
            return Ok(WsSel::Existing(d));
        }
        if let Some(d) = s.strip_prefix('m').and_then(rel) {
            return Ok(WsSel::OnMonitor(d));
        }
        if let Some(d) = s.strip_prefix('r').and_then(rel) {
            return Ok(WsSel::Open(d));
        }
        if let Some(d) = rel(s) {
            return Ok(WsSel::Rel(d));
        }
        match s.parse::<i32>() {
            Ok(n) if n >= 1 => Ok(WsSel::Id(n)),
            _ => Err(bad()),
        }
    }
}

impl fmt::Display for WsSel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WsSel::Id(n) => write!(f, "{n}"),
            WsSel::Rel(d) => write!(f, "{d:+}"),
            WsSel::Existing(d) => write!(f, "e{d:+}"),
            WsSel::OnMonitor(d) => write!(f, "m{d:+}"),
            WsSel::Open(d) => write!(f, "r{d:+}"),
            WsSel::Previous => write!(f, "previous"),
            WsSel::Empty => write!(f, "empty"),
            WsSel::Name(n) => write!(f, "name:{n}"),
            WsSel::Special(n) if n.is_empty() => write!(f, "special"),
            WsSel::Special(n) => write!(f, "special:{n}"),
        }
    }
}

/// What a workspace selector resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Id(i32),
    Name(String),
    /// Special workspace; the empty string is the default one.
    Special(String),
}

/// A workspace as seen by the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsInfo {
    pub id: i32,
    pub windows: usize,
}

/// Resolves a selector. `cur` is the active workspace id, `all` the existing
/// regular (id >= 1) workspaces.
pub fn resolve_ws(sel: &WsSel, cur: i32, prev: Option<i32>, all: &[WsInfo]) -> Option<Target> {
    match sel {
        WsSel::Id(n) => Some(Target::Id(*n)),
        WsSel::Rel(d) | WsSel::OnMonitor(d) => {
            let n = cur.checked_add(*d)?;
            (cur >= 1 && n >= 1).then_some(Target::Id(n))
        }
        WsSel::Existing(d) | WsSel::Open(d) => {
            let mut ids: Vec<i32> = all
                .iter()
                .filter(|w| w.id >= 1 && (w.windows > 0 || w.id == cur))
                .map(|w| w.id)
                .collect();
            if !ids.contains(&cur) {
                ids.push(cur);
            }
            ids.sort_unstable();
            ids.dedup();
            let pos = ids.iter().position(|&i| i == cur)? as i64;
            let n = (pos + *d as i64).rem_euclid(ids.len() as i64);
            Some(Target::Id(ids[n as usize]))
        }
        WsSel::Previous => prev.map(Target::Id),
        WsSel::Empty => (1..)
            .find(|id| all.iter().all(|w| w.id != *id || w.windows == 0))
            .map(Target::Id),
        WsSel::Name(n) => Some(Target::Name(n.clone())),
        WsSel::Special(n) => Some(Target::Special(n.clone())),
    }
}

/// Window properties a [`WinSel`] can match on.
#[derive(Debug, Clone, Default)]
pub struct WinInfo {
    pub class: String,
    pub title: String,
    pub address: String,
    pub pid: i32,
    pub floating: bool,
}

/// Window selector used by `closewindow`, `focuswindow`, `pass`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WinSel {
    Class(String),
    Title(String),
    Address(String),
    Pid(i32),
    Floating,
    Tiled,
}

impl FromStr for WinSel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() {
            return Err("empty window selector".into());
        }
        Ok(match s.split_once(':') {
            Some(("class" | "initialclass", r)) => WinSel::Class(r.trim().to_string()),
            Some(("title" | "initialtitle", r)) => WinSel::Title(r.trim().to_string()),
            Some(("address", a)) => WinSel::Address(a.trim().to_ascii_lowercase()),
            Some(("pid", p)) => {
                WinSel::Pid(p.trim().parse().map_err(|_| format!("bad pid `{p}`"))?)
            }
            _ if s == "floating" => WinSel::Floating,
            _ if s == "tiled" => WinSel::Tiled,
            // A bare regex is matched against the class, like Hyprland.
            _ => WinSel::Class(s.to_string()),
        })
    }
}

impl WinSel {
    pub fn matches(&self, w: &WinInfo) -> bool {
        let re = |pat: &str, text: &str| regex::Regex::new(pat).is_ok_and(|r| r.is_match(text));
        match self {
            WinSel::Class(r) => re(r, &w.class),
            WinSel::Title(r) => re(r, &w.title),
            WinSel::Address(a) => {
                let strip = |s: &str| s.trim_start_matches("0x").to_ascii_lowercase();
                strip(a) == strip(&w.address)
            }
            WinSel::Pid(p) => *p == w.pid,
            WinSel::Floating => w.floating,
            WinSel::Tiled => !w.floating,
        }
    }
}

/// One coordinate of a `resizeactive` / `moveactive` argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Amount {
    Px(i32),
    /// Percent (`10%` is `Pct(10.0)`).
    Pct(f32),
}

impl Amount {
    fn parse(s: &str) -> Result<Self, String> {
        let bad = || format!("invalid amount `{s}`");
        match s.strip_suffix('%') {
            Some(p) => p.parse().map(Amount::Pct).map_err(|_| bad()),
            None => s.parse().map(Amount::Px).map_err(|_| bad()),
        }
    }

    fn of(self, base: i32) -> i32 {
        match self {
            Amount::Px(p) => p,
            Amount::Pct(p) => (base as f32 * p / 100.0).round() as i32,
        }
    }
}

/// `x y` or `exact x y`, each coordinate in pixels or percent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2Arg {
    pub exact: bool,
    pub x: Amount,
    pub y: Amount,
}

impl FromStr for Vec2Arg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut parts = s.split_whitespace().peekable();
        let exact = parts.next_if_eq(&"exact").is_some();
        let (Some(x), Some(y), None) = (parts.next(), parts.next(), parts.next()) else {
            return Err(format!("expected `[exact] X Y`, got `{s}`"));
        };
        Ok(Vec2Arg {
            exact,
            x: Amount::parse(x)?,
            y: Amount::parse(y)?,
        })
    }
}

impl Vec2Arg {
    /// Result for a current value `cur`. Exact percentages are of `monitor`;
    /// relative percentages are of `rel_base`.
    pub fn resolve(
        &self,
        cur: (i32, i32),
        rel_base: (i32, i32),
        monitor: (i32, i32),
    ) -> (i32, i32) {
        if self.exact {
            (self.x.of(monitor.0), self.y.of(monitor.1))
        } else {
            (cur.0 + self.x.of(rel_base.0), cur.1 + self.y.of(rel_base.1))
        }
    }

    /// Pixel delta (relative form only), percentages of `base`.
    pub fn delta(&self, base: (i32, i32)) -> (i32, i32) {
        (self.x.of(base.0), self.y.of(base.1))
    }
}

/// `splitratio` argument: `+0.1`, `-0.1` or `exact 0.7`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatioArg {
    pub exact: bool,
    pub value: f32,
}

impl FromStr for RatioArg {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let (exact, v) = match s.strip_prefix("exact") {
            Some(rest) => (true, rest.trim()),
            None => (false, s),
        };
        let value = v.parse().map_err(|_| format!("invalid ratio `{s}`"))?;
        Ok(RatioArg { exact, value })
    }
}

impl RatioArg {
    pub fn apply(&self, cur: f32) -> f32 {
        let v = if self.exact {
            self.value
        } else {
            cur + self.value
        };
        v.clamp(0.1, 1.9)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(id: i32, windows: usize) -> WsInfo {
        WsInfo { id, windows }
    }

    #[test]
    fn parse_workspace_selectors() {
        let p = |s: &str| s.parse::<WsSel>();
        assert_eq!(p("3"), Ok(WsSel::Id(3)));
        assert_eq!(p("+1"), Ok(WsSel::Rel(1)));
        assert_eq!(p("-2"), Ok(WsSel::Rel(-2)));
        assert_eq!(p("e+1"), Ok(WsSel::Existing(1)));
        assert_eq!(p("e-1"), Ok(WsSel::Existing(-1)));
        assert_eq!(p("m+1"), Ok(WsSel::OnMonitor(1)));
        assert_eq!(p("r-1"), Ok(WsSel::Open(-1)));
        assert_eq!(p("previous"), Ok(WsSel::Previous));
        assert_eq!(p("empty"), Ok(WsSel::Empty));
        assert_eq!(p("name:web"), Ok(WsSel::Name("web".into())));
        assert_eq!(p("special"), Ok(WsSel::Special(String::new())));
        assert_eq!(p("special:term"), Ok(WsSel::Special("term".into())));
        assert!(p("0").is_err());
        assert!(p("name:").is_err());
        assert!(p("foo").is_err());
        for s in [
            "3",
            "+1",
            "e-1",
            "m+1",
            "r+1",
            "previous",
            "name:a",
            "special:b",
        ] {
            assert_eq!(p(s).unwrap().to_string(), s);
        }
    }

    #[test]
    fn resolve_relative_and_existing() {
        let all = [ws(1, 2), ws(2, 0), ws(4, 1), ws(7, 1)];
        let r = |sel: WsSel, cur| resolve_ws(&sel, cur, Some(7), &all);
        assert_eq!(r(WsSel::Rel(1), 1), Some(Target::Id(2)));
        assert_eq!(r(WsSel::Rel(-1), 1), None);
        // Existing skips empty workspace 2 and wraps around.
        assert_eq!(r(WsSel::Existing(1), 1), Some(Target::Id(4)));
        assert_eq!(r(WsSel::Existing(1), 7), Some(Target::Id(1)));
        assert_eq!(r(WsSel::Existing(-1), 1), Some(Target::Id(7)));
        // The empty current workspace still counts as a stop.
        assert_eq!(r(WsSel::Existing(1), 2), Some(Target::Id(4)));
        assert_eq!(r(WsSel::Previous, 1), Some(Target::Id(7)));
        assert_eq!(r(WsSel::Empty, 1), Some(Target::Id(2)));
        assert_eq!(resolve_ws(&WsSel::Previous, 1, None, &all), None);
        assert_eq!(
            resolve_ws(&WsSel::Name("x".into()), 1, None, &all),
            Some(Target::Name("x".into()))
        );
    }

    #[test]
    fn window_selectors() {
        let w = WinInfo {
            class: "firefox".into(),
            title: "Mozilla Firefox".into(),
            address: "0x55800040".into(),
            pid: 42,
            floating: true,
        };
        let m = |s: &str| s.parse::<WinSel>().unwrap().matches(&w);
        assert!(m("class:^(firefox)$"));
        assert!(m("class:fire"));
        assert!(!m("class:^(kitty)$"));
        assert!(m("title:Mozilla"));
        assert!(m("address:0x55800040"));
        assert!(m("address:55800040"));
        assert!(m("pid:42"));
        assert!(!m("pid:43"));
        assert!(m("floating"));
        assert!(!m("tiled"));
        assert!(m("^firefox$"));
        assert!(!m("class:(unclosed"));
        assert!("pid:x".parse::<WinSel>().is_err());
    }

    #[test]
    fn vec2_args() {
        let a: Vec2Arg = "10 -20".parse().unwrap();
        assert_eq!(a.resolve((100, 100), (800, 600), (1920, 1080)), (110, 80));
        let a: Vec2Arg = "10% 50%".parse().unwrap();
        assert_eq!(a.resolve((100, 100), (800, 600), (1920, 1080)), (180, 400));
        let a: Vec2Arg = "exact 50% 100".parse().unwrap();
        assert_eq!(a.resolve((100, 100), (800, 600), (1920, 1080)), (960, 100));
        assert!("exact 1".parse::<Vec2Arg>().is_err());
        assert!("1 2 3".parse::<Vec2Arg>().is_err());
        assert!("a b".parse::<Vec2Arg>().is_err());
    }

    #[test]
    fn ratio_args() {
        let r: RatioArg = "+0.1".parse().unwrap();
        assert!((r.apply(1.0) - 1.1).abs() < 1e-6);
        let r: RatioArg = "exact 0.5".parse().unwrap();
        assert_eq!(r.apply(1.0), 0.5);
        let r: RatioArg = "exact 5".parse().unwrap();
        assert_eq!(r.apply(1.0), 1.9);
        assert!("abc".parse::<RatioArg>().is_err());
    }
}
