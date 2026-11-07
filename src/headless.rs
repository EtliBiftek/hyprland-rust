//! Headless backend: virtual outputs, no rendering, no input. For CI tests.
//!
//! `HYPR_HEADLESS_OUTPUTS=N` (default 1) outputs named `HEADLESS-1`..., each
//! `HYPR_HEADLESS_SIZE=WxH` (default 1920x1080) unless a `monitor =` rule
//! gives an exact mode. More can be added with `hyprctl output create
//! headless [NAME]`.

use std::time::Duration;

use smithay::reexports::calloop::{
    timer::{TimeoutAction, Timer},
    EventLoop,
};

use crate::CalloopData;

/// `HYPR_HEADLESS_SIZE=WxH`, default 1920x1080.
pub fn size_from_env() -> (i32, i32) {
    std::env::var("HYPR_HEADLESS_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .filter(|&(w, h)| w > 0 && h > 0)
        .unwrap_or((1920, 1080))
}

pub fn init_headless(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = &mut data.state;
    let n: usize = std::env::var("HYPR_HEADLESS_OUTPUTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(1);
    for _ in 0..n {
        state.create_virtual_output(None)?;
    }
    state.init_dmabuf_fallback();

    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);

    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_millis(16)),
        move |_, _, data| {
            let state = &mut data.state;
            // shortcut: no renderer here, so screencopy cannot be served.
            state.screencopy_fail_all();
            let outputs: Vec<_> = state.space.outputs().cloned().collect();
            for output in &outputs {
                state.send_frames(output);
            }
            state.space.refresh();
            state.popups.cleanup();
            let _ = data.display_handle.flush_clients();
            TimeoutAction::ToDuration(Duration::from_millis(16))
        },
    )?;

    Ok(())
}
