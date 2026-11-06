//! wlr-layer-shell (bars, launchers, wallpapers) and xdg-decoration.

use std::time::Duration;

use smithay::{
    delegate_layer_shell, delegate_xdg_decoration,
    desktop::{layer_map_for_output, LayerSurface, PopupKind, WindowSurfaceType},
    output::Output,
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        wayland_server::{
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        shell::{
            wlr_layer::{
                KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface,
                LayerSurfaceCachedState, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{decoration::XdgDecorationHandler, PopupSurface, ToplevelSurface},
        },
    },
};

use crate::Hypr;

impl WlrLayerShellHandler for Hypr {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let Some(output) = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.space.outputs().next().cloned())
        else {
            surface.send_close();
            return;
        };
        let layer = LayerSurface::new(surface, namespace.clone());
        // shortcut: layer rules are parsed and matched but have no effect yet.
        for (k, v) in crate::rules::layer_effects(&self.config.rules.layer, &namespace) {
            tracing::debug!("layer `{namespace}`: rule {k} {v}");
        }
        if layer_map_for_output(&output).map_layer(&layer).is_ok() {
            self.ipc.emit("openlayer", &namespace);
        }
    }

    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: PopupSurface) {
        // shortcut: layer popups are not constrained to the output.
        let _ = self.popups.track_popup(PopupKind::Xdg(popup));
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let found = self.space.outputs().find_map(|o| {
            let layer = layer_map_for_output(o)
                .layers()
                .find(|l| l.layer_surface() == &surface)
                .cloned()?;
            Some((o.clone(), layer))
        });
        let Some((output, layer)) = found else {
            return;
        };
        layer_map_for_output(&output).unmap_layer(&layer);
        self.ipc.emit("closelayer", layer.namespace());
        self.relayout();
        self.refresh_layer_focus();
    }
}
delegate_layer_shell!(Hypr);

impl XdgDecorationHandler for Hypr {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        force_server_side(&toplevel);
    }
    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        force_server_side(&toplevel);
    }
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        force_server_side(&toplevel);
    }
}
delegate_xdg_decoration!(Hypr);

/// Hyprland draws no titlebars: always ask clients to drop theirs.
fn force_server_side(toplevel: &ToplevelSurface) {
    toplevel.with_pending_state(|s| s.decoration_mode = Some(Mode::ServerSide));
    if toplevel.is_initial_configure_sent() {
        toplevel.send_pending_configure();
    }
}

impl Hypr {
    /// Handles a commit of a layer surface root: arrange, initial configure and
    /// relayout of tiled windows when the exclusive zones changed.
    pub fn layer_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self
            .space
            .outputs()
            .find(|o| {
                layer_map_for_output(o)
                    .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                    .is_some()
            })
            .cloned()
        else {
            return;
        };
        let (before, after) = {
            let mut map = layer_map_for_output(&output);
            let before = map.non_exclusive_zone();
            map.arrange();
            let sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<LayerSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            });
            if !sent {
                if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                    layer.layer_surface().send_configure();
                }
            }
            (before, map.non_exclusive_zone())
        };
        if before != after {
            self.relayout();
        }
        self.refresh_layer_focus();
    }

    fn layer_mapped(&self, surface: &WlSurface) -> bool {
        self.space.outputs().any(|o| {
            layer_map_for_output(o)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
    }

    /// Topmost mapped top/overlay layer asking for exclusive keyboard focus.
    pub fn exclusive_layer(&self) -> Option<WlSurface> {
        self.layer_shell_state.layer_surfaces().rev().find_map(|l| {
            let d = with_states(l.wl_surface(), |s| {
                *s.cached_state.get::<LayerSurfaceCachedState>().current()
            });
            (l.alive()
                && d.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && matches!(d.layer, Layer::Top | Layer::Overlay)
                && self.layer_mapped(l.wl_surface()))
            .then(|| l.wl_surface().clone())
        })
    }

    /// Gives focus to an exclusive layer, or hands it back to the last window
    /// once the layer that held it is gone.
    pub fn refresh_layer_focus(&mut self) {
        // Locked: keyboard focus stays on the lock surface.
        if self.session_locked {
            return;
        }
        let keyboard = self.seat.get_keyboard().unwrap();
        if let Some(s) = self.exclusive_layer() {
            if keyboard.current_focus().as_ref() != Some(&s) {
                self.layer_focus = Some(s.clone());
                keyboard.set_focus(self, Some(s), SERIAL_COUNTER.next_serial());
            }
            return;
        }
        let gone = self
            .layer_focus
            .as_ref()
            .is_some_and(|s| !s.is_alive() || !self.layer_mapped(s));
        if gone {
            self.layer_focus = None;
            let w = self.focused_window();
            self.focus_window(w);
        }
    }

    /// Layer surface under `pos` on one of `levels`, with the hit surface and
    /// its global location.
    pub fn layer_at(
        &self,
        pos: Point<f64, Logical>,
        levels: [Layer; 2],
    ) -> Option<(LayerSurface, WlSurface, Point<f64, Logical>)> {
        let output = self.space.outputs().next()?;
        let origin = self.space.output_geometry(output)?.loc.to_f64();
        let map = layer_map_for_output(output);
        levels.into_iter().find_map(|level| {
            let layer = map.layer_under(level, pos - origin)?;
            let loc = map.layer_geometry(layer)?.loc.to_f64() + origin;
            let (s, p) = layer.surface_under(pos - loc, WindowSurfaceType::ALL)?;
            Some((layer.clone(), s, p.to_f64() + loc))
        })
    }

    /// Click-to-focus: layers above windows first, then windows, then layers
    /// below; `OnDemand`/`Exclusive` layers take keyboard focus.
    pub fn click_focus(&mut self, pos: Point<f64, Logical>) {
        let window = self.groupbar_tab_at(pos).or_else(|| self.window_under(pos));
        let layer = self
            .layer_at(pos, [Layer::Overlay, Layer::Top])
            .or_else(|| {
                window
                    .is_none()
                    .then(|| self.layer_at(pos, [Layer::Bottom, Layer::Background]))
                    .flatten()
            });
        if let Some((layer, _, _)) = layer {
            if layer.can_receive_keyboard_focus() {
                let s = layer.wl_surface().clone();
                self.layer_focus = Some(s.clone());
                let keyboard = self.seat.get_keyboard().unwrap();
                keyboard.set_focus(self, Some(s), SERIAL_COUNTER.next_serial());
            }
            return;
        }
        self.focus_window(window);
    }

    /// Frame callbacks for windows and layer surfaces on `output`.
    pub fn send_frames(&self, output: &Output) {
        let time = self.start_time.elapsed();
        self.space.elements().for_each(|window| {
            window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            })
        });
        for layer in layer_map_for_output(output).layers() {
            layer.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
    }

    /// Reply to `hyprctl [-j] layers`.
    pub fn layers_reply(&self, json: bool) -> String {
        let names = ["background", "bottom", "top", "overlay"];
        let mut obj = serde_json::Map::new();
        let mut text = String::new();
        for output in self.space.outputs() {
            let origin = self
                .space
                .output_geometry(output)
                .map(|g| g.loc)
                .unwrap_or_default();
            let map = layer_map_for_output(output);
            let mut levels = serde_json::Map::new();
            text += &format!("Monitor {}:\n", output.name());
            for (i, level) in [Layer::Background, Layer::Bottom, Layer::Top, Layer::Overlay]
                .into_iter()
                .enumerate()
            {
                let mut list = Vec::new();
                text += &format!("\tLayer level {i} ({}):\n", names[i]);
                for l in map.layers_on(level) {
                    let Some(geo) = map.layer_geometry(l) else {
                        continue;
                    };
                    let id = l.wl_surface().id();
                    let addr = 0x5590_0000u64 + id.protocol_id() as u64 * 0x40;
                    let pid = self
                        .display_handle
                        .get_client(id)
                        .ok()
                        .and_then(|c| c.get_credentials(&self.display_handle).ok())
                        .map_or(-1, |c| c.pid);
                    let (x, y) = (geo.loc.x + origin.x, geo.loc.y + origin.y);
                    text += &format!(
                        "\t\tLayer {addr:x}: xywh: {x} {y} {} {}, namespace: {}, pid: {pid}\n",
                        geo.size.w,
                        geo.size.h,
                        l.namespace()
                    );
                    list.push(serde_json::json!({
                        "address": format!("0x{addr:x}"), "x": x, "y": y,
                        "w": geo.size.w, "h": geo.size.h,
                        "namespace": l.namespace(), "pid": pid,
                    }));
                }
                levels.insert(i.to_string(), list.into());
            }
            obj.insert(output.name(), serde_json::json!({ "levels": levels }));
        }
        if json {
            serde_json::Value::Object(obj).to_string()
        } else {
            text
        }
    }
}
