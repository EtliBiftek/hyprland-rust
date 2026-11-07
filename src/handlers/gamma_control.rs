//! wlr-gamma-control-unstable-v1 (gammastep, wlsunset, hyprsunset fallback).
//! The client uploads three 16-bit ramps; they are validated and kept per output
//! in `GammaState::ramps` (restored, i.e. removed, when the control is destroyed).
//! shortcuts: the ramps are not applied to the screen on any backend (winit has no
//! gamma, the TTY backend would write the CRTC `GAMMA_LUT` property from this map).

use std::{collections::HashMap, fs::File, os::unix::fs::FileExt};

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols_wlr::gamma_control::v1::server::{
            zwlr_gamma_control_manager_v1::{self, ZwlrGammaControlManagerV1},
            zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
        },
        wayland_server::{
            backend::ClientId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
            Resource,
        },
    },
};

use crate::Hypr;

/// Entries per channel. gammastep and wlsunset accept any size the compositor reports.
const GAMMA_SIZE: u32 = 256;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, ZwlrGammaControlManagerV1, _>(1, ());
}

#[derive(Default)]
pub struct GammaState {
    /// Output name -> control that owns it.
    owners: Vec<(String, ZwlrGammaControlV1)>,
    /// Output name -> `3 * GAMMA_SIZE` ramp values (red, green, blue).
    #[allow(dead_code)]
    pub ramps: HashMap<String, Vec<u16>>,
}

/// `None` when the output was invalid or already controlled (the object only got `failed`).
pub struct GammaData(Option<String>);

impl GlobalDispatch<ZwlrGammaControlManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrGammaControlManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwlrGammaControlManagerV1,
        request: zwlr_gamma_control_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_gamma_control_manager_v1::Request::GetGammaControl { id, output } = request {
            let name = Output::from_resource(&output).map(|o| o.name());
            let g = &mut state.extra.gamma;
            g.owners.retain(|(_, c)| c.is_alive());
            let free = name
                .filter(|n| !g.owners.iter().any(|(o, _)| o == n))
                .filter(|n| state.space.outputs().any(|o| &o.name() == n));
            match free {
                Some(n) => {
                    let c = data_init.init(id, GammaData(Some(n.clone())));
                    c.gamma_size(GAMMA_SIZE);
                    g.owners.push((n, c));
                }
                None => data_init.init(id, GammaData(None)).failed(),
            }
        }
    }
}

impl Dispatch<ZwlrGammaControlV1, GammaData> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrGammaControlV1,
        request: zwlr_gamma_control_v1::Request,
        data: &GammaData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let zwlr_gamma_control_v1::Request::SetGamma { fd } = request else {
            return;
        };
        let Some(name) = &data.0 else {
            return;
        };
        let mut buf = vec![0u8; GAMMA_SIZE as usize * 3 * 2];
        let file = File::from(fd);
        // The client may have left the file offset at the end, so read at 0.
        // A file longer or shorter than the ramps is an `invalid_gamma` error.
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len != buf.len() as u64 || file.read_exact_at(&mut buf, 0).is_err() {
            resource.post_error(
                zwlr_gamma_control_v1::Error::InvalidGamma,
                "gamma ramps have the wrong size",
            );
            return;
        }
        let ramp = buf
            .chunks_exact(2)
            .map(|c| u16::from_ne_bytes([c[0], c[1]]))
            .collect();
        state.extra.gamma.ramps.insert(name.clone(), ramp);
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        resource: &ZwlrGammaControlV1,
        data: &GammaData,
    ) {
        if let Some(name) = &data.0 {
            state.extra.gamma.owners.retain(|(_, c)| c != resource);
            state.extra.gamma.ramps.remove(name);
        }
    }
}
