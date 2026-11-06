use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    backend::session::Session,
    input::{
        keyboard::{xkb, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    utils::SERIAL_COUNTER,
};

use crate::{
    config::{Dispatcher, Mods},
    state::Hypr,
};

impl Hypr {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);
                let key_state = event.state();

                let action = self
                    .seat
                    .get_keyboard()
                    .unwrap()
                    .input::<Option<Dispatcher>, _>(
                        self,
                        event.key_code(),
                        key_state,
                        serial,
                        time,
                        |state, modifiers, handle| {
                            if key_state != KeyState::Pressed {
                                return FilterResult::Forward;
                            }
                            // Ctrl+Alt+F<n> gives XF86Switch_VT_<n> in the default keymap.
                            let vt = handle
                                .modified_sym()
                                .raw()
                                .wrapping_sub(xkb::keysyms::KEY_XF86Switch_VT_1);
                            if vt < 12 {
                                if let Some(session) = state.session.as_mut() {
                                    if let Err(e) = session.change_vt(vt as i32 + 1) {
                                        tracing::warn!("VT switch failed: {e}");
                                    }
                                    return FilterResult::Intercept(None);
                                }
                            }
                            let mods = Mods {
                                shift: modifiers.shift,
                                ctrl: modifiers.ctrl,
                                alt: modifiers.alt,
                                logo: modifiers.logo,
                            };
                            // Match on the unshifted key so `SUPER SHIFT, 1` works
                            // regardless of keyboard layout's shifted symbol.
                            let syms: Vec<String> = handle
                                .raw_syms()
                                .into_iter()
                                .chain(handle.raw_latin_sym_or_raw_current_sym())
                                .map(|s| xkb::keysym_get_name(s).to_ascii_lowercase())
                                .collect();
                            state
                                .config
                                .binds
                                .iter()
                                .find(|b| b.mods == mods && syms.contains(&b.key))
                                .map(|b| FilterResult::Intercept(Some(b.dispatcher.clone())))
                                .unwrap_or(FilterResult::Forward)
                        },
                    );
                if let Some(action) = action.flatten() {
                    self.dispatch(&action);
                }
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

                // Click to focus.
                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    self.click_focus(pointer.current_location());
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

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
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
}
