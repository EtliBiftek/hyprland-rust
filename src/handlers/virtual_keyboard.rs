//! virtual-keyboard-unstable-v1 (wtype, fcitx5, on-screen keyboards): Smithay feeds
//! the keys into the seat keyboard. Sandboxed (security-context) clients do not see it.

use smithay::{
    delegate_virtual_keyboard_manager, reexports::wayland_server::DisplayHandle,
    wayland::virtual_keyboard::VirtualKeyboardManagerState,
};

use crate::{state::ClientState, Hypr};

pub fn init(dh: &DisplayHandle) -> VirtualKeyboardManagerState {
    VirtualKeyboardManagerState::new::<Hypr, _>(dh, ClientState::trusted)
}

delegate_virtual_keyboard_manager!(Hypr);
