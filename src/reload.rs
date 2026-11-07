//! Config reload, autoreload, `hyprctl keyword` and `getoption`.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use smithay::reexports::calloop::{
    timer::{TimeoutAction, Timer},
    LoopHandle,
};

use crate::{
    config::{Config, ConfigError},
    options,
    state::Hypr,
    wm::spawn,
    CalloopData,
};

/// File modification times of the config and every `source`d file.
type Stamps = Vec<(PathBuf, Option<SystemTime>)>;

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

#[derive(Default)]
pub struct ReloadState {
    /// Config file the compositor was started with (`None`: built-in defaults).
    pub path: Option<PathBuf>,
    /// Lines shown by `hyprctl configerrors`.
    pub errors: Vec<String>,
    /// `hyprctl seterror <color> <msg>` (the on-screen bar is not drawn yet).
    pub seterror: Option<(String, String)>,
    stamps: Stamps,
}

impl ReloadState {
    fn set_errors(&mut self, errors: &[ConfigError]) {
        self.errors = errors.iter().map(|e| e.to_string()).collect();
    }

    /// Remembers the mtimes of the main file and every `source`d file.
    fn restamp(&mut self, files: &[PathBuf]) {
        self.stamps = files
            .iter()
            .chain(&self.path)
            .map(|f| (f.clone(), mtime(f)))
            .collect();
    }

    fn changed(&self) -> bool {
        self.stamps.iter().any(|(p, t)| mtime(p) != *t)
    }
}

/// Records the startup config state and starts the autoreload poll.
pub fn init(
    handle: &LoopHandle<'_, CalloopData>,
    h: &mut Hypr,
    path: Option<PathBuf>,
    errors: &[ConfigError],
) {
    h.reload.path = path;
    h.reload.set_errors(errors);
    let files = h.config.files.clone();
    h.reload.restamp(&files);
    // shortcut: mtime polling once a second instead of inotify (no new
    // dependency); switch to an inotify fd source if latency or load matters.
    let timer = Timer::from_duration(Duration::from_secs(1));
    let _ = handle.insert_source(timer, |_, _, data| {
        let h = &mut data.state;
        if !h.config.misc.disable_autoreload && h.reload.changed() {
            tracing::info!("config file changed, reloading");
            h.reload_config();
        }
        TimeoutAction::ToDuration(Duration::from_secs(1))
    });
}

impl Hypr {
    /// Re-reads the config file given at startup and applies it.
    pub fn reload_config(&mut self) {
        let loaded = match &self.reload.path {
            Some(p) => Config::load(p).map_err(|e| format!("cannot read {}: {e}", p.display())),
            None => Ok((Config::with_default_binds(), Vec::new())),
        };
        match loaded {
            Ok((config, errors)) => {
                for e in &errors {
                    tracing::warn!("config error: {e}");
                }
                self.reload.set_errors(&errors);
                self.config = config;
                self.apply_monitor_rules();
                crate::apply_env(&self.config);
                self.lua_loaded();
                // Hyprland runs `exec` (not `exec-once`) again on every reload.
                for cmd in self.config.exec.clone() {
                    spawn(&cmd);
                }
                self.relayout();
                self.refresh_all_rules();
                // Runtime settings that live outside `Config` get re-applied here.
                self.apply_input_config();
            }
            Err(e) => {
                tracing::error!("config reload failed: {e}");
                self.reload.errors = vec![e];
            }
        }
        let files = self.config.files.clone();
        self.reload.restamp(&files);
        self.reload.seterror = None;
        self.ipc.emit("configreloaded", "");
    }

    /// `hyprctl keyword <key> <value>`.
    pub fn keyword(&mut self, args: &str) -> String {
        let (key, value) = match options::parse_keyword(args) {
            Ok(kv) => kv,
            Err(e) => return e.into(),
        };
        match self.config.apply_keyword(key, value) {
            Ok(()) => {
                if key == "exec" {
                    spawn(value);
                }
                self.relayout();
                self.refresh_all_rules();
                if key.starts_with("input") || key.starts_with("device") {
                    self.apply_input_config();
                }
                "ok".into()
            }
            Err(e) => e,
        }
    }

    /// `hyprctl getoption <key>`.
    pub fn getoption(&self, key: &str, json: bool) -> String {
        let key = key.trim().to_ascii_lowercase();
        match options::get_option(&self.config, &key) {
            Some(v) => {
                options::format_option(&key, &v, self.config.set_options.contains(&key), json)
            }
            None => "no such option".into(),
        }
    }
}
