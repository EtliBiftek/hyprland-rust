//! wp-alpha-modifier-v1: Smithay stores the multiplier in the surface state.
//! shortcuts: nothing reads it back, so surfaces are always drawn opaque.

use smithay::{
    delegate_alpha_modifier, reexports::wayland_server::DisplayHandle,
    wayland::alpha_modifier::AlphaModifierState,
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) -> AlphaModifierState {
    AlphaModifierState::new::<Hypr>(dh)
}

delegate_alpha_modifier!(Hypr);
