//! xdg-dialog-v1: a toplevel can be marked as a (modal) dialog.
//! shortcuts: `set_modal` / `unset_modal` are accepted and ignored (no modal
//! focus lock, no auto-float for dialogs beyond what window rules do).

use smithay::reexports::{
    wayland_protocols::xdg::dialog::v1::server::{
        xdg_dialog_v1::{self, XdgDialogV1},
        xdg_wm_dialog_v1::{self, XdgWmDialogV1},
    },
    wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New},
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, XdgWmDialogV1, _>(1, ());
}

impl GlobalDispatch<XdgWmDialogV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<XdgWmDialogV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<XdgWmDialogV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &XdgWmDialogV1,
        request: xdg_wm_dialog_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let xdg_wm_dialog_v1::Request::GetXdgDialog { id, .. } = request {
            data_init.init(id, ());
        }
    }
}

impl Dispatch<XdgDialogV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &XdgDialogV1,
        _request: xdg_dialog_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}
