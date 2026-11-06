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
        "animations" if json => "[[],\n[]]".into(),
        "animations" => "animations:\n\nbeziers:\n".into(),
        "layouts" if json => r#"["dwindle"]"#.into(),
        "layouts" => "dwindle\n".into(),
        "decorations" => "none".into(),
        "rollinglog" if json => "[\n\"log\":\"\"]".into(),
        "rollinglog" => String::new(),
        "globalshortcuts" if json => "[]\n".into(),
        "globalshortcuts" => "none".into(),
        "workspacerules" if json => "[]".into(),
        "workspacerules" => String::new(),
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
            caps_lock: false,
            num_lock: false,
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
        "Hyprland {} (hyprland-rust)\n\nSystem Information:\nDistro: {os}\nKernel: {kernel}\nbackend: {}\nconfigProvider: hyprlang\n",
        env!("CARGO_PKG_VERSION"),
        h.ipc.backend
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
