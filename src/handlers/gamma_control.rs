//! wlr-gamma-control-unstable-v1 (gammastep, wlsunset, hyprsunset fallback).
//! The client uploads three 16-bit ramps; they are validated and kept per output
//! in `GammaState::ramps` (restored, i.e. removed, when the control is destroyed).
//! The tty backend writes them (combined with the CTM) as DRM gamma ramps.
//! shortcuts: winit and headless do not apply them (winit has no
//! gamma), and the CRTC gamma size is fixed at 256.

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
pub const GAMMA_SIZE: u32 = 256;

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
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_ne_bytes(*c))
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

/// The ramps an output should show: the client's gamma ramps (or identity)
/// scaled by the diagonal of the CTM (hyprsunset's colour temperature).
/// `None`: nothing set, restore the identity ramps.
// shortcut: off-diagonal CTM entries are ignored (no DRM `CTM` property or
// shader yet); hyprsunset and night-light tools only use the diagonal.
pub fn effective_ramps(
    gamma: Option<&[u16]>,
    ctm: Option<&[f64; 9]>,
    size: usize,
) -> Option<[Vec<u16>; 3]> {
    if gamma.is_none() && ctm.is_none() {
        return None;
    }
    let identity = |i: usize| ((i * 65535) / (size - 1).max(1)) as u16;
    let base = |c: usize, i: usize| match gamma {
        Some(g) if g.len() == 3 * size => g[c * size + i],
        _ => identity(i),
    };
    let scale = |c: usize| ctm.map_or(1.0, |m| m[c * 4].clamp(0.0, 1.0));
    Some(std::array::from_fn(|c| {
        (0..size)
            .map(|i| (base(c, i) as f64 * scale(c)).round() as u16)
            .collect()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_combine_gamma_and_ctm() {
        assert!(effective_ramps(None, None, 256).is_none());
        let warm = [1.0, 0.0, 0.0, 0.0, 0.8, 0.0, 0.0, 0.0, 0.5];
        let r = effective_ramps(None, Some(&warm), 256).unwrap();
        assert_eq!((r[0][255], r[1][255], r[2][255]), (65535, 52428, 32768));
        assert_eq!(r[0][0], 0);
        let g: Vec<u16> = (0..768).map(|_| 1000).collect();
        let r = effective_ramps(Some(&g), Some(&warm), 256).unwrap();
        assert_eq!((r[0][10], r[2][10]), (1000, 500));
    }
}
