//! wlr-output-management-unstable-v1, read-only: nwg-displays and wlr-randr can
//! list the outputs. Every `apply`/`test` is answered with `failed`.
//! shortcut: heads are announced once at bind (no events when outputs change)
//! and configuration is not applied; upgrade together with `monitor=` rules.

use smithay::{
    reexports::{
        wayland_protocols_wlr::output_management::v1::server::{
            zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
            zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
            zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
            zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
            zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
        },
        wayland_server::{
            protocol::wl_output::Transform, Client, DataInit, Dispatch, DisplayHandle,
            GlobalDispatch, New, Resource,
        },
    },
    utils::SERIAL_COUNTER,
};

use crate::Hypr;

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
            let Ok(head) =
                client.create_resource::<ZwlrOutputHeadV1, _, Hypr>(handle, mgr.version(), ())
            else {
                continue;
            };
            mgr.head(&head);
            head.name(output.name());
            head.description(output.description());
            let props = output.physical_properties();
            head.physical_size(props.size.w, props.size.h);
            let current = output.current_mode();
            for mode in output.modes() {
                let Ok(m) =
                    client.create_resource::<ZwlrOutputModeV1, _, Hypr>(handle, mgr.version(), ())
                else {
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
            head.transform(Transform::Normal);
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
                data_init.init(id, ());
            }
            zwlr_output_manager_v1::Request::Stop => resource.finished(),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, .. } => {
                data_init.init(id, ());
            }
            zwlr_output_configuration_v1::Request::Apply
            | zwlr_output_configuration_v1::Request::Test => resource.failed(),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputConfigurationHeadV1,
        _request: zwlr_output_configuration_head_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputHeadV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputHeadV1,
        _request: zwlr_output_head_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputModeV1,
        _request: zwlr_output_mode_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}
