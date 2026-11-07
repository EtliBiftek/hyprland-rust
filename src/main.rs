//! hyprland-rust: a Hyprland-inspired dynamic tiling Wayland compositor in Rust.
//!
//! The compositor core is based on Smithay's `smallvil` example (MIT).

#![allow(irrefutable_let_patterns)]

mod config;
mod gestures;
mod grabs;
mod groups;
mod handlers;
mod headless;
mod input;
mod ipc;
mod ipc_cmds;
mod layout;
mod lua_config;
mod lua_host;
mod master;
mod monitors;
mod options;
mod reload;
mod rules;
mod selectors;
mod state;
mod tiling;
mod tty;
mod winit;
mod wm;
mod wm_groups;
mod workspace;

use std::path::PathBuf;

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
pub use state::Hypr;

use crate::config::{Config, ConfigError};

pub struct CalloopData {
    state: Hypr,
    display_handle: DisplayHandle,
    /// Only set when running on the TTY backend.
    tty: Option<tty::Tty>,
}

const USAGE: &str = "\
Usage: hyprland-rust [OPTIONS]

Options:
  -c, --config <FILE>    Config file (default: $XDG_CONFIG_HOME/hypr/hyprland.conf)
      --backend <NAME>   `winit` (nested window), `tty` (DRM/KMS, run from a VT)
                         or `headless` (virtual output, no rendering, for CI).
                         Default: winit if WAYLAND_DISPLAY or DISPLAY is set, else tty
      --check-config     Parse the config, print errors and exit
  -h, --help             Show this help
";

/// Returns the config, its parse errors and the file it came from (kept for
/// `hyprctl reload` and autoreload).
fn load_config(path: Option<PathBuf>) -> (Config, Vec<ConfigError>, Option<PathBuf>) {
    let path = path.or_else(Config::default_path);
    let Some(path) = path.filter(|p| p.exists()) else {
        tracing::info!("no config file found, using built-in defaults");
        return (Config::with_default_binds(), Vec::new(), None);
    };
    match Config::load(&path) {
        Ok((config, errors)) => {
            for e in &errors {
                tracing::warn!("config error: {e}");
            }
            tracing::info!("loaded config from {}", path.display());
            (config, errors, Some(path))
        }
        Err(e) => {
            tracing::error!("cannot read {}: {e}; using defaults", path.display());
            let err = ConfigError {
                file: path.clone(),
                line: 0,
                message: e.to_string(),
            };
            (Config::with_default_binds(), vec![err], Some(path))
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    } else {
        tracing_subscriber::fmt().init();
    }

    let mut config_path = None;
    let mut check_only = false;
    let mut backend = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => config_path = args.next().map(PathBuf::from),
            "--check-config" => check_only = true,
            "--backend" => backend = args.next(),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument `{other}`\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let (config, config_errors, loaded_from) = load_config(config_path);
    let error_count = config_errors.len();
    if check_only {
        println!(
            "{} binds, {} config errors",
            config.binds.len(),
            error_count
        );
        std::process::exit(if error_count == 0 { 0 } else { 1 });
    }

    let backend = backend.unwrap_or_else(|| {
        let nested = ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()));
        if nested { "winit" } else { "tty" }.to_string()
    });

    let mut event_loop: EventLoop<'static, CalloopData> = EventLoop::try_new()?;

    let display: Display<Hypr> = Display::new()?;
    let display_handle = display.handle();
    let state = Hypr::new(&mut event_loop, display, config);

    let mut data = CalloopData {
        state,
        display_handle,
        tty: None,
    };

    match backend.as_str() {
        "winit" => crate::winit::init_winit(&mut event_loop, &mut data)?,
        "tty" => crate::tty::init_tty(&mut event_loop, &mut data)?,
        "headless" => crate::headless::init_headless(&mut event_loop, &mut data)?,
        other => {
            eprintln!("unknown backend `{other}` (expected winit, tty or headless)");
            std::process::exit(2);
        }
    }
    tracing::info!("backend: {backend}");
    tracing::info!("listening on WAYLAND_DISPLAY={:?}", data.state.socket_name);
    let wl_socket = data.state.socket_name.to_string_lossy().into_owned();
    data.state.ipc.set_instance_info(&backend, &wl_socket);
    reload::init(
        &event_loop.handle(),
        &mut data.state,
        loaded_from,
        &config_errors,
    );

    for cmd in data
        .state
        .config
        .exec_once
        .iter()
        .chain(&data.state.config.exec)
    {
        wm::spawn(cmd);
    }

    data.state.lua_loaded();
    data.state.lua_fire("hyprland.start", &[]);

    event_loop.run(None, &mut data, |data| data.state.lua_drain())?;

    Ok(())
}
