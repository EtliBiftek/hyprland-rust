//! wp-security-context-v1 (flatpak, sandboxed apps): the sandbox engine gets a
//! listening socket; clients that connect through it are tagged in `ClientState`.
//! shortcuts: tagged clients only lose the privileged globals that filter on
//! `ClientState::trusted` (virtual keyboard, input method, security context
//! itself); the wlr/hyprland screencopy, data-control and similar globals are
//! still visible to them.

use std::sync::Arc;

use smithay::{
    delegate_security_context,
    reexports::wayland_server::DisplayHandle,
    wayland::security_context::{
        SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
        SecurityContextState,
    },
};

use crate::{state::ClientState, Hypr};

pub fn init(dh: &DisplayHandle) -> SecurityContextState {
    SecurityContextState::new::<Hypr, _>(dh, ClientState::trusted)
}

impl SecurityContextHandler for Hypr {
    fn context_created(&mut self, source: SecurityContextListenerSource, context: SecurityContext) {
        let res = self
            .wm
            .loop_handle
            .insert_source(source, move |stream, _, data| {
                let client = Arc::new(ClientState {
                    security_context: Some(context.clone()),
                    ..Default::default()
                });
                if let Err(e) = data.state.display_handle.insert_client(stream, client) {
                    tracing::warn!("sandboxed client rejected: {e}");
                }
            });
        if let Err(e) = res {
            tracing::warn!("security context listener not registered: {e}");
        }
    }
}

delegate_security_context!(Hypr);
