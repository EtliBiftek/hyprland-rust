//! tablet-v2 (zwp_tablet_manager_v2). `TabletSeatHandler` lives in `protocols.rs`.
//! shortcuts: the global exists so clients can bind and create tablet seats, but
//! no tablet device or tool events are produced (no libinput tablet plumbing).

use smithay::{
    delegate_tablet_manager, reexports::wayland_server::DisplayHandle,
    wayland::tablet_manager::TabletManagerState,
};

use crate::Hypr;

pub fn init(dh: &DisplayHandle) -> TabletManagerState {
    TabletManagerState::new::<Hypr>(dh)
}

delegate_tablet_manager!(Hypr);
