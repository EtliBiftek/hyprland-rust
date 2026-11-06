//! Headless backend: one virtual output, no rendering, no input. For CI tests.

use std::time::Duration;

use smithay::{
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::{
        timer::{TimeoutAction, Timer},
        EventLoop,
    },
    utils::Transform,
};

use crate::{CalloopData, Hypr};

/// `HYPR_HEADLESS_SIZE=WxH`, default 1920x1080.
fn size_from_env() -> (i32, i32) {
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

    let mode = Mode {
        size: size_from_env().into(),
        refresh: 60_000,
    };
    let output = Output::new(
        "HEADLESS-1".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "hyprland-rust".into(),
            model: "Headless".into(),
        },
    );
    let _global = output.create_global::<Hypr>(&data.display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
    state.space.map_output(&output, (0, 0));
    state.relayout();

    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);

    event_loop.handle().insert_source(
        Timer::from_duration(Duration::from_millis(16)),
        move |_, _, data| {
            let state = &mut data.state;
            state.space.elements().for_each(|window| {
                window.send_frame(
                    &output,
                    state.start_time.elapsed(),
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                )
            });
            state.space.refresh();
            state.popups.cleanup();
            let _ = data.display_handle.flush_clients();
            TimeoutAction::ToDuration(Duration::from_millis(16))
        },
    )?;

    Ok(())
}
