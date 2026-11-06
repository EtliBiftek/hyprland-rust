//! `hyprland.lua` support (the config format of Hyprland >= 0.55; v0.57 reads
//! nothing else).
//!
//! The script runs once per (re)load against a small `hl` API that mirrors
//! Hyprland v0.57.1's (`src/config/lua/bindings/`). Every `hl.*` call is
//! translated into the equivalent hyprlang line and the resulting text is fed
//! to the normal hyprlang parser, so options, binds, rules, reload and
//! `hyprctl keyword` share one code path. `hl.dsp.*` return the hyprlang
//! dispatcher text (`"name, args"`), which is also how `hyprctl dispatch
//! 'hl.dsp.exit()'` is evaluated.
//!
//! shortcut: no Lua state survives the load, so Lua callbacks (`hl.bind` with
//! a function, `hl.on` events other than `hyprland.start`, `hl.timer`, query
//! functions) are reported as config errors. Upgrade by keeping the `Lua` in
//! `Hypr` and adding a `Dispatcher::Lua(RegistryKey)`.

use std::{cell::RefCell, path::Path, rc::Rc};

use mlua::{Lua, Table, Value};

#[derive(Default)]
struct Out {
    lines: Vec<String>,
    errors: Vec<String>,
    /// Inside `hl.on("hyprland.start", ...)`: `hl.exec_cmd` means exec-once.
    on_start: bool,
}

type Shared = Rc<RefCell<Out>>;

/// Runs a Lua config and returns the equivalent hyprlang text plus the
/// errors of individual `hl.*` calls. `Err` means the script itself failed.
pub fn to_hyprlang(src: &str, dir: &Path, name: &str) -> Result<(String, Vec<String>), String> {
    let lua = Lua::new();
    let out: Shared = Rc::default();
    install(&lua, &out, dir).map_err(|e| e.to_string())?;
    lua.load(src)
        .set_name(name)
        .exec()
        .map_err(|e| e.to_string())?;
    let out = out.borrow();
    Ok((out.lines.join("\n"), out.errors.clone()))
}

/// Evaluates a Lua dispatcher expression such as `hl.dsp.exit()` (what
/// `hyprctl dispatch` takes in Hyprland >= 0.57) into `(name, args)`.
pub fn eval_dispatch(expr: &str) -> Result<(String, String), String> {
    let lua = Lua::new();
    let out: Shared = Rc::default();
    install(&lua, &out, Path::new(".")).map_err(|e| e.to_string())?;
    let v: Value = lua
        .load(expr)
        .set_name("dispatch")
        .eval()
        .map_err(|e| e.to_string())?;
    match v {
        Value::String(s) => Ok(split_dsp(&s.to_string_lossy())),
        _ => Err("expected a dispatcher such as hl.dsp.exit()".into()),
    }
}

fn split_dsp(text: &str) -> (String, String) {
    match text.split_once(',') {
        Some((n, a)) => (n.trim().into(), a.trim().into()),
        None => (text.trim().into(), String::new()),
    }
}

// ------------------------------------------------------------- values

fn is_color_key(key: &str) -> bool {
    key.contains("col.") || key.rsplit([':', '.']).next() == Some("color")
}

/// A Lua value as hyprlang text. Integers under color keys become
/// `0xAARRGGBB`; gradient tables `{ colors = {...}, angle = N }` become
/// `c1 c2 Ndeg`; arrays are joined with spaces.
fn scalar(v: &Value, key: &str) -> Result<String, String> {
    Ok(match v {
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) if is_color_key(key) => format!("0x{:08x}", *i as u32),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_string_lossy(),
        Value::Table(t) => {
            if let Ok(Value::Table(colors)) = t.get::<Value>("colors") {
                let mut parts = Vec::new();
                for c in colors.sequence_values::<Value>() {
                    parts.push(scalar(&c.map_err(|e| e.to_string())?, key)?);
                }
                if let Ok(a) = t.get::<Value>("angle") {
                    if !matches!(a, Value::Nil) {
                        parts.push(format!("{}deg", scalar(&a, "angle")?));
                    }
                }
                parts.join(" ")
            } else {
                let mut parts = Vec::new();
                for x in t.sequence_values::<Value>() {
                    parts.push(scalar(&x.map_err(|e| e.to_string())?, key)?);
                }
                parts.join(" ")
            }
        }
        other => {
            return Err(format!(
                "`{key}`: unsupported value of type {}",
                other.type_name()
            ))
        }
    })
}

/// Is this table a value (gradient or array) rather than a category?
fn is_value_table(t: &Table) -> bool {
    t.contains_key("colors").unwrap_or(false) || t.raw_len() > 0
}

/// `{ general = { gaps_in = 5, col = { active_border = ... } } }` into
/// `general:gaps_in = 5`, `general:col.active_border = ...`.
fn flatten(prefix: &str, t: &Table, lines: &mut Vec<String>) -> Result<(), String> {
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        let key = if prefix.is_empty() {
            k
        } else if prefix.rsplit(':').next() == Some("col") {
            format!("{prefix}.{k}")
        } else {
            format!("{prefix}:{k}")
        };
        match &v {
            Value::Table(sub) if !is_value_table(sub) => flatten(&key, sub, lines)?,
            _ => lines.push(format!("{key} = {}", scalar(&v, &key)?)),
        }
    }
    Ok(())
}

// --------------------------------------------------------- dispatchers

/// Argument of a `hl.dsp.*` call: a table of named fields or a plain value.
struct A(Value);

impl A {
    fn s(&self, k: &str) -> Option<String> {
        match &self.0 {
            Value::Table(t) => match t.get::<Value>(k).ok()? {
                Value::Nil => None,
                v => scalar(&v, k).ok(),
            },
            _ => None,
        }
    }

    fn b(&self, k: &str) -> Option<bool> {
        match &self.0 {
            Value::Table(t) => match t.get::<Value>(k).ok()? {
                Value::Boolean(b) => Some(b),
                _ => None,
            },
            _ => None,
        }
    }

    /// The plain (non-table) argument, or the named field.
    fn arg_or(&self, k: &str) -> Option<String> {
        match &self.0 {
            Value::Nil | Value::Table(_) => self.s(k),
            v => scalar(v, k).ok(),
        }
    }

    fn need(&self, k: &str, what: &str) -> Result<String, String> {
        self.arg_or(k)
            .ok_or_else(|| format!("{what}: missing `{k}`"))
    }
}

fn dir(s: &str) -> Result<&'static str, String> {
    Ok(match s {
        "left" | "l" => "l",
        "right" | "r" => "r",
        "up" | "u" | "top" | "t" => "u",
        "down" | "d" | "bottom" | "b" => "d",
        _ => return Err(format!("invalid direction `{s}`")),
    })
}

/// `toggle` / `set` / `unset` as Hyprland's togglable actions.
fn action(a: &A) -> &'static str {
    match a.s("action").as_deref() {
        Some("set" | "enable" | "on") => "on",
        Some("unset" | "disable" | "off") => "off",
        _ => "toggle",
    }
}

const DSP: &[&str] = &[
    "cursor.move_to_corner",
    "cursor.move",
    "group.toggle",
    "group.next",
    "group.prev",
    "group.active",
    "group.move_window",
    "group.lock",
    "group.lock_active",
    "window.close",
    "window.kill",
    "window.signal",
    "window.float",
    "window.fullscreen",
    "window.fullscreen_state",
    "window.pseudo",
    "window.move",
    "window.swap",
    "window.center",
    "window.cycle_next",
    "window.tag",
    "window.clear_tags",
    "window.toggle_swallow",
    "window.pin",
    "window.bring_to_top",
    "window.alter_zorder",
    "window.set_prop",
    "window.deny_from_group",
    "window.drag",
    "window.resize",
    "workspace.rename",
    "workspace.change_id",
    "workspace.move",
    "workspace.swap_monitors",
    "workspace.toggle_special",
    "exec_cmd",
    "exec_raw",
    "exit",
    "reload_config",
    "submap",
    "pass",
    "send_shortcut",
    "send_key_state",
    "layout",
    "dpms",
    "event",
    "global",
    "force_renderer_reload",
    "force_idle",
    "release_input_capture",
    "focus",
    "no_op",
];

/// `hl.dsp.<path>(arg)` as hyprlang dispatcher text.
fn translate(path: &str, a: &A) -> Result<String, String> {
    let unsupported = || Err(format!("hl.dsp.{path} is not supported yet"));
    Ok(match path {
        "exec_cmd" | "exec_raw" => format!("exec, {}", a.need("cmd", "exec_cmd")?),
        "exit" => "exit".into(),
        "reload_config" => "exec, hyprctl reload".into(),
        "submap" => format!("submap, {}", a.need("name", "submap")?),
        "pass" => format!("pass, {}", a.need("window", "pass")?),
        "layout" => format!("layoutmsg, {}", a.need("message", "layout")?),
        "dpms" => format!("dpms, {}", a.arg_or("action").unwrap_or("toggle".into())),
        "event" => format!("event, {}", a.need("data", "event")?),
        // Both are no-ops here.
        "force_renderer_reload" | "no_op" => "forcerendererreload".into(),
        "focus" => {
            if let Some(d) = a.s("direction") {
                format!("movefocus, {}", dir(&d)?)
            } else if let Some(ws) = a.s("workspace") {
                if a.b("on_current_monitor") == Some(true) {
                    format!("focusworkspaceoncurrentmonitor, {ws}")
                } else {
                    format!("workspace, {ws}")
                }
            } else if let Some(m) = a.s("monitor") {
                format!("focusmonitor, {m}")
            } else if let Some(w) = a.s("window") {
                format!("focuswindow, {w}")
            } else if a.b("urgent_or_last") == Some(true) {
                "focusurgentorlast".into()
            } else if a.b("last") == Some(true) {
                "focuscurrentorlast".into()
            } else {
                return Err("hl.dsp.focus: needs direction, workspace, monitor or window".into());
            }
        }
        "window.close" => match a.s("window") {
            Some(w) => format!("closewindow, {w}"),
            None => "killactive".into(),
        },
        "window.kill" => match a.s("window") {
            Some(w) => format!("killwindow, {w}"),
            None => "forcekillactive".into(),
        },
        "window.signal" => format!("signal, {}", a.need("signal", "window.signal")?),
        "window.float" => match action(a) {
            "on" => "setfloating".into(),
            "off" => "settiled".into(),
            _ => "togglefloating".into(),
        },
        "window.fullscreen" => {
            let mode = match a.s("mode").as_deref() {
                Some("maximized" | "1") => 1,
                _ => 0,
            };
            format!("fullscreen, {mode}")
        }
        "window.pseudo" => "togglepseudo".into(),
        "window.move" => {
            if let Some(d) = a.s("direction") {
                format!("movewindow, {}", dir(&d)?)
            } else if let Some(ws) = a.s("workspace") {
                if a.b("follow") == Some(false) {
                    format!("movetoworkspacesilent, {ws}")
                } else {
                    format!("movetoworkspace, {ws}")
                }
            } else if let (Some(x), Some(y)) = (a.s("x"), a.s("y")) {
                if a.b("relative") == Some(true) {
                    format!("moveactive, {x} {y}")
                } else {
                    format!("moveactive, exact {x} {y}")
                }
            } else if let Some(d) = a.s("into_group").or(a.s("into_or_create_group")) {
                format!("moveintogroup, {}", dir(&d)?)
            } else if let Some(d) = a.s("out_of_group") {
                match d.as_str() {
                    "true" => "moveoutofgroup".into(),
                    d => format!("moveoutofgroup, {}", dir(d)?),
                }
            } else {
                return unsupported();
            }
        }
        "window.swap" => {
            if let Some(d) = a.s("direction") {
                format!("swapwindow, {}", dir(&d)?)
            } else if a.b("prev") == Some(true) {
                "swapnext, prev".into()
            } else if a.b("next") == Some(true) {
                "swapnext".into()
            } else {
                return unsupported();
            }
        }
        "window.center" => "centerwindow".into(),
        "window.cycle_next" => match a.b("next") {
            Some(false) => "cyclenext, prev".into(),
            _ => "cyclenext".into(),
        },
        "window.tag" => format!("tagwindow, {}", a.need("tag", "window.tag")?),
        "window.toggle_swallow" => "toggleswallow".into(),
        "window.pin" => "pin".into(),
        "window.bring_to_top" => "bringactivetotop".into(),
        "window.alter_zorder" => format!("alterzorder, {}", a.need("mode", "alter_zorder")?),
        "window.set_prop" => format!(
            "setprop, active {} {}",
            a.need("prop", "set_prop")?,
            a.need("value", "set_prop")?
        ),
        "window.deny_from_group" => format!("denywindowfromgroup, {}", action(a)),
        "window.drag" => "movewindow".into(),
        "window.resize" => match (a.s("x"), a.s("y")) {
            (Some(x), Some(y)) if a.b("relative") == Some(true) => format!("resizeactive, {x} {y}"),
            (Some(x), Some(y)) => format!("resizeactive, exact {x} {y}"),
            _ => "resizewindow".into(),
        },
        "group.toggle" => "togglegroup".into(),
        "group.next" => "changegroupactive, f".into(),
        "group.prev" => "changegroupactive, b".into(),
        "group.active" => format!("changegroupactive, {}", a.need("index", "group.active")?),
        "group.move_window" => match a.b("forward") {
            Some(false) => "movegroupwindow, b".into(),
            _ => "movegroupwindow".into(),
        },
        "group.lock" if a.b("all") == Some(true) => format!("lockgroups, {}", action(a)),
        "group.lock" | "group.lock_active" => format!("lockactivegroup, {}", action(a)),
        "workspace.rename" => format!(
            "renameworkspace, {} {}",
            a.need("workspace", "workspace.rename")?,
            a.s("name").unwrap_or_default()
        ),
        "workspace.toggle_special" => match a.arg_or("name") {
            Some(n) => format!("togglespecialworkspace, {n}"),
            None => "togglespecialworkspace".into(),
        },
        "workspace.move" => format!(
            "moveworkspacetomonitor, {} {}",
            a.s("workspace").unwrap_or("current".into()),
            a.need("monitor", "workspace.move")?
        ),
        "workspace.swap_monitors" => format!(
            "swapactiveworkspaces, {} {}",
            a.need("monitor1", "swap_monitors")?,
            a.need("monitor2", "swap_monitors")?
        ),
        "cursor.move_to_corner" => {
            format!(
                "movecursortocorner, {}",
                a.need("corner", "move_to_corner")?
            )
        }
        "cursor.move" => format!(
            "movecursor, {} {}",
            a.need("x", "cursor.move")?,
            a.need("y", "cursor.move")?
        ),
        _ => return unsupported(),
    })
}

// -------------------------------------------------------- toplevel API

/// `"SUPER + SHIFT + Q"` into (`"SUPER SHIFT"`, `"Q"`).
fn split_keys(keys: &str) -> (String, String) {
    let mut parts: Vec<&str> = keys.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or("").to_string();
    (parts.join(" "), key)
}

fn bind_line(keys: &str, dsp: &str, opts: Option<&Table>) -> String {
    let (mods, key) = split_keys(keys);
    let mut flags = String::new();
    let mut desc = None;
    if let Some(o) = opts {
        let on = |k: &str| o.get::<bool>(k).unwrap_or(false);
        for (field, flag) in [
            ("locked", 'l'),
            ("release", 'r'),
            ("repeating", 'e'),
            ("non_consuming", 'n'),
            ("mouse", 'm'),
            ("transparent", 't'),
            ("ignore_mods", 'i'),
            ("long_press", 'o'),
        ] {
            if on(field) {
                flags.push(flag);
            }
        }
        desc = o.get::<String>("description").ok();
    }
    match desc {
        Some(d) => format!("bind{flags}d = {mods}, {key}, {d}, {dsp}"),
        None => format!("bind{flags} = {mods}, {key}, {dsp}"),
    }
}

/// `hl.window_rule{ name, match = {...}, effect = value, ... }` as a
/// `windowrule { }` block.
fn window_rule_block(t: &Table) -> Result<String, String> {
    let mut lines = vec!["windowrule {".to_string()];
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        match (k.as_str(), &v) {
            ("match", Value::Table(m)) => {
                for p in m.pairs::<String, Value>() {
                    let (mk, mv) = p.map_err(|e| e.to_string())?;
                    lines.push(format!("  match:{mk} = {}", scalar(&mv, &mk)?));
                }
            }
            _ => lines.push(format!("  {k} = {}", scalar(&v, &k)?)),
        }
    }
    lines.push("}".into());
    Ok(lines.join("\n"))
}

/// `hl.layer_rule{ match = { namespace = "waybar" }, blur = true }` as an
/// inline `layerrule`.
fn layer_rule_line(t: &Table) -> Result<String, String> {
    // Lua table order is unspecified: matchers first, then sorted effects.
    let (mut matchers, mut effects) = (Vec::new(), Vec::new());
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        match (k.as_str(), &v) {
            ("name", _) => {}
            ("match", Value::Table(m)) => {
                for p in m.pairs::<String, Value>() {
                    let (mk, mv) = p.map_err(|e| e.to_string())?;
                    matchers.push(format!("match:{mk} {}", scalar(&mv, &mk)?));
                }
            }
            _ => effects.push(format!("{k} {}", scalar(&v, &k)?)),
        }
    }
    matchers.sort();
    effects.sort();
    matchers.extend(effects);
    Ok(format!("layerrule = {}", matchers.join(", ")))
}

/// `hl.workspace_rule{ workspace = "2", layout = "master", ... }` as
/// `workspace = 2, layout:master, ...`.
fn workspace_rule_line(t: &Table) -> Result<String, String> {
    let ws = t
        .get::<Value>("workspace")
        .ok()
        .filter(|v| !matches!(v, Value::Nil))
        .ok_or("workspace_rule: missing `workspace`")?;
    let mut parts = vec![scalar(&ws, "workspace")?];
    let mut flat = Vec::new();
    for pair in t.pairs::<String, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        if k == "workspace" {
            continue;
        }
        match &v {
            Value::Table(sub) if !is_value_table(sub) => flatten(&k, sub, &mut flat)?,
            _ => flat.push(format!("{k} = {}", scalar(&v, &k)?)),
        }
    }
    parts.extend(flat.into_iter().map(|l| l.replacen(" = ", ":", 1)));
    Ok(format!("workspace = {}", parts.join(", ")))
}

fn device_block(t: &Table) -> Result<String, String> {
    let mut lines = vec!["device {".to_string()];
    let mut flat = Vec::new();
    flatten("", t, &mut flat)?;
    // `name` first: the parser opens a new device block on it.
    flat.sort_by_key(|l| !l.starts_with("name ="));
    lines.extend(flat.into_iter().map(|l| format!("  {l}")));
    lines.push("}".into());
    Ok(lines.join("\n"))
}

fn install(lua: &Lua, out: &Shared, dir: &Path) -> mlua::Result<()> {
    let g = lua.globals();
    let package: Table = g.get("package")?;
    let path: String = package.get("path")?;
    package.set(
        "path",
        format!(
            "{}/?.lua;{}/?/init.lua;{path}",
            dir.display(),
            dir.display()
        ),
    )?;

    let hl = lua.create_table()?;

    // hl.dsp.*
    let dsp = lua.create_table()?;
    for &p in DSP {
        let f = lua.create_function(move |_, v: Value| {
            translate(p, &A(v)).map_err(mlua::Error::external)
        })?;
        match p.split_once('.') {
            Some((group, name)) => {
                let t: Table = match dsp.get::<Value>(group)? {
                    Value::Table(t) => t,
                    _ => {
                        let t = lua.create_table()?;
                        dsp.set(group, t.clone())?;
                        t
                    }
                };
                t.set(name, f)?;
            }
            None => dsp.set(p, f)?,
        }
    }
    hl.set("dsp", dsp)?;

    // Collects a line, or records an error and keeps the script running
    // (Hyprland also reports per-call errors instead of aborting).
    let push = |out: &Shared, r: Result<String, String>| {
        let mut o = out.borrow_mut();
        match r {
            Ok(l) => o.lines.push(l),
            Err(e) => o.errors.push(e),
        }
    };

    let o = out.clone();
    hl.set(
        "config",
        lua.create_function(move |_, t: Table| {
            let mut lines = Vec::new();
            match flatten("", &t, &mut lines) {
                Ok(()) => o.borrow_mut().lines.extend(lines),
                Err(e) => o.borrow_mut().errors.push(format!("hl.config: {e}")),
            }
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "bind",
        lua.create_function(move |_, (keys, d, opts): (String, Value, Option<Table>)| {
            let r = match &d {
                Value::String(s) => Ok(bind_line(&keys, &s.to_string_lossy(), opts.as_ref())),
                Value::Function(_) => Err(format!(
                    "hl.bind(\"{keys}\"): Lua function binds are not supported yet"
                )),
                _ => Err(format!(
                    "hl.bind(\"{keys}\"): expected a hl.dsp.* dispatcher"
                )),
            };
            push(&o, r);
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "unbind",
        lua.create_function(move |_, keys: String| {
            let (mods, key) = split_keys(&keys);
            push(&o, Ok(format!("unbind = {mods}, {key}")));
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "exec_cmd",
        lua.create_function(move |_, cmd: String| {
            let kw = if o.borrow().on_start {
                "exec-once"
            } else {
                "exec"
            };
            push(&o, Ok(format!("{kw} = {cmd}")));
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "on",
        lua.create_function(move |_, (event, f): (String, mlua::Function)| {
            if event == "hyprland.start" {
                o.borrow_mut().on_start = true;
                let r = f.call::<()>(());
                o.borrow_mut().on_start = false;
                r?;
            } else {
                push(
                    &o,
                    Err(format!(
                        "hl.on(\"{event}\"): only \"hyprland.start\" is supported yet"
                    )),
                );
            }
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "define_submap",
        lua.create_function(move |_, args: mlua::Variadic<Value>| {
            let name = match args.first() {
                Some(Value::String(s)) => s.to_string_lossy(),
                _ => return Err(mlua::Error::external("define_submap: missing name")),
            };
            let f = match args.last() {
                Some(Value::Function(f)) => f.clone(),
                _ => return Err(mlua::Error::external("define_submap: missing function")),
            };
            push(&o, Ok(format!("submap = {name}")));
            let r = f.call::<()>(());
            push(&o, Ok("submap = reset".into()));
            r
        })?,
    )?;

    for (name, kind) in [
        ("window_rule", 0),
        ("layer_rule", 1),
        ("workspace_rule", 2),
        ("device", 3),
    ] {
        let o = out.clone();
        hl.set(
            name,
            lua.create_function(move |_, t: Table| {
                let r = match kind {
                    0 => window_rule_block(&t),
                    1 => layer_rule_line(&t),
                    2 => workspace_rule_line(&t),
                    _ => device_block(&t),
                };
                push(&o, r.map_err(|e| format!("hl.{name}: {e}")));
                Ok(())
            })?,
        )?;
    }

    let o = out.clone();
    hl.set(
        "env",
        lua.create_function(move |_, (k, v): (String, String)| {
            std::env::set_var(&k, &v);
            push(&o, Ok(format!("env = {k},{v}")));
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "monitor",
        lua.create_function(move |_, t: Table| {
            let f = |k: &str, d: &str| {
                t.get::<Value>(k)
                    .ok()
                    .and_then(|v| scalar(&v, k).ok())
                    .filter(|s| s != "nil" && !s.is_empty())
                    .unwrap_or(d.into())
            };
            let line = format!(
                "monitor = {}, {}, {}, {}",
                f("output", ""),
                f("mode", "preferred"),
                f("position", "auto"),
                f("scale", "auto")
            );
            push(&o, Ok(line));
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "curve",
        lua.create_function(move |_, (name, t): (String, Table)| {
            if let Ok(Value::Table(p)) = t.get::<Value>("points") {
                let pts: Vec<String> = p
                    .sequence_values::<Table>()
                    .filter_map(|x| x.ok())
                    .flat_map(|xy| {
                        xy.sequence_values::<f64>()
                            .filter_map(|n| n.ok())
                            .map(|n| n.to_string())
                            .collect::<Vec<_>>()
                    })
                    .collect();
                push(&o, Ok(format!("bezier = {name}, {}", pts.join(", "))));
            }
            Ok(())
        })?,
    )?;

    let o = out.clone();
    hl.set(
        "animation",
        lua.create_function(move |_, t: Table| {
            let leaf: String = t.get("leaf").unwrap_or_default();
            // `bool` from Lua is truthiness (nil = false), so check for nil.
            let on = !matches!(t.get::<Value>("enabled"), Ok(Value::Boolean(false))) as i32;
            let speed = t.get::<f64>("speed").unwrap_or(1.0);
            let curve: String = t
                .get::<String>("bezier")
                .or_else(|_| t.get::<String>("spring"))
                .unwrap_or("default".into());
            let mut line = format!("animation = {leaf}, {on}, {speed}, {curve}");
            if let Ok(style) = t.get::<String>("style") {
                line.push_str(&format!(", {style}"));
            }
            push(&o, Ok(line));
            Ok(())
        })?,
    )?;

    // Accepted without effect.
    for name in ["gesture", "permission", "plugin", "dispatch"] {
        hl.set(
            name,
            lua.create_function(|_, _: mlua::Variadic<Value>| Ok(()))?,
        )?;
    }
    hl.set(
        "version",
        lua.create_function(|_, ()| Ok(env!("CARGO_PKG_VERSION")))?,
    )?;

    let d = dir.to_path_buf();
    hl.set(
        "load",
        lua.create_function(move |lua, file: String| {
            let path = d.join(&file);
            let src = std::fs::read_to_string(&path).map_err(mlua::Error::external)?;
            lua.load(src).set_name(file).exec()
        })?,
    )?;

    // Anything else: a config error instead of "attempt to call a nil value".
    let o = out.clone();
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, String)| {
            let o = o.clone();
            lua.create_function(move |_, _: mlua::Variadic<Value>| {
                o.borrow_mut()
                    .errors
                    .push(format!("hl.{key} is not supported yet"));
                Ok(())
            })
        })?,
    )?;
    hl.set_metatable(Some(meta));

    g.set("hl", hl)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(src: &str) -> (String, Vec<String>) {
        to_hyprlang(src, Path::new("."), "test.lua").unwrap()
    }

    #[test]
    fn config_tables_flatten() {
        let (text, errors) = conv(
            r#"hl.config({ general = { gaps_in = 5, border_size = 2,
                 col = { active_border = { colors = {"rgba(33ccffee)", "rgba(00ff99ee)"}, angle = 45 },
                         inactive_border = "rgba(595959aa)" } },
                 decoration = { shadow = { color = 0xee1a1a1a } },
                 input = { kb_layout = "tr", touchpad = { natural_scroll = true } } })"#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        for line in [
            "general:gaps_in = 5",
            "general:border_size = 2",
            "general:col.active_border = rgba(33ccffee) rgba(00ff99ee) 45deg",
            "general:col.inactive_border = rgba(595959aa)",
            "decoration:shadow:color = 0xee1a1a1a",
            "input:kb_layout = tr",
            "input:touchpad:natural_scroll = true",
        ] {
            assert!(
                text.lines().any(|l| l == line),
                "missing `{line}` in\n{text}"
            );
        }
    }

    #[test]
    fn binds_and_dispatchers() {
        let (text, errors) = conv(
            r#"local mod = "SUPER"
               hl.bind(mod .. " + Q", hl.dsp.exec_cmd("kitty"))
               hl.bind(mod .. " + SHIFT + 1", hl.dsp.window.move({ workspace = 1 }))
               hl.bind(mod .. " + left", hl.dsp.focus({ direction = "left" }))
               hl.bind(mod .. " + V", hl.dsp.window.float({ action = "toggle" }))
               hl.bind(mod .. " + mouse:272", hl.dsp.window.drag(), { mouse = true })
               hl.bind("XF86AudioMute", hl.dsp.exec_cmd("wpctl"), { locked = true, repeating = true })
               hl.bind(mod .. " + J", hl.dsp.layout("togglesplit"), { description = "split" })
               hl.bind(mod .. " + S", hl.dsp.workspace.toggle_special("magic"))
               hl.bind(mod .. " + F", function() end)"#,
        );
        for line in [
            "bind = SUPER, Q, exec, kitty",
            "bind = SUPER SHIFT, 1, movetoworkspace, 1",
            "bind = SUPER, left, movefocus, l",
            "bind = SUPER, V, togglefloating",
            "bindm = SUPER, mouse:272, movewindow",
            "bindle = , XF86AudioMute, exec, wpctl",
            "bindd = SUPER, J, split, layoutmsg, togglesplit",
            "bind = SUPER, S, togglespecialworkspace, magic",
        ] {
            assert!(
                text.lines().any(|l| l == line),
                "missing `{line}` in\n{text}"
            );
        }
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("Lua function binds"));
    }

    #[test]
    fn start_exec_rules_submaps() {
        let (text, errors) = conv(
            r#"hl.exec_cmd("swaybg")
               hl.on("hyprland.start", function() hl.exec_cmd("waybar") end)
               hl.window_rule({ name = "k", match = { class = "kitty" }, float = true })
               hl.layer_rule({ match = { namespace = "waybar" }, blur = true })
               hl.workspace_rule({ workspace = "2", layout = "master" })
               hl.define_submap("resize", function()
                   hl.bind("right", hl.dsp.window.resize({ x = 10, y = 0, relative = true }), { repeating = true })
               end)
               hl.timer(1000, function() end)"#,
        );
        for line in [
            "exec = swaybg",
            "exec-once = waybar",
            "  match:class = kitty",
            "  float = true",
            "layerrule = match:namespace waybar, blur true",
            "workspace = 2, layout:master",
            "submap = resize",
            "binde = , right, resizeactive, 10 0",
            "submap = reset",
        ] {
            assert!(
                text.lines().any(|l| l == line),
                "missing `{line}` in\n{text}"
            );
        }
        assert_eq!(errors, vec!["hl.timer is not supported yet".to_string()]);
    }

    #[test]
    fn dispatch_expression() {
        assert_eq!(
            eval_dispatch("hl.dsp.exit()").unwrap(),
            ("exit".into(), "".into())
        );
        assert_eq!(
            eval_dispatch("hl.dsp.focus({ workspace = 3 })").unwrap(),
            ("workspace".into(), "3".into())
        );
        assert!(eval_dispatch("hl.dsp.window.fullscreen_state({})").is_err());
    }
}
