use smithay::{
    backend::{
        renderer::{
            damage::OutputDamageTracker, element::surface::WaylandSurfaceRenderElement,
            gles::GlesRenderer, ImportDma,
        },
        winit::{self, WinitEvent},
    },
    desktop::{layer_map_for_output, Space, Window},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::calloop::EventLoop,
    utils::Transform,
};

use crate::{CalloopData, Hypr};

pub fn init_winit(
    event_loop: &mut EventLoop<CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let display_handle = &mut data.display_handle;
    let state = &mut data.state;

    let (mut backend, winit) = winit::init::<GlesRenderer>()?;

    let mode = Mode {
        size: backend.window_size(),
        refresh: 60_000,
    };

    let output = Output::new(
        "winit".to_string(),
        PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "hyprland-rust".into(),
            model: "Winit".into(),
        },
    );
    let _global = output.create_global::<Hypr>(display_handle);
    output.change_current_state(
        Some(mode),
        Some(Transform::Flipped180),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(mode);

    state.add_output(&output);

    let mut damage_tracker = OutputDamageTracker::from_output(&output);
    state.init_dmabuf(backend.renderer().dmabuf_formats());

    std::env::set_var("WAYLAND_DISPLAY", &state.socket_name);

    event_loop
        .handle()
        .insert_source(winit, move |event, _, data| {
            let display = &mut data.display_handle;
            let state = &mut data.state;

            match event {
                WinitEvent::Resized { size, .. } => {
                    output.change_current_state(
                        Some(Mode {
                            size,
                            refresh: 60_000,
                        }),
                        None,
                        None,
                        None,
                    );
                    layer_map_for_output(&output).arrange();
                    state.relayout();
                }
                WinitEvent::Input(event) => state.process_input_event(event),
                WinitEvent::Redraw => {
                    let elements = if state.session_locked {
                        Vec::new()
                    } else {
                        crate::render::output_elements(state, backend.renderer(), &output)
                    };
                    let age = backend.buffer_age().unwrap_or(0);
                    let result = {
                        let (renderer, mut framebuffer) = backend.bind().unwrap();
                        if state.session_locked {
                            // Locked: only the lock surfaces, no windows or borders.
                            let lock = state.lock_elements(renderer);
                            smithay::desktop::space::render_output::<
                                _,
                                WaylandSurfaceRenderElement<GlesRenderer>,
                                _,
                                _,
                            >(
                                &output,
                                renderer,
                                &mut framebuffer,
                                1.0,
                                age,
                                std::iter::empty::<&Space<Window>>(),
                                &lock,
                                &mut damage_tracker,
                                [0.1, 0.1, 0.1, 1.0],
                            )
                            .unwrap()
                        } else {
                            damage_tracker
                                .render_output(
                                    renderer,
                                    &mut framebuffer,
                                    age,
                                    &elements,
                                    [0.1, 0.1, 0.1, 1.0],
                                )
                                .unwrap()
                        }
                    };
                    // Nothing changed: skip the swap.
                    if let Some(damage) = result.damage {
                        backend.submit(Some(damage)).unwrap();
                    }

                    state.screencopy_process(backend.renderer(), &output);
                    state.send_frames(&output);

                    state.space.refresh();
                    state.popups.cleanup();
                    let _ = display.flush_clients();

                    // Ask for redraw to schedule new frame.
                    backend.window().request_redraw();
                }
                WinitEvent::CloseRequested => {
                    state.loop_signal.stop();
                }
                _ => (),
            };
        })?;

    Ok(())
}
