//! wlr-output-management-unstable-v1: nwg-displays and wlr-randr can list the
//! outputs and change position, scale and transform.
//! shortcuts: heads are announced once at bind (no events when outputs change);
//! other modes and disabling an output answer `failed`; apply mode changes once
//! the backends can switch modes at runtime.

use std::sync::Mutex;

use smithay::{
    output::{Mode, Output},
    reexports::{
        wayland_protocols_wlr::output_management::v1::server::{
            zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
            zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
            zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
            zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
            zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
        },
    },
    utils::SERIAL_COUNTER,
};

use crate::{config::MonPos, Hypr};

/// What one `zwlr_output_configuration_head_v1` asks for.
pub struct Pending {
    output: Output,
    position: Option<(i32, i32)>,
    scale: Option<f64>,
    transform: Option<u8>,
    unsupported: bool,
}

#[derive(Default)]
pub struct Configuration {
    heads: Vec<ZwlrOutputConfigurationHeadV1>,
    unsupported: bool,
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, ZwlrOutputManagerV1, _>(4, ());
}

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for Hypr {
    fn bind(
        state: &mut Self,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let mgr = data_init.init(resource, ());
        for output in state.space.outputs() {
            let Ok(head) = client.create_resource::<ZwlrOutputHeadV1, _, Hypr>(
                handle,
                mgr.version(),
                output.clone(),
            ) else {
                continue;
            };
            mgr.head(&head);
            head.name(output.name());
            head.description(output.description());
            let props = output.physical_properties();
            head.physical_size(props.size.w, props.size.h);
            let current = output.current_mode();
            for mode in output.modes() {
                let Ok(m) = client.create_resource::<ZwlrOutputModeV1, _, Hypr>(
                    handle,
                    mgr.version(),
                    mode,
                ) else {
                    continue;
                };
                head.mode(&m);
                m.size(mode.size.w, mode.size.h);
                m.refresh(mode.refresh);
                if output.preferred_mode() == Some(mode) {
                    m.preferred();
                }
                if current == Some(mode) {
                    head.current_mode(&m);
                }
            }
            head.enabled(1);
            let loc = output.current_location();
            head.position(loc.x, loc.y);
            head.transform(output.current_transform().into());
            head.scale(output.current_scale().fractional_scale());
            if head.version() >= 2 {
                head.make(props.make);
                head.model(props.model);
            }
        }
        mgr.done(u32::from(SERIAL_COUNTER.next_serial()));
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, .. } => {
                data_init.init(id, Mutex::new(Configuration::default()));
            }
            zwlr_output_manager_v1::Request::Stop => resource.finished(),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, Mutex<Configuration>> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        data: &Mutex<Configuration>,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_configuration_v1::Request;
        let mut cfg = data.lock().unwrap();
        match request {
            Request::EnableHead { id, head } => {
                let Some(output) = head.data::<Output>().cloned() else {
                    cfg.unsupported = true;
                    return;
                };
                let h = data_init.init(
                    id,
                    Mutex::new(Pending {
                        output,
                        position: None,
                        scale: None,
                        transform: None,
                        unsupported: false,
                    }),
                );
                cfg.heads.push(h);
            }
            Request::DisableHead { .. } => cfg.unsupported = true,
            Request::Apply | Request::Test => {
                let apply = matches!(request, Request::Apply);
                let pending: Vec<_> = cfg
                    .heads
                    .iter()
                    .filter_map(|h| h.data::<Mutex<Pending>>())
                    .collect();
                if cfg.unsupported || pending.iter().any(|p| p.lock().unwrap().unsupported) {
                    resource.failed();
                    return;
                }
                if apply {
                    for p in pending {
                        state.apply_output_change(&p.lock().unwrap());
                    }
                    state.relayout();
                }
                resource.succeeded();
            }
            _ => {}
        }
    }
}

impl Hypr {
    /// Keeps the change in the monitor rules, so a later relayout or `keyword`
    /// does not undo it (a config reload does).
    fn apply_output_change(&mut self, p: &Pending) {
        let (name, desc) = (p.output.name(), p.output.description());
        let mut rule = self.config.monitor_rule(&name, &desc);
        rule.name = name.clone();
        let (x, y) = p.position.unwrap_or_else(|| {
            let loc = p.output.current_location();
            (loc.x, loc.y)
        });
        rule.position = MonPos::Exact(x, y);
        rule.scale = p.scale.or(rule.scale);
        rule.transform = p.transform.unwrap_or(rule.transform);
        self.config.monitors.retain(|r| r.name != name);
        self.config.monitors.push(rule.clone());
        self.apply_monitor_rule(&p.output, &rule);
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, Mutex<Pending>> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        data: &Mutex<Pending>,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_output_configuration_head_v1::Request;
        let mut p = data.lock().unwrap();
        match request {
            Request::SetMode { mode } => {
                if mode.data::<Mode>().copied() != p.output.current_mode() {
                    p.unsupported = true;
                }
            }
            Request::SetCustomMode { .. } => p.unsupported = true,
            Request::SetPosition { x, y } => p.position = Some((x, y)),
            Request::SetScale { scale } => p.scale = Some(scale),
            Request::SetTransform {
                transform: WEnum::Value(t),
            } => p.transform = Some(t as u32 as u8),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputHeadV1, Output> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputHeadV1,
        _request: zwlr_output_head_v1::Request,
        _data: &Output,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, Mode> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputModeV1,
        _request: zwlr_output_mode_v1::Request,
        _data: &Mode,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}
