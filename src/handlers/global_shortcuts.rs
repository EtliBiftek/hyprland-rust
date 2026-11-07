//! hyprland-global-shortcuts-v1 (quickshell, end-4 dots) and the `global`
//! dispatcher: `global, appid:name` sends pressed/released to the client that
//! registered that shortcut.

use std::time::Duration;

use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use crate::Hypr;

use super::hyprland_protocols::global_shortcuts::server::{
    hyprland_global_shortcut_v1::{self, HyprlandGlobalShortcutV1},
    hyprland_global_shortcuts_manager_v1::{self, HyprlandGlobalShortcutsManagerV1},
};

struct Shortcut {
    resource: HyprlandGlobalShortcutV1,
    /// `app_id:id`, as used by the `global` dispatcher.
    key: String,
}

#[derive(Default)]
pub struct GlobalShortcutsState {
    shortcuts: Vec<Shortcut>,
}

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, HyprlandGlobalShortcutsManagerV1, _>(1, ());
}

impl GlobalDispatch<HyprlandGlobalShortcutsManagerV1, ()> for Hypr {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<HyprlandGlobalShortcutsManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<HyprlandGlobalShortcutsManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &HyprlandGlobalShortcutsManagerV1,
        request: hyprland_global_shortcuts_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let hyprland_global_shortcuts_manager_v1::Request::RegisterShortcut {
            shortcut,
            id,
            app_id,
            ..
        } = request
        {
            let obj = data_init.init(shortcut, ());
            let key = format!("{app_id}:{id}");
            let shortcuts = &mut state.global_shortcuts.shortcuts;
            if shortcuts
                .iter()
                .any(|s| s.key == key && s.resource.is_alive())
            {
                resource.post_error(
                    hyprland_global_shortcuts_manager_v1::Error::AlreadyTaken,
                    "shortcut already taken",
                );
            } else {
                shortcuts.push(Shortcut { resource: obj, key });
            }
        }
    }
}

impl Dispatch<HyprlandGlobalShortcutV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &HyprlandGlobalShortcutV1,
        _request: hyprland_global_shortcut_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        // Only `destroy`; cleanup happens in `destroyed`.
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &HyprlandGlobalShortcutV1,
        _data: &(),
    ) {
        state
            .global_shortcuts
            .shortcuts
            .retain(|s| &s.resource != resource);
    }
}

impl Hypr {
    /// Sends pressed/released for `app_id:name`. Returns false if nothing is registered.
    pub fn global_shortcut(&mut self, key: &str, pressed: bool) -> bool {
        let t: Duration = self.start_time.elapsed();
        let secs = t.as_secs();
        let (hi, lo, ns) = ((secs >> 32) as u32, secs as u32, t.subsec_nanos());
        let mut found = false;
        for s in self
            .global_shortcuts
            .shortcuts
            .iter()
            .filter(|s| s.key == key)
        {
            found = true;
            if pressed {
                s.resource.pressed(hi, lo, ns);
            } else {
                s.resource.released(hi, lo, ns);
            }
        }
        if !found {
            tracing::debug!("global shortcut `{key}` is not registered");
        }
        found
    }
}
