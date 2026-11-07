//! wlr-virtual-pointer-unstable-v1 (wayvnc, wlrctl pointer, ydotool-like tools):
//! requests are replayed on the seat pointer.
//! shortcuts: `follow_mouse` focus and bindm grabs are not triggered by virtual
//! motion; the optional output is ignored (one output); axis events are forwarded
//! per `frame` without the source/discrete refinements.

use smithay::{
    backend::input::{Axis, ButtonState},
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent},
    reexports::{
        wayland_protocols_wlr::virtual_pointer::v1::server::{
            zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
            zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
        },
        wayland_server::{
            protocol::wl_pointer, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
            WEnum,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, ZwlrVirtualPointerManagerV1, _>(2, ());
}

impl GlobalDispatch<ZwlrVirtualPointerManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrVirtualPointerManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrVirtualPointerManagerV1,
        request: zwlr_virtual_pointer_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_virtual_pointer_manager_v1::Request as R;
        match request {
            R::CreateVirtualPointer { id, .. } | R::CreateVirtualPointerWithOutput { id, .. } => {
                data_init.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrVirtualPointerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwlrVirtualPointerV1,
        request: zwlr_virtual_pointer_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_virtual_pointer_v1::Request as R;
        if state.session_locked {
            return;
        }
        let Some(pointer) = state.seat.get_pointer() else {
            return;
        };
        match request {
            R::Motion { time, dx, dy } => {
                let mut pos = pointer.current_location() + Point::from((dx, dy));
                if let Some(geo) = state.output_geometry() {
                    let geo = geo.to_f64();
                    pos.x = pos.x.clamp(geo.loc.x, geo.loc.x + geo.size.w - 1.0);
                    pos.y = pos.y.clamp(geo.loc.y, geo.loc.y + geo.size.h - 1.0);
                }
                let under = state.surface_under(pos);
                pointer.motion(
                    state,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
            }
            R::MotionAbsolute {
                time,
                x,
                y,
                x_extent,
                y_extent,
            } => {
                let Some(geo) = state.output_geometry() else {
                    return;
                };
                if x_extent == 0 || y_extent == 0 {
                    return;
                }
                let geo = geo.to_f64();
                let pos: Point<f64, Logical> = (
                    geo.loc.x + x as f64 / x_extent as f64 * geo.size.w,
                    geo.loc.y + y as f64 / y_extent as f64 * geo.size.h,
                )
                    .into();
                let under = state.surface_under(pos);
                pointer.motion(
                    state,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
            }
            R::Button {
                time,
                button,
                state: s,
            } => {
                let pressed = matches!(s, WEnum::Value(wl_pointer::ButtonState::Pressed));
                if pressed {
                    let pos = pointer.current_location();
                    state.focus_grab_button(pos);
                    state.click_focus(pos);
                }
                pointer.button(
                    state,
                    &ButtonEvent {
                        button,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
            }
            R::Axis { time, axis, value } => {
                let axis = match axis {
                    WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Axis::Horizontal,
                    _ => Axis::Vertical,
                };
                pointer.axis(state, AxisFrame::new(time).value(axis, value));
            }
            R::Frame => pointer.frame(state),
            _ => {}
        }
    }
}
