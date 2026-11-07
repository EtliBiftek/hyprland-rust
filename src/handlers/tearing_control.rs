//! wp-tearing-control-v1: games hint that async page flips are fine.
//! shortcuts: the hint is accepted and ignored (no `general:allow_tearing`
//! handling, no async flips on the TTY backend yet).

use smithay::reexports::{
    wayland_protocols::wp::tearing_control::v1::server::{
        wp_tearing_control_manager_v1::{self, WpTearingControlManagerV1},
        wp_tearing_control_v1::{self, WpTearingControlV1},
    },
    wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New},
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, WpTearingControlManagerV1, _>(1, ());
}

impl GlobalDispatch<WpTearingControlManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<WpTearingControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<WpTearingControlManagerV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WpTearingControlManagerV1,
        request: wp_tearing_control_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let wp_tearing_control_manager_v1::Request::GetTearingControl { id, .. } = request {
            data_init.init(id, ());
        }
    }
}

impl Dispatch<WpTearingControlV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WpTearingControlV1,
        _request: wp_tearing_control_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // `set_presentation_hint` and `destroy`.
    }
}
