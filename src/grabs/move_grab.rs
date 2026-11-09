use crate::Hypr;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

impl Hypr {
    /// `general:snap:*` applied to a window dragged to `loc`.
    fn snapped(&self, w: &Window, loc: Point<i32, Logical>) -> Point<i32, Logical> {
        use crate::{
            layout::{snap_position, Rect, SnapOpts},
            options::{get_option, OptValue},
        };
        let opt = |k| get_option(&self.config, k);
        let int = |k| match opt(k) {
            Some(OptValue::Int(v)) => v as i32,
            _ => 0,
        };
        let on = |k| opt(k) == Some(OptValue::Bool(true));
        let (Some(mon), Some(ws), true) = (
            self.output_geometry(),
            self.ws_of(w),
            on("general:snap:enabled"),
        ) else {
            return loc;
        };
        let rect = |x, y, s: smithay::utils::Size<i32, Logical>| Rect {
            x,
            y,
            w: s.w,
            h: s.h,
        };
        let others: Vec<Rect> = self.workspaces[ws]
            .floating
            .iter()
            .filter(|o| *o != w)
            .filter_map(|o| self.space.element_geometry(o))
            .map(|g| rect(g.loc.x, g.loc.y, g.size))
            .collect();
        let border = self.config.general.border_size;
        let mut pad = if on("general:snap:border_overlap") {
            border
        } else {
            2 * border
        };
        if on("general:snap:respect_gaps") {
            pad += 2 * self.config.general.gaps_in;
        }
        let (x, y) = snap_position(
            rect(loc.x, loc.y, w.geometry().size),
            &others,
            rect(mon.loc.x, mon.loc.y, mon.size),
            SnapOpts {
                win_gap: int("general:snap:window_gap"),
                mon_gap: int("general:snap:monitor_gap"),
                border,
                pad,
            },
        );
        (x, y).into()
    }
}

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Hypr>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<Hypr> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        let location = data.snapped(&self.window, new_location.to_i32_round());
        data.space.map_element(self.window.clone(), location, true);
    }

    fn relative_motion(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        // Linux kernel's linux/input-event-codes.h header file, e.g. BTN_LEFT.
        const BTN_LEFT: u32 = 0x110;

        if !handle.current_pressed().contains(&BTN_LEFT) {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Hypr, handle: &mut PointerInnerHandle<'_, Hypr>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Hypr,
        handle: &mut PointerInnerHandle<'_, Hypr>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Hypr> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Hypr) {}
}
