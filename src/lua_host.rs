//! The compositor half of `hyprland.lua`: runs function binds, `hl.on`
//! handlers, `hl.timer` callbacks and `hyprctl eval` from `lua_config.rs`
//! against `Hypr` (the `Host` trait).

use std::{rc::Rc, time::Duration};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::{
    config::Dispatcher,
    lua_config::{ipc_to_lua_event, EvArg, Host, Runtime},
    state::Hypr,
};

struct LuaHost<'a>(&'a mut Hypr);

impl Host for LuaHost<'_> {
    fn dispatch(&mut self, name: &str, args: &str) -> Result<(), String> {
        let d = Dispatcher::parse(name, args)?;
        self.0.dispatch(&d);
        Ok(())
    }

    fn keyword(&mut self, line: &str) -> Result<(), String> {
        let (key, value) = line
            .split_once('=')
            .filter(|_| !line.contains('\n'))
            .ok_or_else(|| {
                let first = line.lines().next().unwrap_or("");
                format!("`{first}`: not supported inside a Lua callback yet")
            })?;
        match self.0.keyword(&format!("{} {}", key.trim(), value.trim())) {
            r if r == "ok" => Ok(()),
            r => Err(r),
        }
    }

    fn request(&mut self, req: &str) -> String {
        crate::ipc::handle_request(self.0, req)
    }

    fn submap(&self) -> String {
        self.0.wm.submap.clone()
    }
}

impl Hypr {
    fn lua(&self) -> Option<Rc<Runtime>> {
        self.config.lua.0.clone()
    }

    /// Logs callback errors (also shown by `hyprctl configerrors`) and arms
    /// timers created by the last Lua call.
    fn lua_after(&mut self, rt: &Rc<Runtime>) {
        for e in rt.take_errors() {
            tracing::warn!("lua: {e}");
            if !self.reload.errors.contains(&e) {
                self.reload.errors.push(e);
            }
        }
        for (idx, ms) in rt.take_timers() {
            let generation = rt.generation;
            let timer = Timer::from_duration(Duration::from_millis(ms));
            let _ = self.wm.loop_handle.insert_source(timer, move |_, _, data| {
                let h = &mut data.state;
                // A reload replaced the runtime: its timers die with it.
                let Some(rt) = h.lua().filter(|rt| rt.generation == generation) else {
                    return TimeoutAction::Drop;
                };
                let next = rt.fire_timer(idx, &mut LuaHost(h));
                h.lua_after(&rt);
                match next {
                    Some(ms) => TimeoutAction::ToDuration(Duration::from_millis(ms)),
                    None => TimeoutAction::Drop,
                }
            });
        }
    }

    /// After a (re)load: run `hl.dispatch` calls made while loading and arm
    /// the timers it created.
    pub fn lua_loaded(&mut self) {
        let Some(rt) = self.lua() else { return };
        for text in rt.take_deferred() {
            let (name, args) = text.split_once(',').unwrap_or((&text, ""));
            match Dispatcher::parse(name.trim(), args.trim()) {
                Ok(d) => self.dispatch(&d),
                Err(e) => tracing::warn!("lua: hl.dispatch: {e}"),
            }
        }
        self.lua_after(&rt);
    }

    /// The dispatcher `__lua, id` (a function bind or gesture callback).
    pub fn lua_bind(&mut self, id: usize) {
        let Some(rt) = self.lua() else {
            tracing::warn!("__lua {id}: the config is not a hyprland.lua");
            return;
        };
        rt.call_bind(id, &mut LuaHost(self));
        self.lua_after(&rt);
    }

    /// Runs `hl.on(event)` handlers now (`hyprland.start`/`shutdown`).
    pub fn lua_fire(&mut self, event: &str, args: &[EvArg]) {
        let Some(rt) = self.lua() else { return };
        if rt.has_handlers(event) {
            rt.fire(event, args, &mut LuaHost(self));
            self.lua_after(&rt);
        }
    }

    /// Called once per event-loop iteration: turns the IPC events emitted
    /// since the last call into `hl.on` events. Handlers run here, never
    /// inside a dispatcher, so they see the finished state.
    pub fn lua_drain(&mut self) {
        let events = std::mem::take(&mut self.ipc.lua_events);
        if events.is_empty() || self.lua().is_none() {
            return;
        }
        for (event, data) in events {
            if let Some((name, args)) = ipc_to_lua_event(&event, &data) {
                self.lua_fire(name, &args);
            } else if event == "fullscreen" {
                // `fullscreen>>0|1` names no window: it is the focused one.
                if let Some(w) = self.focused_window() {
                    let addr = crate::ipc::window_addr(&w);
                    self.lua_fire("window.fullscreen", &[EvArg::Window(addr)]);
                }
            }
        }
    }

    /// `hyprctl eval`: `None` when the config is not a hyprland.lua.
    pub fn lua_eval(&mut self, code: &str) -> Option<Result<String, String>> {
        let rt = self.lua()?;
        let r = rt.eval(code, &mut LuaHost(self));
        self.lua_after(&rt);
        Some(r)
    }
}
