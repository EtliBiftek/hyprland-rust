mod compositor;
pub mod layer_shell;
pub mod protocols;
mod xdg_shell;

use crate::Hypr;

//
// Wl Seat
//

use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
    ServerDndGrabHandler,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::{delegate_data_device, delegate_output, delegate_seat, delegate_xdg_activation};

impl SeatHandler for Hypr {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Hypr> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        // Stored only: the cursor is still drawn as a plain square.
        self.cursor_status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);
    }
}

delegate_seat!(Hypr);

//
// Wl Data Device
//

impl SelectionHandler for Hypr {
    type SelectionUserData = ();
}

impl DataDeviceHandler for Hypr {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for Hypr {}
impl ServerDndGrabHandler for Hypr {}

delegate_data_device!(Hypr);

//
// Wl Output & Xdg Output
//

impl OutputHandler for Hypr {}
delegate_output!(Hypr);

//
// Xdg Activation: a request to activate an unfocused window marks it urgent.
//

impl XdgActivationHandler for Hypr {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        _data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if let Some((_, window)) = self.window_for_surface(&surface) {
            if self.focused_window().as_ref() != Some(&window) {
                let addr = crate::ipc::window_addr(&window);
                self.ipc.emit("urgent", addr.trim_start_matches("0x"));
            }
        }
    }
}

delegate_xdg_activation!(Hypr);
