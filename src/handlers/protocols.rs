//! Protocol globals: selection and data control, viewporter, scale, presentation,
//! cursor shape, idle (inhibit and ext-idle-notify), session lock, pointer
//! constraints, keyboard shortcuts inhibit and xdg-foreign.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::Duration,
};

use smithay::{
    backend::renderer::{
        element::{
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            Kind,
        },
        ImportAll, Renderer,
    },
    delegate_content_type, delegate_cursor_shape, delegate_data_control, delegate_ext_data_control,
    delegate_fractional_scale, delegate_idle_inhibit, delegate_keyboard_shortcuts_inhibit,
    delegate_pointer_constraints, delegate_presentation, delegate_primary_selection,
    delegate_relative_pointer, delegate_session_lock, delegate_single_pixel_buffer,
    delegate_viewporter, delegate_xdg_foreign,
    input::pointer::PointerHandle,
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            RegistrationToken,
        },
        wayland_protocols::ext::idle_notify::v1::server::{
            ext_idle_notification_v1::{self, ExtIdleNotificationV1},
            ext_idle_notifier_v1::{self, ExtIdleNotifierV1},
        },
        wayland_server::{
            backend::ClientId,
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        content_type::ContentTypeState,
        cursor_shape::CursorShapeManagerState,
        fractional_scale::{
            with_fractional_scale, FractionalScaleHandler, FractionalScaleManagerState,
        },
        idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
        pointer_constraints::{
            with_pointer_constraint, PointerConstraintsHandler, PointerConstraintsState,
        },
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        selection::{
            ext_data_control::DataControlHandler as ExtDataControlHandler,
            ext_data_control::DataControlState as ExtDataControlState,
            primary_selection::{PrimarySelectionHandler, PrimarySelectionState},
            wlr_data_control::DataControlHandler as WlrDataControlHandler,
            wlr_data_control::DataControlState as WlrDataControlState,
        },
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
        single_pixel_buffer::SinglePixelBufferState,
        viewporter::ViewporterState,
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

use crate::Hypr;

//
// Selections: primary selection, wlr and ext data control (clipboard managers).
//

impl PrimarySelectionHandler for Hypr {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}
delegate_primary_selection!(Hypr);

impl WlrDataControlHandler for Hypr {
    fn data_control_state(&self) -> &WlrDataControlState {
        &self.wlr_data_control_state
    }
}
delegate_data_control!(Hypr);

impl ExtDataControlHandler for Hypr {
    fn data_control_state(&self) -> &ExtDataControlState {
        &self.ext_data_control_state
    }
}
delegate_ext_data_control!(Hypr);

//
// Surface extensions: viewporter, fractional scale, presentation, single pixel
// buffers, content type, cursor shape.
//

delegate_viewporter!(Hypr);

impl FractionalScaleHandler for Hypr {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        // Only 1x output for now.
        with_states(&surface, |states| {
            with_fractional_scale(states, |fs| fs.set_preferred_scale(1.0));
        });
    }
}
delegate_fractional_scale!(Hypr);

delegate_presentation!(Hypr);
delegate_single_pixel_buffer!(Hypr);
delegate_content_type!(Hypr);
delegate_cursor_shape!(Hypr);

//
// Idle: idle-inhibit (surface inhibitors) and ext-idle-notify (timeouts).
//
// smithay's IdleNotifierState needs a calloop loop handle for `Hypr`, but the
// event loop data is `CalloopData`. So ext-idle-notify is implemented here
// with timers on `wm.loop_handle` instead.
// shortcut: idle timers are re-armed on every input event; upgrade to a
// per-seat timer wheel if this shows up in profiles.
//

/// Surfaces with an idle inhibitor, and the live ext-idle-notify objects.
#[derive(Default)]
pub struct IdleState {
    notifications: Vec<ExtIdleNotificationV1>,
    inhibitors: Vec<WlSurface>,
}

impl IdleState {
    fn is_inhibited(&self) -> bool {
        !self.inhibitors.is_empty()
    }
}

/// User data of one `ext_idle_notification_v1`.
pub struct IdleNotificationData {
    timeout: Duration,
    ignore_inhibitors: bool,
    idled: AtomicBool,
    timer: Mutex<Option<RegistrationToken>>,
}

impl Hypr {
    /// Call on every input event: resumes idle notifications and re-arms timers.
    pub fn notify_idle_activity(&mut self) {
        self.idle.notifications.retain(|n| n.is_alive());
        for n in self.idle.notifications.clone() {
            let Some(data) = n.data::<IdleNotificationData>() else {
                continue;
            };
            if data.idled.swap(false, Ordering::AcqRel) {
                n.resumed();
            }
            self.arm_idle_timer(&n);
        }
    }

    fn arm_idle_timer(&self, notification: &ExtIdleNotificationV1) {
        let Some(data) = notification.data::<IdleNotificationData>() else {
            return;
        };
        let old = data.timer.lock().unwrap().take();
        if let Some(token) = old {
            self.wm.loop_handle.remove(token);
        }
        let n = notification.clone();
        let timer = Timer::from_duration(data.timeout);
        let token = self.wm.loop_handle.insert_source(timer, move |_, _, cd| {
            let Some(d) = n.data::<IdleNotificationData>() else {
                return TimeoutAction::Drop;
            };
            if !d.ignore_inhibitors && cd.state.idle.is_inhibited() {
                // Poll until the inhibitors are gone.
                return TimeoutAction::ToDuration(d.timeout.max(Duration::from_secs(1)));
            }
            if !d.idled.swap(true, Ordering::AcqRel) {
                n.idled();
            }
            *d.timer.lock().unwrap() = None;
            TimeoutAction::Drop
        });
        *data.timer.lock().unwrap() = token.ok();
    }

    fn new_idle_notification(
        &mut self,
        id: New<ExtIdleNotificationV1>,
        timeout: u32,
        ignore_inhibitors: bool,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let data = IdleNotificationData {
            timeout: Duration::from_millis(u64::from(timeout)),
            ignore_inhibitors,
            idled: AtomicBool::new(false),
            timer: Mutex::new(None),
        };
        let notification = data_init.init(id, data);
        self.arm_idle_timer(&notification);
        self.idle.notifications.push(notification);
    }
}

impl IdleInhibitHandler for Hypr {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.push(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.retain(|s| *s != surface);
    }
}
delegate_idle_inhibit!(Hypr);

impl GlobalDispatch<ExtIdleNotifierV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ExtIdleNotifierV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ExtIdleNotifierV1,
        request: ext_idle_notifier_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // The seat argument is ignored: there is only one seat.
        match request {
            ext_idle_notifier_v1::Request::GetIdleNotification { id, timeout, .. } => {
                state.new_idle_notification(id, timeout, false, data_init)
            }
            ext_idle_notifier_v1::Request::GetInputIdleNotification { id, timeout, .. } => {
                state.new_idle_notification(id, timeout, true, data_init)
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, IdleNotificationData> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ExtIdleNotificationV1,
        _request: ext_idle_notification_v1::Request,
        _data: &IdleNotificationData,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Only `destroy` exists; cleanup happens in `destroyed`.
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        resource: &ExtIdleNotificationV1,
        data: &IdleNotificationData,
    ) {
        state.idle.notifications.retain(|n| n.id() != resource.id());
        if let Some(token) = data.timer.lock().unwrap().take() {
            state.wm.loop_handle.remove(token);
        }
    }
}

//
// Session lock: while locked only the lock surfaces are drawn and get input.
// The keybinds are bypassed in `input.rs`, so nothing else is reachable.
//

impl SessionLockHandler for Hypr {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.session_lock_state
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        self.session_locked = true;
        confirmation.lock();
    }

    fn unlock(&mut self) {
        self.session_locked = false;
        self.lock_surfaces.clear();
        let focus = self.focused_window();
        self.focus_window(focus);
    }

    fn new_surface(&mut self, surface: LockSurface, _output: WlOutput) {
        if let Some(mode) = self.space.outputs().next().and_then(|o| o.current_mode()) {
            surface.with_pending_state(|s| {
                s.size = Some((mode.size.w as u32, mode.size.h as u32).into())
            });
        }
        surface.send_configure();
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(
                self,
                Some(surface.wl_surface().clone()),
                SERIAL_COUNTER.next_serial(),
            );
        }
        self.lock_surfaces.push(surface);
    }
}
delegate_session_lock!(Hypr);

impl Hypr {
    /// Render elements of the live lock surfaces, drawn instead of the windows
    /// while the session is locked.
    pub fn lock_elements<R>(&self, renderer: &mut R) -> Vec<WaylandSurfaceRenderElement<R>>
    where
        R: Renderer + ImportAll,
        R::TextureId: Clone + 'static,
    {
        let mut out = Vec::new();
        for s in self.lock_surfaces.iter().filter(|s| s.alive()) {
            out.extend(render_elements_from_surface_tree::<
                R,
                WaylandSurfaceRenderElement<R>,
            >(
                renderer,
                s.wl_surface(),
                (0, 0),
                1.0,
                1.0,
                Kind::Unspecified,
            ));
        }
        out
    }
}

//
// Pointer: constraints (activated when the pointer is already on the surface)
// and relative pointer (motion is sent from input.rs).
//

impl PointerConstraintsHandler for Hypr {
    fn new_constraint(&mut self, surface: &WlSurface, pointer: &PointerHandle<Self>) {
        if pointer.current_focus().as_ref() == Some(surface) {
            with_pointer_constraint(surface, pointer, |c| {
                if let Some(c) = c {
                    c.activate();
                }
            });
        }
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: Point<f64, Logical>,
    ) {
    }
}
delegate_pointer_constraints!(Hypr);
delegate_relative_pointer!(Hypr);

//
// Keyboard shortcuts inhibit: `input.rs` skips binds while it is active.
//

impl KeyboardShortcutsInhibitHandler for Hypr {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // shortcut: activated on creation, not on keyboard focus change.
        inhibitor.activate();
    }
}
delegate_keyboard_shortcuts_inhibit!(Hypr);

impl XdgForeignHandler for Hypr {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}
delegate_xdg_foreign!(Hypr);
