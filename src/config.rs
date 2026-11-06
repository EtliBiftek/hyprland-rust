//! A parser for a subset of the hyprlang configuration syntax used by Hyprland.
//!
//! Supported so far:
//! - `# comments`
//! - variables: `$mod = SUPER`, used later as `$mod`
//! - categories: `general { gaps_in = 5 }` (nesting allowed) and the
//!   `general:gaps_in = 5` shorthand
//! - `bind = MODS, key, dispatcher, args`
//! - `exec-once = command` and `exec = command`
//! - `source = path` (relative to the including file)

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    master::{MasterCfg, NewOnActive, NewStatus, Orientation},
    selectors::{Amount, RatioArg, Vec2Arg, WinSel, WsSel},
};

/// Modifier mask used by key bindings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl Mods {
    /// Parses strings like `SUPER SHIFT`, `SUPER_SHIFT`, `CTRL+ALT`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut mods = Mods::default();
        for part in s.split([' ', '_', '+', '&']).filter(|p| !p.is_empty()) {
            match part.to_ascii_uppercase().as_str() {
                "SHIFT" => mods.shift = true,
                "CTRL" | "CONTROL" => mods.ctrl = true,
                "ALT" | "MOD1" => mods.alt = true,
                "SUPER" | "WIN" | "LOGO" | "MOD4" | "META" => mods.logo = true,
                other => return Err(format!("unknown modifier `{other}`")),
            }
        }
        Ok(mods)
    }
}

/// A direction for `movefocus` / `movewindow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "l" | "left" => Ok(Direction::Left),
            "r" | "right" => Ok(Direction::Right),
            "u" | "up" | "t" | "top" => Ok(Direction::Up),
            "d" | "down" | "b" | "bottom" => Ok(Direction::Down),
            other => Err(format!("unknown direction `{other}`")),
        }
    }
}

/// Argument of `changegroupactive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStep {
    Forward,
    Backward,
    /// 1-based; 0 or below selects the last member.
    Index(i32),
}

/// `lock` / `unlock` / `toggle` argument of the group lock dispatchers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Enable,
    Disable,
    Flip,
}

impl Toggle {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "" | "toggle" => Ok(Toggle::Flip),
            "lock" | "enable" | "on" | "1" | "true" => Ok(Toggle::Enable),
            "unlock" | "disable" | "off" | "0" | "false" => Ok(Toggle::Disable),
            other => Err(format!("invalid argument `{other}`")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Toggle::Enable => "lock",
            Toggle::Disable => "unlock",
            Toggle::Flip => "toggle",
        }
    }

    /// New value of a flag currently `cur`.
    pub fn apply(self, cur: bool) -> bool {
        match self {
            Toggle::Enable => true,
            Toggle::Disable => false,
            Toggle::Flip => !cur,
        }
    }
}

/// Actions a key binding can trigger (named after Hyprland's dispatchers).
#[derive(Debug, Clone, PartialEq)]
pub enum Dispatcher {
    Exec(String),
    KillActive,
    ForceKillActive,
    Exit,
    Workspace(WsSel),
    MoveToWorkspace(WsSel),
    MoveToWorkspaceSilent(WsSel),
    ToggleFloating,
    SetFloating,
    SetTiled,
    /// `fullscreen` (mode 0, and 2 which we treat the same).
    Fullscreen,
    /// `fullscreen, 1`: fill the tiling area, keep gaps and borders.
    Maximize,
    MoveFocus(Direction),
    CycleNext,
    ToggleSplit,
    MoveWindow(Direction),
    SwapWindow(Direction),
    ResizeActive(Vec2Arg),
    MoveActive(Vec2Arg),
    CenterWindow,
    Pin,
    FocusCurrentOrLast,
    FocusUrgentOrLast,
    SplitRatio(RatioArg),
    /// `layoutmsg <message> [args]` for the layout of the focused workspace.
    LayoutMsg(String),
    ToggleGroup,
    ChangeGroupActive(GroupStep),
    MoveIntoGroup(Direction),
    /// Direction is optional.
    MoveOutOfGroup(Option<Direction>),
    MoveWindowOrGroup(Direction),
    /// `true` moves the window backward (`movegroupwindow, b`).
    MoveGroupWindow(bool),
    LockGroups(Toggle),
    LockActiveGroup(Toggle),
    DenyWindowFromGroup(Toggle),
    SetIgnoreGroupLock(Toggle),
    /// `swapnext`; `true` for `swapnext prev`.
    SwapNext(bool),
    CloseWindow(WinSel),
    FocusWindow(WinSel),
    /// Forwards the triggering key to the selected window (key binds only).
    Pass(WinSel),
    /// Empty name for the default special workspace.
    ToggleSpecialWorkspace(String),
    /// Empty name resets to the global submap.
    Submap(String),
    /// `tagwindow, +tag [window]`: add, remove (`-`) or toggle a static tag.
    TagWindow(String, Option<WinSel>),
    /// `bindm ... movewindow`: drag the window under the pointer.
    MouseMove,
    /// `bindm ... resizewindow`: resize the window under the pointer.
    MouseResize,
    /// `renameworkspace, ID NAME`.
    RenameWorkspace(i32, String),
    /// `bringactivetotop`: raise the focused floating window.
    BringActiveToTop,
    /// `alterzorder top|bottom [window]`; `true` for top.
    AlterZOrder(bool, Option<WinSel>),
    /// `movecursortocorner N`: 0 bottom-left, 1 bottom-right, 2 top-right, 3 top-left.
    MoveCursorToCorner(u8),
    /// `movecursor X Y` (absolute, layout coordinates).
    MoveCursor(i32, i32),
    /// `togglepseudo`: keep the window's own size, centered in its slot.
    TogglePseudo,
    /// `setprop [window|active] PROP VALUE`; `None` is the focused window.
    SetProp(Option<WinSel>, String, String),
    /// `killwindow [window]`: kill the client process.
    KillWindow(Option<WinSel>),
    /// `signal N [window]`: send a signal to a window's process.
    Signal(i32, Option<WinSel>),
    /// `event DATA`: emits `custom>>DATA` on the event socket.
    Event(String),
    /// `dpms on|off|toggle`.
    Dpms(Toggle),
    /// Accepted for compatibility, no effect on a single monitor or yet.
    Noop(&'static str, String),
}

/// Strips the `[rule; rule]` prefix of `exec`. Rules are accepted but ignored.
pub fn strip_exec_rules(cmd: &str) -> &str {
    let t = cmd.trim_start();
    match t.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((_, rest)) => rest.trim_start(),
        None => t,
    }
}

impl Dispatcher {
    /// Hyprland name and argument, as `hyprctl binds` shows them.
    pub fn name_arg(&self) -> (&'static str, String) {
        let dir = |d: &Direction| {
            match d {
                Direction::Left => "l",
                Direction::Right => "r",
                Direction::Up => "u",
                Direction::Down => "d",
            }
            .to_string()
        };
        let sel = |s: &WinSel| match s {
            WinSel::Class(c) => format!("class:{c}"),
            WinSel::Title(t) => format!("title:{t}"),
            WinSel::Address(a) => format!("address:{a}"),
            WinSel::Pid(p) => format!("pid:{p}"),
            WinSel::Floating => "floating".into(),
            WinSel::Tiled => "tiled".into(),
        };
        let vec2 = |v: &Vec2Arg| {
            let amount = |a: &Amount| match a {
                Amount::Px(p) => p.to_string(),
                Amount::Pct(p) => format!("{p}%"),
            };
            format!(
                "{}{} {}",
                if v.exact { "exact " } else { "" },
                amount(&v.x),
                amount(&v.y)
            )
        };
        match self {
            Dispatcher::Exec(c) => ("exec", c.clone()),
            Dispatcher::KillActive => ("killactive", String::new()),
            Dispatcher::ForceKillActive => ("forcekillactive", String::new()),
            Dispatcher::Exit => ("exit", String::new()),
            Dispatcher::Workspace(s) => ("workspace", s.to_string()),
            Dispatcher::MoveToWorkspace(s) => ("movetoworkspace", s.to_string()),
            Dispatcher::MoveToWorkspaceSilent(s) => ("movetoworkspacesilent", s.to_string()),
            Dispatcher::ToggleFloating => ("togglefloating", String::new()),
            Dispatcher::SetFloating => ("setfloating", String::new()),
            Dispatcher::SetTiled => ("settiled", String::new()),
            Dispatcher::Fullscreen => ("fullscreen", String::new()),
            Dispatcher::Maximize => ("fullscreen", "1".into()),
            Dispatcher::MoveFocus(d) => ("movefocus", dir(d)),
            Dispatcher::CycleNext => ("cyclenext", String::new()),
            Dispatcher::ToggleSplit => ("togglesplit", String::new()),
            Dispatcher::MoveWindow(d) => ("movewindow", dir(d)),
            Dispatcher::SwapWindow(d) => ("swapwindow", dir(d)),
            Dispatcher::ResizeActive(v) => ("resizeactive", vec2(v)),
            Dispatcher::MoveActive(v) => ("moveactive", vec2(v)),
            Dispatcher::CenterWindow => ("centerwindow", String::new()),
            Dispatcher::Pin => ("pin", String::new()),
            Dispatcher::FocusCurrentOrLast => ("focuscurrentorlast", String::new()),
            Dispatcher::FocusUrgentOrLast => ("focusurgentorlast", String::new()),
            Dispatcher::SplitRatio(r) => (
                "splitratio",
                format!("{}{}", if r.exact { "exact " } else { "" }, r.value),
            ),
            Dispatcher::LayoutMsg(m) => ("layoutmsg", m.clone()),
            Dispatcher::ToggleGroup => ("togglegroup", String::new()),
            Dispatcher::ChangeGroupActive(s) => (
                "changegroupactive",
                match s {
                    GroupStep::Forward => "f".into(),
                    GroupStep::Backward => "b".into(),
                    GroupStep::Index(i) => i.to_string(),
                },
            ),
            Dispatcher::MoveIntoGroup(d) => ("moveintogroup", dir(d)),
            Dispatcher::MoveOutOfGroup(d) => {
                ("moveoutofgroup", d.as_ref().map_or_else(String::new, dir))
            }
            Dispatcher::MoveWindowOrGroup(d) => ("movewindoworgroup", dir(d)),
            Dispatcher::MoveGroupWindow(back) => (
                "movegroupwindow",
                if *back { "b".into() } else { String::new() },
            ),
            Dispatcher::LockGroups(t) => ("lockgroups", t.name().into()),
            Dispatcher::LockActiveGroup(t) => ("lockactivegroup", t.name().into()),
            Dispatcher::DenyWindowFromGroup(t) => ("denywindowfromgroup", t.name().into()),
            Dispatcher::SetIgnoreGroupLock(t) => ("setignoregrouplock", t.name().into()),
            Dispatcher::SwapNext(p) => ("swapnext", if *p { "prev".into() } else { String::new() }),
            Dispatcher::CloseWindow(s) => ("closewindow", sel(s)),
            Dispatcher::FocusWindow(s) => ("focuswindow", sel(s)),
            Dispatcher::Pass(s) => ("pass", sel(s)),
            Dispatcher::ToggleSpecialWorkspace(n) => ("togglespecialworkspace", n.clone()),
            Dispatcher::Submap(n) if n.is_empty() => ("submap", "reset".into()),
            Dispatcher::Submap(n) => ("submap", n.clone()),
            Dispatcher::TagWindow(t, None) => ("tagwindow", t.clone()),
            Dispatcher::TagWindow(t, Some(s)) => ("tagwindow", format!("{t} {}", sel(s))),
            Dispatcher::MouseMove => ("movewindow", String::new()),
            Dispatcher::MouseResize => ("resizewindow", String::new()),
            Dispatcher::RenameWorkspace(id, n) => ("renameworkspace", format!("{id} {n}")),
            Dispatcher::BringActiveToTop => ("bringactivetotop", String::new()),
            Dispatcher::AlterZOrder(top, s) => (
                "alterzorder",
                format!(
                    "{}{}",
                    if *top { "top" } else { "bottom" },
                    s.as_ref()
                        .map_or_else(String::new, |s| format!(", {}", sel(s)))
                ),
            ),
            Dispatcher::MoveCursorToCorner(c) => ("movecursortocorner", c.to_string()),
            Dispatcher::MoveCursor(x, y) => ("movecursor", format!("{x} {y}")),
            Dispatcher::TogglePseudo => ("togglepseudo", String::new()),
            Dispatcher::SetProp(s, p, v) => (
                "setprop",
                format!(
                    "{} {p} {v}",
                    s.as_ref().map_or_else(|| "active".into(), sel)
                ),
            ),
            Dispatcher::KillWindow(s) => ("killwindow", s.as_ref().map_or_else(String::new, sel)),
            Dispatcher::Signal(n, Some(s)) => ("signalwindow", format!("{n}, {}", sel(s))),
            Dispatcher::Signal(n, None) => ("signal", n.to_string()),
            Dispatcher::Event(d) => ("event", d.clone()),
            Dispatcher::Dpms(t) => ("dpms", t.name().into()),
            Dispatcher::Noop(name, arg) => (*name, arg.clone()),
        }
    }

    pub fn parse(name: &str, arg: &str) -> Result<Self, String> {
        let arg = arg.trim();
        let dir = || Direction::parse(arg);
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "exec" => Dispatcher::Exec(arg.to_string()),
            "killactive" => Dispatcher::KillActive,
            "forcekillactive" => Dispatcher::ForceKillActive,
            "exit" => Dispatcher::Exit,
            "workspace" | "focusworkspaceoncurrentmonitor" => Dispatcher::Workspace(arg.parse()?),
            "movetoworkspace" => Dispatcher::MoveToWorkspace(arg.parse()?),
            "movetoworkspacesilent" => Dispatcher::MoveToWorkspaceSilent(arg.parse()?),
            "togglefloating" => Dispatcher::ToggleFloating,
            "setfloating" => Dispatcher::SetFloating,
            "settiled" => Dispatcher::SetTiled,
            "fullscreen" => match arg {
                "" | "0" | "2" => Dispatcher::Fullscreen,
                "1" => Dispatcher::Maximize,
                other => return Err(format!("invalid fullscreen mode `{other}`")),
            },
            "movefocus" => Dispatcher::MoveFocus(dir()?),
            "cyclenext" => Dispatcher::CycleNext,
            "togglesplit" => Dispatcher::ToggleSplit,
            "movewindow" => Dispatcher::MoveWindow(dir()?),
            "swapwindow" => Dispatcher::SwapWindow(dir()?),
            "resizeactive" => Dispatcher::ResizeActive(arg.parse()?),
            "moveactive" => Dispatcher::MoveActive(arg.parse()?),
            "centerwindow" => Dispatcher::CenterWindow,
            "pin" => Dispatcher::Pin,
            "focuscurrentorlast" => Dispatcher::FocusCurrentOrLast,
            "focusurgentorlast" => Dispatcher::FocusUrgentOrLast,
            "splitratio" => Dispatcher::SplitRatio(arg.parse()?),
            "layoutmsg" if arg.is_empty() => return Err("layoutmsg: missing message".into()),
            "layoutmsg" => Dispatcher::LayoutMsg(arg.to_string()),
            "togglegroup" => Dispatcher::ToggleGroup,
            "changegroupactive" => Dispatcher::ChangeGroupActive(match arg {
                "" | "f" | "forward" => GroupStep::Forward,
                "b" | "back" | "backward" => GroupStep::Backward,
                n => GroupStep::Index(n.parse().map_err(|_| format!("invalid index `{n}`"))?),
            }),
            "moveintogroup" => Dispatcher::MoveIntoGroup(dir()?),
            "moveoutofgroup" => {
                Dispatcher::MoveOutOfGroup(if arg.is_empty() { None } else { Some(dir()?) })
            }
            "movewindoworgroup" => Dispatcher::MoveWindowOrGroup(dir()?),
            "movegroupwindow" => Dispatcher::MoveGroupWindow(matches!(arg, "b" | "back")),
            "lockgroups" => Dispatcher::LockGroups(Toggle::parse(arg)?),
            "lockactivegroup" => Dispatcher::LockActiveGroup(Toggle::parse(arg)?),
            "denywindowfromgroup" => Dispatcher::DenyWindowFromGroup(Toggle::parse(arg)?),
            "setignoregrouplock" => Dispatcher::SetIgnoreGroupLock(Toggle::parse(arg)?),
            "swapnext" => Dispatcher::SwapNext(matches!(arg, "prev" | "p" | "previous")),
            "closewindow" => Dispatcher::CloseWindow(arg.parse()?),
            "focuswindow" => Dispatcher::FocusWindow(arg.parse()?),
            "pass" => Dispatcher::Pass(arg.parse()?),
            "togglespecialworkspace" => Dispatcher::ToggleSpecialWorkspace(arg.to_string()),
            "submap" => Dispatcher::Submap(if arg == "reset" {
                String::new()
            } else {
                arg.to_string()
            }),
            "tagwindow" => {
                let (tag, rest) = match arg.split_once(char::is_whitespace) {
                    Some((t, r)) => (t, r.trim()),
                    None => (arg, ""),
                };
                if tag.trim_start_matches(['+', '-']).is_empty() {
                    return Err("tagwindow: missing tag".into());
                }
                let sel = if rest.is_empty() {
                    None
                } else {
                    Some(rest.parse()?)
                };
                Dispatcher::TagWindow(tag.to_string(), sel)
            }
            "renameworkspace" => {
                let (id, name) = split_word(arg);
                let id = id
                    .parse::<i32>()
                    .map_err(|_| format!("renameworkspace: invalid id `{id}`"))?;
                if name.is_empty() {
                    return Err("renameworkspace: missing name".into());
                }
                Dispatcher::RenameWorkspace(id, name.to_string())
            }
            "bringactivetotop" => Dispatcher::BringActiveToTop,
            "alterzorder" => {
                let (mode, rest) = split_word(arg);
                let top = match mode {
                    "top" => true,
                    "bottom" => false,
                    other => return Err(format!("alterzorder: invalid mode `{other}`")),
                };
                Dispatcher::AlterZOrder(top, opt_sel(rest)?)
            }
            "movecursortocorner" => match arg.parse::<u8>() {
                Ok(c @ 0..=3) => Dispatcher::MoveCursorToCorner(c),
                _ => return Err(format!("movecursortocorner: invalid corner `{arg}`")),
            },
            "movecursor" => {
                let (x, rest) = split_word(arg);
                let (y, _) = split_word(rest);
                match (x.parse::<i32>(), y.parse::<i32>()) {
                    (Ok(x), Ok(y)) => Dispatcher::MoveCursor(x, y),
                    _ => return Err(format!("movecursor: expected `X Y`, got `{arg}`")),
                }
            }
            "togglepseudo" => Dispatcher::TogglePseudo,
            "setprop" => {
                let (sel, rest) = split_word(arg);
                let (prop, value) = split_word(rest);
                if prop.is_empty() || value.is_empty() {
                    return Err("setprop: expected `[window] PROP VALUE`".into());
                }
                let sel = if sel == "active" {
                    None
                } else {
                    Some(sel.parse::<WinSel>()?)
                };
                Dispatcher::SetProp(sel, prop.to_string(), value.to_string())
            }
            "killwindow" => Dispatcher::KillWindow(opt_sel(arg)?),
            "signal" | "signalwindow" => {
                let (sig, rest) = split_word(arg);
                let sig: i32 = sig
                    .parse()
                    .map_err(|_| format!("{name}: invalid signal `{sig}`"))?;
                Dispatcher::Signal(sig, opt_sel(rest)?)
            }
            "event" => Dispatcher::Event(arg.to_string()),
            "dpms" => Dispatcher::Dpms(Toggle::parse(split_word(arg).0)?),
            "focusmonitor" => Dispatcher::Noop("focusmonitor", arg.to_string()),
            "movecurrentworkspacetomonitor" => {
                Dispatcher::Noop("movecurrentworkspacetomonitor", arg.to_string())
            }
            "moveworkspacetomonitor" => Dispatcher::Noop("moveworkspacetomonitor", arg.to_string()),
            "swapactiveworkspaces" => Dispatcher::Noop("swapactiveworkspaces", arg.to_string()),
            "toggleswallow" => Dispatcher::Noop("toggleswallow", arg.to_string()),
            "forcerendererreload" => Dispatcher::Noop("forcerendererreload", arg.to_string()),
            other => return Err(format!("unknown dispatcher `{other}`")),
        })
    }
}

/// Splits off the first word; commas and spaces both separate arguments.
fn split_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(|c: char| c.is_whitespace() || c == ',') {
        Some(i) => (
            &s[..i],
            s[i..].trim_matches(|c: char| c.is_whitespace() || c == ','),
        ),
        None => (s, ""),
    }
}

/// Optional window selector argument; empty means the focused window.
fn opt_sel(s: &str) -> Result<Option<WinSel>, String> {
    let s = s.trim_matches(|c: char| c.is_whitespace() || c == ',');
    if s.is_empty() {
        Ok(None)
    } else {
        s.parse().map(Some)
    }
}

/// Flags from the letters after `bind` (`bindel` = locked + repeat).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BindFlags {
    /// `l`: works while locked (we have no lock screen yet, so always true).
    pub locked: bool,
    /// `r`: fire on key release.
    pub release: bool,
    /// `e`: repeat while held.
    pub repeat: bool,
    /// `n`: also forward the key to the client.
    pub non_consuming: bool,
    /// `m`: mouse bind (`bindm`).
    pub mouse: bool,
    /// `t`: not blocked by shortcut inhibitors (accepted, we have none).
    pub transparent: bool,
    /// `i`: ignore modifiers.
    pub ignore_mods: bool,
    /// `o`: long press (approximated by a normal press).
    pub long_press: bool,
    /// `d`: has a description argument.
    pub has_description: bool,
}

impl BindFlags {
    /// Parses a config keyword such as `bind`, `bindel`, `bindm`.
    pub fn from_keyword(key: &str) -> Option<Self> {
        let letters = key.strip_prefix("bind")?;
        let mut f = BindFlags::default();
        for c in letters.chars() {
            match c {
                'l' => f.locked = true,
                'r' => f.release = true,
                'e' => f.repeat = true,
                'n' => f.non_consuming = true,
                'm' => f.mouse = true,
                't' => f.transparent = true,
                'i' => f.ignore_mods = true,
                'o' => f.long_press = true,
                'd' => f.has_description = true,
                // Accepted, no effect: separate, click, drag, ...
                's' | 'c' | 'g' | 'k' | 'p' => {}
                _ => return None,
            }
        }
        Some(f)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bind {
    pub mods: Mods,
    /// Lower-cased xkb keysym name (`return`, `q`, `1`), `code:N` or `mouse:N`.
    pub key: String,
    pub dispatcher: Dispatcher,
    pub flags: BindFlags,
    pub description: String,
    /// Submap this bind belongs to; empty for the global one.
    pub submap: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct General {
    pub gaps_in: i32,
    pub gaps_out: i32,
    pub border_size: i32,
    /// RGBA, 0.0-1.0
    pub active_border_color: [f32; 4],
    pub inactive_border_color: [f32; 4],
    /// `general:layout`: `dwindle` or `master`.
    pub layout: String,
}

impl Default for General {
    fn default() -> Self {
        // Same defaults as Hyprland.
        Self {
            gaps_in: 5,
            gaps_out: 20,
            border_size: 1,
            active_border_color: [1.0, 1.0, 1.0, 1.0],
            inactive_border_color: [0.27, 0.27, 0.27, 1.0],
            layout: "dwindle".into(),
        }
    }
}

/// `workspace = <id|name:x>, layout:master, layoutopt:orientation:top`.
/// Only the parts that matter for layout are kept.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceRule {
    /// Workspace name: `3` or `foo` (for `name:foo`).
    pub workspace: String,
    pub layout: Option<String>,
    pub orientation: Option<Orientation>,
}

/// Hyprland's `0xAARRGGBB` as RGBA floats.
const fn argb(v: u32) -> [f32; 4] {
    [
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
        (v >> 24) as f32 / 255.0,
    ]
}

/// `group:groupbar { }`. Titles are not rendered (no text rendering yet), so
/// the font options are accepted and ignored.
#[derive(Debug, Clone, PartialEq)]
pub struct Groupbar {
    pub enabled: bool,
    pub height: i32,
    pub gaps_in: i32,
    pub disable_when_only: bool,
    pub col_active: [f32; 4],
    pub col_inactive: [f32; 4],
    pub col_locked_active: [f32; 4],
    pub col_locked_inactive: [f32; 4],
}

/// `group { }`.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupCfg {
    pub auto_group: bool,
    pub insert_after_current: bool,
    pub focus_removed_window: bool,
    pub border_active: [f32; 4],
    pub border_inactive: [f32; 4],
    pub border_locked_active: [f32; 4],
    pub border_locked_inactive: [f32; 4],
    pub groupbar: Groupbar,
}

impl Default for GroupCfg {
    fn default() -> Self {
        Self {
            auto_group: true,
            insert_after_current: true,
            focus_removed_window: true,
            border_active: argb(0x66ffff00),
            border_inactive: argb(0x66777700),
            border_locked_active: argb(0x66ff5500),
            border_locked_inactive: argb(0x66775500),
            groupbar: Groupbar {
                enabled: true,
                height: 14,
                gaps_in: 2,
                disable_when_only: false,
                col_active: argb(0x66ffff00),
                col_inactive: argb(0x66777700),
                col_locked_active: argb(0x66ff5500),
                col_locked_inactive: argb(0x66775500),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decoration {
    pub rounding: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Dwindle {
    /// Keep the split orientation chosen when a window was opened, even if the
    /// container is later resized.
    pub preserve_split: bool,
    /// Split ratio, 0.1-1.9 like Hyprland (1.0 = half).
    pub default_split_ratio: f32,
    /// 0: new window goes where the cursor is, 1: first, 2: second.
    pub force_split: i32,
    pub smart_split: bool,
    /// Accepted; only matters for mouse resizing, which tiled windows lack.
    pub smart_resizing: bool,
    pub permanent_direction_override: bool,
    pub special_scale_factor: f32,
    pub split_width_multiplier: f32,
    pub use_active_for_splits: bool,
}

impl Default for Dwindle {
    fn default() -> Self {
        Self {
            preserve_split: false,
            default_split_ratio: 1.0,
            force_split: 0,
            smart_split: false,
            smart_resizing: true,
            permanent_direction_override: false,
            special_scale_factor: 1.0,
            split_width_multiplier: 1.0,
            use_active_for_splits: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccelProfile {
    Flat,
    Adaptive,
}

/// Pointer-device options. `None` means "leave the libinput default / inherit".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceOpts {
    /// -1.0..=1.0
    pub sensitivity: Option<f64>,
    pub accel_profile: Option<AccelProfile>,
    pub natural_scroll: Option<bool>,
    pub left_handed: Option<bool>,
    pub tap_to_click: Option<bool>,
    pub disable_while_typing: Option<bool>,
    pub scroll_factor: Option<f32>,
}

impl DeviceOpts {
    /// Fields set in `other` win.
    fn overlay(&mut self, other: &DeviceOpts) {
        self.sensitivity = other.sensitivity.or(self.sensitivity);
        self.accel_profile = other.accel_profile.or(self.accel_profile);
        self.natural_scroll = other.natural_scroll.or(self.natural_scroll);
        self.left_handed = other.left_handed.or(self.left_handed);
        self.tap_to_click = other.tap_to_click.or(self.tap_to_click);
        self.disable_while_typing = other.disable_while_typing.or(self.disable_while_typing);
        self.scroll_factor = other.scroll_factor.or(self.scroll_factor);
    }

    /// Sets one option by its Hyprland name. `Ok(false)` if the name is unknown.
    fn set(&mut self, key: &str, value: &str) -> Result<bool, String> {
        match key {
            "sensitivity" => {
                let v: f64 = value
                    .parse()
                    .map_err(|_| format!("invalid float `{value}`"))?;
                self.sensitivity = Some(v.clamp(-1.0, 1.0));
            }
            "accel_profile" => {
                self.accel_profile = match value.split_whitespace().next().unwrap_or("") {
                    "flat" => Some(AccelProfile::Flat),
                    "adaptive" => Some(AccelProfile::Adaptive),
                    // `custom ...` is not supported; keep libinput's default.
                    "custom" | "" => None,
                    other => return Err(format!("invalid accel_profile `{other}`")),
                }
            }
            "natural_scroll" => self.natural_scroll = Some(parse_bool(value)?),
            "left_handed" => self.left_handed = Some(parse_bool(value)?),
            "tap-to-click" => self.tap_to_click = Some(parse_bool(value)?),
            "disable_while_typing" => self.disable_while_typing = Some(parse_bool(value)?),
            "scroll_factor" => {
                self.scroll_factor = Some(
                    value
                        .parse()
                        .map_err(|_| format!("invalid float `{value}`"))?,
                )
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// A `device { name = ...; ... }` block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceOverride {
    pub name: String,
    pub opts: DeviceOpts,
}

/// libinput/Hyprland device names compare lower-cased with `-` for spaces.
fn norm_device_name(s: &str) -> String {
    s.to_lowercase().replace(' ', "-")
}

/// The `input { }` category.
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub kb_layout: String,
    pub kb_variant: String,
    pub kb_model: String,
    pub kb_options: String,
    pub kb_rules: String,
    pub repeat_rate: i32,
    pub repeat_delay: i32,
    pub numlock_by_default: bool,
    /// 0 = click to focus, 1 = focus follows the pointer, 2 = like 0 (we do
    /// not separate hover from keyboard focus), 3 = pointer never changes focus.
    pub follow_mouse: i32,
    /// Options for every pointer device (`input:sensitivity`, ...).
    pub pointer: DeviceOpts,
    /// `touchpad { }`, applied on top of `pointer` for touchpads.
    pub touchpad: DeviceOpts,
    pub devices: Vec<DeviceOverride>,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            kb_layout: String::new(),
            kb_variant: String::new(),
            kb_model: String::new(),
            kb_options: String::new(),
            kb_rules: String::new(),
            repeat_rate: 25,
            repeat_delay: 600,
            numlock_by_default: false,
            follow_mouse: 1,
            pointer: DeviceOpts::default(),
            touchpad: DeviceOpts::default(),
            devices: Vec::new(),
        }
    }
}

impl Input {
    /// Applies `input:*` and `device:*` keys. Unknown keys are ignored (many
    /// well-known options are not implemented yet).
    fn apply(&mut self, key: &str, value: &str) -> Result<(), String> {
        if let Some(k) = key.strip_prefix("device:") {
            if k == "name" {
                self.devices.push(DeviceOverride {
                    name: norm_device_name(value),
                    opts: DeviceOpts::default(),
                });
                return Ok(());
            }
            let dev = self
                .devices
                .last_mut()
                .ok_or("device option before `name`")?;
            if !dev.opts.set(k, value)? {
                tracing::debug!("config option `{key}` is not implemented yet, ignoring");
            }
            return Ok(());
        }
        let k = key.strip_prefix("input:").unwrap_or(key);
        if let Some(k) = k.strip_prefix("touchpad:") {
            if !self.touchpad.set(k, value)? {
                tracing::debug!("config option `{key}` is not implemented yet, ignoring");
            }
            return Ok(());
        }
        match k {
            "kb_layout" => self.kb_layout = value.into(),
            "kb_variant" => self.kb_variant = value.into(),
            "kb_model" => self.kb_model = value.into(),
            "kb_options" => self.kb_options = value.into(),
            "kb_rules" => self.kb_rules = value.into(),
            "repeat_rate" => self.repeat_rate = parse_int(value)?.max(0),
            "repeat_delay" => self.repeat_delay = parse_int(value)?.max(0),
            "numlock_by_default" => self.numlock_by_default = parse_bool(value)?,
            "follow_mouse" => self.follow_mouse = parse_int(value)?.clamp(0, 3),
            _ => {
                if !self.pointer.set(k, value)? {
                    tracing::debug!("config option `{key}` is not implemented yet, ignoring");
                }
            }
        }
        Ok(())
    }

    /// Effective options for a libinput device: global, then touchpad
    /// (with Hyprland's touchpad defaults), then `device { }` overrides.
    pub fn opts_for(&self, device_name: &str, touchpad: bool) -> DeviceOpts {
        let mut o = self.pointer.clone();
        if touchpad {
            // Touchpads ignore `input:natural_scroll`; they have their own.
            o.natural_scroll = Some(false);
            o.tap_to_click = Some(true);
            o.disable_while_typing = Some(true);
            o.overlay(&self.touchpad);
        }
        let name = norm_device_name(device_name);
        for d in self.devices.iter().filter(|d| d.name == name) {
            o.overlay(&d.opts);
        }
        o
    }

    /// Scroll factor for finger (touchpad) scrolling.
    pub fn touchpad_scroll_factor(&self) -> f64 {
        self.touchpad.scroll_factor.unwrap_or(1.0) as f64
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Misc {
    pub disable_autoreload: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Config {
    pub misc: Misc,
    /// Config file plus every `source`d file that was read (for autoreload).
    pub files: Vec<PathBuf>,
    /// Option keys (lowercase `cat:key`) the user set explicitly (`getoption` `set`).
    pub set_options: HashSet<String>,
    pub general: General,
    pub decoration: Decoration,
    pub dwindle: Dwindle,
    pub master: MasterCfg,
    pub group: GroupCfg,
    /// `binds:ignore_group_lock`.
    pub ignore_group_lock: bool,
    pub workspace_rules: Vec<WorkspaceRule>,
    pub input: Input,
    pub binds: Vec<Bind>,
    pub exec_once: Vec<String>,
    pub exec: Vec<String>,
    /// `windowrule` / `layerrule` (see `rules.rs`).
    pub rules: crate::rules::Rules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.file.display(), self.line, self.message)
    }
}

impl Config {
    /// The default config location: `$XDG_CONFIG_HOME/hypr/hyprland.conf`.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        // Like Hyprland 0.55-0.56: hyprland.lua wins over hyprland.conf.
        let dir = base.join("hypr");
        let lua = dir.join("hyprland.lua");
        Some(if lua.exists() {
            lua
        } else {
            dir.join("hyprland.conf")
        })
    }

    /// Loads a config file. Errors on individual lines are collected and
    /// returned alongside the config, so one typo doesn't discard everything.
    pub fn load(path: &Path) -> std::io::Result<(Config, Vec<ConfigError>)> {
        let text = std::fs::read_to_string(path)?;
        let mut parser = Parser::default();
        if path.extension().is_some_and(|e| e == "lua") {
            let dir = path.parent().unwrap_or(Path::new("."));
            let name = path.display().to_string();
            match crate::lua_config::to_hyprlang(&text, dir, &name) {
                Ok((hyprlang, lua_errors)) => {
                    parser.parse(&hyprlang, path);
                    parser
                        .errors
                        .extend(lua_errors.into_iter().map(|message| ConfigError {
                            file: path.to_path_buf(),
                            line: 0,
                            message,
                        }));
                }
                Err(message) => parser.errors.push(ConfigError {
                    file: path.to_path_buf(),
                    line: 0,
                    message,
                }),
            }
        } else {
            parser.parse(&text, path);
        }
        Ok((parser.config, parser.errors))
    }

    pub fn parse_str(text: &str) -> (Config, Vec<ConfigError>) {
        let mut parser = Parser::default();
        parser.parse(text, Path::new("<string>"));
        (parser.config, parser.errors)
    }

    /// Bindings used when no config file exists.
    /// Applies one config line at runtime (`hyprctl keyword <key> <value>`).
    /// On error the config may be partially changed only for `source`.
    pub fn apply_keyword(&mut self, key: &str, value: &str) -> Result<(), String> {
        let mut parser = Parser {
            config: std::mem::take(self),
            ..Default::default()
        };
        let r = parser.apply(
            &key.trim().to_ascii_lowercase(),
            value.trim(),
            Path::new("."),
        );
        *self = parser.config;
        r
    }

    pub fn with_default_binds() -> Config {
        let (config, errors) = Config::parse_str(DEFAULT_CONFIG);
        debug_assert!(errors.is_empty(), "{errors:?}");
        config
    }
}

pub const DEFAULT_CONFIG: &str = r#"
$mod = SUPER
$terminal = kitty

bind = $mod, Return, exec, $terminal
bind = $mod, Q, killactive,
bind = $mod SHIFT, E, exit,
bind = $mod, V, togglefloating,
bind = $mod, F, fullscreen,
bind = $mod, J, togglesplit,
bind = $mod, Tab, cyclenext,
bind = $mod, left, movefocus, l
bind = $mod, right, movefocus, r
bind = $mod, up, movefocus, u
bind = $mod, down, movefocus, d
bind = $mod, 1, workspace, 1
bind = $mod, 2, workspace, 2
bind = $mod, 3, workspace, 3
bind = $mod, 4, workspace, 4
bind = $mod, 5, workspace, 5
bind = $mod, 6, workspace, 6
bind = $mod, 7, workspace, 7
bind = $mod, 8, workspace, 8
bind = $mod, 9, workspace, 9
bind = $mod, 0, workspace, 10
bind = $mod SHIFT, 1, movetoworkspace, 1
bind = $mod SHIFT, 2, movetoworkspace, 2
bind = $mod SHIFT, 3, movetoworkspace, 3
bind = $mod SHIFT, 4, movetoworkspace, 4
bind = $mod SHIFT, 5, movetoworkspace, 5
bind = $mod SHIFT, 6, movetoworkspace, 6
bind = $mod SHIFT, 7, movetoworkspace, 7
bind = $mod SHIFT, 8, movetoworkspace, 8
bind = $mod SHIFT, 9, movetoworkspace, 9
bind = $mod SHIFT, 0, movetoworkspace, 10
"#;

#[derive(Default)]
struct Parser {
    config: Config,
    errors: Vec<ConfigError>,
    vars: HashMap<String, String>,
    depth: usize,
    /// Current `submap = name` block; binds are tagged with it.
    submap: String,
    /// `(key, value)` lines of the open `windowrule { }` / `layerrule { }` block.
    block: Vec<(String, String)>,
}

impl Parser {
    fn parse(&mut self, text: &str, file: &Path) {
        if !self.config.files.iter().any(|f| f == file) && file != Path::new("<string>") {
            self.config.files.push(file.to_path_buf());
        }
        let mut categories: Vec<String> = Vec::new();

        for (idx, raw_line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }

            // End of a rule block: build the rule from the collected lines.
            if line == "}"
                && matches!(
                    categories.last().map(String::as_str),
                    Some("windowrule" | "layerrule")
                )
            {
                let kind = categories.pop().unwrap_or_default();
                self.finish_block(&kind, file, line_no);
                continue;
            }

            let mut err = |message: String| {
                self.errors.push(ConfigError {
                    file: file.to_path_buf(),
                    line: line_no,
                    message,
                })
            };

            if line == "}" {
                if categories.pop().is_none() {
                    err("unexpected `}`".into());
                }
                continue;
            }

            if let Some(name) = line.strip_suffix('{') {
                categories.push(name.trim().to_ascii_lowercase());
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                err(format!("expected `key = value`, found `{line}`"));
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            if let Some(var) = key.strip_prefix('$') {
                let expanded = self.expand(value);
                self.vars.insert(var.to_string(), expanded);
                continue;
            }

            let value = self.expand(value);
            let mut full_key = categories.join(":");
            if !full_key.is_empty() {
                full_key.push(':');
            }
            full_key.push_str(&key.to_ascii_lowercase());

            if let Err(message) = self.apply(&full_key, &value, file) {
                self.errors.push(ConfigError {
                    file: file.to_path_buf(),
                    line: line_no,
                    message,
                });
            }
        }

        if !categories.is_empty() {
            self.errors.push(ConfigError {
                file: file.to_path_buf(),
                line: text.lines().count(),
                message: format!("unclosed category `{}`", categories.join(":")),
            });
        }
    }

    fn finish_block(&mut self, kind: &str, file: &Path, line: usize) {
        let pairs = std::mem::take(&mut self.block);
        let rules = &mut self.config.rules;
        let r = if kind == "windowrule" {
            crate::rules::parse_window_pairs(&pairs).map(|r| rules.window.push(r))
        } else {
            crate::rules::parse_layer_pairs(&pairs).map(|r| rules.layer.push(r))
        };
        if let Err(message) = r {
            self.errors.push(ConfigError {
                file: file.to_path_buf(),
                line,
                message,
            });
        }
    }

    /// Replaces `$name` with variable values, longest names first so that
    /// `$mainMod` isn't clobbered by `$main`.
    fn expand(&self, value: &str) -> String {
        let mut names: Vec<&String> = self.vars.keys().collect();
        names.sort_by_key(|n| std::cmp::Reverse(n.len()));
        let mut out = value.to_string();
        for name in names {
            out = out.replace(&format!("${name}"), &self.vars[name]);
        }
        out
    }

    fn apply(&mut self, key: &str, value: &str, file: &Path) -> Result<(), String> {
        self.apply_inner(key, value, file)?;
        self.config.set_options.insert(key.to_string());
        Ok(())
    }

    fn apply_inner(&mut self, key: &str, value: &str, file: &Path) -> Result<(), String> {
        let cfg = &mut self.config;
        match key {
            k if BindFlags::from_keyword(k).is_some() => {
                let flags = BindFlags::from_keyword(k).unwrap_or_default();
                let mut bind = parse_bind(value, flags)?;
                bind.submap = self.submap.clone();
                cfg.binds.push(bind);
            }
            "unbind" => {
                let (mods, key) = value
                    .split_once(',')
                    .ok_or("unbind: expected `MODS, key`")?;
                let (mods, key) = (Mods::parse(mods)?, key.trim().to_ascii_lowercase());
                let before = cfg.binds.len();
                cfg.binds
                    .retain(|b| !(b.mods == mods && b.key == key && b.submap == self.submap));
                if cfg.binds.len() == before {
                    tracing::debug!("unbind: no bind matches `{value}`");
                }
            }
            "submap" => {
                self.submap = if value == "reset" {
                    String::new()
                } else {
                    value.to_string()
                }
            }
            k if k.starts_with("input:") || k.starts_with("device:") => {
                cfg.input.apply(k, value)?
            }
            "windowrule" | "windowrulev2" => cfg
                .rules
                .window
                .push(crate::rules::parse_inline_window(value)?),
            "layerrule" => cfg
                .rules
                .layer
                .push(crate::rules::parse_inline_layer(value)?),
            k if k.starts_with("windowrule:") || k.starts_with("layerrule:") => {
                // Line inside a `windowrule { }` block.
                let (_, key) = k.split_once(':').unwrap_or_default();
                self.block.push((key.to_string(), value.to_string()));
            }
            "exec-once" => cfg.exec_once.push(value.to_string()),
            "exec" => cfg.exec.push(value.to_string()),
            "source" => {
                let path = expand_home(value);
                let path = if path.is_relative() {
                    file.parent().unwrap_or(Path::new(".")).join(path)
                } else {
                    path
                };
                if self.depth > 16 {
                    return Err("`source` nested too deeply".into());
                }
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
                self.depth += 1;
                self.parse(&text, &path);
                self.depth -= 1;
            }
            "general:gaps_in" => cfg.general.gaps_in = parse_int(value)?,
            "general:gaps_out" => cfg.general.gaps_out = parse_int(value)?,
            "general:border_size" => cfg.general.border_size = parse_int(value)?,
            "general:col.active_border" => cfg.general.active_border_color = parse_color(value)?,
            "general:col.inactive_border" => {
                cfg.general.inactive_border_color = parse_color(value)?
            }
            "misc:disable_autoreload" => cfg.misc.disable_autoreload = parse_bool(value)?,
            "decoration:rounding" => cfg.decoration.rounding = parse_int(value)?,
            "dwindle:preserve_split" => cfg.dwindle.preserve_split = parse_bool(value)?,
            "dwindle:default_split_ratio" => {
                let r: f32 = value
                    .parse()
                    .map_err(|_| format!("invalid float `{value}`"))?;
                cfg.dwindle.default_split_ratio = r.clamp(0.1, 1.9);
            }
            "general:layout" => cfg.general.layout = value.to_string(),
            "dwindle:force_split" => cfg.dwindle.force_split = parse_int(value)?.clamp(0, 2),
            "dwindle:smart_split" => cfg.dwindle.smart_split = parse_bool(value)?,
            "dwindle:smart_resizing" => cfg.dwindle.smart_resizing = parse_bool(value)?,
            "dwindle:permanent_direction_override" => {
                cfg.dwindle.permanent_direction_override = parse_bool(value)?
            }
            "dwindle:special_scale_factor" => {
                cfg.dwindle.special_scale_factor = parse_float(value)?.clamp(0.0, 1.0)
            }
            "dwindle:split_width_multiplier" => {
                cfg.dwindle.split_width_multiplier = parse_float(value)?.clamp(0.1, 3.0)
            }
            "dwindle:use_active_for_splits" => {
                cfg.dwindle.use_active_for_splits = parse_bool(value)?
            }
            "master:allow_small_split" => cfg.master.allow_small_split = parse_bool(value)?,
            "master:special_scale_factor" => {
                cfg.master.special_scale_factor = parse_float(value)?.clamp(0.0, 1.0)
            }
            "master:mfact" => cfg.master.mfact = parse_float(value)?.clamp(0.0, 1.0),
            "master:new_status" => {
                cfg.master.new_status = match value {
                    "master" => NewStatus::Master,
                    "slave" => NewStatus::Slave,
                    "inherit" => NewStatus::Inherit,
                    other => return Err(format!("invalid new_status `{other}`")),
                }
            }
            "master:new_on_top" => cfg.master.new_on_top = parse_bool(value)?,
            "master:new_on_active" => {
                cfg.master.new_on_active = match value {
                    "none" => NewOnActive::None,
                    "before" => NewOnActive::Before,
                    "after" => NewOnActive::After,
                    other => return Err(format!("invalid new_on_active `{other}`")),
                }
            }
            "master:orientation" => {
                cfg.master.orientation =
                    Orientation::parse(value).ok_or(format!("invalid orientation `{value}`"))?
            }
            "master:slave_count_for_center_master" => {
                cfg.master.slave_count_for_center_master = parse_int(value)?.clamp(0, 10)
            }
            "master:center_master_fallback" => {
                cfg.master.center_master_fallback = Orientation::parse(value)
                    .filter(|o| *o != Orientation::Center)
                    .ok_or(format!("invalid center_master_fallback `{value}`"))?
            }
            "master:smart_resizing" => cfg.master.smart_resizing = parse_bool(value)?,
            "master:drop_at_cursor" => cfg.master.drop_at_cursor = parse_bool(value)?,
            "master:always_keep_position" => cfg.master.always_keep_position = parse_bool(value)?,
            "master:focus_master_on_close" => cfg.master.focus_master_on_close = parse_bool(value)?,
            "group:auto_group" => cfg.group.auto_group = parse_bool(value)?,
            "group:insert_after_current" => cfg.group.insert_after_current = parse_bool(value)?,
            "group:focus_removed_window" => cfg.group.focus_removed_window = parse_bool(value)?,
            "group:col.border_active" => cfg.group.border_active = parse_color(value)?,
            "group:col.border_inactive" => cfg.group.border_inactive = parse_color(value)?,
            "group:col.border_locked_active" => {
                cfg.group.border_locked_active = parse_color(value)?
            }
            "group:col.border_locked_inactive" => {
                cfg.group.border_locked_inactive = parse_color(value)?
            }
            "group:groupbar:enabled" => cfg.group.groupbar.enabled = parse_bool(value)?,
            "group:groupbar:height" => cfg.group.groupbar.height = parse_int(value)?.clamp(0, 64),
            "group:groupbar:gaps_in" => cfg.group.groupbar.gaps_in = parse_int(value)?.max(0),
            "group:groupbar:disable_when_only" => {
                cfg.group.groupbar.disable_when_only = parse_bool(value)?
            }
            "group:groupbar:col.active" => cfg.group.groupbar.col_active = parse_color(value)?,
            "group:groupbar:col.inactive" => cfg.group.groupbar.col_inactive = parse_color(value)?,
            "group:groupbar:col.locked_active" => {
                cfg.group.groupbar.col_locked_active = parse_color(value)?
            }
            "group:groupbar:col.locked_inactive" => {
                cfg.group.groupbar.col_locked_inactive = parse_color(value)?
            }
            "binds:ignore_group_lock" => cfg.ignore_group_lock = parse_bool(value)?,
            "workspace" => {
                if let Some(rule) = parse_workspace_rule(value) {
                    // A later rule for the same workspace replaces the earlier one.
                    cfg.workspace_rules
                        .retain(|r| r.workspace != rule.workspace);
                    cfg.workspace_rules.push(rule);
                }
            }
            // Accept but ignore well-known options that aren't implemented yet,
            // so real Hyprland configs load without a flood of errors.
            k if is_known_unimplemented(k) => {
                tracing::debug!("config option `{k}` is not implemented yet, ignoring");
            }
            other => return Err(format!("unknown option `{other}`")),
        }
        Ok(())
    }
}

fn is_known_unimplemented(key: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "general:",
        "decoration:",
        "animations",
        "animation",
        "bezier",
        "gestures:",
        "misc:",
        // Options of implemented categories that have no effect yet (the
        // implemented ones are matched before this list is consulted).
        "dwindle:pseudotile",
        "dwindle:no_gaps_when_only",
        "dwindle:split_bias",
        "dwindle:precise_mouse_move",
        "master:no_gaps_when_only",
        "master:new_is_master",
        "master:inherit_fullscreen",
        "master:center_ignores_reserved",
        "group:",
        "binds:",
        "xwayland:",
        "cursor:",
        "monitor",
        "env",
        "workspace",
    ];
    PREFIXES.iter().any(|p| key.starts_with(p))
}

fn strip_comment(line: &str) -> &str {
    // `##` is an escaped `#` in hyprlang; keep it simple and treat a single `#`
    // as the start of a comment.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            if bytes.get(i + 1) == Some(&b'#') {
                i += 2;
                continue;
            }
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn parse_bind(value: &str, flags: BindFlags) -> Result<Bind, String> {
    let mut parts = value.splitn(if flags.has_description { 5 } else { 4 }, ',');
    let mods = parts.next().ok_or("bind: missing modifiers")?;
    let key = parts.next().ok_or("bind: missing key")?.trim();
    let description = if flags.has_description {
        parts.next().ok_or("bind: missing description")?.trim()
    } else {
        ""
    };
    let dispatcher = parts.next().ok_or("bind: missing dispatcher")?;
    let arg = parts.next().unwrap_or("");
    if key.is_empty() {
        return Err("bind: empty key".into());
    }
    let dispatcher = match (flags.mouse, dispatcher.trim()) {
        (true, "movewindow") => Dispatcher::MouseMove,
        (true, "resizewindow") => Dispatcher::MouseResize,
        (true, other) => return Err(format!("bindm: unsupported dispatcher `{other}`")),
        _ => Dispatcher::parse(dispatcher, arg)?,
    };
    let key = key.to_ascii_lowercase();
    if flags.mouse && !key.starts_with("mouse:") {
        return Err(format!("bindm: key must be `mouse:<code>`, got `{key}`"));
    }
    Ok(Bind {
        mods: Mods::parse(mods)?,
        key,
        dispatcher,
        flags,
        description: description.to_string(),
        submap: String::new(),
    })
}

fn parse_int(value: &str) -> Result<i32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid integer `{value}`"))
}

fn parse_float(value: &str) -> Result<f32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid float `{value}`"))
}

/// `workspace = 2, layout:master, layoutopt:orientation:top, ...`. Returns
/// `None` for rules without a plain id/`name:` selector or without layout bits.
fn parse_workspace_rule(value: &str) -> Option<WorkspaceRule> {
    let mut parts = value.split(',').map(str::trim);
    let sel = parts.next()?;
    let workspace = match sel.strip_prefix("name:") {
        Some(n) => n.to_string(),
        None if sel.parse::<i32>().is_ok() => sel.to_string(),
        None => return None,
    };
    let mut rule = WorkspaceRule {
        workspace,
        layout: None,
        orientation: None,
    };
    for p in parts {
        if let Some(l) = p.strip_prefix("layout:") {
            rule.layout = Some(l.trim().to_string());
        } else if let Some(o) = p.strip_prefix("layoutopt:orientation:") {
            rule.orientation = Orientation::parse(o);
        }
    }
    (rule.layout.is_some() || rule.orientation.is_some()).then_some(rule)
}

fn parse_bool(value: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(format!("invalid boolean `{value}`")),
    }
}

/// Parses `rgba(RRGGBBAA)`, `rgb(RRGGBB)` or `0xAARRGGBB`. For gradients
/// (`rgba(...) rgba(...) 45deg`) only the first color is used for now.
pub fn parse_color(value: &str) -> Result<[f32; 4], String> {
    let first = value.split_whitespace().next().unwrap_or("");
    let bad = || format!("invalid color `{value}`");
    let hex = |s: &str| u32::from_str_radix(s, 16).map_err(|_| bad());
    let (r, g, b, a) = if let Some(inner) = first
        .strip_prefix("rgba(")
        .and_then(|s| s.strip_suffix(')'))
    {
        if inner.len() != 8 {
            return Err(bad());
        }
        let v = hex(inner)?;
        (v >> 24, (v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff)
    } else if let Some(inner) = first.strip_prefix("rgb(").and_then(|s| s.strip_suffix(')')) {
        if inner.len() != 6 {
            return Err(bad());
        }
        let v = hex(inner)?;
        (v >> 16, (v >> 8) & 0xff, v & 0xff, 0xff)
    } else if let Some(inner) = first.strip_prefix("0x") {
        if inner.len() != 8 {
            return Err(bad());
        }
        let v = hex(inner)?;
        ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff, v >> 24)
    } else {
        return Err(bad());
    };
    Ok([r, g, b, a].map(|c| c as f32 / 255.0))
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses() {
        let (cfg, errors) = Config::parse_str(DEFAULT_CONFIG);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cfg.binds.len(), 31);
        assert_eq!(
            cfg.binds[0],
            Bind {
                mods: Mods {
                    logo: true,
                    ..Default::default()
                },
                key: "return".into(),
                dispatcher: Dispatcher::Exec("kitty".into()),
                flags: BindFlags::default(),
                description: String::new(),
                submap: String::new(),
            }
        );
    }

    #[test]
    fn bind_flags_and_description() {
        let f = |k: &str| BindFlags::from_keyword(k);
        assert_eq!(f("bind"), Some(BindFlags::default()));
        let el = f("bindel").unwrap();
        assert!(el.locked && el.repeat && !el.release);
        assert!(f("bindm").unwrap().mouse);
        assert!(f("bindx").is_none());
        assert!(f("unbind").is_none());

        let (cfg, errors) = Config::parse_str(
            "bindd = SUPER, T, Open terminal, exec, kitty --single\nbindr = SUPER, X, killactive,\nbindm = SUPER, mouse:272, movewindow\nbindm = SUPER, mouse:273, resizewindow\nbindn = , code:10, exec, true",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cfg.binds[0].description, "Open terminal");
        assert_eq!(
            cfg.binds[0].dispatcher,
            Dispatcher::Exec("kitty --single".into())
        );
        assert!(cfg.binds[1].flags.release);
        assert_eq!(cfg.binds[2].dispatcher, Dispatcher::MouseMove);
        assert_eq!(cfg.binds[3].dispatcher, Dispatcher::MouseResize);
        assert_eq!(cfg.binds[2].key, "mouse:272");
        assert!(cfg.binds[4].flags.non_consuming);
        assert_eq!(cfg.binds[4].key, "code:10");
        let (_, errors) = Config::parse_str("bindm = SUPER, Q, movewindow");
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn submaps_and_unbind() {
        let (cfg, errors) = Config::parse_str(
            "bind = SUPER, R, submap, resize\nsubmap = resize\nbind = , l, resizeactive, 10 0\nbind = , escape, submap, reset\nsubmap = reset\nbind = SUPER, Q, killactive,\nunbind = SUPER, Q",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cfg.binds.len(), 3);
        assert_eq!(cfg.binds[0].submap, "");
        assert_eq!(cfg.binds[0].dispatcher, Dispatcher::Submap("resize".into()));
        assert_eq!(cfg.binds[1].submap, "resize");
        assert_eq!(cfg.binds[2].submap, "resize");
        assert_eq!(cfg.binds[2].dispatcher, Dispatcher::Submap(String::new()));
    }

    #[test]
    fn new_dispatchers_parse() {
        let p = |n: &str, a: &str| Dispatcher::parse(n, a);
        assert_eq!(p("fullscreen", "1"), Ok(Dispatcher::Maximize));
        assert_eq!(p("fullscreen", ""), Ok(Dispatcher::Fullscreen));
        assert!(p("fullscreen", "9").is_err());
        assert_eq!(
            p("movewindow", "l"),
            Ok(Dispatcher::MoveWindow(Direction::Left))
        );
        assert!(p("movewindow", "").is_err());
        assert_eq!(p("swapnext", "prev"), Ok(Dispatcher::SwapNext(true)));
        assert_eq!(p("swapnext", ""), Ok(Dispatcher::SwapNext(false)));
        assert!(p("closewindow", "class:^(foo)$").is_ok());
        assert!(p("workspace", "e+1").is_ok());
        assert!(p("workspace", "0").is_err());
        assert!(p("resizeactive", "10%").is_err());
        assert_eq!(strip_exec_rules("[float; size 10 10] kitty"), "kitty");
        assert_eq!(strip_exec_rules("kitty [x]"), "kitty [x]");
    }

    #[test]
    fn input_config() {
        let (cfg, errors) = Config::parse_str(
            "input {\n kb_layout = tr\n kb_options = caps:escape\n repeat_rate = 40\n repeat_delay = 300\n numlock_by_default = true\n follow_mouse = 2\n sensitivity = -0.5\n accel_profile = flat\n natural_scroll = yes\n touchpad {\n  natural_scroll = true\n  scroll_factor = 0.5\n  tap-to-click = no\n }\n force_no_accel = 1\n}\ndevice {\n name = My Fancy Mouse\n sensitivity = 0.7\n left_handed = true\n}",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let i = &cfg.input;
        assert_eq!(i.kb_layout, "tr");
        assert_eq!(i.kb_options, "caps:escape");
        assert_eq!((i.repeat_rate, i.repeat_delay), (40, 300));
        assert!(i.numlock_by_default);
        assert_eq!(i.follow_mouse, 2);
        // A plain mouse: global options.
        let m = i.opts_for("Logitech Mouse", false);
        assert_eq!(m.sensitivity, Some(-0.5));
        assert_eq!(m.accel_profile, Some(AccelProfile::Flat));
        assert_eq!(m.natural_scroll, Some(true));
        assert_eq!(m.tap_to_click, None);
        // Touchpad: own natural_scroll, defaults for tap/dwt, touchpad overrides.
        let t = i.opts_for("synaptics", true);
        assert_eq!(t.natural_scroll, Some(true));
        assert_eq!(t.tap_to_click, Some(false));
        assert_eq!(t.disable_while_typing, Some(true));
        assert_eq!(t.sensitivity, Some(-0.5));
        assert_eq!(i.touchpad_scroll_factor(), 0.5);
        // Per-device override, name matched case/space-insensitively.
        let d = i.opts_for("my fancy mouse", false);
        assert_eq!(d.sensitivity, Some(0.7));
        assert_eq!(d.left_handed, Some(true));
        assert_eq!(d.accel_profile, Some(AccelProfile::Flat));
        // Out of range and bad values.
        let (cfg, errors) = Config::parse_str(
            "input:sensitivity = 5\ninput:accel_profile = bogus\ndevice:sensitivity = 1",
        );
        assert_eq!(cfg.input.pointer.sensitivity, Some(1.0));
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn categories_and_shorthand() {
        let (cfg, errors) = Config::parse_str(
            "general {\n  gaps_in = 3 # inner\n  gaps_out = 10\n}\ngeneral:border_size = 2\ndecoration {\n rounding = 8\n blur {\n  enabled = true\n }\n}",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cfg.general.gaps_in, 3);
        assert_eq!(cfg.general.gaps_out, 10);
        assert_eq!(cfg.general.border_size, 2);
        assert_eq!(cfg.decoration.rounding, 8);
    }

    #[test]
    fn variables_longest_first() {
        let (cfg, errors) =
            Config::parse_str("$main = ALT\n$mainMod = SUPER\nbind = $mainMod, Q, killactive,");
        assert!(errors.is_empty(), "{errors:?}");
        assert!(cfg.binds[0].mods.logo);
        assert!(!cfg.binds[0].mods.alt);
    }

    #[test]
    fn exec_with_commas_kept() {
        let (cfg, _) = Config::parse_str("bind = SUPER, R, exec, notify-send a,b");
        assert_eq!(
            cfg.binds[0].dispatcher,
            Dispatcher::Exec("notify-send a,b".into())
        );
    }

    #[test]
    fn errors_are_reported_per_line() {
        let (cfg, errors) = Config::parse_str(
            "general:gaps_in = abc\nbind = SUPER, Q, nope,\ngeneral:gaps_out = 7",
        );
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0].line, 1);
        assert_eq!(errors[1].line, 2);
        assert_eq!(cfg.general.gaps_out, 7);
    }

    #[test]
    fn colors() {
        assert_eq!(
            parse_color("rgba(ff000080)").unwrap(),
            [1.0, 0.0, 0.0, 128.0 / 255.0]
        );
        assert_eq!(parse_color("rgb(00ff00)").unwrap(), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(parse_color("0xff0000ff").unwrap(), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(
            parse_color("rgba(33ccffee) rgba(00ff99ee) 45deg").unwrap()[0],
            0x33 as f32 / 255.0
        );
        assert!(parse_color("blue").is_err());
    }

    #[test]
    fn keyword_applies_one_line() {
        let mut cfg = Config::with_default_binds();
        cfg.apply_keyword("general:gaps_in", "10").unwrap();
        assert_eq!(cfg.general.gaps_in, 10);
        assert!(cfg.set_options.contains("general:gaps_in"));
        let n = cfg.binds.len();
        cfg.apply_keyword("bind", "SUPER,X,exec,foo").unwrap();
        assert_eq!(cfg.binds.len(), n + 1);
        assert!(cfg.apply_keyword("general:gaps_in", "x").is_err());
        assert!(cfg.apply_keyword("nope:nope", "1").is_err());
        assert_eq!(cfg.general.gaps_in, 10);
    }

    #[test]
    fn tracks_misc_and_files() {
        let (cfg, e) = Config::parse_str(
            "misc {
 disable_autoreload = true
}",
        );
        assert!(e.is_empty());
        assert!(cfg.misc.disable_autoreload);
    }

    #[test]
    fn unimplemented_options_are_ignored() {
        let (_, errors) = Config::parse_str(
            "animations {\n enabled = yes\n bezier = a, 0.05, 0.9, 0.1, 1.05\n}\nmonitor = ,preferred,auto,1\ninput {\n kb_layout = tr\n}",
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn layout_options_parse() {
        let (cfg, e) = Config::parse_str(
            "general {
 layout = master
}
master {
 mfact = 0.7
 new_status = inherit
 new_on_top = true
 new_on_active = before
 orientation = center
 slave_count_for_center_master = 3
 center_master_fallback = right
 always_keep_position = yes
 allow_small_split = true
}
dwindle {
 force_split = 2
 smart_split = true
 split_width_multiplier = 1.5
 use_active_for_splits = false
 permanent_direction_override = true
 pseudotile = true
}
master:inherit_fullscreen = true
workspace = 2, layout:master, layoutopt:orientation:top
workspace = name:web, layout:dwindle, rounding:false
workspace = r[1-3], layout:master",
        );
        assert!(e.is_empty(), "{e:?}");
        assert_eq!(cfg.general.layout, "master");
        let m = &cfg.master;
        assert_eq!(m.mfact, 0.7);
        assert_eq!(m.new_status, NewStatus::Inherit);
        assert!(m.new_on_top && m.always_keep_position && m.allow_small_split);
        assert_eq!(m.new_on_active, NewOnActive::Before);
        assert_eq!(m.orientation, Orientation::Center);
        assert_eq!(m.slave_count_for_center_master, 3);
        assert_eq!(m.center_master_fallback, Orientation::Right);
        let d = &cfg.dwindle;
        assert_eq!(d.force_split, 2);
        assert!(d.smart_split && !d.use_active_for_splits && d.permanent_direction_override);
        assert_eq!(d.split_width_multiplier, 1.5);
        assert_eq!(cfg.workspace_rules.len(), 2);
        assert_eq!(cfg.workspace_rules[0].workspace, "2");
        assert_eq!(cfg.workspace_rules[0].orientation, Some(Orientation::Top));
        assert_eq!(cfg.workspace_rules[1].workspace, "web");
        assert_eq!(cfg.workspace_rules[1].layout.as_deref(), Some("dwindle"));

        let (_, e) = Config::parse_str(
            "master {
 new_status = nope
}
dwindle:nope = 1",
        );
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn window_and_monitor_dispatchers_parse() {
        let p = |n: &str, a: &str| Dispatcher::parse(n, a);
        assert_eq!(
            p("renameworkspace", "2 work area"),
            Ok(Dispatcher::RenameWorkspace(2, "work area".into()))
        );
        assert!(p("renameworkspace", "x name").is_err());
        assert!(p("renameworkspace", "2").is_err());
        assert_eq!(
            p("movecursor", "100, 200"),
            Ok(Dispatcher::MoveCursor(100, 200))
        );
        assert!(p("movecursor", "100").is_err());
        assert_eq!(
            p("movecursortocorner", "3"),
            Ok(Dispatcher::MoveCursorToCorner(3))
        );
        assert!(p("movecursortocorner", "4").is_err());
        assert_eq!(
            p("alterzorder", "top"),
            Ok(Dispatcher::AlterZOrder(true, None))
        );
        assert_eq!(
            p("alterzorder", "bottom, class:^(foo)$"),
            Ok(Dispatcher::AlterZOrder(
                false,
                Some(WinSel::Class("^(foo)$".into()))
            ))
        );
        assert!(p("alterzorder", "middle").is_err());
        assert_eq!(p("togglepseudo", ""), Ok(Dispatcher::TogglePseudo));
        assert_eq!(p("bringactivetotop", ""), Ok(Dispatcher::BringActiveToTop));
        assert_eq!(p("dpms", "toggle"), Ok(Dispatcher::Dpms(Toggle::Flip)));
        assert_eq!(p("dpms", "off"), Ok(Dispatcher::Dpms(Toggle::Disable)));
        assert_eq!(
            p("event", "hello world"),
            Ok(Dispatcher::Event("hello world".into()))
        );
        assert_eq!(p("signal", "9"), Ok(Dispatcher::Signal(9, None)));
        assert!(p("signal", "kill").is_err());
        assert_eq!(
            p("setprop", "active bordersize 4"),
            Ok(Dispatcher::SetProp(None, "bordersize".into(), "4".into()))
        );
        assert_eq!(
            p("focusworkspaceoncurrentmonitor", "2"),
            Ok(Dispatcher::Workspace(WsSel::Id(2)))
        );
        assert_eq!(
            p("focusmonitor", "DP-1"),
            Ok(Dispatcher::Noop("focusmonitor", "DP-1".into()))
        );
        assert!(p("nosuchthing", "").is_err());
    }

    #[test]
    fn group_options_and_dispatchers_parse() {
        let (cfg, e) = Config::parse_str(
            "group {
 auto_group = false
 insert_after_current = false
 focus_removed_window = false
 col.border_active = rgba(ff000080)
 groupbar {
  enabled = true
  height = 20
  col.active = rgb(00ff00)
  render_titles = false
 }
}
binds {
 ignore_group_lock = true
}",
        );
        assert!(e.is_empty(), "{e:?}");
        assert!(!cfg.group.auto_group && !cfg.group.insert_after_current);
        assert_eq!(cfg.group.border_active, [1.0, 0.0, 0.0, 128.0 / 255.0]);
        assert_eq!(cfg.group.groupbar.height, 20);
        assert_eq!(cfg.group.groupbar.col_active, [0.0, 1.0, 0.0, 1.0]);
        assert!(cfg.ignore_group_lock);

        let p = |n: &str, a: &str| Dispatcher::parse(n, a);
        assert_eq!(p("togglegroup", ""), Ok(Dispatcher::ToggleGroup));
        assert_eq!(
            p("changegroupactive", "b"),
            Ok(Dispatcher::ChangeGroupActive(GroupStep::Backward))
        );
        assert_eq!(
            p("changegroupactive", "2"),
            Ok(Dispatcher::ChangeGroupActive(GroupStep::Index(2)))
        );
        assert_eq!(
            p("moveintogroup", "l"),
            Ok(Dispatcher::MoveIntoGroup(Direction::Left))
        );
        assert_eq!(
            p("moveoutofgroup", ""),
            Ok(Dispatcher::MoveOutOfGroup(None))
        );
        assert_eq!(
            p("movegroupwindow", "b"),
            Ok(Dispatcher::MoveGroupWindow(true))
        );
        assert_eq!(
            p("lockgroups", "toggle"),
            Ok(Dispatcher::LockGroups(Toggle::Flip))
        );
        assert_eq!(
            p("lockactivegroup", "unlock"),
            Ok(Dispatcher::LockActiveGroup(Toggle::Disable))
        );
        assert!(p("lockgroups", "maybe").is_err());
        assert_eq!(
            p("layoutmsg", "mfact +0.1"),
            Ok(Dispatcher::LayoutMsg("mfact +0.1".into()))
        );
        assert!(p("layoutmsg", "").is_err());
        assert!(!Toggle::Flip.apply(true));
    }
}
