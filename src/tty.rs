//! TTY backend: DRM/KMS outputs, libinput input and a libseat session.
//!
//! Started from Smithay's `anvil` udev backend (MIT). One `DrmOutput` per
//! connected connector (each on its own CRTC), connectors hot-plugged through
//! udev, modes chosen by `monitor =` rules, xcursor cursor.
//! shortcut: one GPU (the primary one); other GPUs and DRM leasing are not
//! used. A mode change from a `monitor =` rule applies when the connector is
//! (re)connected, positions and scale apply at once.

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
                memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement,
            },
            gles::GlesRenderer,
            ImportDma,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        drm::control::{connector, crtc, Device as _, Mode as DrmMode, ModeTypeFlags},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::{backend::GlobalId, DisplayHandle},
    },
    utils::DeviceFd,
};

use crate::{config::MonMode, cursor::CursorElement, CalloopData, Hypr};

smithay::backend::renderer::element::render_elements! {
    pub TtyElement<=GlesRenderer>;
    Fx=crate::render::FxElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
}

type Allocator = GbmAllocator<DrmDeviceFd>;
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;

struct TtyOutput {
    output: Output,
    global: GlobalId,
    crtc: crtc::Handle,
    connector: connector::Handle,
    surface: DrmOutput<Allocator, Exporter, (), DrmDeviceFd>,
    /// A frame is queued and we are waiting for its vblank.
    frame_pending: bool,
    /// `dpms off` already cleared the CRTC.
    dpms_off: bool,
    /// A retry timer is armed (last render had no damage or failed).
    timer_armed: bool,
    /// Gamma ramps last written to the CRTC (`None`: identity).
    gamma: Option<[Vec<u16>; 3]>,
}

pub struct Tty {
    handle: LoopHandle<'static, CalloopData>,
    manager: DrmOutputManager<Allocator, Exporter, (), DrmDeviceFd>,
    renderer: GlesRenderer,
    outputs: Vec<TtyOutput>,

    /// False while another VT is active.
    active: bool,
}

/// Connector name as Hyprland shows it (`DP-1`, `HDMI-A-1`, `Virtual-1`).
fn connector_name(c: &connector::Info) -> String {
    format!("{}-{}", c.interface().as_str(), c.interface_id())
}

/// The DRM mode a `monitor =` rule asks for.
fn pick_mode(modes: &[DrmMode], want: &MonMode) -> Option<DrmMode> {
    let area = |m: &DrmMode| m.size().0 as u32 * m.size().1 as u32;
    let preferred = || {
        modes
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or(modes.first())
            .copied()
    };
    match want {
        MonMode::Preferred => preferred(),
        MonMode::HighRes => modes
            .iter()
            .max_by_key(|m| (area(m), m.vrefresh()))
            .copied(),
        MonMode::HighRr => modes
            .iter()
            .max_by_key(|m| (m.vrefresh(), area(m)))
            .copied(),
        MonMode::Exact { w, h, refresh } => modes
            .iter()
            .filter(|m| m.size() == (*w as u16, *h as u16))
            .min_by_key(|m| match refresh {
                Some(r) => ((m.vrefresh() as f64 - r).abs() * 1000.0) as i64,
                None => -(m.vrefresh() as i64),
            })
            .copied()
            .or_else(preferred),
    }
}

impl Tty {
    /// Sets up every connected connector that has no output yet and drops
    /// outputs whose connector went away.
    fn scan_connectors(&mut self, state: &mut Hypr, dh: &DisplayHandle) {
        let drm = self.manager.device();
        let Ok(res) = drm.resource_handles() else {
            return;
        };
        let infos: Vec<connector::Info> = res
            .connectors()
            .iter()
            .filter_map(|&c| drm.get_connector(c, true).ok())
            .collect();

        // Gone: disconnected connectors.
        let gone: Vec<usize> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| {
                !infos
                    .iter()
                    .any(|c| c.handle() == o.connector && c.state() == connector::State::Connected)
            })
            .map(|(i, _)| i)
            .collect();
        for i in gone.into_iter().rev() {
            let o = self.outputs.remove(i);
            tracing::info!("output {} disconnected", o.output.name());
            state.remove_output(&o.output.name());
            dh.remove_global::<Hypr>(o.global);
        }

        // New: connected connectors with modes.
        for conn in infos {
            if conn.state() != connector::State::Connected
                || conn.modes().is_empty()
                || self.outputs.iter().any(|o| o.connector == conn.handle())
            {
                continue;
            }
            let name = connector_name(&conn);
            let rule = state.config.monitor_rule(&name, "");
            if rule.disabled {
                tracing::info!("output {name} disabled by a monitor rule");
                continue;
            }
            let used: Vec<crtc::Handle> = self.outputs.iter().map(|o| o.crtc).collect();
            let drm = self.manager.device();
            let Some(crtc) = conn
                .encoders()
                .iter()
                .filter_map(|&e| drm.get_encoder(e).ok())
                .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
                .find(|c| !used.contains(c))
            else {
                tracing::warn!("output {name}: no free CRTC");
                continue;
            };
            let Some(drm_mode) = pick_mode(conn.modes(), &rule.mode) else {
                continue;
            };
            tracing::info!(
                "output {name}: {}x{}@{}",
                drm_mode.size().0,
                drm_mode.size().1,
                drm_mode.vrefresh()
            );
            let (phys_w, phys_h) = conn.size().unwrap_or((0, 0));
            let output = Output::new(
                name.clone(),
                PhysicalProperties {
                    size: (phys_w as i32, phys_h as i32).into(),
                    subpixel: Subpixel::Unknown,
                    make: "Unknown".into(),
                    model: "Unknown".into(),
                },
            );
            let global = output.create_global::<Hypr>(dh);
            let wl_mode = Mode::from(drm_mode);
            output.change_current_state(Some(wl_mode), None, None, None);
            output.set_preferred(wl_mode);
            state.add_output(&output);
            let surface = match self.manager.initialize_output::<_, TtyElement>(
                crtc,
                drm_mode,
                &[conn.handle()],
                &output,
                None,
                &mut self.renderer,
                &DrmOutputRenderElements::default(),
            ) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("cannot initialize output {name}: {e:?}");
                    state.remove_output(&name);
                    dh.remove_global::<Hypr>(global);
                    continue;
                }
            };
            self.outputs.push(TtyOutput {
                output,
                global,
                crtc,
                connector: conn.handle(),
                surface,
                frame_pending: false,
                dpms_off: false,
                timer_armed: false,
                gamma: None,
            });
        }
    }
}

pub fn init_tty(
    event_loop: &mut EventLoop<'static, CalloopData>,
    data: &mut CalloopData,
) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new()?;
    let seat = session.seat();

    // Like Hyprland (aquamarine): `AQ_DRM_DEVICES=/dev/dri/card1:...` picks
    // the GPU (first entry); `WLR_DRM_DEVICES` is honoured too.
    let forced = ["AQ_DRM_DEVICES", "WLR_DRM_DEVICES"]
        .iter()
        .find_map(|v| std::env::var(v).ok())
        .and_then(|s| s.split(':').next().map(std::path::PathBuf::from))
        .filter(|p| !p.as_os_str().is_empty());
    let gpu_path = match forced {
        Some(p) => p,
        None => primary_gpu(&seat)?
            .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
            .ok_or("no GPU found")?,
    };
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
    let renderer = unsafe { GlesRenderer::new(context)? };

    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), None);
    let manager = DrmOutputManager::new(
        drm,
        allocator,
        exporter,
        Some(gbm),
        [Fourcc::Argb8888, Fourcc::Abgr8888],
        render_formats,
    );

    let handle = event_loop.handle();
    let mut tty = Tty {
        handle: handle.clone(),
        manager,
        renderer,
        outputs: Vec::new(),
        active: true,
    };
    tty.scan_connectors(&mut data.state, &data.display_handle);
    if tty.outputs.is_empty() {
        return Err("no connected monitor found".into());
    }

    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| "libinput: cannot assign seat")?;

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
            DrmEvent::VBlank(crtc) => {
                let Some(tty) = data.tty.as_mut() else {
                    return;
                };
                let Some(o) = tty.outputs.iter_mut().find(|o| o.crtc == crtc) else {
                    return;
                };
                if let Err(e) = o.surface.frame_submitted() {
                    tracing::warn!("frame_submitted failed: {e:?}");
                }
                o.frame_pending = false;
                render(data, crtc);
            }
            DrmEvent::Error(e) => tracing::error!("drm error: {e:?}"),
        })
        .map_err(|e| e.error)?;

    // Monitor hotplug: rescan the connectors of our GPU.
    let udev = UdevBackend::new(&seat)?;
    handle
        .insert_source(udev, |event, _, data| {
            if let UdevEvent::Changed { .. } = event {
                if let Some(tty) = data.tty.as_mut() {
                    tty.scan_connectors(&mut data.state, &data.display_handle);
                }
                render_all(data);
            }
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
                    for o in &mut tty.outputs {
                        o.frame_pending = false;
                    }
                    render_all(data);
                }
            }
        })
        .map_err(|e| e.error)?;

    std::env::set_var("WAYLAND_DISPLAY", &data.state.socket_name);
    data.state.session = Some(session);
    data.state.init_dmabuf(tty.renderer.dmabuf_formats());
    data.tty = Some(tty);
    handle.insert_idle(render_all);
    Ok(())
}

fn render_all(data: &mut CalloopData) {
    let crtcs: Vec<crtc::Handle> = data
        .tty
        .as_ref()
        .map(|t| t.outputs.iter().map(|o| o.crtc).collect())
        .unwrap_or_default();
    for crtc in crtcs {
        render(data, crtc);
    }
}

fn render(data: &mut CalloopData, crtc: crtc::Handle) {
    let CalloopData {
        state,
        display_handle,
        tty,
    } = data;
    let Some(tty) = tty.as_mut() else {
        return;
    };
    let Some(idx) = tty.outputs.iter().position(|o| o.crtc == crtc) else {
        return;
    };
    if !tty.active || tty.outputs[idx].frame_pending {
        return;
    }
    let output = tty.outputs[idx].output.clone();
    let Some(geo) = state.space.output_geometry(&output) else {
        return;
    };
    let scale = output.current_scale().fractional_scale();

    state.screencopy_process(&mut tty.renderer, &output);

    // wlr-gamma-control / hyprland-ctm-control: DRM gamma ramps.
    let name = output.name();
    let want = crate::handlers::gamma_control::effective_ramps(
        state.extra.gamma.ramps.get(&name).map(|v| v.as_slice()),
        state.extra.ctm.applied.get(&name),
        crate::handlers::gamma_control::GAMMA_SIZE as usize,
    );
    if want != tty.outputs[idx].gamma {
        let identity: [Vec<u16>; 3] =
            std::array::from_fn(|_| (0..256u32).map(|i| (i * 65535 / 255) as u16).collect());
        let r = want.as_ref().unwrap_or(&identity);
        if let Err(e) = tty.manager.device().set_gamma(crtc, &r[0], &r[1], &r[2]) {
            tracing::warn!("gamma on {name}: {e}");
        }
        tty.outputs[idx].gamma = want;
    }

    // Front to back: cursor, borders, windows. Locked: cursor and lock surfaces.
    let mut elements: Vec<TtyElement> = Vec::new();
    let pointer = state.seat.get_pointer().unwrap().current_location();
    if geo.to_f64().contains(pointer) && state.cursor_visible() {
        let local = pointer - geo.loc.to_f64();
        elements.extend(
            state
                .cursors
                .elements(&mut tty.renderer, &state.cursor_status, local, scale)
                .into_iter()
                .map(|c| match c {
                    CursorElement::Image(e) => TtyElement::Cursor(e),
                    CursorElement::Surface(e) => TtyElement::Surface(e),
                }),
        );
    }
    if state.session_locked {
        let lock = state.lock_elements(&mut tty.renderer, &output);
        elements.extend(lock.into_iter().map(TtyElement::Surface));
    } else {
        let fx = crate::render::output_elements(state, &mut tty.renderer, &output);
        elements.extend(fx.into_iter().map(TtyElement::Fx));
    }

    // `dpms off`: a black frame.
    let out = &mut tty.outputs[idx];
    let rendered = if !state.wm.dpms_on {
        // `dpms off`: planes off and CRTC inactive; the next queued frame
        // turns it back on.
        if !out.dpms_off {
            if let Err(e) = out.surface.with_compositor(|c| c.clear()) {
                tracing::warn!("dpms off failed: {e:?}");
            }
            out.dpms_off = true;
            out.frame_pending = false;
        }
        false
    } else {
        match out.surface.render_frame(
            &mut tty.renderer,
            &elements,
            [0.1, 0.1, 0.1, 1.0],
            FrameFlags::DEFAULT,
        ) {
            // After `dpms off` an unchanged frame must still be queued.
            Ok(result) => !result.is_empty || out.dpms_off,
            Err(e) => {
                tracing::warn!("render_frame failed: {e:?}");
                false
            }
        }
    };
    out.dpms_off = !state.wm.dpms_on;
    if rendered {
        match out.surface.queue_frame(()) {
            Ok(()) => out.frame_pending = true,
            Err(e) => tracing::warn!("queue_frame failed: {e:?}"),
        }
    }
    // No vblank will come without a queued frame: poll again in ~one frame.
    if !out.frame_pending && !out.timer_armed {
        out.timer_armed = true;
        let timer = Timer::from_duration(Duration::from_millis(16));
        let _ = tty.handle.insert_source(timer, move |_, _, data| {
            if let Some(o) = data
                .tty
                .as_mut()
                .and_then(|t| t.outputs.iter_mut().find(|o| o.crtc == crtc))
            {
                o.timer_armed = false;
            }
            render(data, crtc);
            TimeoutAction::Drop
        });
    }

    state.send_frames(&output);
    state.space.refresh();
    state.popups.cleanup();
    let _ = display_handle.flush_clients();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_mode_falls_back_to_preferred() {
        // No real DRM modes can be built without a device; the selection
        // logic is exercised in CI on VKMS (tests/tty-vkms.sh).
        assert!(pick_mode(&[], &MonMode::Preferred).is_none());
    }
}
