//! wlr-foreign-toplevel-management-unstable-v1 (waybar `wlr/taskbar`, docks) and
//! ext-foreign-toplevel-list-v1.
//!
//! `sync_toplevels` runs every frame (from `send_frames`) and diffs the mapped
//! windows against what was announced, so title, app_id, focus, fullscreen and
//! maximize changes need no hooks in the window manager.
//! shortcuts: `set_minimized`/`set_rectangle` are ignored (no minimize in this
//! WM), `parent` is not sent, one output only.

use smithay::{
    delegate_foreign_toplevel_list,
    desktop::Window,
    reexports::{
        wayland_protocols_wlr::foreign_toplevel::v1::server::{
            zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
            zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        },
    },
    wayland::foreign_toplevel_list::{
        ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
    },
};

use crate::{
    config::Dispatcher,
    ipc::{class_title, is_mapped, window_addr},
    selectors::WinSel,
    Hypr,
};

/// What clients were last told about a window.
#[derive(Clone, PartialEq, Default)]
struct Snap {
    title: String,
    app_id: String,
    active: bool,
    fullscreen: bool,
    maximized: bool,
}

impl Snap {
    /// `state` array of the wlr protocol: maximized 0, minimized 1, activated 2, fullscreen 3.
    fn state_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        for (on, id) in [
            (self.maximized, 0u32),
            (self.active, 2),
            (self.fullscreen, 3),
        ] {
            if on {
                v.extend_from_slice(&id.to_ne_bytes());
            }
        }
        v
    }
}

struct Entry {
    window: Window,
    ext: ForeignToplevelHandle,
    wlr: Vec<ZwlrForeignToplevelHandleV1>,
    last: Snap,
}

pub struct ToplevelState {
    ext: ForeignToplevelListState,
    managers: Vec<ZwlrForeignToplevelManagerV1>,
    entries: Vec<Entry>,
}

impl ToplevelState {
    pub fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<Hypr, ZwlrForeignToplevelManagerV1, _>(3, ());
        Self {
            ext: ForeignToplevelListState::new::<Hypr>(dh),
            managers: Vec::new(),
            entries: Vec::new(),
        }
    }
}

impl ForeignToplevelListHandler for Hypr {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.toplevels.ext
    }
}
delegate_foreign_toplevel_list!(Hypr);

/// Creates the handle for `mgr`'s client and sends the full state.
fn announce(
    dh: &DisplayHandle,
    mgr: &ZwlrForeignToplevelManagerV1,
    window: &Window,
    snap: &Snap,
    output: Option<&smithay::output::Output>,
) -> Option<ZwlrForeignToplevelHandleV1> {
    let client = dh.get_client(mgr.id()).ok()?;
    let handle = client
        .create_resource::<ZwlrForeignToplevelHandleV1, _, Hypr>(
            dh,
            mgr.version(),
            window_addr(window),
        )
        .ok()?;
    mgr.toplevel(&handle);
    handle.title(snap.title.clone());
    handle.app_id(snap.app_id.clone());
    if let Some(o) = output {
        for wl in o.client_outputs(&client) {
            handle.output_enter(&wl);
        }
    }
    handle.state(snap.state_bytes());
    handle.done();
    Some(handle)
}

impl GlobalDispatch<ZwlrForeignToplevelManagerV1, ()> for Hypr {
    fn bind(
        state: &mut Self,
        handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrForeignToplevelManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let mgr = data_init.init(resource, ());
        let output = state.space.outputs().next().cloned();
        let mut entries = std::mem::take(&mut state.toplevels.entries);
        for e in &mut entries {
            if let Some(h) = announce(handle, &mgr, &e.window, &e.last, output.as_ref()) {
                e.wlr.push(h);
            }
        }
        state.toplevels.entries = entries;
        state.toplevels.managers.push(mgr);
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrForeignToplevelManagerV1,
        request: zwlr_foreign_toplevel_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Request::Stop = request {
            state.toplevels.managers.retain(|m| m != resource);
            resource.finished();
        }
    }
}

impl Dispatch<ZwlrForeignToplevelHandleV1, String> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwlrForeignToplevelHandleV1,
        request: zwlr_foreign_toplevel_handle_v1::Request,
        addr: &String,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Request as R;
        let Some((_, window)) = state.find_window(&WinSel::Address(addr.to_ascii_lowercase()))
        else {
            return;
        };
        match request {
            R::Activate { .. } => state.dispatch(&Dispatcher::FocusWindow(WinSel::Address(
                addr.to_ascii_lowercase(),
            ))),
            R::Close => {
                if let Some(t) = window.toplevel() {
                    t.send_close();
                } else if let Some(x) = window.x11_surface() {
                    let _ = x.close();
                }
            }
            R::SetMaximized => state.set_wm_slot(&window, false, true),
            R::UnsetMaximized => state.set_wm_slot(&window, false, false),
            R::SetFullscreen { .. } => state.set_wm_slot(&window, true, true),
            R::UnsetFullscreen => state.set_wm_slot(&window, true, false),
            _ => {}
        }
    }
}

impl Hypr {
    /// Sets or clears the fullscreen/maximized slot of the window's workspace.
    fn set_wm_slot(&mut self, w: &Window, fullscreen: bool, on: bool) {
        let Some(i) = self.ws_of(w) else {
            return;
        };
        let ws = &mut self.workspaces[i];
        let slot = if fullscreen {
            &mut ws.fullscreen
        } else {
            &mut ws.maximized
        };
        if on {
            *slot = Some(w.clone());
        } else if slot.as_ref() == Some(w) {
            *slot = None;
        }
        self.relayout();
    }

    /// Announces new windows, sends changes and closes gone windows.
    pub fn sync_toplevels(&mut self) {
        self.toplevels.managers.retain(|m| m.is_alive());
        let open: Vec<Window> = self
            .workspaces
            .iter()
            .flat_map(|ws| ws.windows())
            .filter(is_mapped)
            .collect();

        let mut i = 0;
        while i < self.toplevels.entries.len() {
            if open.contains(&self.toplevels.entries[i].window) {
                i += 1;
                continue;
            }
            let e = self.toplevels.entries.remove(i);
            for h in &e.wlr {
                h.closed();
            }
            self.toplevels.ext.remove_toplevel(&e.ext);
        }

        let active = self.focused_window();
        let output = self.space.outputs().next().cloned();
        let dh = self.display_handle.clone();
        for w in open {
            let (app_id, title) = class_title(&w);
            let (fullscreen, maximized) = self.ws_of(&w).map_or((false, false), |i| {
                let ws = &self.workspaces[i];
                (
                    ws.fullscreen.as_ref() == Some(&w),
                    ws.maximized.as_ref() == Some(&w),
                )
            });
            let snap = Snap {
                title,
                app_id,
                active: active.as_ref() == Some(&w),
                fullscreen,
                maximized,
            };
            let tl = &mut self.toplevels;
            let Some(e) = tl.entries.iter_mut().find(|e| e.window == w) else {
                let ext = tl.ext.new_toplevel::<Hypr>(&snap.title, &snap.app_id);
                let wlr = tl
                    .managers
                    .iter()
                    .filter_map(|m| announce(&dh, m, &w, &snap, output.as_ref()))
                    .collect();
                tl.entries.push(Entry {
                    window: w,
                    ext,
                    wlr,
                    last: snap,
                });
                continue;
            };
            if e.last == snap {
                continue;
            }
            if e.last.title != snap.title {
                e.ext.send_title(&snap.title);
            }
            if e.last.app_id != snap.app_id {
                e.ext.send_app_id(&snap.app_id);
            }
            e.ext.send_done();
            e.wlr.retain(|h| h.is_alive());
            for h in &e.wlr {
                if e.last.title != snap.title {
                    h.title(snap.title.clone());
                }
                if e.last.app_id != snap.app_id {
                    h.app_id(snap.app_id.clone());
                }
                h.state(snap.state_bytes());
                h.done();
            }
            e.last = snap;
        }
    }
}
