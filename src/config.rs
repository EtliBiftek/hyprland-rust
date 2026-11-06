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

/// Actions a key binding can trigger (named after Hyprland's dispatchers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatcher {
    Exec(String),
    KillActive,
    Exit,
    Workspace(usize),
    MoveToWorkspace(usize),
    ToggleFloating,
    Fullscreen,
    MoveFocus(Direction),
    CycleNext,
    ToggleSplit,
}

impl Dispatcher {
    pub fn parse(name: &str, arg: &str) -> Result<Self, String> {
        let arg = arg.trim();
        let workspace = |a: &str| -> Result<usize, String> {
            a.parse::<usize>()
                .ok()
                .filter(|n| (1..=crate::workspace::MAX_WORKSPACES).contains(n))
                .ok_or_else(|| {
                    format!(
                        "invalid workspace `{a}` (1-{})",
                        crate::workspace::MAX_WORKSPACES
                    )
                })
        };
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "exec" => Dispatcher::Exec(arg.to_string()),
            "killactive" => Dispatcher::KillActive,
            "exit" => Dispatcher::Exit,
            "workspace" => Dispatcher::Workspace(workspace(arg)?),
            "movetoworkspace" | "movetoworkspacesilent" => {
                Dispatcher::MoveToWorkspace(workspace(arg)?)
            }
            "togglefloating" => Dispatcher::ToggleFloating,
            "fullscreen" => Dispatcher::Fullscreen,
            "movefocus" => Dispatcher::MoveFocus(Direction::parse(arg)?),
            "cyclenext" => Dispatcher::CycleNext,
            "togglesplit" => Dispatcher::ToggleSplit,
            other => return Err(format!("unknown dispatcher `{other}`")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    pub mods: Mods,
    /// Lower-cased xkb keysym name, e.g. `return`, `q`, `1`.
    pub key: String,
    pub dispatcher: Dispatcher,
}

#[derive(Debug, Clone, PartialEq)]
pub struct General {
    pub gaps_in: i32,
    pub gaps_out: i32,
    pub border_size: i32,
    /// RGBA, 0.0-1.0
    pub active_border_color: [f32; 4],
    pub inactive_border_color: [f32; 4],
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
}

impl Default for Dwindle {
    fn default() -> Self {
        Self {
            preserve_split: false,
            default_split_ratio: 1.0,
        }
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
    pub binds: Vec<Bind>,
    pub exec_once: Vec<String>,
    pub exec: Vec<String>,
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
        Some(base.join("hypr").join("hyprland.conf"))
    }

    /// Loads a config file. Errors on individual lines are collected and
    /// returned alongside the config, so one typo doesn't discard everything.
    pub fn load(path: &Path) -> std::io::Result<(Config, Vec<ConfigError>)> {
        let text = std::fs::read_to_string(path)?;
        let mut parser = Parser::default();
        parser.parse(&text, path);
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
            "bind" | "binde" | "bindr" | "bindl" => cfg.binds.push(parse_bind(value)?),
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
        "input:",
        "gestures:",
        "misc:",
        "dwindle:",
        "master:",
        "xwayland:",
        "cursor:",
        "monitor",
        "env",
        "windowrule",
        "windowrulev2",
        "layerrule",
        "workspace",
        "bindm",
        "device",
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

fn parse_bind(value: &str) -> Result<Bind, String> {
    let mut parts = value.splitn(4, ',');
    let mods = parts.next().ok_or("bind: missing modifiers")?;
    let key = parts.next().ok_or("bind: missing key")?.trim();
    let dispatcher = parts.next().ok_or("bind: missing dispatcher")?;
    let arg = parts.next().unwrap_or("");
    if key.is_empty() {
        return Err("bind: empty key".into());
    }
    Ok(Bind {
        mods: Mods::parse(mods)?,
        key: key.to_ascii_lowercase(),
        dispatcher: Dispatcher::parse(dispatcher, arg)?,
    })
}

fn parse_int(value: &str) -> Result<i32, String> {
    value
        .parse()
        .map_err(|_| format!("invalid integer `{value}`"))
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
            }
        );
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
}
