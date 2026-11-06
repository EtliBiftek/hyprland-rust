use std::time::Duration;

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
            KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        },
        libinput::LibinputInputBackend,
        session::Session,
    },
    input::{
        keyboard::{xkb, Keycode, KeysymHandle, ModifiersState, XkbConfig},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        input,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::{
    config::{AccelProfile, Dispatcher, Mods},
    selectors::WinSel,
    state::Hypr,
};

/// What a key event should do, decided inside the keyboard filter.
#[derive(Default)]
struct KeyOutcome {
    /// Do not forward the event to the focused client.
    consume: bool,
    /// Dispatchers of the matching binds, in config order.
    run: Vec<Dispatcher>,
    /// A `binde` bind that should start repeating.
    repeat: Option<Dispatcher>,
    /// Surface that must get the release of a key we `pass`ed to it.
    pass_release: Option<WlSurface>,
}

impl Hypr {
    /// Applies `input { }` to the keyboard (layout, repeat, numlock) and to
    /// the libinput devices seen so far. Call again after a config reload.
    pub fn apply_input_config(&mut self) {
        let i = self.config.input.clone();
        let kb = self.seat.get_keyboard().unwrap();
        let xkb_config = XkbConfig {
            rules: &i.kb_rules,
            model: &i.kb_model,
            layout: &i.kb_layout,
            variant: &i.kb_variant,
            options: (!i.kb_options.is_empty()).then(|| i.kb_options.clone()),
        };
        // An invalid layout keeps the previous keymap.
        if let Err(e) = kb.set_xkb_config(self, xkb_config) {
            tracing::warn!("invalid keyboard layout settings: {e:?}");
        }
        kb.change_repeat_info(i.repeat_rate, i.repeat_delay);
        if i.numlock_by_default {
            let mut m = kb.modifier_state();
            m.num_lock = true;
            kb.set_modifier_state(m);
        }
        for mut d in self.wm.libinput_devices.clone() {
            self.configure_libinput(&mut d);
        }
    }

    /// Tracks libinput devices and applies the pointer options to new ones.
    pub fn libinput_device_event(&mut self, event: &InputEvent<LibinputInputBackend>) {
        match event {
            InputEvent::DeviceAdded { device } => {
                let mut d = device.clone();
                self.configure_libinput(&mut d);
                self.wm.libinput_devices.push(d);
            }
            InputEvent::DeviceRemoved { device } => {
                let name = device.sysname().to_string();
                self.wm.libinput_devices.retain(|d| d.sysname() != name);
            }
            _ => {}
        }
    }

    fn configure_libinput(&self, d: &mut input::Device) {
        if !d.has_capability(input::DeviceCapability::Pointer) {
            return;
        }
        let touchpad = d.config_tap_finger_count() > 0;
        let o = self.config.input.opts_for(d.name(), touchpad);
        tracing::debug!(
            device = d.name(),
            touchpad,
            ?o,
            "configuring libinput device"
        );
        if let Some(s) = o.sensitivity {
            let _ = d.config_accel_set_speed(s);
        }
        if let Some(p) = o.accel_profile {
            let _ = d.config_accel_set_profile(match p {
                AccelProfile::Flat => input::AccelProfile::Flat,
                AccelProfile::Adaptive => input::AccelProfile::Adaptive,
            });
        }
        if let Some(v) = o.natural_scroll {
            let _ = d.config_scroll_set_natural_scroll_enabled(v);
        }
        if let Some(v) = o.left_handed {
            let _ = d.config_left_handed_set(v);
        }
        if let Some(v) = o.tap_to_click {
            let _ = d.config_tap_set_enabled(v);
        }
        if let Some(v) = o.disable_while_typing {
            let _ = d.config_dwt_set_enabled(v);
        }
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                self.on_key(event.key_code(), event.state(), Event::time_msec(&event));
            }
            InputEvent::PointerMotion { event, .. } => {
                // Relative motion (libinput mice): move and clamp to the output.
                let pointer = self.seat.get_pointer().unwrap();
                let mut pos = pointer.current_location() + event.delta();
                if let Some(geo) = self.output_geometry() {
                    let geo = geo.to_f64();
                    pos.x = pos.x.clamp(geo.loc.x, geo.loc.x + geo.size.w - 1.0);
                    pos.y = pos.y.clamp(geo.loc.y, geo.loc.y + geo.size.h - 1.0);
                }
                self.follow_mouse(pos);
                let under = self.surface_under(pos);
                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();

                self.follow_mouse(pos);
                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    // `bindm`: start dragging instead of clicking.
                    if let Some(resize) = self.mouse_bind(button) {
                        if self.start_mouse_grab(resize, button) {
                            return;
                        }
                    }
                    // Click to focus (follow_mouse = 3 detaches focus from the pointer).
                    if self.config.input.follow_mouse != 3 {
                        let window = self.window_under(pointer.current_location());
                        self.focus_window(window);
                    }
                }

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                // `touchpad:scroll_factor` applies to finger scrolling only.
                // shortcut: one factor for all touchpads, not per `device { }`.
                let factor = if source == AxisSource::Finger {
                    self.config.input.touchpad_scroll_factor()
                } else {
                    1.0
                };
                let horizontal_amount = factor
                    * event.amount(Axis::Horizontal).unwrap_or_else(|| {
                        event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                    });
                let vertical_amount = factor
                    * event.amount(Axis::Vertical).unwrap_or_else(|| {
                        event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                    });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            _ => {}
        }
    }

    /// `follow_mouse = 1`: keyboard focus follows the window under the pointer.
    fn follow_mouse(&mut self, pos: Point<f64, Logical>) {
        if self.config.input.follow_mouse != 1 {
            return;
        }
        if self.seat.get_pointer().is_some_and(|p| p.is_grabbed()) {
            return;
        }
        if let Some(w) = self.window_under(pos) {
            if self.focused_window().as_ref() != Some(&w) {
                self.focus_window(Some(w));
            }
        }
    }

    /// The `bindm` bind for `button` with the current modifiers. `Some(true)`
    /// for `resizewindow`, `Some(false)` for `movewindow`.
    fn mouse_bind(&self, button: u32) -> Option<bool> {
        let m = self.seat.get_keyboard()?.modifier_state();
        let mods = Mods {
            shift: m.shift,
            ctrl: m.ctrl,
            alt: m.alt,
            logo: m.logo,
        };
        let key = format!("mouse:{button}");
        self.config
            .binds
            .iter()
            .find(|b| {
                b.flags.mouse
                    && b.submap == self.wm.submap
                    && b.key == key
                    && (b.flags.ignore_mods || b.mods == mods)
            })
            .map(|b| b.dispatcher == Dispatcher::MouseResize)
    }

    fn on_key(&mut self, code: Keycode, key_state: KeyState, time: u32) {
        let serial = SERIAL_COUNTER.next_serial();
        let kb = self.seat.get_keyboard().unwrap();
        let (out, mods_changed) =
            kb.input_intercept(self, code, key_state, |state, mods, handle| {
                state.match_key(code, key_state, mods, &handle)
            });
        if !out.consume {
            kb.input_forward(self, code, key_state, serial, time, mods_changed);
        }
        if let Some(surface) = out.pass_release {
            self.forward_key(surface, code, key_state, time);
        }
        for d in out.run {
            match d {
                Dispatcher::Pass(sel) => self.pass_key(&sel, code, key_state, time),
                d => self.dispatch(&d),
            }
        }
        if let Some(d) = out.repeat {
            self.start_repeat(code.raw(), d);
        }
    }

    /// Decides what a key event triggers. Runs inside the keyboard filter, so
    /// it must not dispatch anything itself.
    fn match_key(
        &mut self,
        code: Keycode,
        key_state: KeyState,
        modifiers: &ModifiersState,
        handle: &KeysymHandle<'_>,
    ) -> KeyOutcome {
        let mut out = KeyOutcome::default();
        let raw = code.raw();
        let pressed = key_state == KeyState::Pressed;

        if pressed {
            // Ctrl+Alt+F<n> gives XF86Switch_VT_<n> in the default keymap.
            let vt = handle
                .modified_sym()
                .raw()
                .wrapping_sub(xkb::keysyms::KEY_XF86Switch_VT_1);
            if vt < 12 {
                if let Some(session) = self.session.as_mut() {
                    if let Err(e) = session.change_vt(vt as i32 + 1) {
                        tracing::warn!("VT switch failed: {e}");
                    }
                    out.consume = true;
                    return out;
                }
            }
        } else {
            if self.wm.repeat.as_ref().is_some_and(|r| r.0 == raw) {
                self.stop_repeat();
            }
            if let Some(p) = self.wm.passed_keys.iter().position(|(c, _)| *c == raw) {
                out.pass_release = Some(self.wm.passed_keys.remove(p).1);
            }
            // A consumed press consumes its release too.
            if let Some(p) = self.wm.consumed_keys.iter().position(|c| *c == raw) {
                self.wm.consumed_keys.remove(p);
                out.consume = true;
            }
        }

        let mut mods = Mods {
            shift: modifiers.shift,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            logo: modifiers.logo,
        };
        if pressed {
            self.wm.key_mods.retain(|(c, _)| *c != raw);
            self.wm.key_mods.push((raw, mods));
        } else if let Some(p) = self.wm.key_mods.iter().position(|(c, _)| *c == raw) {
            // Use the modifiers from the press: they may be released first.
            mods = self.wm.key_mods.remove(p).1;
        }
        // Match on the unshifted key so `SUPER SHIFT, 1` works
        // regardless of keyboard layout's shifted symbol.
        let syms: Vec<String> = handle
            .raw_syms()
            .into_iter()
            .chain(handle.raw_latin_sym_or_raw_current_sym())
            .map(|s| xkb::keysym_get_name(s).to_ascii_lowercase())
            .collect();
        for b in &self.config.binds {
            if b.flags.mouse || b.submap != self.wm.submap {
                continue;
            }
            let key_ok = match b.key.strip_prefix("code:") {
                Some(n) => n.parse::<u32>() == Ok(raw),
                None => syms.contains(&b.key),
            };
            let mods_ok = b.flags.ignore_mods || b.mods == mods;
            if !key_ok || !mods_ok {
                continue;
            }
            // Release binds fire on release, all others on press.
            if b.flags.release == pressed {
                continue;
            }
            out.run.push(b.dispatcher.clone());
            // shortcut: release binds never consume (their press already
            // reached the client, which must also see the release).
            if !b.flags.non_consuming && !b.flags.release {
                out.consume = true;
            }
            if b.flags.repeat && pressed {
                out.repeat = Some(b.dispatcher.clone());
            }
        }
        if pressed && out.consume {
            self.wm.consumed_keys.push(raw);
        }
        out
    }

    /// `binde`: repeat `d` at the configured repeat rate while the key is held.
    fn start_repeat(&mut self, raw: u32, d: Dispatcher) {
        self.stop_repeat();
        let rate = self.config.input.repeat_rate.max(1) as u64;
        let delay = self.config.input.repeat_delay as u64;
        let timer = Timer::from_duration(Duration::from_millis(delay));
        let token = self.wm.loop_handle.insert_source(timer, move |_, _, data| {
            data.state.dispatch(&d);
            TimeoutAction::ToDuration(Duration::from_millis(1000 / rate))
        });
        match token {
            Ok(t) => self.wm.repeat = Some((raw, t)),
            Err(e) => tracing::warn!("cannot start key repeat: {}", e.error),
        }
    }

    fn stop_repeat(&mut self) {
        if let Some((_, token)) = self.wm.repeat.take() {
            self.wm.loop_handle.remove(token);
        }
    }

    /// `pass`: sends the triggering key press to the selected window.
    fn pass_key(&mut self, sel: &WinSel, code: Keycode, state: KeyState, time: u32) {
        if state != KeyState::Pressed {
            return;
        }
        let Some((_, w)) = self.find_window(sel) else {
            return;
        };
        let Some(surface) = w.toplevel().map(|t| t.wl_surface().clone()) else {
            return;
        };
        self.forward_key(surface.clone(), code, state, time);
        self.wm.passed_keys.push((code.raw(), surface));
    }

    /// Delivers a key event to `surface` without changing the focus for good.
    fn forward_key(&mut self, surface: WlSurface, code: Keycode, state: KeyState, time: u32) {
        let kb = self.seat.get_keyboard().unwrap();
        let prev = kb.current_focus();
        kb.set_focus(self, Some(surface), SERIAL_COUNTER.next_serial());
        kb.input_forward(self, code, state, SERIAL_COUNTER.next_serial(), time, true);
        kb.set_focus(self, prev, SERIAL_COUNTER.next_serial());
    }
}
