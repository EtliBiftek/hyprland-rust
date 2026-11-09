use crate::{grabs::resize_grab, state::ClientState, Hypr};
use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_shm,
    reexports::wayland_server::{
        protocol::{wl_buffer, wl_surface::WlSurface},
        Client,
    },
    wayland::{
        buffer::BufferHandler,
        compositor::{
            get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler,
            CompositorState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use super::xdg_shell;

impl CompositorHandler for Hypr {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(x) = client.get_data::<smithay::xwayland::XWaylandClientData>() {
            return &x.compositor_state;
        }
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            // All workspaces, not just `space`: windows on hidden workspaces
            // still need on_commit(), rules and their initial configure.
            if let Some((_, window)) = self.window_for_surface(&root) {
                let old_geo = window.geometry();
                window.on_commit();
                // The client-side decoration offset/size is only known after the
                // client commits, so re-place tiled windows when it changes.
                if window.geometry() != old_geo {
                    self.relayout();
                }
                self.rules_on_commit(&window);
                self.check_window_mapped(&window);
                // A null buffer unmaps the toplevel: it leaves the layout
                // until it maps again (like Hyprland).
                if window.toplevel().is_some()
                    && crate::ipc::is_mapped(&window)
                    && !has_buffer(&root)
                {
                    self.unmap_toplevel(&window);
                }
            } else if let Some(w) = self.unmapped_toplevel(&root) {
                w.on_commit();
                if has_buffer(&root) {
                    self.wm.unmapped.retain(|x| x != &w);
                    self.add_window(w.clone());
                    self.check_window_mapped(&w);
                }
            }
        };

        self.layer_commit(surface);
        self.lock_commit(surface);
        let toplevel = self
            .window_for_surface(surface)
            .map(|(_, w)| w)
            .or_else(|| self.unmapped_toplevel(surface))
            .and_then(|w| w.toplevel().cloned());
        xdg_shell::handle_commit(&mut self.popups, toplevel.as_ref(), surface);
        resize_grab::handle_commit(&mut self.space, surface);
    }
}

fn has_buffer(s: &WlSurface) -> bool {
    smithay::backend::renderer::utils::with_renderer_surface_state(s, |st| st.buffer().is_some())
        .unwrap_or(false)
}

impl Hypr {
    pub fn unmapped_toplevel(&self, s: &WlSurface) -> Option<smithay::desktop::Window> {
        self.wm
            .unmapped
            .iter()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == s))
            .cloned()
    }

    /// Takes a toplevel that committed a null buffer out of its workspace;
    /// it comes back through `add_window` on its next buffer.
    fn unmap_toplevel(&mut self, w: &smithay::desktop::Window) {
        self.remove_window_obj(w);
        crate::ipc::clear_mapped(w);
        self.wm.unmapped.push(w.clone());
    }
}

impl BufferHandler for Hypr {
    fn buffer_destroyed(&mut self, _buffer: &wl_buffer::WlBuffer) {}
}

impl ShmHandler for Hypr {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

delegate_compositor!(Hypr);
delegate_shm!(Hypr);
