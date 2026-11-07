//! `hyprland.lua` support (the config format of Hyprland >= 0.55; v0.57 reads
//! nothing else).
//!
//! The script runs once per (re)load against an `hl` API that mirrors
//! Hyprland v0.57.1's (`src/config/lua/bindings/`). Every `hl.*` call made
//! while loading is translated into the equivalent hyprlang line and the
//! resulting text is fed to the normal hyprlang parser, so options, binds,
//! rules, reload and `hyprctl keyword` share one code path. `hl.dsp.*` return
//! the hyprlang dispatcher text (`"name, args"`), which is also how `hyprctl
//! dispatch 'hl.dsp.exit()'` is evaluated.
//!
//! The Lua state then stays alive in a [`Runtime`] (held by `Config::lua`):
//! function binds become the dispatcher `__lua, N`, and `hl.on` handlers,
//! `hl.timer` callbacks and gesture callbacks run later against a [`Host`]
//! (the compositor, `lua_host.rs`). Inside a callback `hl.dispatch` runs a
//! dispatcher at once, `hl.config`/`hl.bind`/... apply like `hyprctl
//! keyword`, and `hl.get_*` return tables built from the hyprctl JSON with
//! the field names of Hyprland's Lua objects.

use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use serde_json::Value as J;

/// Limits for the config script: generous, but a stray `while true do end`
/// or a runaway table must not hang or kill the compositor on (re)load.
const CONFIG_MAX_INSTRUCTIONS: u64 = 500_000_000;
const CONFIG_MAX_MEMORY: usize = 256 << 20;
/// One bind, event or timer callback (Hyprland also times callbacks out).
const CALLBACK_MAX_INSTRUCTIONS: u64 = 50_000_000;
/// `hyprctl dispatch` input comes from any local client: tight limits.
const DISPATCH_MAX_INSTRUCTIONS: u64 = 5_000_000;
const DISPATCH_MAX_MEMORY: usize = 16 << 20;

/// Instructions used and allowed since the last entry into Lua.
#[derive(Clone, Default)]
struct Budget(Rc<Cell<(u64, u64)>>);

impl Budget {
    fn reset(&self, max: u64) {
        self.0.set((0, max));
    }
}

/// Aborts the script once `budget` runs out and caps its memory.
fn guard(lua: &Lua, budget: &Budget, max_mem: usize) -> mlua::Result<()> {
    lua.set_memory_limit(max_mem)?;
    const STEP: u32 = 10_000;
    let b = budget.clone();
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(STEP),
        move |_, _| {
            let (used, max) = b.0.get();
            let used = used + STEP as u64;
            b.0.set((used, max));
            if used > max {
                Err(mlua::Error::RuntimeError(
                    "Lua script took too long (instruction limit reached)".into(),
                ))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
    Ok(())
}

/// What Lua callbacks can do to the running compositor.
pub trait Host {
    /// Runs a hyprlang dispatcher, e.g. (`"workspace"`, `"3"`).
    fn dispatch(&mut self, name: &str, args: &str) -> Result<(), String>;
    /// Applies one hyprlang line (`key = value`) like `hyprctl keyword`.
    fn keyword(&mut self, line: &str) -> Result<(), String>;
    /// A hyprctl request such as `j/clients`; returns the reply text.
    fn request(&mut self, req: &str) -> String;
    /// The active submap (empty: the global one).
    fn submap(&self) -> String;
}

/// Argument of a Lua event, turned into a Lua object when it fires.
#[derive(Debug, Clone, PartialEq)]
pub enum EvArg {
    /// Window address (`0x...`).
    Window(String),
    Workspace(i64),
    /// Monitor name.
    Monitor(String),
    Str(String),
    Bool(bool),
    Int(i64),
}

struct TimerSlot {
    f: Function,
    ms: u64,
    repeat: bool,
    live: Rc<Cell<bool>>,
}

#[derive(Default)]
struct State {
    lines: Vec<String>,
    errors: Vec<String>,
    /// Function binds and gesture callbacks: `__lua, N` calls `callbacks[N]`.
    callbacks: Vec<Function>,
    /// `hl.on` handlers; the flag is cleared by `subscription:remove()`.
    events: Vec<(String, Function, Rc<Cell<bool>>)>,
    timers: Vec<TimerSlot>,
    /// Timers created since the compositor last armed them.
    new_timers: Vec<usize>,
    /// `hl.dispatch` calls made while loading: run once the compositor is up.
    deferred: Vec<String>,
}

type Shared = Rc<RefCell<State>>;

/// Named registry slot holding the host function while a callback runs.
const HOST_KEY: &str = "hypr_host";

/// Calls the host while a callback runs; `None` while loading the config.
fn host(lua: &Lua, op: &str, a: &str, b: &str) -> mlua::Result<Option<String>> {
    match lua.named_registry_value::<Value>(HOST_KEY)? {
        Value::Function(f) => f.call::<String>((op, a, b)).map(Some),
        _ => Ok(None),
    }
}

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// A loaded `hyprland.lua` whose callbacks the compositor can run.
pub struct Runtime {
    lua: Lua,
    st: Shared,
    budget: Budget,
    /// Unique per load: timers armed for an older config check it.
    pub generation: u64,
}

/// `Config::lua`: the runtime of the loaded `hyprland.lua`, if any.
#[derive(Clone, Default)]
pub struct Handle(pub Option<Rc<Runtime>>);

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "Handle(lua)"
        } else {
            "Handle(none)"
        })
    }
}

impl PartialEq for Handle {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

/// Runs a Lua config and returns the equivalent hyprlang text plus the
/// errors of individual `hl.*` calls. `Err` means the script itself failed.
pub fn to_hyprlang(src: &str, dir: &Path, name: &str) -> Result<(String, Vec<String>), String> {
    Runtime::load(src, dir, name).map(|(_, text, errors)| (text, errors))
}

impl Runtime {
    /// Runs the config script; returns the runtime, the hyprlang text and
    /// the per-call errors.
    pub fn load(
        src: &str,
        dir: &Path,
        name: &str,
    ) -> Result<(Runtime, String, Vec<String>), String> {
        let lua = Lua::new();
        let budget = Budget::default();
        budget.reset(CONFIG_MAX_INSTRUCTIONS);
        guard(&lua, &budget, CONFIG_MAX_MEMORY).map_err(|e| e.to_string())?;
        let st: Shared = Rc::default();
        install(&lua, &st, Some(dir)).map_err(|e| e.to_string())?;
        lua.load(src)
            .set_name(name)
            .exec()
            .map_err(|e| e.to_string())?;
        let (text, errors) = {
            let mut s = st.borrow_mut();
            (s.lines.join("\n"), std::mem::take(&mut s.errors))
        };
        let rt = Runtime {
            lua,
            st,
            budget,
            generation: GENERATION.fetch_add(1, Ordering::Relaxed),
        };
        Ok((rt, text, errors))
    }

    /// Runs `f` with `host` reachable from `hl.*`. Nested calls (a callback
    /// whose dispatcher runs another callback) share the outer budget.
    fn with_host<R>(
        &self,
        host: &mut dyn Host,
        f: impl FnOnce(&Lua) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        let prev: Value = self.lua.named_registry_value(HOST_KEY)?;
        if matches!(prev, Value::Nil) {
            self.budget.reset(CALLBACK_MAX_INSTRUCTIONS);
        }
        let r = self.lua.scope(|scope| {
            let hf = scope.create_function_mut(|_, (op, a, b): (String, String, String)| {
                Ok(match op.as_str() {
                    "dispatch" => host.dispatch(&a, &b).err().unwrap_or_default(),
                    "keyword" => host.keyword(&a).err().unwrap_or_default(),
                    "request" => host.request(&a),
                    "submap" => host.submap(),
                    _ => String::new(),
                })
            })?;
            self.lua.set_named_registry_value(HOST_KEY, hf)?;
            f(&self.lua)
        });
        self.lua.set_named_registry_value(HOST_KEY, prev)?;
        r
    }

    fn report(&self, what: &str, r: mlua::Result<()>) {
        if let Err(e) = r {
            self.st.borrow_mut().errors.push(format!("{what}: {e}"));
        }
    }

    /// The function bind / gesture callback `__lua, id`.
    pub fn call_bind(&self, id: usize, host: &mut dyn Host) {
        let f = self.st.borrow().callbacks.get(id).cloned();
        let Some(f) = f else {
            self.report(
                "bind",
                Err(mlua::Error::external(format!("no Lua callback {id}"))),
            );
            return;
        };
        let r = self.with_host(host, |_| f.call::<()>(()));
        self.report("bind", r);
    }

    pub fn has_handlers(&self, event: &str) -> bool {
        self.st
            .borrow()
            .events
            .iter()
            .any(|(e, _, live)| e == event && live.get())
    }

    /// Runs every live `hl.on(event, ...)` handler with `args` as objects.
    pub fn fire(&self, event: &str, args: &[EvArg], host: &mut dyn Host) {
        let handlers: Vec<Function> = self
            .st
            .borrow()
            .events
            .iter()
            .filter(|(e, _, live)| e == event && live.get())
            .map(|(_, f, _)| f.clone())
            .collect();
        if handlers.is_empty() {
            return;
        }
        let r = self.with_host(host, |lua| {
            let world = World::fetch(lua)?;
            let mut vals = Vec::new();
            for a in args {
                vals.push(match a {
                    EvArg::Window(addr) => to_lua(lua, &world.window_by_addr(addr))?,
                    EvArg::Workspace(id) => to_lua(lua, &world.ws_obj(*id, 0))?,
                    EvArg::Monitor(name) => to_lua(lua, &world.mon_by_name(name))?,
                    EvArg::Str(s) => Value::String(lua.create_string(s)?),
                    EvArg::Bool(b) => Value::Boolean(*b),
                    EvArg::Int(i) => Value::Integer(*i),
                });
            }
            for f in handlers {
                if let Err(e) = f.call::<()>(mlua::MultiValue::from_iter(vals.iter().cloned())) {
                    self.st
                        .borrow_mut()
                        .errors
                        .push(format!("hl.on(\"{event}\"): {e}"));
                }
            }
            Ok(())
        });
        self.report(event, r);
    }

    /// Runs timer `idx`; returns the delay until it fires again, if any.
    pub fn fire_timer(&self, idx: usize, host: &mut dyn Host) -> Option<u64> {
        let (f, ms, repeat, live) = {
            let s = self.st.borrow();
            let t = s.timers.get(idx)?;
            (t.f.clone(), t.ms, t.repeat, t.live.clone())
        };
        if !live.get() {
            return None;
        }
        let r = self.with_host(host, |_| f.call::<()>(()));
        self.report("hl.timer", r);
        if repeat && live.get() {
            Some(ms)
        } else {
            live.set(false);
            None
        }
    }

    /// `hyprctl eval <code>`: runs in the config's Lua state, like Hyprland.
    /// Returns the value of the chunk as text (empty for nil).
    pub fn eval(&self, code: &str, host: &mut dyn Host) -> Result<String, String> {
        let r = self.with_host(host, |lua| {
            let v: Value = lua.load(code).set_name("eval").eval()?;
            Ok(match v {
                Value::Nil => String::new(),
                Value::Table(_) | Value::Function(_) | Value::UserData(_) => {
                    format!("{v:?}")
                }
                v => v.to_string()?,
            })
        });
        r.map_err(|e| e.to_string())
    }

    /// Timers created since the last call, as (index, delay in ms).
    pub fn take_timers(&self) -> Vec<(usize, u64)> {
        let mut s = self.st.borrow_mut();
        let new = std::mem::take(&mut s.new_timers);
        new.into_iter().map(|i| (i, s.timers[i].ms)).collect()
    }

    /// `hl.dispatch` calls made while loading, as hyprlang dispatcher text.
    pub fn take_deferred(&self) -> Vec<String> {
        std::mem::take(&mut self.st.borrow_mut().deferred)
    }

    /// Errors raised by callbacks since the last call.
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut self.st.borrow_mut().errors)
    }
}

/// The `hl.on` event for an IPC event (`EVENT>>DATA` on `.socket2.sock`),
/// so one emit point feeds both.
// shortcut: `window.fullscreen`, `window.update_state` and the `*_early`
// events have no IPC counterpart that names the window; add direct calls
// from `wm.rs` when a dotfile needs them.
pub fn ipc_to_lua_event(event: &str, data: &str) -> Option<(&'static str, Vec<EvArg>)> {
    let mut f = data.split(',');
    let first = f.next().unwrap_or("").to_string();
    let win = || EvArg::Window(format!("0x{}", first.trim_start_matches("0x")));
    let ws = |s: &str| s.parse().ok().map(EvArg::Workspace);
    Some(match event {
        "openwindow" => ("window.open", vec![win()]),
        "closewindow" => ("window.close", vec![win()]),
        "activewindowv2" if !first.is_empty() => ("window.active", vec![win()]),
        "windowtitlev2" => ("window.title", vec![win()]),
        "urgent" => ("window.urgent", vec![win()]),
        "pin" => ("window.pin", vec![win()]),
        "movewindowv2" => ("window.move_to_workspace", vec![win(), ws(f.next()?)?]),
        "workspacev2" => ("workspace.active", vec![ws(&first)?]),
        "createworkspacev2" => ("workspace.created", vec![ws(&first)?]),
        "destroyworkspacev2" => ("workspace.removed", vec![ws(&first)?]),
        "activespecialv2" => ("workspace.special_active", ws(&first).into_iter().collect()),
        "focusedmonv2" => ("monitor.focused", vec![EvArg::Monitor(first)]),
        "monitoraddedv2" => ("monitor.added", vec![EvArg::Monitor(f.next()?.into())]),
        "monitorremovedv2" => ("monitor.removed", vec![EvArg::Monitor(f.next()?.into())]),
        "submap" => ("keybinds.submap", vec![EvArg::Str(data.into())]),
        "openlayer" => ("layer.opened", vec![EvArg::Str(data.into())]),
        "closelayer" => ("layer.closed", vec![EvArg::Str(data.into())]),
        "configreloaded" => ("config.reloaded", vec![]),
        _ => return None,
    })
}

/// Evaluates a Lua dispatcher expression such as `hl.dsp.exit()` (what
/// `hyprctl dispatch` takes in Hyprland >= 0.57) into `(name, args)`.
///
/// Any local client can send this, so it runs sandboxed: no `os`, `io`,
/// `package` or file loading, and tight instruction/memory limits (otherwise
/// `hl.dsp.exit() or os.exit()` would kill the compositor and a loop would
/// hang it).
pub fn eval_dispatch(expr: &str) -> Result<(String, String), String> {
    let libs = StdLib::TABLE | StdLib::STRING | StdLib::UTF8 | StdLib::MATH;
    let lua = Lua::new_with(libs, LuaOptions::default()).map_err(|e| e.to_string())?;
    let budget = Budget::default();
    budget.reset(DISPATCH_MAX_INSTRUCTIONS);
    guard(&lua, &budget, DISPATCH_MAX_MEMORY).map_err(|e| e.to_string())?;
    for f in ["dofile", "loadfile", "load", "require"] {
        lua.globals()
            .set(f, Value::Nil)
            .map_err(|e| e.to_string())?;
    }
    let st: Shared = Rc::default();
    install(&lua, &st, None).map_err(|e| e.to_string())?;
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

// ------------------------------------------------------------- queries

fn to_lua(lua: &Lua, v: &J) -> mlua::Result<Value> {
    Ok(match v {
        J::Null => Value::Nil,
        J::Bool(b) => Value::Boolean(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        J::String(s) => Value::String(lua.create_string(s)?),
        J::Array(a) => {
            let t = lua.create_table()?;
            for (i, x) in a.iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
        J::Object(o) => {
            let t = lua.create_table()?;
            for (k, x) in o {
                t.raw_set(k.as_str(), to_lua(lua, x)?)?;
            }
            Value::Table(t)
        }
    })
}

/// `focusHistoryID` -> `focus_history_id`, `refreshRate` -> `refresh_rate`.
fn snake(k: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in k.chars() {
        if c.is_ascii_uppercase() && prev_lower {
            out.push('_');
        }
        prev_lower = c.is_ascii_lowercase() || c.is_ascii_digit();
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// A hyprctl JSON object with snake_case keys and Hyprland's Lua names.
fn renamed(o: &J) -> serde_json::Map<String, J> {
    let mut m = serde_json::Map::new();
    if let J::Object(src) = o {
        for (k, v) in src {
            let k = match k.as_str() {
                "hasfullscreen" => "has_fullscreen".into(),
                "lastwindow" => "last_window".into(),
                "lastwindowtitle" => "last_window_title".into(),
                "ispersistent" => "is_persistent".into(),
                "specialWorkspace" => "active_special_workspace".into(),
                "vrr" => "vrr_active".into(),
                "grouped" => "group".into(),
                k => snake(k),
            };
            m.insert(k, v.clone());
        }
    }
    m
}

fn xy(v: Option<&J>, kx: &str, ky: &str) -> J {
    let a = v.and_then(J::as_array);
    let n = |i: usize| a.and_then(|a| a.get(i)).cloned().unwrap_or(J::from(0));
    serde_json::json!({ kx: n(0), ky: n(1) })
}

/// Snapshot of the hyprctl JSON used to build Lua objects.
#[derive(Default)]
struct World {
    clients: Vec<J>,
    workspaces: Vec<J>,
    monitors: Vec<J>,
    active: String,
    cursor: J,
}

fn list(j: J) -> Vec<J> {
    match j {
        J::Array(a) => a,
        _ => Vec::new(),
    }
}

impl World {
    /// `Ok(None)`-free: outside a callback every list is empty.
    fn fetch(lua: &Lua) -> mlua::Result<World> {
        let get = |req: &str| -> mlua::Result<J> {
            Ok(host(lua, "request", req, "")?
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(J::Null))
        };
        let active = get("j/activewindow")?;
        Ok(World {
            clients: list(get("j/clients")?),
            workspaces: list(get("j/workspaces")?),
            monitors: list(get("j/monitors")?),
            active: active["address"].as_str().unwrap_or("").into(),
            cursor: get("j/cursorpos")?,
        })
    }

    fn win_obj(&self, c: &J, depth: u8) -> J {
        let mut m = renamed(c);
        m.insert("at".into(), xy(c.get("at"), "x", "y"));
        m.insert("size".into(), xy(c.get("size"), "x", "y"));
        let addr = c["address"].as_str().unwrap_or("");
        m.insert(
            "active".into(),
            J::Bool(!addr.is_empty() && addr == self.active),
        );
        if depth < 2 {
            if let Some(id) = c["workspace"]["id"].as_i64() {
                m.insert("workspace".into(), self.ws_obj(id, depth + 1));
            }
            if let Some(mon) = c["monitor"].as_i64() {
                let mon = self.monitors.iter().find(|x| x["id"].as_i64() == Some(mon));
                m.insert(
                    "monitor".into(),
                    mon.map_or(J::Null, |x| self.mon_obj(x, depth + 1)),
                );
            }
        }
        J::Object(m)
    }

    fn window_by_addr(&self, addr: &str) -> J {
        self.clients
            .iter()
            .find(|c| c["address"].as_str() == Some(addr))
            .map_or_else(
                || serde_json::json!({ "address": addr }),
                |c| self.win_obj(c, 0),
            )
    }

    fn ws_obj(&self, id: i64, depth: u8) -> J {
        let Some(w) = self
            .workspaces
            .iter()
            .find(|w| w["id"].as_i64() == Some(id))
        else {
            return serde_json::json!({ "id": id, "name": id.to_string() });
        };
        let mut m = renamed(w);
        let shown = self.monitors.iter().any(|mon| {
            mon["activeWorkspace"]["id"].as_i64() == Some(id)
                || mon["specialWorkspace"]["id"].as_i64() == Some(id)
        });
        m.insert("special".into(), J::Bool(id < 0));
        m.insert("active".into(), J::Bool(shown));
        m.insert("visible".into(), J::Bool(shown));
        m.insert("is_empty".into(), J::Bool(w["windows"].as_i64() == Some(0)));
        if depth < 2 {
            let mon = self.monitors.iter().find(|x| x["name"] == w["monitor"]);
            m.insert(
                "monitor".into(),
                mon.map_or(J::Null, |x| self.mon_obj(x, depth + 1)),
            );
        }
        J::Object(m)
    }

    fn mon_obj(&self, mon: &J, depth: u8) -> J {
        let mut m = renamed(mon);
        m.insert(
            "position".into(),
            serde_json::json!({ "x": mon["x"], "y": mon["y"] }),
        );
        m.insert(
            "size".into(),
            serde_json::json!({ "width": mon["width"], "height": mon["height"] }),
        );
        m.insert("enabled".into(), J::Bool(mon["disabled"] != J::Bool(true)));
        m.remove("disabled");
        let ws = |k: &str| mon[k]["id"].as_i64().filter(|id| *id != 0);
        for (key, src) in [
            ("active_workspace", "activeWorkspace"),
            ("active_special_workspace", "specialWorkspace"),
        ] {
            let v = match ws(src) {
                Some(id) if depth < 2 => self.ws_obj(id, depth + 1),
                Some(_) => mon[src].clone(),
                None => J::Null,
            };
            m.insert(key.into(), v);
        }
        J::Object(m)
    }

    fn mon_by_name(&self, name: &str) -> J {
        self.monitors
            .iter()
            .find(|m| m["name"].as_str() == Some(name))
            .map_or_else(
                || serde_json::json!({ "name": name }),
                |m| self.mon_obj(m, 0),
            )
    }

    fn focused_mon(&self) -> Option<&J> {
        self.monitors
            .iter()
            .find(|m| m["focused"] == J::Bool(true))
            .or(self.monitors.first())
    }

    fn mon_at(&self, x: f64, y: f64) -> Option<&J> {
        self.monitors.iter().find(|m| {
            let f = |k: &str| m[k].as_f64().unwrap_or(0.0);
            x >= f("x") && y >= f("y") && x < f("x") + f("width") && y < f("y") + f("height")
        })
    }
}

/// A workspace argument: id, name or a workspace object.
fn ws_id(world: &World, v: &Value) -> Option<i64> {
    match v {
        Value::Integer(i) => Some(*i),
        Value::Number(n) => Some(*n as i64),
        Value::Table(t) => t.get::<i64>("id").ok(),
        Value::String(s) => {
            let s = s.to_string_lossy();
            s.parse().ok().or_else(|| {
                world
                    .workspaces
                    .iter()
                    .find(|w| w["name"].as_str() == Some(s.as_str()))
                    .and_then(|w| w["id"].as_i64())
            })
        }
        _ => None,
    }
}

/// `hl.get_*`: queries of the running compositor (nil while loading).
fn install_queries(lua: &Lua, hl: &Table) -> mlua::Result<()> {
    type Q = fn(&Lua, &World, mlua::Variadic<Value>) -> mlua::Result<J>;
    let qs: Vec<(&str, Q)> = vec![
        ("get_windows", |_, w, _| {
            Ok(J::Array(
                w.clients.iter().map(|c| w.win_obj(c, 0)).collect(),
            ))
        }),
        ("get_window", |_, w, a| {
            let key = match a.first() {
                Some(Value::String(s)) => s.to_string_lossy(),
                _ => return Ok(J::Null),
            };
            let key = key.strip_prefix("address:").unwrap_or(&key).to_string();
            Ok(w.clients
                .iter()
                .find(|c| {
                    c["address"].as_str() == Some(key.as_str())
                        || c["class"].as_str() == Some(key.as_str())
                })
                .map_or(J::Null, |c| w.win_obj(c, 0)))
        }),
        ("get_active_window", |_, w, _| {
            Ok(w.clients
                .iter()
                .find(|c| !w.active.is_empty() && c["address"].as_str() == Some(w.active.as_str()))
                .map_or(J::Null, |c| w.win_obj(c, 0)))
        }),
        ("get_last_window", |_, w, _| {
            Ok(w.clients
                .iter()
                .find(|c| c["focusHistoryID"].as_i64() == Some(1))
                .map_or(J::Null, |c| w.win_obj(c, 0)))
        }),
        ("get_workspaces", |_, w, _| {
            Ok(J::Array(
                w.workspaces
                    .iter()
                    .filter_map(|x| x["id"].as_i64())
                    .map(|id| w.ws_obj(id, 0))
                    .collect(),
            ))
        }),
        ("get_workspace", |_, w, a| {
            let v = a.first().cloned().unwrap_or(Value::Nil);
            Ok(ws_id(w, &v).map_or(J::Null, |id| w.ws_obj(id, 0)))
        }),
        ("get_active_workspace", |_, w, _| {
            Ok(w.focused_mon()
                .and_then(|m| m["activeWorkspace"]["id"].as_i64())
                .map_or(J::Null, |id| w.ws_obj(id, 0)))
        }),
        ("get_active_special_workspace", |_, w, _| {
            Ok(w.focused_mon()
                .and_then(|m| m["specialWorkspace"]["id"].as_i64())
                .filter(|id| *id != 0)
                .map_or(J::Null, |id| w.ws_obj(id, 0)))
        }),
        ("get_workspace_windows", |_, w, a| {
            let v = a.first().cloned().unwrap_or(Value::Nil);
            let id = ws_id(w, &v);
            Ok(J::Array(
                w.clients
                    .iter()
                    .filter(|c| id.is_some() && c["workspace"]["id"].as_i64() == id)
                    .map(|c| w.win_obj(c, 0))
                    .collect(),
            ))
        }),
        ("get_monitors", |_, w, _| {
            Ok(J::Array(
                w.monitors.iter().map(|m| w.mon_obj(m, 0)).collect(),
            ))
        }),
        ("get_monitor", |_, w, a| {
            Ok(match a.first() {
                Some(Value::String(s)) => {
                    let s = s.to_string_lossy();
                    w.monitors
                        .iter()
                        .find(|m| m["name"].as_str() == Some(s.as_str()))
                }
                Some(Value::Integer(i)) => w.monitors.iter().find(|m| m["id"].as_i64() == Some(*i)),
                _ => None,
            }
            .map_or(J::Null, |m| w.mon_obj(m, 0)))
        }),
        ("get_active_monitor", |_, w, _| {
            Ok(w.focused_mon().map_or(J::Null, |m| w.mon_obj(m, 0)))
        }),
        ("get_monitor_at", |_, w, a| {
            let n = |i: usize| match a.get(i) {
                Some(Value::Integer(v)) => *v as f64,
                Some(Value::Number(v)) => *v,
                _ => 0.0,
            };
            let (x, y) = match a.first() {
                Some(Value::Table(t)) => (
                    t.get::<f64>("x").unwrap_or(0.0),
                    t.get::<f64>("y").unwrap_or(0.0),
                ),
                _ => (n(0), n(1)),
            };
            Ok(w.mon_at(x, y).map_or(J::Null, |m| w.mon_obj(m, 0)))
        }),
        ("get_monitor_at_cursor", |_, w, _| {
            let (x, y) = (
                w.cursor["x"].as_f64().unwrap_or(0.0),
                w.cursor["y"].as_f64().unwrap_or(0.0),
            );
            Ok(w.mon_at(x, y)
                .or(w.focused_mon())
                .map_or(J::Null, |m| w.mon_obj(m, 0)))
        }),
        ("get_cursor_pos", |_, w, _| {
            Ok(serde_json::json!({ "x": w.cursor["x"], "y": w.cursor["y"] }))
        }),
    ];
    for (name, q) in qs {
        hl.set(
            name,
            lua.create_function(move |lua, a: mlua::Variadic<Value>| {
                if host(lua, "submap", "", "")?.is_none() {
                    return Ok(Value::Nil);
                }
                let w = World::fetch(lua)?;
                to_lua(lua, &q(lua, &w, a)?)
            })?,
        )?;
    }

    // Raw hyprctl replies.
    for (name, req) in [("get_layers", "j/layers"), ("get_devices", "j/devices")] {
        hl.set(
            name,
            lua.create_function(move |lua, ()| {
                let j = host(lua, "request", req, "")?
                    .and_then(|s| serde_json::from_str::<J>(&s).ok())
                    .unwrap_or(J::Null);
                to_lua(lua, &j)
            })?,
        )?;
    }
    hl.set(
        "get_current_submap",
        lua.create_function(|lua, ()| Ok(host(lua, "submap", "", "")?.unwrap_or_default()))?,
    )?;
    hl.set(
        "get_config",
        lua.create_function(|lua, key: String| {
            let reply = host(lua, "request", &format!("j/getoption {key}"), "")?;
            let j: J = reply
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(J::Null);
            for ty in ["int", "float", "bool", "str", "custom"] {
                if let Some(v) = j.get(ty) {
                    return Ok((to_lua(lua, v)?, Value::Nil));
                }
            }
            let msg = lua.create_string(format!("unknown config key '{key}'"))?;
            Ok((Value::Nil, Value::String(msg)))
        })?,
    )?;
    for name in ["get_urgent_window", "get_last_workspace"] {
        // shortcut: no urgency or workspace history yet; returns nil.
        hl.set(name, lua.create_function(|_, ()| Ok(Value::Nil))?)?;
    }
    hl.set(
        "get_loaded_plugins",
        lua.create_function(|lua, ()| lua.create_table())?,
    )?;
    hl.set("is_key_down", lua.create_function(|_, _: Value| Ok(false))?)?;
    Ok(())
}

/// `hl.notification.create{ text, duration, icon?, color? }` as `hyprctl notify`.
fn notify_request(t: &Table) -> mlua::Result<String> {
    let text: String = t.get("text")?;
    let ms = ["duration", "timeout", "time"]
        .iter()
        .find_map(|k| t.get::<f64>(*k).ok())
        .ok_or_else(|| {
            mlua::Error::external(
                "hl.notification.create: 'duration' (or 'timeout' / 'time') is required",
            )
        })?;
    let icon = match t.get::<Value>("icon")? {
        Value::Integer(i) => i,
        Value::String(s) => match s.to_string_lossy().as_str() {
            "warning" | "warn" => 0,
            "info" => 1,
            "hint" => 2,
            "error" | "err" => 3,
            "confused" | "question" => 4,
            "ok" => 5,
            _ => -1,
        },
        _ => -1,
    };
    let color = match t.get::<Value>("color")? {
        Value::Nil => "0".to_string(),
        v => scalar(&v, "color").map_err(mlua::Error::external)?,
    };
    Ok(format!("notify {icon} {} {color} {text}", ms as i64))
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
        "global" => format!("global, {}", a.need("name", "global")?),
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
        // shortcut: no separate client fullscreen state; internal 2/1 map to
        // fullscreen/maximize and a client-only "spoof" (internal 0) is a no-op.
        "window.fullscreen_state" => match a.s("internal").as_deref() {
            Some("2") => "fullscreen, 0".into(),
            Some("1") => "fullscreen, 1".into(),
            _ => "forcerendererreload".into(),
        },
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

/// Collects a line while loading, or applies it like `hyprctl keyword`
/// inside a callback. Errors are recorded and the script keeps running
/// (Hyprland also reports per-call errors instead of aborting).
fn emit(lua: &Lua, st: &Shared, r: Result<String, String>) -> mlua::Result<()> {
    match r {
        Ok(line) => match host(lua, "keyword", &line, "")? {
            Some(err) if !err.is_empty() => st.borrow_mut().errors.push(err),
            Some(_) => {}
            None => st.borrow_mut().lines.push(line),
        },
        Err(e) => st.borrow_mut().errors.push(e),
    }
    Ok(())
}

/// Stores a Lua callback; `__lua, N` is the dispatcher that runs it.
fn callback(st: &Shared, f: Function) -> String {
    let mut s = st.borrow_mut();
    s.callbacks.push(f);
    format!("__lua, {}", s.callbacks.len() - 1)
}

/// `hl.gesture{ fingers, direction, action, ... }` as a `gesture =` line.
fn gesture_line(st: &Shared, t: &Table) -> Result<String, String> {
    let fingers: i64 = t
        .get("fingers")
        .map_err(|_| "hl.gesture: missing `fingers`")?;
    let dir: String = t
        .get("direction")
        .map_err(|_| "hl.gesture: missing `direction`")?;
    let mut parts = vec![fingers.to_string(), dir];
    for (k, flag) in [("mods", "mod"), ("scale", "scale")] {
        if let Ok(v) = t.get::<Value>(k) {
            if !matches!(v, Value::Nil) {
                parts.push(format!("{flag}:{}", scalar(&v, k)?));
            }
        }
    }
    match t.get::<Value>("action").map_err(|e| e.to_string())? {
        Value::String(s) => {
            parts.push(s.to_string_lossy());
            if let Ok(Value::String(arg)) = t.get::<Value>("arg") {
                parts.push(arg.to_string_lossy());
            }
        }
        Value::Function(f) => {
            parts.push("dispatcher".into());
            parts.push(callback(st, f));
        }
        _ => return Err("hl.gesture: `action` must be a name or a function".into()),
    }
    Ok(format!("gesture = {}", parts.join(", ")))
}

/// `dir` is the config directory; `None` means sandboxed (no `package`,
/// `hl.load`, `hl.env` and no callbacks).
fn install(lua: &Lua, st: &Shared, dir: Option<&Path>) -> mlua::Result<()> {
    let g = lua.globals();
    if let Some(dir) = dir {
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
    }

    // mlua's safe stdlib has no `debug` (it can break memory safety); config
    // scripts use `debug.getinfo(1, "S").source` to find their own path, so
    // provide that much.
    if dir.is_some() && matches!(g.get::<Value>("debug")?, Value::Nil) {
        let debug = lua.create_table()?;
        debug.set(
            "getinfo",
            lua.create_function(|lua, (what, _): (Value, Value)| {
                let level = match what {
                    Value::Integer(l) if l >= 0 => l as usize,
                    _ => return Ok(Value::Nil),
                };
                let Some(info) = lua.inspect_stack(level) else {
                    return Ok(Value::Nil);
                };
                let src = info.source();
                let t = lua.create_table()?;
                t.set("source", src.source.as_deref().unwrap_or("=?"))?;
                t.set("short_src", src.short_src.as_deref().unwrap_or("?"))?;
                t.set("what", src.what)?;
                t.set("linedefined", src.line_defined.map_or(-1, |l| l as i64))?;
                t.set("currentline", info.curr_line())?;
                Ok(Value::Table(t))
            })?,
        )?;
        debug.set(
            "traceback",
            lua.create_function(|_, msg: Option<String>| Ok(msg.unwrap_or_default()))?,
        )?;
        g.set("debug", debug)?;
    }

    let hl = lua.create_table()?;

    // hl.dsp.*
    let dsp = lua.create_table()?;
    for &p in DSP {
        let s = st.clone();
        let sandboxed = dir.is_none();
        let f = lua.create_function(move |_, v: Value| match translate(p, &A(v)) {
            Ok(d) => Ok(d),
            Err(e) if sandboxed => Err(mlua::Error::external(e)),
            // A config error, not a script abort: the bind does nothing.
            Err(e) => {
                s.borrow_mut().errors.push(e);
                Ok("forcerendererreload".into())
            }
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

    let s = st.clone();
    hl.set(
        "config",
        lua.create_function(move |lua, t: Table| {
            let mut lines = Vec::new();
            match flatten("", &t, &mut lines) {
                Ok(()) => {
                    for l in lines {
                        emit(lua, &s, Ok(l))?;
                    }
                }
                Err(e) => s.borrow_mut().errors.push(format!("hl.config: {e}")),
            }
            Ok(())
        })?,
    )?;

    let s = st.clone();
    let sandboxed = dir.is_none();
    hl.set(
        "bind",
        lua.create_function(
            move |lua, (keys, d, opts): (String, Value, Option<Table>)| {
                let r = match d {
                    Value::String(d) => Ok(bind_line(&keys, &d.to_string_lossy(), opts.as_ref())),
                    Value::Function(f) if !sandboxed => {
                        Ok(bind_line(&keys, &callback(&s, f), opts.as_ref()))
                    }
                    _ => Err(format!(
                        "hl.bind(\"{keys}\"): expected a hl.dsp.* dispatcher or a function"
                    )),
                };
                emit(lua, &s, r)
            },
        )?,
    )?;

    let s = st.clone();
    hl.set(
        "unbind",
        lua.create_function(move |lua, keys: String| {
            let (mods, key) = split_keys(&keys);
            emit(lua, &s, Ok(format!("unbind = {mods}, {key}")))
        })?,
    )?;

    // Runs now inside a callback; while loading it is an `exec` line, which
    // Hyprland also runs on every (re)load.
    let s = st.clone();
    hl.set(
        "exec_cmd",
        lua.create_function(move |lua, cmd: String| {
            match host(lua, "dispatch", "exec", &cmd)? {
                Some(err) if !err.is_empty() => s.borrow_mut().errors.push(err),
                Some(_) => {}
                None => s.borrow_mut().lines.push(format!("exec = {cmd}")),
            }
            Ok(())
        })?,
    )?;

    // hl.dispatch(hl.dsp.x(...)) or hl.dispatch(function): runs at once in a
    // callback; while loading it runs once the compositor is up.
    let s = st.clone();
    hl.set(
        "dispatch",
        lua.create_function(move |lua, d: Value| {
            match d {
                Value::String(text) => {
                    let text = text.to_string_lossy();
                    let (name, args) = split_dsp(&text);
                    match host(lua, "dispatch", &name, &args)? {
                        Some(err) if !err.is_empty() => {
                            s.borrow_mut().errors.push(format!("hl.dispatch: {err}"))
                        }
                        Some(_) => {}
                        None => s.borrow_mut().deferred.push(text),
                    }
                }
                Value::Function(f) => f.call::<()>(())?,
                _ => {
                    return Err(mlua::Error::external(
                        "hl.dispatch: expected a hl.dsp.* dispatcher",
                    ))
                }
            }
            Ok(true)
        })?,
    )?;

    if dir.is_some() {
        let s = st.clone();
        hl.set(
            "on",
            lua.create_function(move |lua, (event, f): (String, Function)| {
                let live = Rc::new(Cell::new(true));
                s.borrow_mut().events.push((event, f, live.clone()));
                let sub = lua.create_table()?;
                sub.set(
                    "remove",
                    lua.create_function(move |_, _: mlua::Variadic<Value>| {
                        live.set(false);
                        Ok(())
                    })?,
                )?;
                Ok(sub)
            })?,
        )?;

        let s = st.clone();
        hl.set(
            "timer",
            lua.create_function(move |lua, (f, opts): (Value, Value)| {
                // A bad call is a config error; the rest of the script still runs.
                let fail = |msg: &str| {
                    s.borrow_mut().errors.push(format!("hl.timer: {msg}"));
                    Ok(Value::Nil)
                };
                let (Value::Function(f), Value::Table(opts)) = (f, opts) else {
                    return fail(
                        "expected (function, { timeout = ms, type = \"oneshot\" | \"repeat\" })",
                    );
                };
                let ms = match opts.get::<Value>("timeout")? {
                    Value::Integer(i) => i as f64,
                    Value::Number(n) => n,
                    _ => return fail("opts.timeout must be a number (ms)"),
                };
                if ms <= 0.0 {
                    return fail("opts.timeout must be > 0");
                }
                let repeat = match opts.get::<Option<String>>("type")?.as_deref() {
                    Some("repeat") => true,
                    Some("oneshot") | None => false,
                    Some(_) => return fail("opts.type must be \"repeat\" or \"oneshot\""),
                };
                let live = Rc::new(Cell::new(true));
                {
                    let mut st = s.borrow_mut();
                    st.timers.push(TimerSlot {
                        f,
                        ms: ms as u64,
                        repeat,
                        live: live.clone(),
                    });
                    let idx = st.timers.len() - 1;
                    st.new_timers.push(idx);
                }
                let t = lua.create_table()?;
                t.set("timeout", ms as u64)?;
                let stop = lua.create_function(move |_, _: mlua::Variadic<Value>| {
                    live.set(false);
                    Ok(())
                })?;
                for name in ["stop", "cancel", "remove"] {
                    t.set(name, stop.clone())?;
                }
                Ok(Value::Table(t))
            })?,
        )?;

        let s = st.clone();
        hl.set(
            "gesture",
            lua.create_function(move |lua, t: Table| {
                let r = gesture_line(&s, &t);
                emit(lua, &s, r)
            })?,
        )?;
    }

    let s = st.clone();
    hl.set(
        "define_submap",
        lua.create_function(move |lua, args: mlua::Variadic<Value>| {
            let name = match args.first() {
                Some(Value::String(s)) => s.to_string_lossy(),
                _ => return Err(mlua::Error::external("define_submap: missing name")),
            };
            let f = match args.last() {
                Some(Value::Function(f)) => f.clone(),
                _ => return Err(mlua::Error::external("define_submap: missing function")),
            };
            emit(lua, &s, Ok(format!("submap = {name}")))?;
            let r = f.call::<()>(());
            emit(lua, &s, Ok("submap = reset".into()))?;
            r
        })?,
    )?;

    for (name, kind) in [
        ("window_rule", 0),
        ("layer_rule", 1),
        ("workspace_rule", 2),
        ("device", 3),
    ] {
        let s = st.clone();
        hl.set(
            name,
            lua.create_function(move |lua, t: Table| {
                // Window rules are always named so `rule:set_enabled()` can
                // address them (`windowrule[NAME]:enable`).
                let rule_name = if kind == 0 {
                    let n = match t.get::<Value>("name")? {
                        Value::String(n) => n.to_string_lossy(),
                        _ => format!("lua-rule-{}", s.borrow().lines.len()),
                    };
                    t.set("name", n.as_str())?;
                    Some(n)
                } else {
                    None
                };
                let r = match kind {
                    0 => window_rule_block(&t),
                    1 => layer_rule_line(&t),
                    2 => workspace_rule_line(&t),
                    _ => device_block(&t),
                };
                emit(lua, &s, r.map_err(|e| format!("hl.{name}: {e}")))?;
                let Some(n) = rule_name else {
                    return Ok(Value::Nil);
                };
                let obj = lua.create_table()?;
                obj.set("name", n.as_str())?;
                let s = s.clone();
                obj.set(
                    "set_enabled",
                    lua.create_function(move |lua, (_, on): (Value, Value)| {
                        let on = !matches!(on, Value::Boolean(false) | Value::Nil);
                        emit(lua, &s, Ok(format!("windowrule[{n}]:enable = {on}")))
                    })?,
                )?;
                Ok(Value::Table(obj))
            })?,
        )?;
    }

    // Process-wide side effects: config scripts only, not sandboxed dispatch.
    if dir.is_some() {
        let s = st.clone();
        hl.set(
            "env",
            lua.create_function(move |lua, (k, v): (String, String)| {
                std::env::set_var(&k, &v);
                emit(lua, &s, Ok(format!("env = {k},{v}")))
            })?,
        )?;
    }

    let s = st.clone();
    hl.set(
        "monitor",
        lua.create_function(move |lua, t: Table| {
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
            emit(lua, &s, Ok(line))
        })?,
    )?;

    let s = st.clone();
    hl.set(
        "curve",
        lua.create_function(move |lua, (name, t): (String, Table)| {
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
                emit(lua, &s, Ok(format!("bezier = {name}, {}", pts.join(", "))))?;
            }
            Ok(())
        })?,
    )?;

    let s = st.clone();
    hl.set(
        "animation",
        lua.create_function(move |lua, t: Table| {
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
            emit(lua, &s, Ok(line))
        })?,
    )?;

    let notification = lua.create_table()?;
    notification.set(
        "create",
        lua.create_function(|lua, t: Table| {
            let req = notify_request(&t)?;
            host(lua, "request", &req, "")?;
            lua.create_table()
        })?,
    )?;
    notification.set("get", lua.create_function(|lua, ()| lua.create_table())?)?;
    hl.set("notification", notification)?;

    install_queries(lua, &hl)?;

    // Accepted without effect.
    for name in [
        "permission",
        "plugin",
        "clear_crashed_lockscreen",
        "exec_scheduled_prop_refresh_immediately",
    ] {
        hl.set(
            name,
            lua.create_function(|_, _: mlua::Variadic<Value>| Ok(()))?,
        )?;
    }
    hl.set(
        "version",
        lua.create_function(|_, ()| Ok(env!("CARGO_PKG_VERSION")))?,
    )?;

    if let Some(dir) = dir {
        let d = dir.to_path_buf();
        hl.set(
            "load",
            lua.create_function(move |lua, file: String| {
                let path = d.join(&file);
                let src = std::fs::read_to_string(&path).map_err(mlua::Error::external)?;
                lua.load(src).set_name(file).exec()
            })?,
        )?;
    }

    // Anything else: a config error instead of "attempt to call a nil value".
    let s = st.clone();
    let meta = lua.create_table()?;
    meta.set(
        "__index",
        lua.create_function(move |lua, (_, key): (Table, String)| {
            let s = s.clone();
            lua.create_function(move |_, _: mlua::Variadic<Value>| {
                s.borrow_mut()
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
               hl.bind(mod .. " + F", function() end)
               hl.bind(mod .. " + G", hl.dsp.global("quickshell:search"))"#,
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
            "bind = SUPER, F, __lua, 0",
            "bind = SUPER, G, global, quickshell:search",
        ] {
            assert!(
                text.lines().any(|l| l == line),
                "missing `{line}` in\n{text}"
            );
        }
        assert!(errors.is_empty(), "{errors:?}");
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
               hl.timer(1000, function() end)
               hl.gesture({ fingers = 3, direction = "horizontal", action = "workspace" })
               hl.gesture({ fingers = 4, direction = "up", action = function() end })"#,
        );
        for line in [
            "exec = swaybg",
            "gesture = 3, horizontal, workspace",
            "gesture = 4, up, dispatcher, __lua, 0",
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
        assert!(
            !text
                .lines()
                .any(|l| l.starts_with("exec") && l.contains("waybar")),
            "start handlers run later:\n{text}"
        );
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("hl.timer: expected (function"),
            "{errors:?}"
        );
    }

    /// Records what callbacks do; serves canned hyprctl JSON.
    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
    }

    impl Host for Fake {
        fn dispatch(&mut self, name: &str, args: &str) -> Result<(), String> {
            self.log.push(format!("dispatch {name} {args}"));
            Ok(())
        }
        fn keyword(&mut self, line: &str) -> Result<(), String> {
            self.log.push(format!("keyword {line}"));
            Ok(())
        }
        fn request(&mut self, req: &str) -> String {
            if req.starts_with("notify") {
                self.log.push(req.into());
            }
            match req {
                "j/clients" => {
                    r#"[{"address":"0xa","class":"kitty","title":"t","at":[10,20],
                    "size":[300,200],"workspace":{"id":2,"name":"2"},"monitor":0,
                    "floating":true,"focusHistoryID":0}]"#
                }
                "j/activewindow" => r#"{"address":"0xa","class":"kitty"}"#,
                "j/workspaces" => {
                    r#"[{"id":2,"name":"2","monitor":"HEADLESS-1","windows":1,
                    "hasfullscreen":false}]"#
                }
                "j/monitors" => {
                    r#"[{"id":0,"name":"HEADLESS-1","width":1920,"height":1080,
                    "x":0,"y":0,"focused":true,"activeWorkspace":{"id":2,"name":"2"},
                    "specialWorkspace":{"id":0,"name":""}}]"#
                }
                "j/cursorpos" => r#"{"x":5,"y":6}"#,
                "j/getoption cursor:zoom_factor" => {
                    r#"{"option":"cursor:zoom_factor","float":1.5,"set":false}"#
                }
                _ => "unknown request",
            }
            .into()
        }
        fn submap(&self) -> String {
            "resize".into()
        }
    }

    fn load(src: &str) -> (Runtime, String, Vec<String>) {
        Runtime::load(src, Path::new("."), "test.lua").unwrap()
    }

    #[test]
    fn function_binds_run_against_the_host() {
        let (rt, text, errors) = load(
            r#"hl.bind("SUPER + Z", function()
                   local ws = hl.get_active_workspace()
                   local w = hl.get_active_window()
                   hl.dispatch(hl.dsp.focus({ workspace = ws.id + 1 }))
                   hl.exec_cmd("echo " .. w.class .. " " .. w.at.x .. " " .. w.workspace.monitor.name)
                   hl.config({ general = { gaps_in = hl.get_config("cursor:zoom_factor") } })
                   hl.notification.create({ text = hl.get_current_submap(), duration = 500, icon = "ok" })
                   local c = hl.get_cursor_pos()
                   hl.dispatch(hl.dsp.exec_cmd("pos " .. c.x .. "," .. c.y))
               end)"#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert!(text.contains("bind = SUPER, Z, __lua, 0"), "{text}");
        let mut h = Fake::default();
        rt.call_bind(0, &mut h);
        assert_eq!(rt.take_errors(), Vec::<String>::new());
        assert_eq!(
            h.log,
            [
                "dispatch workspace 3",
                "dispatch exec echo kitty 10 HEADLESS-1",
                "keyword general:gaps_in = 1.5",
                "notify 5 500 0 resize",
                "dispatch exec pos 5,6",
            ]
        );
        // Queries outside a callback (no compositor yet) are nil.
        let (_, _, errors) = load("assert(hl.get_active_window() == nil)");
        assert!(errors.is_empty());
    }

    #[test]
    fn events_timers_and_deferred_dispatch() {
        let (rt, _, errors) = load(
            r#"hl.dispatch(hl.dsp.focus({ workspace = 4 }))
               hl.on("hyprland.start", function() hl.exec_cmd("waybar") end)
               local sub = hl.on("window.open", function(w)
                   hl.exec_cmd("open " .. w.class .. " " .. tostring(w.floating) .. " " .. w.size.y)
               end)
               hl.on("workspace.active", function(ws) hl.exec_cmd("ws " .. ws.name) end)
               local n = 0
               hl.timer(function() n = n + 1; hl.exec_cmd("tick " .. n) end, { timeout = 100, type = "repeat" })
               hl.bind("SUPER + X", function() sub:remove() end)"#,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(rt.take_deferred(), ["workspace, 4"]);
        assert_eq!(rt.take_timers(), [(0, 100)]);
        let mut h = Fake::default();
        rt.fire("hyprland.start", &[], &mut h);
        rt.fire("window.open", &[EvArg::Window("0xa".into())], &mut h);
        rt.fire("workspace.active", &[EvArg::Workspace(2)], &mut h);
        assert_eq!(rt.fire_timer(0, &mut h), Some(100));
        assert_eq!(rt.fire_timer(0, &mut h), Some(100));
        assert!(rt.has_handlers("window.open"));
        rt.call_bind(0, &mut h);
        assert!(!rt.has_handlers("window.open"));
        rt.fire("window.open", &[EvArg::Window("0xa".into())], &mut h);
        assert_eq!(rt.take_errors(), Vec::<String>::new());
        assert_eq!(
            h.log,
            [
                "dispatch exec waybar",
                "dispatch exec open kitty true 200",
                "dispatch exec ws 2",
                "dispatch exec tick 1",
                "dispatch exec tick 2",
            ]
        );
    }

    #[test]
    fn dotfile_lua_idioms() {
        let dir = std::env::temp_dir().join(format!("hypr-lua-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("lib.lua"),
            r#"return assert(debug.getinfo(1, "S").source:match("^@(.*)/"))"#,
        )
        .unwrap();
        let src = format!(
            r#"local root = dofile("{}/lib.lua")
               assert(root:find("hypr%-lua"), root)
               local r = hl.window_rule({{ name = "Mode", match = {{ class = ".*" }}, float = true }})
               r:set_enabled(false)
               local anon = hl.window_rule({{ match = {{ class = "x" }}, float = true }})
               assert(anon.name:find("^lua%-rule%-"))
               hl.bind("SUPER + ALT + F", hl.dsp.window.fullscreen_state({{ internal = 0, client = 3 }}))
               hl.bind("SUPER + B", hl.dsp.window.move({{ bogus = 1 }}))"#,
            dir.display().to_string().replace('\\', "/")
        );
        let (text, errors) = to_hyprlang(&src, &dir, "test.lua").unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert!(text.contains("windowrule[Mode]:enable = false"), "{text}");
        assert!(
            text.contains("bind = SUPER ALT, F, forcerendererreload"),
            "{text}"
        );
        // A bad hl.dsp.* call is a config error, not a script abort.
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn ipc_events_map_to_lua_events() {
        let w = |a: &str| EvArg::Window(a.into());
        assert_eq!(
            ipc_to_lua_event("openwindow", "abc,2,kitty,t"),
            Some(("window.open", vec![w("0xabc")]))
        );
        assert_eq!(
            ipc_to_lua_event("movewindowv2", "abc,3,3"),
            Some((
                "window.move_to_workspace",
                vec![w("0xabc"), EvArg::Workspace(3)]
            ))
        );
        assert_eq!(
            ipc_to_lua_event("workspacev2", "-98,special:magic"),
            Some(("workspace.active", vec![EvArg::Workspace(-98)]))
        );
        assert_eq!(ipc_to_lua_event("activewindowv2", ""), None);
        assert_eq!(ipc_to_lua_event("activewindow", "kitty,t"), None);
        assert_eq!(
            ipc_to_lua_event("configreloaded", ""),
            Some(("config.reloaded", vec![]))
        );
    }

    #[test]
    fn eval_shares_the_config_state() {
        let (rt, _, _) = load("counter = 41");
        let mut h = Fake::default();
        assert_eq!(
            rt.eval("counter = counter + 1; return counter", &mut h),
            Ok("42".into())
        );
        assert_eq!(
            rt.eval("hl.dispatch(hl.dsp.exit())", &mut h),
            Ok("true".into())
        );
        assert_eq!(h.log, ["dispatch exit "]);
        assert!(rt.eval("error('x')", &mut h).is_err());
    }

    #[test]
    fn callback_errors_and_loops_are_contained() {
        let (rt, _, _) = load(
            r#"hl.bind("A", function() error("boom") end)
               hl.bind("B", function() while true do end end)
               hl.bind("C", function() hl.dispatch(hl.dsp.exit()) end)"#,
        );
        let mut h = Fake::default();
        rt.call_bind(0, &mut h);
        let t = std::time::Instant::now();
        rt.call_bind(1, &mut h);
        assert!(t.elapsed().as_secs() < 10);
        rt.call_bind(7, &mut h);
        let errors = rt.take_errors();
        assert_eq!(errors.len(), 3, "{errors:?}");
        assert!(errors[0].contains("boom"));
        assert!(errors[1].contains("instruction limit"));
        // The budget resets per callback: later binds still run.
        rt.call_bind(2, &mut h);
        assert_eq!(h.log, ["dispatch exit "]);
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
        assert!(eval_dispatch("hl.dsp.send_key_state({})").is_err());
    }

    #[test]
    fn dispatch_is_sandboxed() {
        // os/io/package/file loading are gone, so these cannot kill or read.
        for expr in [
            "os.exit() or hl.dsp.exit()",
            "os.exit()",
            "io.open('/etc/passwd')",
            "dofile('/etc/passwd')",
            "require('os')",
            "load('return 1')()",
        ] {
            assert!(eval_dispatch(expr).is_err(), "{expr} should fail");
        }
        // A loop or memory bomb is cut off instead of hanging the compositor.
        let t = std::time::Instant::now();
        assert!(eval_dispatch("hl.version() while true do end").is_err());
        assert!(eval_dispatch("string.rep('x', 1 << 30)").is_err());
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        // Normal expressions still work.
        assert_eq!(eval_dispatch("hl.dsp.exit()").unwrap().0, "exit");
    }

    #[test]
    fn config_loop_is_cut_off() {
        let r = to_hyprlang("while true do end", Path::new("."), "loop.lua");
        assert!(r.unwrap_err().contains("instruction limit"));
    }
}
