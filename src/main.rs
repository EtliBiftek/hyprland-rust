//! hyprland-rust: a Hyprland-inspired dynamic tiling Wayland compositor in Rust.
//!
//! The compositor core is based on Smithay's `smallvil` example (MIT).

#![allow(irrefutable_let_patterns)]

mod config;
mod grabs;
mod handlers;
mod input;
mod layout;
mod state;
mod winit;
mod wm;
mod workspace;

use std::path::PathBuf;

use smithay::reexports::{
    calloop::EventLoop,
    wayland_server::{Display, DisplayHandle},
};
pub use state::Hypr;

use crate::config::Config;

pub struct CalloopData {
    state: Hypr,
    display_handle: DisplayHandle,
}

const USAGE: &str = "\
Usage: hyprland-rust [OPTIONS]

Options:
  -c, --config <FILE>    Config file (default: $XDG_CONFIG_HOME/hypr/hyprland.conf)
      --check-config     Parse the config, print errors and exit
  -h, --help             Show this help
";

fn load_config(path: Option<PathBuf>) -> (Config, usize) {
    let path = path.or_else(Config::default_path);
    let Some(path) = path.filter(|p| p.exists()) else {
        tracing::info!("no config file found, using built-in defaults");
        return (Config::with_default_binds(), 0);
    };
    match Config::load(&path) {
        Ok((config, errors)) => {
            for e in &errors {
                tracing::warn!("config error: {e}");
            }
            tracing::info!("loaded config from {}", path.display());
            (config, errors.len())
        }
        Err(e) => {
            tracing::error!("cannot read {}: {e}; using defaults", path.display());
            (Config::with_default_binds(), 1)
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
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => config_path = args.next().map(PathBuf::from),
            "--check-config" => check_only = true,
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

    let (config, error_count) = load_config(config_path);
    if check_only {
        println!(
            "{} binds, {} config errors",
            config.binds.len(),
            error_count
        );
        std::process::exit(if error_count == 0 { 0 } else { 1 });
    }

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;

    let display: Display<Hypr> = Display::new()?;
    let display_handle = display.handle();
    let state = Hypr::new(&mut event_loop, display, config);

    let mut data = CalloopData {
        state,
        display_handle,
    };

    crate::winit::init_winit(&mut event_loop, &mut data)?;
    tracing::info!("listening on WAYLAND_DISPLAY={:?}", data.state.socket_name);

    for cmd in data
        .state
        .config
        .exec_once
        .iter()
        .chain(&data.state.config.exec)
    {
        wm::spawn(cmd);
    }

    event_loop.run(None, &mut data, move |_| {})?;

    Ok(())
}
