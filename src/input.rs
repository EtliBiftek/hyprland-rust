use std::time::Duration;

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, GestureBeginEvent,
            GestureEndEvent, GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _,
            InputBackend, InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent,
            PointerButtonEvent, PointerMotionEvent,
        },
        libinput::LibinputInputBackend,
        session::Session,
    },
    input::{
        keyboard::{xkb, Keycode, KeysymHandle, ModifiersState, XkbConfig},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            RelativeMotionEvent,
        },
    },
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        input,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
};

use crate::{
    config::{AccelProfile, Dispatcher, Mods},
    gestures::{Kind, Outcome, Tracker},
    selectors::WinSel,
    state::Hypr,
};

impl Hypr {
    fn current_mods(&self) -> Mods {
        let m = self
            .seat
            .get_keyboard()
            .map(|k| k.modifier_state())
            .unwrap_or_default();
        Mods {
            shift: m.shift,
            ctrl: m.ctrl,
            alt: m.alt,
            logo: m.logo,
        }
    }

    /// A touchpad gesture starts: ours if a `gesture =` for this finger count
    /// (and modifiers) exists, else it goes to the client (pointer-gestures).
    fn gesture_begin(&mut self, kind: Kind, fingers: u32, time: u32) {
        let t = Tracker::new(kind, fingers, self.current_mods());
        let ours = self.config.gestures.iter().any(|g| {
            g.fingers == fingers
                && g.mods == t.mods
                && g.action != "unset"
                && (g.direction.starts_with("pinch") == (kind == Kind::Pinch))
        });
        self.wm.gesture = Some((t, ours));
        if ours {
            return;
        }
        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        match kind {
            Kind::Swipe => pointer.gesture_swipe_begin(
                self,
                &GestureSwipeBeginEvent {
                    serial,
                    time,
                    fingers,
                },
            ),
            Kind::Pinch => pointer.gesture_pinch_begin(
                self,
                &GesturePinchBeginEvent {
                    serial,
                    time,
                    fingers,
                },
            ),
        }
    }

    fn gesture_update(
        &mut self,
        delta: Point<f64, Logical>,
        scale: Option<f64>,
        rotation: f64,
        time: u32,
    ) {
        let Some((mut t, ours)) = self.wm.gesture.take() else {
            return;
        };
        t.dx += delta.x;
        t.dy += delta.y;
        if let Some(s) = scale {
            t.scale = s;
        }
        if ours {
            // `move` / `resize` follow the fingers.
            if let Some(g) = t.live_action(&self.config.gestures) {
                let name = if g.action == "move" {
                    "moveactive"
                } else {
                    "resizeactive"
                };
                let f = g.scale as f64;
                let arg = format!("{} {}", (delta.x * f).round(), (delta.y * f).round());
                if let Ok(d) = Dispatcher::parse(name, &arg) {
                    self.dispatch(&d);
                }
            }
        } else {
            let pointer = self.seat.get_pointer().unwrap();
            match t.kind {
                Kind::Swipe => {
                    pointer.gesture_swipe_update(self, &GestureSwipeUpdateEvent { time, delta })
                }
                Kind::Pinch => pointer.gesture_pinch_update(
                    self,
                    &GesturePinchUpdateEvent {
                        time,
                        delta,
                        scale: t.scale,
                        rotation,
                    },
                ),
            }
        }
        self.wm.gesture = Some((t, ours));
    }

    fn gesture_end(&mut self, cancelled: bool, time: u32) {
        let Some((t, ours)) = self.wm.gesture.take() else {
            return;
        };
        if !ours {
            let pointer = self.seat.get_pointer().unwrap();
            let serial = SERIAL_COUNTER.next_serial();
            match t.kind {
                Kind::Swipe => pointer.gesture_swipe_end(
                    self,
                    &GestureSwipeEndEvent {
                        serial,
                        time,
                        cancelled,
                    },
                ),
                Kind::Pinch => pointer.gesture_pinch_end(
                    self,
                    &GesturePinchEndEvent {
                        serial,
                        time,
                        cancelled,
                    },
                ),
            }
            return;
        }
        if cancelled {
            return;
        }
        match t.finish(&self.config.gestures, &self.config.swipe) {
            Outcome::Dispatch(name, _) if name == "noop" => {}
            Outcome::Dispatch(name, args) => match Dispatcher::parse(&name, &args) {
                Ok(d) => self.dispatch(&d),
                Err(e) => tracing::warn!("gesture: {e}"),
            },
            Outcome::Unhandled => {}
        }
    }
}

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
    /// `global` binds whose key was released (they get `released`).
    global_release: Vec<String>,
    /// `o` binds: run after the key is held for `repeat_delay`.
    long_press: Option<Dispatcher>,
    /// `bindm` on a keyboard key: start (`Some(resize)`) a pointer grab.
    key_grab: Option<bool>,
    /// The key that holds a keyboard `bindm` grab was released.
    end_key_grab: bool,
}

impl Hypr {
    /// Applies `input { }` to the keyboard (layout, repeat, numlock) and to
    /// the libinput devices seen so far. Call again after a config reload.
    /// The keymap settings (`KB_KEYS` order) for a keyboard: `input { }`
    /// overridden by its `device { }` block.
    fn keymap_for(&self, device: Option<&str>) -> [String; 5] {
        let i = &self.config.input;
        let mut k = [
            i.kb_layout.clone(),
            i.kb_variant.clone(),
            i.kb_model.clone(),
            i.kb_options.clone(),
            i.kb_rules.clone(),
        ];
        if let Some(d) = device.and_then(|n| {
            let n = n.to_lowercase().replace(' ', "-");
            i.devices.iter().find(|d| d.name == n)
        }) {
            for (a, b) in k.iter_mut().zip(&d.opts.kb) {
                if let Some(b) = b {
                    a.clone_from(b);
                }
            }
        }
        k
    }

    fn set_keymap(&mut self, k: [String; 5]) {
        if self.wm.keymap.as_ref() == Some(&k) {
            return;
        }
        let kb = self.seat.get_keyboard().unwrap();
        let xkb_config = XkbConfig {
            layout: &k[0],
            variant: &k[1],
            model: &k[2],
            options: (!k[3].is_empty()).then(|| k[3].clone()),
            rules: &k[4],
        };
        // An invalid layout keeps the previous keymap.
        if let Err(e) = kb.set_xkb_config(self, xkb_config) {
            tracing::warn!("invalid keyboard layout settings: {e:?}");
            return;
        }
        self.wm.keymap = Some(k);
    }

    /// A key from `device`: switch to its `device { kb_* }` keymap if it has one.
    fn keymap_follow_device(&mut self, device: &str) {
        let k = self.keymap_for(Some(device));
        self.set_keymap(k);
    }

    pub fn apply_input_config(&mut self) {
        let i = self.config.input.clone();
        let kb = self.seat.get_keyboard().unwrap();
        self.wm.keymap = None;
        let k = self.keymap_for(None);
        self.set_keymap(k);
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
        self.notify_idle_activity();
        match event {
            InputEvent::Keyboard { event, .. } => {
                if event.state() == KeyState::Pressed {
                    let name = smithay::backend::input::Device::name(&event.device());
                    self.keymap_follow_device(&name);
                }
                self.on_key(event.key_code(), event.state(), Event::time_msec(&event));
            }
            InputEvent::PointerMotion { event, .. } => {
                // Relative motion (libinput mice): move and clamp to the output.
                let pointer = self.seat.get_pointer().unwrap();
                let from = pointer.current_location();
                let pos = self.clamp_to_outputs(from, from + event.delta());
                self.pointer_on_monitor(pos);
                self.follow_mouse(pos);
                let under = self.surface_under(pos);
                // Relative motion goes to the client even when the pointer is clamped.
                pointer.relative_motion(
                    self,
                    under.clone(),
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta(),
                        utime: event.time(),
                    },
                );
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
                // Absolute devices (winit, tablets) map to the focused output.
                let Some(output_geo) = self.output_geometry() else {
                    return;
                };

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();

                self.pointer_on_monitor(pos);
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

                if ButtonState::Pressed == button_state && self.wm.kill_mode {
                    self.wm.kill_mode = false;
                    if let Some(w) = self.window_under(pointer.current_location()) {
                        self.wm.target = Some(w);
                        self.kill_window(None, true);
                        self.wm.target = None;
                    }
                    self.wm.consumed_buttons.push(button);
                    return;
                }
                if ButtonState::Pressed == button_state {
                    self.focus_grab_button(pointer.current_location());
                }
                if ButtonState::Pressed == button_state
                    && !pointer.is_grabbed()
                    && !self.session_locked
                {
                    // `bindm`: start dragging instead of clicking.
                    if let Some(resize) = self.mouse_bind(button) {
                        if self.start_mouse_grab(resize, button) {
                            return;
                        }
                    }
                    if self.pointer_bind(&format!("mouse:{button}"), true) {
                        self.wm.consumed_buttons.push(button);
                        return;
                    }
                    // Click to focus (follow_mouse = 3 detaches focus from the pointer).
                    if self.config.input.follow_mouse != 3 {
                        self.click_focus(pointer.current_location());
                    }
                }
                if ButtonState::Released == button_state {
                    if !pointer.is_grabbed() {
                        self.pointer_bind(&format!("mouse:{button}"), false);
                    }
                    if let Some(p) = self.wm.consumed_buttons.iter().position(|b| *b == button) {
                        self.wm.consumed_buttons.remove(p);
                        return;
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

                let amount = |a| event.amount_v120(a).or(event.amount(a)).unwrap_or(0.0);
                if source == AxisSource::Wheel
                    && self.wheel_bind(
                        event.time_msec(),
                        amount(Axis::Vertical),
                        amount(Axis::Horizontal),
                    )
                {
                    return;
                }

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
            InputEvent::GestureSwipeBegin { event, .. } => {
                self.gesture_begin(Kind::Swipe, event.fingers(), event.time_msec())
            }
            InputEvent::GestureSwipeUpdate { event, .. } => {
                self.gesture_update(event.delta(), None, 0.0, event.time_msec())
            }
            InputEvent::GestureSwipeEnd { event, .. } => {
                self.gesture_end(event.cancelled(), event.time_msec())
            }
            InputEvent::GesturePinchBegin { event, .. } => {
                self.gesture_begin(Kind::Pinch, event.fingers(), event.time_msec())
            }
            InputEvent::GesturePinchUpdate { event, .. } => self.gesture_update(
                event.delta(),
                Some(event.scale()),
                event.rotation(),
                event.time_msec(),
            ),
            InputEvent::GesturePinchEnd { event, .. } => {
                self.gesture_end(event.cancelled(), event.time_msec())
            }
            InputEvent::GestureHoldBegin { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                pointer.gesture_hold_begin(
                    self,
                    &GestureHoldBeginEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        fingers: event.fingers(),
                    },
                );
            }
            InputEvent::GestureHoldEnd { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                pointer.gesture_hold_end(
                    self,
                    &GestureHoldEndEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                        cancelled: event.cancelled(),
                    },
                );
            }
            _ => {}
        }
    }

    /// `follow_mouse = 1`: keyboard focus follows the window under the pointer.
    fn follow_mouse(&mut self, pos: Point<f64, Logical>) {
        if self.config.input.follow_mouse != 1 || self.session_locked {
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
    /// Runs the non-`bindm` binds of a mouse button or wheel key. Returns
    /// whether the event was consumed.
    fn pointer_bind(&mut self, key: &str, pressed: bool) -> bool {
        let Some(m) = self.seat.get_keyboard().map(|k| k.modifier_state()) else {
            return false;
        };
        let mods = Mods {
            shift: m.shift,
            ctrl: m.ctrl,
            alt: m.alt,
            logo: m.logo,
        };
        let inhibited = self.seat.keyboard_shortcuts_inhibited();
        let hits: Vec<_> = self
            .config
            .binds
            .iter()
            .filter(|b| {
                !b.flags.mouse
                    && b.key == key
                    && b.flags.release != pressed
                    && (b.submap == self.wm.submap || b.flags.universal)
                    && (!self.session_locked || b.flags.locked)
                    && (!inhibited || b.flags.bypass)
                    && (b.flags.ignore_mods || b.mods == mods)
            })
            .map(|b| (b.dispatcher.clone(), !b.flags.non_consuming))
            .collect();
        let consume = hits.iter().any(|h| h.1);
        for (d, _) in hits {
            self.dispatch(&d);
        }
        consume
    }

    /// `mouse_up/down/left/right` binds, one per `binds:scroll_event_delay`.
    fn wheel_bind(&mut self, time: u32, vertical: f64, horizontal: f64) -> bool {
        let Some(key) = crate::config::wheel_key(vertical, horizontal) else {
            return false;
        };
        let delay = self.config.scroll_event_delay.unwrap_or(300);
        if time.wrapping_sub(self.wm.last_wheel_bind) < delay
            || self.seat.get_pointer().is_some_and(|p| p.is_grabbed())
        {
            return false;
        }
        self.wm.last_wheel_bind = time;
        self.pointer_bind(key, true)
    }

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
        for name in out.global_release {
            self.global_shortcut(&name, false);
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
        if let Some(d) = out.long_press {
            let raw = code.raw();
            let delay = Duration::from_millis(self.config.input.repeat_delay.max(1) as u64);
            if let Ok(token) =
                self.wm
                    .loop_handle
                    .insert_source(Timer::from_duration(delay), move |_, _, data| {
                        // Still held: the release would have cancelled us.
                        data.state.wm.long_press = None;
                        data.state.dispatch(&d);
                        TimeoutAction::Drop
                    })
            {
                self.wm.long_press = Some((raw, token));
            }
        }
        if let Some(resize) = out.key_grab {
            if self.start_mouse_grab(resize, 0) {
                self.wm.key_grab = Some(code.raw());
            }
        }
        if out.end_key_grab {
            if let Some(p) = self.seat.get_pointer() {
                p.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
            }
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
        if !pressed && self.wm.key_grab == Some(raw) {
            self.wm.key_grab = None;
            out.end_key_grab = true;
            out.consume = true;
        }
        if !pressed && self.wm.long_press.as_ref().is_some_and(|l| l.0 == raw) {
            if let Some((_, token)) = self.wm.long_press.take() {
                self.wm.loop_handle.remove(token);
            }
        }
        // Locked: only `l` binds; inhibited shortcuts: only `p` binds.
        let locked = self.session_locked;
        let inhibited = self.seat.keyboard_shortcuts_inhibited();
        let syms: Vec<String> = handle
            .raw_syms()
            .into_iter()
            .chain(handle.raw_latin_sym_or_raw_current_sym())
            .map(|s| xkb::keysym_get_name(s).to_ascii_lowercase())
            .collect();
        if pressed && self.wm.kill_mode && syms.iter().any(|s| s == "escape") {
            self.wm.kill_mode = false;
            out.consume = true;
            return out;
        }
        for b in &self.config.binds {
            if (b.submap != self.wm.submap && !b.flags.universal)
                || (locked && !b.flags.locked)
                || (inhibited && !b.flags.bypass)
            {
                continue;
            }
            // `bindm` on a keyboard key: hold it to drag/resize.
            if b.flags.mouse {
                if pressed
                    && !b.key.starts_with("mouse:")
                    && b.mods == mods
                    && syms.contains(&b.key)
                {
                    out.key_grab = Some(b.dispatcher == Dispatcher::MouseResize);
                    out.consume = true;
                }
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
                if let (Dispatcher::Global(name), false) = (&b.dispatcher, pressed) {
                    out.global_release.push(name.clone());
                }
                continue;
            }
            if b.flags.long_press && pressed {
                out.long_press = Some(b.dispatcher.clone());
                out.consume = true;
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
        let Some(surface) = crate::xwayland::wl_surface_of(&w) else {
            return;
        };
        self.forward_key(surface.clone(), code, state, time);
        self.wm.passed_keys.push((code.raw(), surface));
    }

    /// Delivers a key event to `surface` without changing the focus for good.
    fn forward_key(&mut self, surface: WlSurface, code: Keycode, state: KeyState, time: u32) {
        // Locked: keyboard focus stays on the lock surface.
        if self.session_locked {
            return;
        }
        let kb = self.seat.get_keyboard().unwrap();
        let prev = kb.current_focus();
        kb.set_focus(self, Some(surface), SERIAL_COUNTER.next_serial());
        kb.input_forward(self, code, state, SERIAL_COUNTER.next_serial(), time, true);
        kb.set_focus(self, prev, SERIAL_COUNTER.next_serial());
    }
}
