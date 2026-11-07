//! ext-workspace-v1 (waybar `ext/workspaces`, shells): one group holding every
//! normal workspace, diffed against the window manager every frame like
//! `sync_toplevels`.
//! shortcuts: only `activate` is honoured (applied at once, not at `commit`);
//! deactivate/assign/remove/create_workspace are ignored; special workspaces
//! are not listed; a single group (one output).

use smithay::reexports::{
    wayland_protocols::ext::workspace::v1::server::{
        ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
        ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
        ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
    },
    wayland_server::{
        backend::ClientId, Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
};

use crate::{config::Dispatcher, selectors::WsSel, Hypr};

pub fn init(dh: &DisplayHandle) {
    dh.create_global::<Hypr, ExtWorkspaceManagerV1, _>(1, ());
}

struct Handle {
    id: i32,
    res: ExtWorkspaceHandleV1,
    name: String,
    active: bool,
}

struct Mgr {
    res: ExtWorkspaceManagerV1,
    group: ExtWorkspaceGroupHandleV1,
    handles: Vec<Handle>,
}

#[derive(Default)]
pub struct WorkspaceState {
    mgrs: Vec<Mgr>,
}

impl GlobalDispatch<ExtWorkspaceManagerV1, ()> for Hypr {
    fn bind(
        state: &mut Self,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let res = data_init.init(resource, ());
        let Ok(group) =
            client.create_resource::<ExtWorkspaceGroupHandleV1, _, Hypr>(handle, res.version(), ())
        else {
            return;
        };
        res.workspace_group(&group);
        group.capabilities(ext_workspace_group_handle_v1::GroupCapabilities::empty());
        for o in state.space.outputs() {
            for wl in o.client_outputs(client) {
                group.output_enter(&wl);
            }
        }
        state.extra.workspaces.mgrs.push(Mgr {
            res,
            group,
            handles: Vec::new(),
        });
        // The workspaces follow in `sync_ext_workspaces`, which also sends `done`.
        state.sync_ext_workspaces();
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: ext_workspace_manager_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_workspace_manager_v1::Request::Stop = request {
            state.extra.workspaces.mgrs.retain(|m| &m.res != resource);
            resource.finished();
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Hypr {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ExtWorkspaceGroupHandleV1,
        _request: ext_workspace_group_handle_v1::Request,
        _data: &(),
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceHandleV1, i32> for Hypr {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ExtWorkspaceHandleV1,
        request: ext_workspace_handle_v1::Request,
        id: &i32,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_workspace_handle_v1::Request::Activate = request {
            state.dispatch(&Dispatcher::Workspace(WsSel::Id(*id)));
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: ClientId,
        resource: &ExtWorkspaceHandleV1,
        _data: &i32,
    ) {
        for m in &mut state.extra.workspaces.mgrs {
            m.handles.retain(|h| &h.res != resource);
        }
    }
}

impl Hypr {
    /// Announces new workspaces, sends name/active changes and removals.
    pub fn sync_ext_workspaces(&mut self) {
        let dh = self.display_handle.clone();
        let want: Vec<(i32, String, bool)> = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(_, ws)| ws.id > 0)
            .map(|(i, ws)| (ws.id, ws.name.clone(), i == self.active_workspace))
            .collect();
        let mgrs = &mut self.extra.workspaces.mgrs;
        mgrs.retain(|m| m.res.is_alive());
        for m in mgrs.iter_mut() {
            let mut changed = false;
            m.handles.retain(|h| {
                let keep = want.iter().any(|(id, ..)| *id == h.id);
                if !keep {
                    m.group.workspace_leave(&h.res);
                    h.res.removed();
                    changed = true;
                }
                keep
            });
            let Ok(client) = dh.get_client(m.res.id()) else {
                continue;
            };
            for (id, name, active) in &want {
                let state_bits = |a: bool| {
                    if a {
                        ext_workspace_handle_v1::State::Active
                    } else {
                        ext_workspace_handle_v1::State::empty()
                    }
                };
                if let Some(h) = m.handles.iter_mut().find(|h| h.id == *id) {
                    if h.name != *name {
                        h.name = name.clone();
                        h.res.name(name.clone());
                        changed = true;
                    }
                    if h.active != *active {
                        h.active = *active;
                        h.res.state(state_bits(*active));
                        changed = true;
                    }
                    continue;
                }
                let Ok(res) = client.create_resource::<ExtWorkspaceHandleV1, _, Hypr>(
                    &dh,
                    m.res.version(),
                    *id,
                ) else {
                    continue;
                };
                m.res.workspace(&res);
                res.id(id.to_string());
                res.name(name.clone());
                res.coordinates(Vec::new());
                res.state(state_bits(*active));
                res.capabilities(ext_workspace_handle_v1::WorkspaceCapabilities::Activate);
                m.group.workspace_enter(&res);
                m.handles.push(Handle {
                    id: *id,
                    res,
                    name: name.clone(),
                    active: *active,
                });
                changed = true;
            }
            if changed {
                m.res.done();
            }
        }
    }
}
