//! input-method-v2 (fcitx5, ibus). Popups (candidate windows) are tracked by the
//! popup manager like xdg popups.
//! shortcuts: the popup surfaces are tracked and positioned but never drawn by the
//! backends, so a candidate window is invisible until they render `popups`.

use smithay::{
    delegate_input_method_manager,
    desktop::{PopupKind, PopupManager},
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle},
    utils::{Logical, Rectangle},
    wayland::input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
};

use crate::{state::ClientState, Hypr};

pub fn init(dh: &DisplayHandle) -> InputMethodManagerState {
    InputMethodManagerState::new::<Hypr, _>(dh, ClientState::trusted)
}

impl InputMethodHandler for Hypr {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(e) = self.popups.track_popup(PopupKind::from(surface)) {
            tracing::warn!("input method popup not tracked: {e}");
        }
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|p| p.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {}

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.window_for_surface(parent)
            .and_then(|(_, w)| self.space.element_geometry(&w))
            .unwrap_or_default()
    }
}

delegate_input_method_manager!(Hypr);
