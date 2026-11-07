//! hyprland-ctm-control-v1 (hyprsunset): per-output 3x3 colour transform matrix.
//! shortcuts: matrices are validated and stored (`CtmState::applied`, keyed by
//! output name) but not applied to the screen; the TTY backend would write the
//! DRM `CTM` property (or a render shader) from this map.

use std::{collections::HashMap, sync::Mutex};

use smithay::{
    output::Output,
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
};

use super::hyprland_protocols::ctm_control::server::hyprland_ctm_control_manager_v1::{
    self, HyprlandCtmControlManagerV1,
};
use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandCtmControlManagerV1, _>(2, ());
}

#[derive(Default)]
pub struct CtmState {
    /// Committed matrix per output name (row-major, as sent).
    #[allow(dead_code)]
    pub applied: HashMap<String, [f64; 9]>,
}

/// Matrices set since the last `commit`, per bound manager.
#[derive(Default)]
pub struct CtmPending(Mutex<Vec<(String, [f64; 9])>>);

impl GlobalDispatch<HyprlandCtmControlManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandCtmControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, CtmPending::default());
    }
}

impl Dispatch<HyprlandCtmControlManagerV1, CtmPending> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &HyprlandCtmControlManagerV1,
        request: hyprland_ctm_control_manager_v1::Request,
        data: &CtmPending,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use hyprland_ctm_control_manager_v1::Request as R;
        match request {
            R::SetCtmForOutput {
                output,
                mat0,
                mat1,
                mat2,
                mat3,
                mat4,
                mat5,
                mat6,
                mat7,
                mat8,
            } => {
                let m = [mat0, mat1, mat2, mat3, mat4, mat5, mat6, mat7, mat8];
                if m.iter().any(|v| *v < 0.0) {
                    resource.post_error(
                        hyprland_ctm_control_manager_v1::Error::InvalidMatrix,
                        "matrix values must not be negative",
                    );
                    return;
                }
                if let Some(o) = Output::from_resource(&output) {
                    let mut p = data.0.lock().unwrap();
                    p.retain(|(n, _)| *n != o.name());
                    p.push((o.name(), m));
                }
            }
            R::Commit => {
                for (name, m) in data.0.lock().unwrap().drain(..) {
                    state.extra.ctm.applied.insert(name, m);
                }
            }
            _ => {}
        }
    }
}
