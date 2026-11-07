//! TTY backend: DRM/KMS output, libinput input and a libseat session.
//!
//! A simplified version of Smithay's `anvil` udev backend (MIT).
//! shortcut: single GPU, first connected monitor only, no hotplug and a square
//! cursor; upgrade with udev events + one DrmOutput per CRTC for multi-monitor.

use std::time::Duration;

use smithay::{
    backend::{
        allocator::{
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc,
        },
        drm::{
            compositor::FrameFlags,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            DrmDevice, DrmDeviceFd, DrmEvent,
        },
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::{
                solid::{SolidColorBuffer, SolidColorRenderElement},
                surface::WaylandSurfaceRenderElement,
                Kind,
            },
            gles::GlesRenderer,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        drm::control::{connector, crtc, Device as _, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
    },
    utils::DeviceFd,
};

use crate::{CalloopData, Hypr};

smithay::backend::renderer::element::render_elements! {
    pub TtyElement<=GlesRenderer>;
    Fx=crate::render::FxElement,
    Solid=SolidColorRenderElement,
    Lock=WaylandSurfaceRenderElement<GlesRenderer>,
}

type Allocator = GbmAllocator<DrmDeviceFd>;
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;

pub struct Tty {
    handle: LoopHandle<'static, CalloopData>,
    manager: DrmOutputManager<Allocator, Exporter, (), DrmDeviceFd>,
    surface: DrmOutput<Allocator, Exporter, (), DrmDeviceFd>,
    renderer: GlesRenderer,
    output: Output,
    /// Persistent cursor buffer so the cursor keeps its element id.
    cursor: SolidColorBuffer,
    /// False while another VT is active.
    active: bool,
    /// A frame is queued and we are waiting for its vblank.
    frame_pending: bool,
    /// A retry timer is armed (last render had no damage or failed).
    timer_armed: bool,
}

/// First connected connector that has modes, plus a CRTC that can drive it.
fn find_connector(drm: &DrmDevice) -> Option<(connector::Info, crtc::Handle)> {
    let res = drm.resource_handles().ok()?;
    res.connectors()
        .iter()
        .filter_map(|&c| drm.get_connector(c, false).ok())
        .filter(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
        .find_map(|c| {
            let crtc = c
                .encoders()
                .iter()
                .filter_map(|&e| drm.get_encoder(e).ok())
                .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                .next()?;
            Some((c, crtc))
        })
}

pub fn init_tty(
    event_loop: &mut EventLoop<'static, CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new()?;
    let seat = session.seat();

    let gpu_path = primary_gpu(&seat)?
        .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
        .ok_or("no GPU found")?;
    tracing::info!("using GPU {}", gpu_path.display());

    let fd = session.open(
        &gpu_path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;
    // Safety: the GBM device outlives the display (both live until exit).
    let egl = unsafe { EGLDisplay::new(gbm.clone())? };
    let context = EGLContext::new(&egl)?;
    let render_formats = context.dmabuf_render_formats().clone();
    // Safety: the context is only used by this renderer.
    let mut renderer = unsafe { GlesRenderer::new(context)? };

    let (conn, crtc) = find_connector(&drm).ok_or("no connected monitor found")?;
    let drm_mode = conn
        .modes()
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .copied()
        .unwrap_or(conn.modes()[0]);
    let name = format!("{}-{}", conn.interface().as_str(), conn.interface_id());
    tracing::info!("output {name}: {}x{}", drm_mode.size().0, drm_mode.size().1);

    let (phys_w, phys_h) = conn.size().unwrap_or((0, 0));
    let output = Output::new(
        name,
        PhysicalProperties {
            size: (phys_w as i32, phys_h as i32).into(),
            subpixel: Subpixel::Unknown,
            make: "Unknown".into(),
            model: "Unknown".into(),
        },
    );
    let wl_mode = Mode::from(drm_mode);
    let _global = output.create_global::<Hypr>(&data.display_handle);
    output.change_current_state(Some(wl_mode), None, None, Some((0, 0).into()));
    output.set_preferred(wl_mode);
    data.state.space.map_output(&output, (0, 0));

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let mut manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        [Fourcc::Argb8888, Fourcc::Abgr8888],
        render_formats,
    );
    let surface = manager
        .initialize_output::<_, TtyElement>(
            crtc,
            drm_mode,
            &[conn.handle()],
            &output,
            None,
            &mut renderer,
            &DrmOutputRenderElements::default(),
        )
        .map_err(|e| format!("cannot initialize output: {e:?}"))?;

    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| "libinput: cannot assign seat")?;

    let handle = event_loop.handle();
    handle
        .insert_source(
            LibinputInputBackend::new(libinput.clone()),
            |event, _, data| {
                data.state.libinput_device_event(&event);
                data.state.process_input_event(event)
            },
        )
        .map_err(|e| e.error)?;

    handle
        .insert_source(drm_notifier, |event, _, data| match event {
            DrmEvent::VBlank(_) => {
                if let Some(tty) = data.tty.as_mut() {
                    if let Err(e) = tty.surface.frame_submitted() {
                        tracing::warn!("frame_submitted failed: {e:?}");
                    }
                    tty.frame_pending = false;
                }
                render(data);
            }
            DrmEvent::Error(e) => tracing::error!("drm error: {e:?}"),
        })
        .map_err(|e| e.error)?;

    handle
        .insert_source(notifier, move |event, _, data| {
            let Some(tty) = data.tty.as_mut() else {
                return;
            };
            match event {
                SessionEvent::PauseSession => {
                    tracing::info!("session paused");
                    libinput.suspend();
                    tty.manager.pause();
                    tty.active = false;
                }
                SessionEvent::ActivateSession => {
                    tracing::info!("session resumed");
                    if let Err(e) = libinput.resume() {
                        tracing::error!("libinput resume failed: {e:?}");
                    }
                    if let Err(e) = tty.manager.activate(false) {
                        tracing::error!("drm activate failed: {e}");
                    }
                    tty.active = true;
                    tty.frame_pending = false;
                    render(data);
                }
            }
        })
        .map_err(|e| e.error)?;

    std::env::set_var("WAYLAND_DISPLAY", &data.state.socket_name);
    data.state.session = Some(session);
    data.tty = Some(Tty {
        handle: handle.clone(),
        manager,
        surface,
        renderer,
        output,
        cursor: SolidColorBuffer::new((8, 8), [1.0, 1.0, 1.0, 1.0]),
        active: true,
        frame_pending: false,
        timer_armed: false,
    });
    handle.insert_idle(render);
    Ok(())
}

fn render(data: &mut CalloopData) {
    let CalloopData {
        state,
        display_handle,
        tty,
    } = data;
    let Some(tty) = tty.as_mut() else {
        return;
    };
    if !tty.active || tty.frame_pending {
        return;
    }

    state.screencopy_process(&mut tty.renderer, &tty.output);

    // Front to back: cursor, borders, windows. Locked: cursor and lock surfaces.
    let pointer = state.seat.get_pointer().unwrap().current_location();
    let mut elements = vec![TtyElement::Solid(SolidColorRenderElement::from_buffer(
        &tty.cursor,
        pointer.to_i32_round::<i32>().to_physical(1),
        1.0,
        1.0,
        Kind::Unspecified,
    ))];
    if state.session_locked {
        let lock = state.lock_elements(&mut tty.renderer);
        elements.extend(lock.into_iter().map(TtyElement::Lock));
    } else {
        let fx = crate::render::output_elements(state, &mut tty.renderer, &tty.output);
        elements.extend(fx.into_iter().map(TtyElement::Fx));
    }

    let rendered = match tty.surface.render_frame(
        &mut tty.renderer,
        &elements,
        [0.1, 0.1, 0.1, 1.0],
        FrameFlags::DEFAULT,
    ) {
        Ok(result) => !result.is_empty,
        Err(e) => {
            tracing::warn!("render_frame failed: {e:?}");
            false
        }
    };
    if rendered {
        match tty.surface.queue_frame(()) {
            Ok(()) => tty.frame_pending = true,
            Err(e) => tracing::warn!("queue_frame failed: {e:?}"),
        }
    }
    // No vblank will come without a queued frame: poll again in ~one frame.
    if !tty.frame_pending && !tty.timer_armed {
        tty.timer_armed = true;
        let timer = Timer::from_duration(Duration::from_millis(16));
        let _ = tty.handle.insert_source(timer, |_, _, data| {
            if let Some(tty) = data.tty.as_mut() {
                tty.timer_armed = false;
            }
            render(data);
            TimeoutAction::Drop
        });
    }

    state.send_frames(&tty.output);
    state.space.refresh();
    state.popups.cleanup();
    let _ = display_handle.flush_clients();
}
