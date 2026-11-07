//! hyprland-lock-notify-v1 (hypridle): `locked` / `unlocked` when an
//! ext-session-lock client takes or drops the lock.

use smithay::reexports::wayland_server::{
    backend::ClientId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use super::hyprland_protocols::lock_notify::server::{
    hyprland_lock_notification_v1::{self, HyprlandLockNotificationV1},
    hyprland_lock_notifier_v1::{self, HyprlandLockNotifierV1},
};
use crate::Hypr;

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandLockNotifierV1, _>(1, ());
}

#[derive(Default)]
pub struct LockNotifyState {
    notifications: Vec<HyprlandLockNotificationV1>,
}

impl GlobalDispatch<HyprlandLockNotifierV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandLockNotifierV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<HyprlandLockNotifierV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &HyprlandLockNotifierV1,
        request: hyprland_lock_notifier_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let hyprland_lock_notifier_v1::Request::GetLockNotification { id } = request {
            let n = data_init.init(id, ());
            if state.session_locked {
                n.locked();
            }
            state.extra.lock_notify.notifications.push(n);
        }
    }
}

impl Dispatch<HyprlandLockNotificationV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &HyprlandLockNotificationV1,
        _request: hyprland_lock_notification_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Only `destroy`.
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        resource: &HyprlandLockNotificationV1,
        _data: &(),
    ) {
        state
            .extra
            .lock_notify
            .notifications
            .retain(|n| n != resource);
    }
}

impl Hypr {
    /// Tells every notification object that the session lock changed.
    pub fn lock_notify(&mut self, locked: bool) {
        for n in self
            .extra
            .lock_notify
            .notifications
            .iter()
            .filter(|n| n.is_alive())
        {
            if locked {
                n.locked();
            } else {
                n.unlocked();
            }
        }
    }
}
