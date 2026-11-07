//! Window rules and layer rules: parsing, matching and evaluation. Pure logic
//! (no Smithay types); applying the results to windows is in
//! `wm/window_rules.rs`.
//!
//! Syntax. Hyprland v0.57.1 itself no longer reads hyprlang (its config is
//! Lua: `hl.window_rule({ match = { class = "kitty" }, float = true })`), but
//! the prop and effect names are the same snake_case ones that the last
//! hyprlang release (v0.53-v0.56) used, so the canonical syntax here is that
//! hyprlang form:
//!
//! ```text
//! windowrule = match:class kitty, match:float 1, float on, size 800 600
//! windowrule {            # block form, `name` makes the rule "named"
//!   name = foo
//!   match:class = kitty
//!   float = on
//! }
//! layerrule = blur on, match:namespace waybar
//! ```
//!
//! The pre-v0.53 forms are still accepted and mapped to the canonical one, so
//! old configs keep working:
//! `windowrulev2 = float, class:^(kitty)$, title:foo` and
//! `windowrule = float, ^(kitty)$` (second field is a class regex),
//! `layerrule = blur, waybar`.
//!
//! Semantics (from the wiki at that tag): all matchers of a rule must match;
//! regexes must match the whole string (RE2 `FullMatch`; `negative:` prefix
//! inverts); rules apply top to bottom, last match wins, but named rules are
//! evaluated before anonymous ones. Static effects are applied once when the
//! window opens (with the initial class/title); dynamic effects are
//! re-evaluated whenever a property changes.

// Opacity, rounding, layer effects and `Other` effects are parsed and stored
// but nothing reads them yet (no renderer support).
#![allow(dead_code)]

use std::str::FromStr;

use regex::Regex;

use crate::selectors::WsSel;

// ------------------------------------------------------------------ regex

/// A window/layer regex: whole-string match, optional `negative:` prefix.
#[derive(Debug, Clone)]
pub struct Re {
    src: String,
    neg: bool,
    re: Regex,
}

impl PartialEq for Re {
    fn eq(&self, o: &Self) -> bool {
        self.src == o.src && self.neg == o.neg
    }
}

impl Re {
    pub fn new(pat: &str) -> Result<Re, String> {
        let pat = pat.trim();
        let (neg, src) = match pat.strip_prefix("negative:") {
            Some(r) => (true, r),
            None => (false, pat),
        };
        let build = |s: &str| Regex::new(&format!("(?s)^(?:{s})$"));
        // `^(*)$` (seen in JaKooLit) is invalid RE2; read `(*` as `(.*`.
        let re = build(src)
            .or_else(|e| build(&src.replace("(*", "(.*")).map_err(|_| e))
            .map_err(|e| format!("invalid regex `{src}`: {e}"))?;
        Ok(Re {
            src: src.to_string(),
            neg,
            re,
        })
    }

    pub fn is_match(&self, text: &str) -> bool {
        self.re.is_match(text) != self.neg
    }
}

// -------------------------------------------------------------- expressions

/// Variables of `size` / `move` expressions (monitor-local pixels).
#[derive(Debug, Clone, Copy, Default)]
pub struct Vars {
    pub monitor_w: f64,
    pub monitor_h: f64,
    pub window_x: f64,
    pub window_y: f64,
    pub window_w: f64,
    pub window_h: f64,
    pub cursor_x: f64,
    pub cursor_y: f64,
}

/// Evaluates `monitor_w*0.5`, `(monitor_h/2)+17`, `100%-w-10`. `N%` is `N`
/// percent of `pct_base`; `w` and `h` are the pre-v0.53 names of the window
/// size. Operators: `+ - * /`, unary minus, parentheses.
pub fn eval_expr(expr: &str, v: &Vars, pct_base: f64) -> Result<f64, String> {
    struct P<'a> {
        s: &'a [u8],
        i: usize,
        v: &'a Vars,
        base: f64,
    }
    impl P<'_> {
        fn peek(&self) -> Option<u8> {
            self.s.get(self.i).copied()
        }
        fn sum(&mut self) -> Result<f64, String> {
            let mut a = self.prod()?;
            while let Some(c @ (b'+' | b'-')) = self.peek() {
                self.i += 1;
                let b = self.prod()?;
                a = if c == b'+' { a + b } else { a - b };
            }
            Ok(a)
        }
        fn prod(&mut self) -> Result<f64, String> {
            let mut a = self.unary()?;
            while let Some(c @ (b'*' | b'/')) = self.peek() {
                self.i += 1;
                let b = self.unary()?;
                if c == b'/' && b == 0.0 {
                    return Err("division by zero".into());
                }
                a = if c == b'*' { a * b } else { a / b };
            }
            Ok(a)
        }
        fn unary(&mut self) -> Result<f64, String> {
            if self.peek() == Some(b'-') {
                self.i += 1;
                return Ok(-self.unary()?);
            }
            self.atom()
        }
        fn atom(&mut self) -> Result<f64, String> {
            match self.peek() {
                Some(b'(') => {
                    self.i += 1;
                    let a = self.sum()?;
                    if self.peek() != Some(b')') {
                        return Err("missing `)`".into());
                    }
                    self.i += 1;
                    Ok(a)
                }
                Some(c) if c.is_ascii_digit() || c == b'.' => {
                    let st = self.i;
                    while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
                        self.i += 1;
                    }
                    let n: f64 = std::str::from_utf8(&self.s[st..self.i])
                        .unwrap_or("")
                        .parse()
                        .map_err(|_| "bad number".to_string())?;
                    if self.peek() == Some(b'%') {
                        self.i += 1;
                        return Ok(n / 100.0 * self.base);
                    }
                    Ok(n)
                }
                Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                    let st = self.i;
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
                    {
                        self.i += 1;
                    }
                    let name = std::str::from_utf8(&self.s[st..self.i]).unwrap_or("");
                    let v = self.v;
                    Ok(match name {
                        "monitor_w" => v.monitor_w,
                        "monitor_h" => v.monitor_h,
                        "window_x" => v.window_x,
                        "window_y" => v.window_y,
                        "window_w" | "w" => v.window_w,
                        "window_h" | "h" => v.window_h,
                        "cursor_x" => v.cursor_x,
                        "cursor_y" => v.cursor_y,
                        other => return Err(format!("unknown variable `{other}`")),
                    })
                }
                _ => Err(format!("unexpected input in expression `{}`", self.dump())),
            }
        }
        fn dump(&self) -> String {
            String::from_utf8_lossy(self.s).into_owned()
        }
    }
    let s: Vec<u8> = expr.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    let mut p = P {
        s: &s,
        i: 0,
        v,
        base: pct_base,
    };
    let r = p.sum()?;
    if p.i != s.len() {
        return Err(format!("trailing input in expression `{expr}`"));
    }
    Ok(r)
}

fn check_expr(e: &str) -> Result<(), String> {
    let v = Vars {
        monitor_w: 1.0,
        monitor_h: 1.0,
        window_x: 1.0,
        window_y: 1.0,
        window_w: 1.0,
        window_h: 1.0,
        cursor_x: 1.0,
        cursor_y: 1.0,
    };
    eval_expr(e, &v, 1.0).map(|_| ())
}

// ------------------------------------------------------------------ effects

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opacity {
    /// `(value, override)`: with override the value is absolute, otherwise a
    /// multiplier.
    pub active: (f32, bool),
    pub inactive: (f32, bool),
    pub fullscreen: (f32, bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BorderColor {
    pub active: Option<[f32; 4]>,
    pub inactive: Option<[f32; 4]>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    // static
    Float(bool),
    Fullscreen(bool),
    Maximize(bool),
    Pin(bool),
    Center(bool),
    NoInitialFocus(bool),
    Size(String, String),
    Move(String, String),
    /// `sel` is `None` for `unset`.
    Workspace {
        sel: Option<String>,
        silent: bool,
    },
    Monitor(String),
    SuppressEvent(Vec<String>),
    // dynamic
    Opacity(Opacity),
    BorderSize(i32),
    BorderColor(BorderColor),
    Rounding(i32),
    /// `+name`, `-name` or `name` (toggle).
    Tag(String),
    /// Known effect that is parsed and stored but not implemented yet.
    Other(String, String),
}

/// Effects that are accepted and stored without behaviour (names from the
/// wiki's effect tables).
const OTHER_EFFECTS: &[&str] = &[
    "allows_input",
    "animation",
    "confine_pointer",
    "content",
    "decorate",
    "dim_around",
    "focus_on_activate",
    "force_rgbx",
    "fullscreen_state",
    "group",
    "idle_inhibit",
    "immediate",
    "keep_aspect_ratio",
    "max_size",
    "min_size",
    "nearest_neighbor",
    "no_anim",
    "no_auto_hdr",
    "no_blur",
    "no_close_for",
    "no_dim",
    "no_focus",
    "no_follow_mouse",
    "no_glow",
    "no_max_size",
    "no_screen_share",
    "no_shadow",
    "no_shortcuts_inhibit",
    "no_vrr",
    "no_wobble",
    "no_xdg_drags",
    "opaque",
    "persistent_size",
    "pseudo",
    "render_unfocused",
    "rounding_power",
    "scroll_mouse",
    "scroll_touchpad",
    "scrolling_width",
    "stay_focused",
    "sync_fullscreen",
    "tonemap",
    "xray",
];

fn flag(arg: &str) -> Result<bool, String> {
    match arg.trim().to_ascii_lowercase().as_str() {
        "" | "on" | "1" | "true" | "yes" => Ok(true),
        "off" | "0" | "false" | "no" => Ok(false),
        other => Err(format!("expected on/off, got `{other}`")),
    }
}

fn two_exprs(name: &str, arg: &str) -> Result<(String, String), String> {
    let mut it = arg.split_whitespace();
    let (Some(a), Some(b), None) = (it.next(), it.next(), it.next()) else {
        return Err(format!("{name}: expected two values, got `{arg}`"));
    };
    check_expr(a).map_err(|e| format!("{name}: {e}"))?;
    check_expr(b).map_err(|e| format!("{name}: {e}"))?;
    Ok((a.to_string(), b.to_string()))
}

fn parse_opacity(arg: &str) -> Result<Opacity, String> {
    let mut vals: Vec<(f32, bool)> = Vec::new();
    for t in arg.split_whitespace() {
        if t == "override" {
            vals.last_mut()
                .ok_or("opacity: `override` without value")?
                .1 = true;
        } else {
            let v: f32 = t.parse().map_err(|_| format!("opacity: bad value `{t}`"))?;
            vals.push((v, false));
        }
    }
    let one = (1.0, false);
    match vals.as_slice() {
        [a] => Ok(Opacity {
            active: *a,
            inactive: *a,
            fullscreen: *a,
        }),
        [a, b] => Ok(Opacity {
            active: *a,
            inactive: *b,
            fullscreen: one,
        }),
        [a, b, c] => Ok(Opacity {
            active: *a,
            inactive: *b,
            fullscreen: *c,
        }),
        _ => Err(format!("opacity: expected 1 to 3 values, got `{arg}`")),
    }
}

/// `c`, `c1 c2` (active inactive) or gradients `c1 c2 45deg [c3 c4 [45deg]]`.
// shortcut: gradients use their first color; angles are ignored.
fn parse_border_color(arg: &str) -> Result<BorderColor, String> {
    let mut groups: Vec<Vec<&str>> = vec![Vec::new()];
    let mut angle = false;
    for t in arg.split_whitespace() {
        if t.ends_with("deg") {
            angle = true;
            groups.push(Vec::new());
        } else {
            groups.last_mut().expect("never empty").push(t);
        }
    }
    groups.retain(|g| !g.is_empty());
    if groups.is_empty() {
        return Err("border_color: no color".into());
    }
    let col = |g: &Vec<&str>| crate::config::parse_color(g[0]);
    if !angle && groups[0].len() == 2 {
        return Ok(BorderColor {
            active: Some(crate::config::parse_color(groups[0][0])?),
            inactive: Some(crate::config::parse_color(groups[0][1])?),
        });
    }
    Ok(BorderColor {
        active: Some(col(&groups[0])?),
        inactive: groups.get(1).map(col).transpose()?,
    })
}

impl Effect {
    /// `name` in the canonical (snake_case) spelling.
    pub fn parse(name: &str, arg: &str) -> Result<Effect, String> {
        let arg = arg.trim();
        let int = |what: &str| {
            arg.parse::<i32>()
                .map_err(|_| format!("{what}: expected an integer, got `{arg}`"))
        };
        Ok(match name {
            "float" => Effect::Float(flag(arg)?),
            "tile" => Effect::Float(!flag(arg)?),
            "fullscreen" => Effect::Fullscreen(flag(arg)?),
            "maximize" => Effect::Maximize(flag(arg)?),
            "pin" => Effect::Pin(flag(arg)?),
            "center" => Effect::Center(flag(arg)?),
            "no_initial_focus" => Effect::NoInitialFocus(flag(arg)?),
            "size" => two_exprs(name, arg).map(|(a, b)| Effect::Size(a, b))?,
            "move" => two_exprs(name, arg).map(|(a, b)| Effect::Move(a, b))?,
            "workspace" => {
                let mut it = arg.split_whitespace();
                let first = it.next().ok_or("workspace: missing argument")?;
                let silent = it.any(|t| t == "silent");
                if first == "unset" {
                    Effect::Workspace {
                        sel: None,
                        silent: false,
                    }
                } else {
                    WsSel::from_str(first)?;
                    Effect::Workspace {
                        sel: Some(first.to_string()),
                        silent,
                    }
                }
            }
            "monitor" => Effect::Monitor(arg.to_string()),
            "suppress_event" => {
                Effect::SuppressEvent(arg.split_whitespace().map(str::to_string).collect())
            }
            "opacity" => Effect::Opacity(parse_opacity(arg)?),
            "border_size" => Effect::BorderSize(int("border_size")?),
            "border_color" => Effect::BorderColor(parse_border_color(arg)?),
            "rounding" => Effect::Rounding(int("rounding")?),
            "tag" => {
                if arg.trim_start_matches(['+', '-']).is_empty() {
                    return Err("tag: missing name".into());
                }
                Effect::Tag(arg.to_string())
            }
            n if OTHER_EFFECTS.contains(&n) => Effect::Other(n.to_string(), arg.to_string()),
            other => return Err(format!("unknown window rule effect `{other}`")),
        })
    }

    pub fn is_static(&self) -> bool {
        matches!(
            self,
            Effect::Float(_)
                | Effect::Fullscreen(_)
                | Effect::Maximize(_)
                | Effect::Pin(_)
                | Effect::Center(_)
                | Effect::NoInitialFocus(_)
                | Effect::Size(..)
                | Effect::Move(..)
                | Effect::Workspace { .. }
                | Effect::Monitor(_)
                | Effect::SuppressEvent(_)
        )
    }
}

// ----------------------------------------------------------------- matchers

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoolProp {
    Xwayland,
    Float,
    Fullscreen,
    Pin,
    Focus,
    Group,
    Modal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntProp {
    FullscreenClient,
    FullscreenInternal,
    Content,
}

/// What a rule can match on.
#[derive(Debug, Clone, Default)]
pub struct WindowProps {
    pub class: String,
    pub title: String,
    pub initial_class: String,
    pub initial_title: String,
    pub floating: bool,
    pub fullscreen: bool,
    pub pinned: bool,
    pub focus: bool,
    pub workspace_id: i32,
    pub workspace_name: String,
    /// Static tags as `name`, dynamic ones as `name*`.
    pub tags: Vec<String>,
    /// 0 none, 1 maximized, 2 fullscreen.
    pub fullscreen_state: i32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Matchers {
    pub class: Option<Re>,
    pub title: Option<Re>,
    pub initial_class: Option<Re>,
    pub initial_title: Option<Re>,
    pub xdg_tag: Option<Re>,
    pub namespace: Option<Re>,
    pub tag: Option<String>,
    pub workspace: Option<String>,
    pub bools: Vec<(BoolProp, bool)>,
    pub ints: Vec<(IntProp, i32)>,
}

fn parse_bool(v: &str) -> Result<bool, String> {
    flag(v).and_then(|b| {
        if v.trim().is_empty() {
            Err("missing value".into())
        } else {
            Ok(b)
        }
    })
}

fn ws_matches(sel: &str, id: i32, name: &str) -> bool {
    let sel = sel.trim();
    if let Ok(n) = sel.parse::<i32>() {
        id == n
    } else if let Some(n) = sel.strip_prefix("name:") {
        name == n
    } else if let Some(n) = sel.strip_prefix("special:") {
        name == format!("special:{n}")
    } else {
        name == sel
    }
}

impl Matchers {
    fn set(&mut self, key: &str, v: &str) -> Result<(), String> {
        let re = || Re::new(v);
        match key {
            "class" => self.class = Some(re()?),
            "title" => self.title = Some(re()?),
            "initial_class" => self.initial_class = Some(re()?),
            "initial_title" => self.initial_title = Some(re()?),
            "xdg_tag" => self.xdg_tag = Some(re()?),
            "namespace" => self.namespace = Some(re()?),
            "tag" => self.tag = Some(v.trim().to_string()),
            // shortcut: only id / name:X / special:X selectors, not `m[..]`, `w[..]`.
            "workspace" | "on_workspace" => self.workspace = Some(v.trim().to_string()),
            "xwayland" => self.bools.push((BoolProp::Xwayland, parse_bool(v)?)),
            "float" => self.bools.push((BoolProp::Float, parse_bool(v)?)),
            "fullscreen" => self.bools.push((BoolProp::Fullscreen, parse_bool(v)?)),
            "pin" => self.bools.push((BoolProp::Pin, parse_bool(v)?)),
            "focus" => self.bools.push((BoolProp::Focus, parse_bool(v)?)),
            "group" => self.bools.push((BoolProp::Group, parse_bool(v)?)),
            "modal" => self.bools.push((BoolProp::Modal, parse_bool(v)?)),
            "fullscreen_state_client" | "fullscreen_state_internal" | "content" => {
                let n = v
                    .trim()
                    .parse()
                    .map_err(|_| format!("match:{key}: expected an integer, got `{v}`"))?;
                let p = match key {
                    "content" => IntProp::Content,
                    "fullscreen_state_client" => IntProp::FullscreenClient,
                    _ => IntProp::FullscreenInternal,
                };
                self.ints.push((p, n));
            }
            other => return Err(format!("unknown match prop `{other}`")),
        }
        Ok(())
    }

    pub fn has_focus(&self) -> bool {
        self.bools.iter().any(|(p, _)| *p == BoolProp::Focus)
    }

    fn focus_is_false(&self) -> bool {
        self.bools.contains(&(BoolProp::Focus, false))
    }

    pub fn matches(&self, p: &WindowProps) -> bool {
        let re = |r: &Option<Re>, t: &str| r.as_ref().is_none_or(|r| r.is_match(t));
        re(&self.class, &p.class)
            && re(&self.title, &p.title)
            && re(&self.initial_class, &p.initial_class)
            && re(&self.initial_title, &p.initial_title)
            && re(&self.xdg_tag, "")
            && self.tag.as_ref().is_none_or(|want| {
                p.tags
                    .iter()
                    .any(|t| t == want || t.strip_suffix('*') == Some(want.as_str()))
            })
            && self
                .workspace
                .as_ref()
                .is_none_or(|s| ws_matches(s, p.workspace_id, &p.workspace_name))
            && self.bools.iter().all(|(prop, want)| {
                let have = match prop {
                    BoolProp::Float => p.floating,
                    BoolProp::Fullscreen => p.fullscreen,
                    BoolProp::Pin => p.pinned,
                    BoolProp::Focus => p.focus,
                    // No XWayland, groups or modal handling yet.
                    BoolProp::Xwayland | BoolProp::Group | BoolProp::Modal => false,
                };
                have == *want
            })
            && self.ints.iter().all(|(prop, want)| match prop {
                IntProp::Content => *want == 0,
                // shortcut: client and internal fullscreen state are the same.
                _ => *want == p.fullscreen_state,
            })
    }
}

// -------------------------------------------------------------------- rules

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: Option<String>,
    pub enabled: bool,
    pub matchers: Matchers,
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerRule {
    #[allow(dead_code)]
    pub name: Option<String>,
    pub namespace: Option<Re>,
    /// Canonical effect name and argument.
    pub effects: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    pub window: Vec<Rule>,
    pub layer: Vec<LayerRule>,
}

impl Rules {
    pub fn uses_focus(&self) -> bool {
        self.window.iter().any(|r| r.matchers.has_focus())
    }
}

/// Builds a rule from `(key, value)` pairs: `name`, `enable`, `match:PROP`
/// and effects.
pub fn parse_window_pairs(pairs: &[(String, String)]) -> Result<Rule, String> {
    let mut rule = Rule {
        name: None,
        enabled: true,
        matchers: Matchers::default(),
        effects: Vec::new(),
    };
    let mut has_match = false;
    for (k, v) in pairs {
        let k = k.trim().to_ascii_lowercase();
        if k == "name" {
            rule.name = Some(v.trim().to_string());
        } else if k == "enable" || k == "enabled" {
            rule.enabled = flag(v)?;
        } else if let Some(m) = k.strip_prefix("match:") {
            rule.matchers.set(m, v)?;
            has_match = true;
        } else {
            rule.effects.push(Effect::parse(&k, v)?);
        }
    }
    if !has_match {
        return Err("window rule needs at least one `match:` prop".into());
    }
    Ok(rule)
}

const LEGACY_PROPS: &[(&str, &str)] = &[
    ("class", "class"),
    ("title", "title"),
    ("initialclass", "initial_class"),
    ("initialtitle", "initial_title"),
    ("floating", "float"),
    ("fullscreen", "fullscreen"),
    ("pinned", "pin"),
    ("workspace", "workspace"),
    ("onworkspace", "on_workspace"),
    ("xwayland", "xwayland"),
    ("tag", "tag"),
    ("focus", "focus"),
    ("group", "group"),
    ("modal", "modal"),
    ("xdgtag", "xdg_tag"),
    ("content", "content"),
    ("fullscreenstate", "fullscreen_state_internal"),
];

/// Pre-v0.53 rule names to canonical effects.
fn legacy_effect(name: &str, arg: &str) -> (String, String) {
    let n = name.to_ascii_lowercase();
    let (n, a) = match n.as_str() {
        "noborder" => ("border_size", "0"),
        "norounding" => ("rounding", "0"),
        "noinitialfocus" => ("no_initial_focus", arg),
        "bordersize" => ("border_size", arg),
        "bordercolor" => ("border_color", arg),
        "suppressevent" => ("suppress_event", arg),
        "stayfocused" => ("stay_focused", arg),
        "idleinhibit" => ("idle_inhibit", arg),
        "keepaspectratio" => ("keep_aspect_ratio", arg),
        "nomaxsize" => ("no_max_size", arg),
        "minsize" => ("min_size", arg),
        "maxsize" => ("max_size", arg),
        "forcergbx" => ("force_rgbx", arg),
        "dimaround" => ("dim_around", arg),
        "nearestneighbor" => ("nearest_neighbor", arg),
        "noanim" => ("no_anim", arg),
        "noblur" => ("no_blur", arg),
        "nodim" => ("no_dim", arg),
        "nofocus" => ("no_focus", arg),
        "noshadow" => ("no_shadow", arg),
        "nofollowmouse" => ("no_follow_mouse", arg),
        "noscreenshare" => ("no_screen_share", arg),
        "noshortcutsinhibit" => ("no_shortcuts_inhibit", arg),
        "novrr" => ("no_vrr", arg),
        "allowsinput" => ("allows_input", arg),
        "focusonactivate" => ("focus_on_activate", arg),
        "renderunfocused" => ("render_unfocused", arg),
        "syncfullscreen" => ("sync_fullscreen", arg),
        "persistentsize" => ("persistent_size", arg),
        "roundingpower" => ("rounding_power", arg),
        "fullscreenstate" => ("fullscreen_state", arg),
        "scrollmouse" => ("scroll_mouse", arg),
        "scrolltouchpad" => ("scroll_touchpad", arg),
        _ => (n.as_str(), arg),
    };
    (n.to_string(), a.to_string())
}

fn split_kv(part: &str) -> (String, String) {
    let p = part.trim();
    match p.split_once(char::is_whitespace) {
        Some((k, v)) => (k.to_string(), v.trim().to_string()),
        None => (p.to_string(), String::new()),
    }
}

fn parts(value: &str) -> Vec<&str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
}

/// Value of `windowrule = ...` or `windowrulev2 = ...`, canonical or legacy.
pub fn parse_inline_window(value: &str) -> Result<Rule, String> {
    let parts = parts(value);
    if parts.iter().any(|p| p.starts_with("match:")) {
        let pairs: Vec<_> = parts.iter().map(|p| split_kv(p)).collect();
        return parse_window_pairs(&pairs);
    }
    let Some((first, matchers)) = parts.split_first().filter(|(_, m)| !m.is_empty()) else {
        return Err(format!("window rule needs a rule and a matcher: `{value}`"));
    };
    let (name, arg) = split_kv(first);
    let mut pairs = vec![legacy_effect(&name, &arg)];
    for m in matchers {
        let prop = m.split_once(':').and_then(|(k, v)| {
            LEGACY_PROPS
                .iter()
                .find(|(l, _)| l.eq_ignore_ascii_case(k.trim()))
                .map(|(_, c)| (format!("match:{c}"), v.trim().to_string()))
        });
        // `windowrule = float, ^(kitty)$`: a bare regex is the class.
        pairs.push(prop.unwrap_or_else(|| ("match:class".into(), m.to_string())));
    }
    parse_window_pairs(&pairs)
}

const LAYER_EFFECTS: &[&str] = &[
    "no_anim",
    "blur",
    "blur_popups",
    "ignore_alpha",
    "dim_around",
    "xray",
    "animation",
    "order",
    "above_lock",
    "no_screen_share",
];

pub fn parse_layer_pairs(pairs: &[(String, String)]) -> Result<LayerRule, String> {
    let mut rule = LayerRule {
        name: None,
        namespace: None,
        effects: Vec::new(),
    };
    for (k, v) in pairs {
        let k = k.trim().to_ascii_lowercase();
        if k == "name" {
            rule.name = Some(v.trim().to_string());
        } else if k == "enable" || k == "enabled" {
            // accepted, rules are always on
        } else if let Some(m) = k.strip_prefix("match:") {
            if m != "namespace" {
                return Err(format!("unknown layer rule match prop `{m}`"));
            }
            rule.namespace = Some(Re::new(v)?);
        } else if LAYER_EFFECTS.contains(&k.as_str()) {
            rule.effects.push((k, v.trim().to_string()));
        } else {
            return Err(format!("unknown layer rule effect `{k}`"));
        }
    }
    if rule.namespace.is_none() {
        return Err("layer rule needs `match:namespace`".into());
    }
    Ok(rule)
}

/// Value of `layerrule = ...`, canonical or legacy (`blur, waybar`).
pub fn parse_inline_layer(value: &str) -> Result<LayerRule, String> {
    let parts = parts(value);
    if parts.iter().any(|p| p.starts_with("match:")) {
        let pairs: Vec<_> = parts.iter().map(|p| split_kv(p)).collect();
        return parse_layer_pairs(&pairs);
    }
    let [rule, ns] = parts.as_slice() else {
        return Err(format!(
            "layer rule needs a rule and a namespace: `{value}`"
        ));
    };
    let (name, arg) = split_kv(rule);
    let (name, arg) = match name.to_ascii_lowercase().as_str() {
        "noanim" => ("no_anim".to_string(), arg),
        "blurpopups" => ("blur_popups".into(), arg),
        "ignorealpha" => ("ignore_alpha".into(), arg),
        "ignorezero" => ("ignore_alpha".into(), "0".into()),
        "dimaround" => ("dim_around".into(), arg),
        "abovelock" => ("above_lock".into(), arg),
        "noscreenshare" => ("no_screen_share".into(), arg),
        other => (other.to_string(), arg),
    };
    parse_layer_pairs(&[(name, arg), ("match:namespace".to_string(), ns.to_string())])
}

/// Effects of every layer rule matching `namespace`, in config order.
pub fn layer_effects<'a>(rules: &'a [LayerRule], namespace: &str) -> Vec<(&'a str, &'a str)> {
    rules
        .iter()
        .filter(|r| r.namespace.as_ref().is_none_or(|n| n.is_match(namespace)))
        .flat_map(|r| r.effects.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .collect()
}

// --------------------------------------------------------------- evaluation

/// Enabled rules in evaluation order: named ones first, then anonymous.
fn ordered(rules: &[Rule]) -> impl Iterator<Item = &Rule> {
    let enabled = |r: &&Rule| r.enabled;
    rules
        .iter()
        .filter(enabled)
        .filter(|r| r.name.is_some())
        .chain(rules.iter().filter(enabled).filter(|r| r.name.is_none()))
}

/// Result of the dynamic effects for one window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DynProps {
    pub opacity: Option<Opacity>,
    pub border_size: Option<i32>,
    pub border_color: Option<BorderColor>,
    pub rounding: Option<i32>,
    /// Dynamic tags without the `*` suffix.
    pub tags: Vec<String>,
}

fn apply_tag(tags: &mut Vec<String>, arg: &str) {
    let (op, name) = match arg.chars().next() {
        Some(c @ ('+' | '-')) => (Some(c), &arg[1..]),
        _ => (None, arg),
    };
    let have = tags.iter().position(|t| t == name);
    match (op, have) {
        (Some('+'), None) | (None, None) => tags.push(name.to_string()),
        (Some('-'), Some(i)) | (None, Some(i)) => {
            tags.remove(i);
        }
        _ => {}
    }
}

/// Tags as listed by `hyprctl clients`: static, then dynamic with `*`.
pub fn all_tags(static_tags: &[String], dynamic: &[String]) -> Vec<String> {
    static_tags
        .iter()
        .cloned()
        .chain(dynamic.iter().map(|t| format!("{t}*")))
        .collect()
}

/// Evaluates dynamic effects. `p.tags` must hold the static tags only; the
/// dynamic ones are computed here, so later rules can match earlier `tag`
/// effects.
pub fn dynamic_props(rules: &[Rule], p: &WindowProps) -> DynProps {
    let static_tags: Vec<String> = p
        .tags
        .iter()
        .filter(|t| !t.ends_with('*'))
        .cloned()
        .collect();
    let mut p = p.clone();
    let mut out = DynProps::default();
    for r in ordered(rules) {
        p.tags = all_tags(&static_tags, &out.tags);
        if !r.matchers.matches(&p) {
            continue;
        }
        for e in &r.effects {
            match e {
                Effect::Opacity(o) => out.opacity = Some(*o),
                Effect::BorderSize(n) => out.border_size = Some(*n),
                Effect::Rounding(n) => out.rounding = Some(*n),
                Effect::BorderColor(c) => {
                    let mut c = *c;
                    // `match:focus 0` + one color sets the inactive color.
                    if r.matchers.focus_is_false() && c.inactive.is_none() {
                        c = BorderColor {
                            active: None,
                            inactive: c.active,
                        };
                    }
                    let old = out.border_color.unwrap_or_default();
                    out.border_color = Some(BorderColor {
                        active: c.active.or(old.active),
                        inactive: c.inactive.or(old.inactive),
                    });
                }
                Effect::Tag(t) => apply_tag(&mut out.tags, t),
                _ => {}
            }
        }
    }
    out
}

/// The static effects of one window, folded (last wins).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StaticPlan {
    pub float: Option<bool>,
    pub fullscreen: bool,
    pub maximize: bool,
    pub pin: bool,
    pub center: bool,
    pub no_initial_focus: bool,
    pub size: Option<(String, String)>,
    pub mv: Option<(String, String)>,
    /// Selector and `silent`.
    pub workspace: Option<(String, bool)>,
    pub suppress_event: Vec<String>,
}

/// `p.tags` must contain static and dynamic tags (see [`all_tags`]).
pub fn static_plan(rules: &[Rule], p: &WindowProps) -> StaticPlan {
    let mut plan = StaticPlan::default();
    for r in ordered(rules).filter(|r| r.matchers.matches(p)) {
        for e in &r.effects {
            match e {
                Effect::Float(b) => plan.float = Some(*b),
                Effect::Fullscreen(b) => plan.fullscreen = *b,
                Effect::Maximize(b) => plan.maximize = *b,
                Effect::Pin(b) => plan.pin = *b,
                Effect::Center(b) => plan.center = *b,
                Effect::NoInitialFocus(b) => plan.no_initial_focus = *b,
                Effect::Size(a, b) => plan.size = Some((a.clone(), b.clone())),
                Effect::Move(a, b) => plan.mv = Some((a.clone(), b.clone())),
                Effect::Workspace { sel, silent } => {
                    plan.workspace = sel.clone().map(|s| (s, *silent))
                }
                Effect::SuppressEvent(v) => plan.suppress_event.clone_from(v),
                // shortcut: single monitor, so `monitor` is accepted and ignored.
                _ => {}
            }
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn props(class: &str, title: &str) -> WindowProps {
        WindowProps {
            class: class.into(),
            title: title.into(),
            initial_class: class.into(),
            initial_title: title.into(),
            workspace_id: 1,
            workspace_name: "1".into(),
            ..Default::default()
        }
    }

    fn rule(v: &str) -> Rule {
        parse_inline_window(v).unwrap_or_else(|e| panic!("{v}: {e}"))
    }

    #[test]
    fn regex_is_full_match_and_negatable() {
        let r = Re::new("kitty").unwrap();
        assert!(r.is_match("kitty"));
        assert!(!r.is_match("kitty2"));
        assert!(Re::new(".*Hyprland.*").unwrap().is_match("a Hyprland b"));
        assert!(Re::new("^(foo|bar)$").unwrap().is_match("bar"));
        let n = Re::new("negative:kitty").unwrap();
        assert!(!n.is_match("kitty"));
        assert!(n.is_match("foot"));
        assert!(Re::new("(").is_err());
    }

    #[test]
    fn expressions() {
        let v = Vars {
            monitor_w: 1000.0,
            monitor_h: 800.0,
            window_w: 200.0,
            window_h: 100.0,
            cursor_x: 50.0,
            ..Default::default()
        };
        let e = |s| eval_expr(s, &v, 1000.0);
        assert_eq!(e("800"), Ok(800.0));
        assert_eq!(e("100%-w"), Ok(800.0));
        assert_eq!(e("50%"), Ok(500.0));
        assert_eq!(e("(monitor_w/2)+17"), Ok(517.0));
        assert_eq!(e("cursor_x-(window_w*0.5)"), Ok(-50.0));
        assert_eq!(e("-h"), Ok(-100.0));
        assert!(e("1/0").is_err());
        assert!(e("foo").is_err());
        assert!(e("1 +").is_err());
    }

    #[test]
    fn canonical_inline_rule() {
        let r =
            rule("match:class kitty, match:float 1, float on, size 800 600, workspace 3 silent");
        assert_eq!(r.matchers.class, Some(Re::new("kitty").unwrap()));
        assert_eq!(r.matchers.bools, vec![(BoolProp::Float, true)]);
        assert_eq!(
            r.effects,
            vec![
                Effect::Float(true),
                Effect::Size("800".into(), "600".into()),
                Effect::Workspace {
                    sel: Some("3".into()),
                    silent: true
                },
            ]
        );
        assert!(parse_inline_window("float on").is_err());
        assert!(parse_inline_window("match:class x, bogus on").is_err());
        assert!(parse_inline_window("match:class x, size 100").is_err());
    }

    #[test]
    fn legacy_v2_and_v1_map_to_canonical() {
        let canon = rule("match:class ^(kitty)$, match:title foo, no_initial_focus on");
        assert_eq!(
            rule("noinitialfocus, class:^(kitty)$, title:foo").effects,
            canon.effects
        );
        assert_eq!(
            rule("noinitialfocus, class:^(kitty)$, title:foo").matchers,
            canon.matchers
        );
        // v1: bare regex is the class
        assert_eq!(
            rule("float, ^(kitty)$").matchers.class,
            Some(Re::new("^(kitty)$").unwrap())
        );
        let r = rule("workspace 2 silent, initialClass:foo, floating:1, onworkspace:3");
        assert_eq!(r.matchers.initial_class, Some(Re::new("foo").unwrap()));
        assert_eq!(r.matchers.workspace, Some("3".into()));
        assert_eq!(
            rule("noborder, class:x").effects,
            vec![Effect::BorderSize(0)]
        );
        assert_eq!(rule("float, class:x").effects, vec![Effect::Float(true)]);
    }

    #[test]
    fn block_pairs() {
        let pairs: Vec<(String, String)> = [
            ("name", "r"),
            ("match:class", "kitty"),
            ("float", "on"),
            ("opacity", "0.8 override 0.5"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let r = parse_window_pairs(&pairs).unwrap();
        assert_eq!(r.name.as_deref(), Some("r"));
        let Effect::Opacity(o) = r.effects[1] else {
            panic!()
        };
        assert_eq!(o.active, (0.8, true));
        assert_eq!(o.inactive, (0.5, false));
    }

    #[test]
    fn matchers_and_order() {
        let mut p = props("kitty", "hello Hyprland");
        let rules = vec![
            rule("match:class kitty, opacity 0.8"),
            rule("match:float 1, opacity 0.5"),
            rule("match:title .*Hyprland.*, rounding 7"),
        ];
        let d = dynamic_props(&rules, &p);
        assert_eq!(d.opacity.unwrap().active.0, 0.8);
        assert_eq!(d.rounding, Some(7));
        p.floating = true;
        assert_eq!(dynamic_props(&rules, &p).opacity.unwrap().active.0, 0.5);
        // named rules are evaluated before anonymous ones
        let rules = vec![
            rule("match:class kitty, border_size 1"),
            parse_window_pairs(&[
                ("name".into(), "n".into()),
                ("match:class".into(), "kitty".into()),
                ("border_size".into(), "9".into()),
            ])
            .unwrap(),
        ];
        assert_eq!(dynamic_props(&rules, &p).border_size, Some(1));
        // workspace and negative matchers
        let r = rule("match:workspace name:web, match:class negative:foo, float on");
        let mut p = props("kitty", "");
        assert!(!r.matchers.matches(&p));
        p.workspace_name = "web".into();
        p.workspace_id = -1337;
        assert!(r.matchers.matches(&p));
        assert!(!rule("match:xwayland 1, float on").matchers.matches(&p));
        assert!(rule("match:xwayland 0, float on").matchers.matches(&p));
    }

    #[test]
    fn tags_chain() {
        let rules = vec![
            rule("match:class footclient, tag +term"),
            rule("match:tag term, tag +code"),
            rule("match:tag code, opacity 0.8"),
            rule("match:tag term*, rounding 3"),
            rule("match:tag term, tag -code"),
        ];
        let d = dynamic_props(&rules, &props("footclient", ""));
        assert_eq!(d.tags, vec!["term".to_string()]);
        assert_eq!(d.opacity.unwrap().active.0, 0.8);
        assert_eq!(d.rounding, Some(3));
        // static tag matches `name` only, `name*` rules need a dynamic one
        let mut p = props("x", "");
        p.tags = vec!["term".into()];
        assert!(dynamic_props(&rules, &p).rounding.is_none());
        let mut v = vec![];
        apply_tag(&mut v, "a");
        apply_tag(&mut v, "a");
        assert!(v.is_empty());
        assert_eq!(
            all_tags(&["s".into()], &["d".into()]),
            vec!["s".to_string(), "d*".to_string()]
        );
    }

    #[test]
    fn border_color_forms() {
        let c = |s| parse_border_color(s).unwrap();
        let red = [1.0, 0.0, 0.0, 1.0];
        assert_eq!(c("rgb(FF0000)").active, Some(red));
        assert_eq!(c("rgb(FF0000)").inactive, None);
        assert_eq!(
            c("rgb(FF0000) rgb(00FF00)").inactive,
            Some([0.0, 1.0, 0.0, 1.0])
        );
        let g = c("rgb(FF0000) rgb(00FF00) 45deg");
        assert_eq!((g.active, g.inactive), (Some(red), None));
        // focus 0 + one color is the inactive color
        let r = rule("match:class x, match:focus 0, border_color rgb(FF0000)");
        let d = dynamic_props(&[r], &props("x", ""));
        assert_eq!(d.border_color.unwrap().inactive, Some(red));
    }

    #[test]
    fn static_plan_folds_last_wins() {
        let rules = vec![
            rule("match:class x, float on, size 800 600, workspace 2"),
            rule("match:class x, tile on, workspace 4 silent, center on, no_initial_focus on"),
            rule("match:class y, fullscreen on"),
        ];
        let plan = static_plan(&rules, &props("x", ""));
        assert_eq!(plan.float, Some(false));
        assert_eq!(plan.size, Some(("800".into(), "600".into())));
        assert_eq!(plan.workspace, Some(("4".into(), true)));
        assert!(plan.center && plan.no_initial_focus && !plan.fullscreen);
        let unset = vec![
            rule("match:class x, workspace 2"),
            rule("match:class x, workspace unset"),
        ];
        assert_eq!(static_plan(&unset, &props("x", "")).workspace, None);
    }

    #[test]
    fn layer_rules() {
        let a = parse_inline_layer("blur on, match:namespace waybar").unwrap();
        let b = parse_inline_layer("ignorezero, waybar").unwrap();
        let c = parse_inline_layer("noanim, selection").unwrap();
        assert_eq!(
            b.effects,
            vec![("ignore_alpha".to_string(), "0".to_string())]
        );
        let all = vec![a, b, c];
        assert_eq!(
            layer_effects(&all, "waybar"),
            vec![("blur", "on"), ("ignore_alpha", "0")]
        );
        assert_eq!(layer_effects(&all, "selection"), vec![("no_anim", "")]);
        assert!(parse_inline_layer("blur on").is_err());
        assert!(parse_inline_layer("bogus, x").is_err());
    }

    #[test]
    fn config_lines_and_blocks() {
        let (cfg, errors) = Config::parse_str(
            "windowrule = match:class kitty, float on\n\
             windowrulev2 = size 100 100, class:foo\n\
             windowrule {\n  name = b\n  match:class = bar\n  match:title = a, b\n  float = on\n}\n\
             layerrule = blur on, match:namespace waybar\n\
             windowrule = bogus\n",
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(errors[0].line, 10);
        assert_eq!(cfg.rules.window.len(), 3);
        assert_eq!(cfg.rules.window[2].name.as_deref(), Some("b"));
        assert_eq!(cfg.rules.layer.len(), 1);
    }
}
