//! Extra `hyprctl` requests that are not plain snapshot queries: `reload`,
//! `keyword`, `getoption`, `configerrors`, `seterror`, `devices`, `cursorpos`,
//! `notify`, `splash`, `systeminfo`, `switchxkblayout`, ...
//!
//! Shapes follow Hyprland v0.57.1 (`src/ipc/s1/Commands.cpp`).

use serde::Serialize;
use smithay::input::keyboard::Layout;

use crate::{config::parse_color, ipc::Request, state::Hypr};

/// Fake but stable device addresses for `devices`.
const KEYBOARD_ADDR: u64 = 0x5580_1000;
const MOUSE_ADDR: u64 = 0x5580_2000;
const KEYBOARD_NAME: &str = "seat0-keyboard";
const MOUSE_NAME: &str = "seat0-mouse";

pub fn handle(h: &mut Hypr, req: &Request) -> Option<String> {
    let cmd = req.cmd.trim();
    let (name, args) = cmd
        .split_once(char::is_whitespace)
        .map_or((cmd, ""), |(n, a)| (n, a.trim()));
    let json = req.json;
    Some(match name {
        "reload" => {
            h.reload_config();
            "ok".into()
        }
        "keyword" => h.keyword(args),
        "getoption" => h.getoption(args, json),
        "configerrors" => config_errors(&h.reload.errors, json),
        "seterror" => seterror(h, args),
        "devices" => devices(h, json),
        "cursorpos" => {
            let p = h.seat.get_pointer()?.current_location();
            let (x, y) = (p.x.floor() as i32, p.y.floor() as i32);
            if json {
                format!("{{\n    \"x\": {x},\n    \"y\": {y}\n}}\n")
            } else {
                format!("{x}, {y}")
            }
        }
        "notify" => notify(args),
        "dismissnotify" => match args.split_whitespace().next() {
            Some(a) if a.parse::<i32>().is_err() => "invalid arg 1".into(),
            _ => "ok".into(),
        },
        "splash" => "Rust never sleeps.".into(),
        "kill" => "ok".into(),
        "switchxkblayout" => switch_xkb_layout(h, args),
        "systeminfo" => system_info(h),
        // v0.57.1 `status`: which config format is loaded.
        "status" => {
            let p = config_provider(h);
            if json {
                serde_json::json!({ "configProvider": p, "configErrors": h.reload.errors.len() })
                    .to_string()
            } else {
                format!(
                    "configProvider: {p}
configErrors: {}",
                    h.reload.errors.len()
                )
            }
        }
        "animations" if json => "[[],\n[]]".into(),
        "animations" => "animations:\n\nbeziers:\n".into(),
        "layouts" if json => r#"["dwindle","master"]"#.into(),
        "layouts" => "dwindle\nmaster\n".into(),
        "decorations" => "none".into(),
        "rollinglog" if json => serde_json::json!({ "log": crate::rolling_log() }).to_string(),
        "rollinglog" => crate::rolling_log(),
        "globalshortcuts" if json => "[]\n".into(),
        "globalshortcuts" => "none".into(),
        "workspacerules" if json => "[]".into(),
        "workspacerules" => String::new(),
        "submap" => {
            let s = if h.wm.submap.is_empty() {
                "default"
            } else {
                h.wm.submap.as_str()
            };
            if json {
                format!("{}\n", serde_json::to_string(s).unwrap_or_default())
            } else {
                format!("{s}\n")
            }
        }
        "locked" if json => format!("{{\n    \"locked\": {}\n}}\n", h.session_locked),
        "locked" => h.session_locked.to_string(),
        // shortcut: no option descriptions are kept, upgrade when config.rs has a table.
        "descriptions" => "[]\n".into(),
        "reloadshaders" if json => r#"{"ok": true}"#.into(),
        "reloadshaders" => "ok".into(),
        "getprop" => getprop(h, args, json),
        // shortcut: only validated, the cursor is not themed yet (see tty.rs gaps).
        "setcursor" => match args.split_whitespace().last().map(str::parse::<i32>) {
            Some(Ok(n)) if n > 0 => "ok".into(),
            Some(Ok(_)) => "size not positive".into(),
            _ => "size not int".into(),
        },
        "plugin" if args.starts_with("list") => "no plugins loaded".into(),
        "plugin" => "plugins are not supported".into(),
        // `output create headless [NAME]` / `output remove NAME`: virtual
        // outputs, on any backend (like Hyprland).
        "output" => {
            let w: Vec<&str> = args.split_whitespace().collect();
            match w.as_slice() {
                ["create", "headless" | "auto", rest @ ..] => {
                    match h.create_virtual_output(rest.first().copied()) {
                        Ok(_) => "ok".into(),
                        Err(e) => e,
                    }
                }
                ["create", other, ..] => format!("unsupported output type `{other}`"),
                ["remove", name] => match h.remove_virtual_output(name) {
                    Ok(()) => "ok".into(),
                    Err(e) => e,
                },
                _ => "usage: output create headless [NAME] | output remove NAME".into(),
            }
        }
        "eval" => eval(h, args),
        _ => return None,
    })
}

fn config_errors(errors: &[String], json: bool) -> String {
    if json {
        let items: Vec<String> = errors
            .iter()
            .map(|e| format!("\n\t{}", serde_json::to_string(e).unwrap_or_default()))
            .collect();
        format!("[{}\n]\n", items.join(","))
    } else {
        errors.iter().map(|e| format!("{e}\n")).collect()
    }
}

/// `seterror <color> <message...>` / `seterror disable`. The error bar is not
/// drawn yet; the message is stored and logged.
fn seterror(h: &mut Hypr, args: &str) -> String {
    let words: Vec<&str> = args.split_whitespace().collect();
    if words.len() < 2 {
        h.reload.seterror = None;
        return match words.first() {
            Some(w) if !w.contains("dis") => "var 1 not color or disable".into(),
            _ => "ok".into(),
        };
    }
    let msg = words[1..].join(" ");
    tracing::warn!("seterror {}: {msg}", words[0]);
    h.reload.seterror = Some((words[0].to_string(), msg));
    "ok".into()
}

/// `notify <icon> <time_ms> <color> [fontsize:N] <message...>`: validated,
/// logged, not drawn.
fn notify(args: &str) -> String {
    let w: Vec<&str> = args.split_whitespace().collect();
    if w.len() < 4 {
        return "not enough args".into();
    }
    if w[0].parse::<i32>().is_err() {
        return "invalid arg 1".into();
    }
    if w[1].parse::<i32>().is_err() {
        return "invalid arg 2".into();
    }
    if w[2] != "0" && parse_color(w[2]).is_err() {
        return "invalid arg 3".into();
    }
    let mut i = 3;
    if let Some(sz) = w[i].strip_prefix("fontsize:") {
        if sz.parse::<f32>().is_err() {
            return "invalid fontsize kwarg".into();
        }
        i += 1;
    }
    if i >= w.len() {
        return "not enough args".into();
    }
    tracing::info!("notify: {}", w[i..].join(" "));
    "ok".into()
}

#[derive(Serialize)]
struct Mouse {
    address: String,
    name: String,
    #[serde(rename = "defaultSpeed")]
    default_speed: f64,
    #[serde(rename = "scrollFactor")]
    scroll_factor: f64,
}

#[derive(Serialize)]
struct Keyboard {
    address: String,
    name: String,
    rules: String,
    model: String,
    layout: String,
    variant: String,
    options: String,
    active_layout_index: i32,
    active_keymap: String,
    #[serde(rename = "capsLock")]
    caps_lock: bool,
    #[serde(rename = "numLock")]
    num_lock: bool,
    main: bool,
}

#[derive(Serialize)]
struct Devices {
    mice: Vec<Mouse>,
    keyboards: Vec<Keyboard>,
    tablets: Vec<()>,
    touch: Vec<()>,
    switches: Vec<()>,
}

/// Active layout index, its name and the number of layouts of the keymap.
fn keymap_info(h: &mut Hypr) -> Option<(usize, String, usize)> {
    let kb = h.seat.get_keyboard()?;
    Some(kb.with_xkb_state(h, |ctx| {
        let xkb = ctx.xkb().lock().unwrap();
        let active = xkb.active_layout();
        (
            active.0 as usize,
            xkb.layout_name(active).to_string(),
            xkb.layouts().count(),
        )
    }))
}

fn devices(h: &mut Hypr, json: bool) -> String {
    // shortcut: one seat keyboard and one pointer, xkb rules/model/variant/
    // options are empty and the layout comes from the environment; read them
    // from the input config once `kb_layout` & co exist.
    let (idx, keymap, _) = keymap_info(h).unwrap_or((0, String::new(), 1));
    let layout = std::env::var("XKB_DEFAULT_LAYOUT").unwrap_or_else(|_| "us".into());
    let mods = h
        .seat
        .get_keyboard()
        .map(|k| k.modifier_state())
        .unwrap_or_default();
    let d = Devices {
        mice: vec![Mouse {
            address: format!("0x{MOUSE_ADDR:x}"),
            name: MOUSE_NAME.into(),
            default_speed: 0.0,
            scroll_factor: -1.0,
        }],
        keyboards: vec![Keyboard {
            address: format!("0x{KEYBOARD_ADDR:x}"),
            name: KEYBOARD_NAME.into(),
            rules: String::new(),
            model: String::new(),
            layout: layout.clone(),
            variant: String::new(),
            options: String::new(),
            active_layout_index: idx as i32,
            active_keymap: keymap.clone(),
            caps_lock: mods.caps_lock,
            num_lock: mods.num_lock,
            main: true,
        }],
        tablets: vec![],
        touch: vec![],
        switches: vec![],
    };
    if json {
        return crate::ipc::json(&d);
    }
    format!(
        "mice:\n\tMouse at {MOUSE_ADDR:x}:\n\t\t{MOUSE_NAME}\n\t\t\tdefault speed: 0.00000\n\t\t\tscroll factor: -1.00\n\n\nKeyboards:\n\tKeyboard at {KEYBOARD_ADDR:x}:\n\t\t{KEYBOARD_NAME}\n\t\t\trules: r \"\", m \"\", l \"{layout}\", v \"\", o \"\"\n\t\t\tactive layout index: {idx}\n\t\t\tactive keymap: {keymap}\n\t\t\tcapsLock: no\n\t\t\tnumLock: no\n\t\t\tmain: yes\n\n\nTablets:\n\n\nTouch:\n\n\nSwitches:\n"
    )
}

/// `switchxkblayout <kb|main|all> <next|prev|index>`.
fn switch_xkb_layout(h: &mut Hypr, args: &str) -> String {
    let mut w = args.split_whitespace();
    let (kb, cmd) = (w.next().unwrap_or(""), w.next().unwrap_or(""));
    if !matches!(kb, "main" | "active" | "current" | "all" | KEYBOARD_NAME) {
        return "device not found".into();
    }
    let Some((cur, _, n)) = keymap_info(h) else {
        return "no device".into();
    };
    let target = match cmd {
        "next" => (cur + 1) % n,
        "prev" => (cur + n - 1) % n,
        other => match other.parse::<usize>() {
            Ok(i) if i < n => i,
            Ok(_) => return format!("layout idx out of range of {n}"),
            Err(_) => return "invalid arg 2".into(),
        },
    };
    if target != cur {
        if let Some(kb) = h.seat.get_keyboard() {
            kb.with_xkb_state(h, |mut ctx| ctx.set_layout(Layout(target as _)));
        }
    }
    // Quickshell and waybar read the new layout name from this event.
    if let Some((_, name, _)) = keymap_info(h) {
        h.ipc
            .emit("activelayout", &format!("{KEYBOARD_NAME},{name}"));
    }
    "ok".into()
}

fn system_info(h: &Hypr) -> String {
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    format!(
        "Hyprland {} (hyprland-rust)

System Information:
Distro: {os}
Kernel: {kernel}
backend: {}
configProvider: {}
",
        env!("CARGO_PKG_VERSION"),
        h.ipc.backend,
        config_provider(h)
    )
}

/// `eval <lua>`: `hl.dsp.*` expressions run as dispatchers, anything else
/// runs like a config script and its settings are applied as keywords.
pub(crate) fn eval(h: &mut Hypr, code: &str) -> String {
    use crate::{config::Dispatcher, lua_config};
    if code.is_empty() {
        return "not enough args".into();
    }
    // A hyprland.lua config: run in its live state, as Hyprland does.
    if !code.starts_with("hl.dsp.") {
        if let Some(r) = h.lua_eval(code) {
            return match r {
                Ok(v) if v.is_empty() => "ok".into(),
                Ok(v) => v,
                Err(e) => e,
            };
        }
    }
    if code.starts_with("hl.dsp.") {
        let (name, arg) = match lua_config::eval_dispatch(code) {
            Ok(p) => p,
            Err(e) => return e,
        };
        return match Dispatcher::parse(&name, &arg) {
            Ok(d) => {
                h.dispatch(&d);
                "ok".into()
            }
            Err(e) => e,
        };
    }
    let (lines, mut errors) = match lua_config::to_hyprlang(code, std::path::Path::new("."), "eval")
    {
        Ok(r) => r,
        Err(e) => return e,
    };
    for l in lines.lines().filter(|l| !l.trim().is_empty()) {
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let r = h.keyword(&format!("{} {}", k.trim(), v.trim()));
        if r != "ok" {
            errors.push(r);
        }
    }
    if errors.is_empty() {
        "ok".into()
    } else {
        errors.join("\n")
    }
}

/// Value of a window property as `(text, json)`; `stored` is what `setprop`
/// saved. Defaults follow Hyprland's rule applicator for an untouched window.
fn prop_value(
    prop: &str,
    stored: Option<&str>,
    cfg: &crate::config::Config,
) -> Option<(String, String)> {
    use crate::options::{get_option, gradient, OptValue};
    let quote = |t: &str| serde_json::to_string(t).unwrap_or_default();
    let same = |t: String| (t.clone(), t);
    let opt_int = |k: &str| match get_option(cfg, k) {
        Some(OptValue::Int(i)) => i.to_string(),
        _ => "0".into(),
    };
    Some(match prop {
        "animation" => (
            stored.unwrap_or("(unset)").into(),
            quote(stored.unwrap_or("")),
        ),
        "min_size" | "max_size" => {
            let d = if prop == "max_size" { "inf" } else { "20" };
            let v: Vec<&str> =
                stored.map_or_else(|| vec![d, d], |s| s.split_whitespace().collect());
            let j: Vec<&str> = v
                .iter()
                .map(|x| {
                    if x.parse::<f64>().is_ok_and(f64::is_finite) {
                        *x
                    } else {
                        "null"
                    }
                })
                .collect();
            (v.join(" "), format!("[{}]", j.join(",")))
        }
        "opacity" | "opacity_inactive" | "opacity_fullscreen" | "scroll_mouse"
        | "scroll_touchpad" => same(stored.unwrap_or("1").into()),
        "rounding_power" => same(stored.unwrap_or("2").into()),
        "idle_inhibit" => same(stored.unwrap_or("0").into()),
        "rounding" => same(stored.map_or_else(|| opt_int("decoration:rounding"), Into::into)),
        "border_size" => same(stored.map_or_else(|| opt_int("general:border_size"), Into::into)),
        "active_border_color" | "inactive_border_color" => {
            let t = match stored.map(crate::config::parse_color) {
                Some(Ok(c)) => gradient(c),
                _ => match get_option(cfg, &format!("general:col.{}", &prop[..prop.len() - 6])) {
                    Some(OptValue::Custom(s)) => s,
                    _ => return None,
                },
            };
            let j = quote(&t);
            (t, j)
        }
        "opacity_override"
        | "opacity_inactive_override"
        | "opacity_fullscreen_override"
        | "allows_input"
        | "decorate"
        | "focus_on_activate"
        | "keep_aspect_ratio"
        | "nearest_neighbor"
        | "no_anim"
        | "no_blur"
        | "no_dim"
        | "no_focus"
        | "no_max_size"
        | "no_shadow"
        | "no_shortcuts_inhibit"
        | "opaque"
        | "dim_around"
        | "force_rgbx"
        | "sync_fullscreen"
        | "immediate"
        | "xray"
        | "render_unfocused"
        | "no_follow_mouse"
        | "no_screen_share"
        | "no_vrr"
        | "persistent_size"
        | "stay_focused" => {
            let v = match stored {
                Some(s) => matches!(s, "true" | "1" | "on" | "yes"),
                None => prop == "decorate",
            };
            same(v.to_string())
        }
        _ => return None,
    })
}

/// `getprop <window> <prop>`.
fn getprop(h: &Hypr, args: &str, json: bool) -> String {
    let mut w = args.split_whitespace();
    let (Some(sel), Some(prop)) = (w.next(), w.next()) else {
        return "not enough args".into();
    };
    let win = if matches!(sel, "active" | "activewindow") {
        h.focused_window()
    } else {
        sel.parse::<crate::selectors::WinSel>()
            .ok()
            .and_then(|s| h.find_window(&s))
            .map(|(_, w)| w)
    };
    let Some(win) = win else {
        return "window not found".into();
    };
    let stored =
        h.wm.props
            .iter()
            .rev()
            .find(|(x, p, _)| x == &win && p == prop)
            .map(|(_, _, v)| v.as_str());
    match prop_value(prop, stored, &h.config) {
        Some((t, _)) if !json => t,
        Some((_, j)) => format!("{{\"{prop}\": {j}}}"),
        None => "prop not found".into(),
    }
}

/// `lua` when the loaded config is a hyprland.lua, else `hyprlang`.
fn config_provider(h: &Hypr) -> &'static str {
    if h.config.lua.0.is_some() {
        "lua"
    } else {
        "hyprlang"
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getprop_values() {
        let cfg = crate::config::Config::default();
        let v = |p: &str, s: Option<&str>| prop_value(p, s, &cfg);
        assert_eq!(
            v("animation", None),
            Some(("(unset)".into(), "\"\"".into()))
        );
        assert_eq!(
            v("max_size", None),
            Some(("inf inf".into(), "[null,null]".into()))
        );
        assert_eq!(
            v("min_size", Some("100 50")),
            Some(("100 50".into(), "[100,50]".into()))
        );
        assert_eq!(v("opacity", None).unwrap().0, "1");
        assert_eq!(v("allows_input", Some("true")).unwrap().1, "true");
        assert_eq!(v("allows_input", None).unwrap().0, "false");
        assert_eq!(
            v("active_border_color", Some("rgb(abcdef)")).unwrap().0,
            "ffabcdef 0deg"
        );
        assert!(v("inactive_border_color", None).unwrap().0.ends_with("deg"));
        assert!(v("nonexistantprop", None).is_none());
    }

    #[test]
    fn config_errors_shapes() {
        let e = vec!["a:1: bad".to_string(), "b:2: \"q\"".to_string()];
        assert_eq!(config_errors(&e, false), "a:1: bad\nb:2: \"q\"\n");
        let j = config_errors(&e, true);
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v[1], "b:2: \"q\"");
        assert_eq!(config_errors(&[], true), "[\n]\n");
    }

    #[test]
    fn notify_validation() {
        assert_eq!(notify("1 1000 rgb(ff0000) hello world"), "ok");
        assert_eq!(notify("1 1000 0 fontsize:20 hi"), "ok");
        assert_eq!(notify("x 1000 0 hi"), "invalid arg 1");
        assert_eq!(notify("1 1000 blue hi"), "invalid arg 3");
        assert_eq!(notify("1 1000 0"), "not enough args");
    }
}
