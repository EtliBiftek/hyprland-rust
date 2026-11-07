//! text-input-v3 (GTK/Qt/Electron text fields). Keyboard focus changes tell the
//! seat's text input about the focused surface (see `SeatHandler::focus_changed`).

use smithay::{
    delegate_text_input_manager, reexports::wayland_server::DisplayHandle,
    wayland::text_input::TextInputManagerState,
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) -> TextInputManagerState {
    TextInputManagerState::new::<Hypr>(dh)
}

delegate_text_input_manager!(Hypr);
