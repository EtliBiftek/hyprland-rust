//! Randomized robustness tests (no external fuzzer): config parsers must never
//! panic, and the tiling layouts must keep every window with a positive size.

use crate::{config::*, layout::*, master::*, rules::*};
use std::panic::{catch_unwind, AssertUnwindSafe};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<'a>(&mut self, xs: &'a [&'a str]) -> &'a str {
        xs[(self.next() % xs.len() as u64) as usize]
    }
    fn n(&mut self, m: u64) -> u64 {
        self.next() % m
    }
}

const ATOMS: &[&str] = &[
    "",
    " ",
    ",",
    ", ",
    ":",
    "=",
    "-",
    "+",
    "+1",
    "-1",
    "0",
    "1",
    "-5",
    "999999999999",
    "2147483647",
    "-2147483648",
    "e+1",
    "m-1",
    "r+0",
    "name:",
    "name:x",
    "special",
    "special:",
    "special:x",
    "previous",
    "empty",
    "exact",
    "%",
    "50%",
    "-50%",
    "100%-w",
    "0.5",
    "-0.0",
    "nan",
    "inf",
    "x",
    "ğ",
    "日本",
    "\u{0}",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    "class:",
    "title:^(a",
    "address:0x",
    "pid:",
    "l",
    "r",
    "u",
    "d",
    "f",
    "b",
    "on",
    "off",
    "toggle",
    "SUPER",
    "SHIFT",
    "mouse:272",
    "code:",
    "code:99999999999",
    "$v",
    "$",
    "#",
    "rgba(",
    "rgba(zz)",
    "0x",
    "deg",
    "45deg",
    "\\",
    "\"",
    "'",
    "windowrule",
    "match:class",
    "float",
    "size",
    "move",
    "monitor_w",
    "*",
    "/",
    "1/0",
    "(((",
    "))",
    "w",
    "h",
    "window_w",
    "cursor_x",
    "  ",
    "\t",
    "\n",
];

const DISPATCHERS: &[&str] = &[
    "exec",
    "killactive",
    "forcekillactive",
    "exit",
    "workspace",
    "movetoworkspace",
    "movetoworkspacesilent",
    "togglefloating",
    "setfloating",
    "settiled",
    "fullscreen",
    "movefocus",
    "cyclenext",
    "togglesplit",
    "movewindow",
    "swapwindow",
    "resizeactive",
    "moveactive",
    "centerwindow",
    "pin",
    "focuscurrentorlast",
    "focusurgentorlast",
    "splitratio",
    "layoutmsg",
    "togglegroup",
    "changegroupactive",
    "moveintogroup",
    "moveoutofgroup",
    "movewindoworgroup",
    "movegroupwindow",
    "lockgroups",
    "lockactivegroup",
    "denywindowfromgroup",
    "setignoregrouplock",
    "swapnext",
    "closewindow",
    "focuswindow",
    "pass",
    "togglespecialworkspace",
    "submap",
    "tagwindow",
    "renameworkspace",
    "bringactivetotop",
    "alterzorder",
    "movecursortocorner",
    "movecursor",
    "togglepseudo",
    "setprop",
    "killwindow",
    "signal",
    "event",
    "dpms",
    "focusmonitor",
    "movecurrentworkspacetomonitor",
    "moveworkspacetomonitor",
    "swapactiveworkspaces",
    "toggleswallow",
    "forcerendererreload",
    "resizewindow",
];

const KEYS: &[&str] = &[
    "bind",
    "binde",
    "bindm",
    "bindrl",
    "bindd",
    "unbind",
    "submap",
    "source",
    "exec",
    "exec-once",
    "workspace",
    "windowrule",
    "windowrulev2",
    "layerrule",
    "general:gaps_in",
    "general:col.active_border",
    "general:layout",
    "input:kb_layout",
    "input:sensitivity",
    "input:touchpad:scroll_factor",
    "device:name",
    "device:sensitivity",
    "master:mfact",
    "master:orientation",
    "dwindle:default_split_ratio",
    "group:groupbar:height",
    "$v",
    "misc:disable_autoreload",
    "monitor",
    "env",
    "animation",
    "bezier",
];

fn word(r: &mut Rng) -> String {
    (0..r.n(5) + 1).map(|_| r.pick(ATOMS)).collect()
}

fn args(r: &mut Rng) -> String {
    (0..r.n(5))
        .map(|_| word(r))
        .collect::<Vec<_>>()
        .join(if r.n(2) == 0 { " " } else { "," })
}

fn run<F: FnOnce()>(what: &str, input: &str, fails: &mut Vec<String>, f: F) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        fails.push(format!("{what}: {input:?}"));
    }
}

#[test]
fn fuzz_parsers() {
    let mut r = Rng(0x9e3779b97f4a7c15);
    let mut fails = Vec::new();
    let vars = Vars {
        monitor_w: 1920.0,
        monitor_h: 1080.0,
        window_x: 0.0,
        window_y: 0.0,
        window_w: 100.0,
        window_h: 100.0,
        cursor_x: 0.0,
        cursor_y: 0.0,
    };
    for _ in 0..40_000 {
        let d = r.pick(DISPATCHERS).to_string();
        let a = args(&mut r);
        run("Dispatcher::parse", &format!("{d} {a}"), &mut fails, || {
            let _ = Dispatcher::parse(&d, &a);
        });
        let line = format!("{} = {}", r.pick(KEYS), args(&mut r));
        run("parse_str", &line, &mut fails, || {
            let _ = Config::parse_str(&line);
        });
        let block = format!("{} {{\n{}\n}}\n{}", r.pick(KEYS), line, word(&mut r));
        run("parse_str(block)", &block, &mut fails, || {
            let _ = Config::parse_str(&block);
        });
        let e = args(&mut r);
        run("eval_expr", &e, &mut fails, || {
            let _ = eval_expr(&e, &vars, 1000.0);
        });
        run("parse_inline_window", &e, &mut fails, || {
            let _ = parse_inline_window(&e);
        });
        run("parse_inline_layer", &e, &mut fails, || {
            let _ = parse_inline_layer(&e);
        });
        run("parse_color", &e, &mut fails, || {
            let _ = parse_color(&e);
        });
        run("Mods::parse", &e, &mut fails, || {
            let _ = Mods::parse(&e);
        });
    }
    fails.sort();
    assert!(
        fails.is_empty(),
        "{} panics, e.g.:\n{}",
        fails.len(),
        fails
            .iter()
            .take(25)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn fuzz_layouts() {
    let mut fails = Vec::new();
    for seed in 1..400u64 {
        let mut r = Rng(seed.wrapping_mul(0x2545f4914f6cdd1d) | 1);
        let area = Rect::new(0, 0, r.n(3000) as i32, r.n(2000) as i32);
        let mut log = Vec::new();
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut d: crate::layout::Dwindle<u32> = crate::layout::Dwindle::default();
            let mut m: Master<u32> = Master::default();
            let mut live: Vec<u32> = Vec::new();
            for step in 0..200u32 {
                let op = r.n(14);
                let w = live.get(r.n(live.len() as u64 + 1) as usize).copied();
                log.push(format!("{op} {w:?}"));
                match op {
                    0..=3 => {
                        d.insert(step, w.as_ref(), area);
                        m.insert(step, w.as_ref());
                        live.push(step);
                    }
                    4 | 5 => {
                        if let Some(w) = w {
                            d.remove(&w);
                            m.remove(&w);
                            live.retain(|x| *x != w);
                        }
                    }
                    6 => {
                        if let Some(w) = w {
                            d.toggle_split(&w, area);
                            d.swap_split(&w, area);
                            d.rotate_split(&w, (r.n(5) as i32 - 2) * 90, area);
                        }
                    }
                    7 => {
                        if let Some(w) = w {
                            d.move_to_root(&w, r.n(2) == 0);
                        }
                    }
                    8 => {
                        if let (Some(a), Some(b)) = (w, live.first().copied()) {
                            d.swap(&a, &b);
                            m.swap(&a, &b);
                        }
                    }
                    9 => {
                        if let Some(w) = w {
                            let dx = r.n(4000) as i32 - 2000;
                            let dy = r.n(4000) as i32 - 2000;
                            d.resize(&w, dx, dy, area);
                            m.resize(&w, dx, dy, area);
                        }
                    }
                    10 => {
                        if let Some(w) = w {
                            let cmds = [
                                "swapwithmaster",
                                "focusmaster",
                                "cyclenext",
                                "cycleprev",
                                "swapnext",
                                "swapprev",
                                "addmaster",
                                "removemaster",
                                "orientationleft",
                                "orientationright",
                                "orientationtop",
                                "orientationbottom",
                                "orientationcenter",
                                "orientationnext",
                                "orientationprev",
                                "mfact",
                                "rollnext",
                                "rollprev",
                                "orientationcycle",
                                "bogus",
                            ];
                            let cmd = cmds[r.n(cmds.len() as u64) as usize];
                            let arg = ["", "0.5", "-2", "exact 0.9", "x", "master", "child"]
                                [r.n(7) as usize];
                            let a: Vec<&str> = arg.split_whitespace().collect();
                            let _ = m.msg(cmd, &a, &w);
                        }
                    }
                    11 => {
                        if let Some(w) = w {
                            d.update_ratio(&w, |x| x * 3.0 - 1.0);
                        }
                    }
                    _ => {
                        let gaps = r.n(60) as i32 - 10;
                        for (name, rects) in [
                            ("dwindle", d.layout(area, gaps)),
                            ("master", m.layout(area, gaps)),
                        ] {
                            for (w, rc) in rects {
                                assert!(
                                    rc.w > 0 && rc.h > 0,
                                    "{name}: window {w} got {rc:?} in {area:?} gaps {gaps}"
                                );
                            }
                        }
                        if let Some(w) = w {
                            let _ = m.next_target(&w, r.n(2) == 0, r.n(2) == 0);
                        }
                    }
                }
                // Invariants: both layouts hold exactly the live windows.
                let mut dw = d.windows();
                dw.sort();
                let mut mw = m.windows();
                mw.sort();
                let mut l = live.clone();
                l.sort();
                assert_eq!(dw, l, "dwindle windows diverged");
                assert_eq!(mw, l, "master windows diverged");
            }
        }));
        if res.is_err() {
            fails.push(format!(
                "seed {seed} area {area:?}: last ops {:?}",
                &log[log.len().saturating_sub(4)..]
            ));
        }
    }
    assert!(
        fails.is_empty(),
        "{} failing seeds:\n{}",
        fails.len(),
        fails
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
