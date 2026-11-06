//! Hyprland-compatible IPC: request socket (`.socket.sock`) and event socket
//! (`.socket2.sock`) under `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`.
//!
//! The top half (request parsing, info structs, JSON/text rendering) is pure
//! and free of Smithay types so it can be unit-tested. The bottom half builds
//! a `Snapshot` from `Hypr` and runs the sockets.
//!
//! Placeholder fields (always a constant): clients `pseudo`, `xwayland`,
//! `pinned`, `fullscreenClient`, `grouped`, `tags`, `swallowing`,
//! `inhibitingIdle`, `xdgTag`, `xdgDescription`, `initialClass`/`initialTitle`
//! (current values), `hidden`, `mapped`; workspaces `ispersistent`,
//! `tiledLayout`; monitors `description`, `make`, `model`, `serial`,
//! `physicalWidth/Height`, `specialWorkspace`, `reserved`, `transform`, `vrr`,
//! `solitary`, `activelyTearing`, `directScanoutTo`, `disabled`,
//! `currentFormat`, `mirrorOf`; version `commit*`, `tag`, `flags`; binds
//! `keycode` and all boolean flags.

use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use serde::Serialize;
use smithay::{
    desktop::Window,
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_server::Resource,
    },
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::{
    config::{Direction, Dispatcher},
    state::Hypr,
    CalloopData,
};

// ---------------------------------------------------------------- pure part

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub json: bool,
    pub cmd: String,
}

/// Parses `[flags/]command args`. Flags are `j` (JSON) and Hyprland's
/// `a`/`r`/`c` (accepted, ignored).
pub fn parse_request(s: &str) -> Request {
    let s = s.trim();
    if let Some((flags, rest)) = s.split_once('/') {
        if flags.chars().all(|c| "jarc".contains(c)) {
            return Request {
                json: flags.contains('j'),
                cmd: rest.trim().to_string(),
            };
        }
    }
    Request {
        json: false,
        cmd: s.to_string(),
    }
}

/// `[[BATCH]]a;b` -> each command parsed on its own.
pub fn parse_batch(s: &str) -> Option<Vec<Request>> {
    let rest = s.trim().strip_prefix("[[BATCH]]")?;
    Some(
        rest.split(';')
            .filter(|c| !c.trim().is_empty())
            .map(parse_request)
            .collect(),
    )
}

#[derive(Serialize, Clone, Debug)]
pub struct WsRef {
    pub id: i32,
    pub name: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub address: String,
    pub mapped: bool,
    pub hidden: bool,
    pub at: [i32; 2],
    pub size: [i32; 2],
    pub workspace: WsRef,
    pub floating: bool,
    pub pseudo: bool,
    pub monitor: i32,
    pub class: String,
    pub title: String,
    pub initial_class: String,
    pub initial_title: String,
    pub pid: i32,
    pub xwayland: bool,
    pub pinned: bool,
    pub fullscreen: i32,
    pub fullscreen_client: i32,
    pub grouped: Vec<String>,
    pub tags: Vec<String>,
    pub swallowing: String,
    #[serde(rename = "focusHistoryID")]
    pub focus_history_id: i32,
    pub inhibiting_idle: bool,
    pub xdg_tag: String,
    pub xdg_description: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct WorkspaceInfo {
    pub id: i32,
    pub name: String,
    pub monitor: String,
    #[serde(rename = "monitorID")]
    pub monitor_id: i32,
    pub windows: i32,
    #[serde(rename = "hasfullscreen")]
    pub has_fullscreen: bool,
    #[serde(rename = "lastwindow")]
    pub last_window: String,
    #[serde(rename = "lastwindowtitle")]
    pub last_window_title: String,
    #[serde(rename = "ispersistent")]
    pub is_persistent: bool,
    #[serde(rename = "tiledLayout")]
    pub tiled_layout: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub width: i32,
    pub height: i32,
    pub physical_width: i32,
    pub physical_height: i32,
    pub refresh_rate: f64,
    pub x: i32,
    pub y: i32,
    pub active_workspace: WsRef,
    pub special_workspace: WsRef,
    pub reserved: [i32; 4],
    pub scale: f64,
    pub transform: i32,
    pub focused: bool,
    pub dpms_status: bool,
    pub vrr: bool,
    pub solitary: String,
    pub actively_tearing: bool,
    pub direct_scanout_to: String,
    pub disabled: bool,
    pub current_format: String,
    pub mirror_of: String,
    pub available_modes: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct BindInfo {
    pub locked: bool,
    pub mouse: bool,
    pub release: bool,
    pub repeat: bool,
    #[serde(rename = "longPress")]
    pub long_press: bool,
    pub non_consuming: bool,
    pub has_description: bool,
    pub modmask: u32,
    pub submap: String,
    pub key: String,
    pub keycode: i32,
    pub catch_all: bool,
    pub description: String,
    pub dispatcher: String,
    pub arg: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct VersionInfo {
    pub branch: String,
    pub commit: String,
    pub version: String,
    pub dirty: bool,
    pub commit_message: String,
    pub commit_date: String,
    pub tag: String,
    pub commits: String,
    pub flags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub clients: Vec<ClientInfo>,
    pub workspaces: Vec<WorkspaceInfo>,
    pub monitors: Vec<MonitorInfo>,
    pub binds: Vec<BindInfo>,
    pub active_workspace: WsRef,
    /// Address of the focused window.
    pub active_window: Option<String>,
}

fn b(v: bool) -> i32 {
    v as i32
}

fn client_text(c: &ClientInfo) -> String {
    format!(
        "Window {} -> {}:\n\tmapped: {}\n\thidden: {}\n\tat: {},{}\n\tsize: {},{}\n\tworkspace: {} ({})\n\tfloating: {}\n\tpseudo: {}\n\tmonitor: {}\n\tclass: {}\n\ttitle: {}\n\tinitialClass: {}\n\tinitialTitle: {}\n\tpid: {}\n\txwayland: {}\n\tpinned: {}\n\tfullscreen: {}\n\tfullscreenClient: {}\n\tgrouped: {}\n\ttags: {}\n\tswallowing: {}\n\tfocusHistoryID: {}\n\tinhibitingIdle: {}\n\txdgTag: {}\n\txdgDescription: {}\n\n",
        c.address.trim_start_matches("0x"),
        c.title,
        b(c.mapped),
        b(c.hidden),
        c.at[0],
        c.at[1],
        c.size[0],
        c.size[1],
        c.workspace.id,
        c.workspace.name,
        b(c.floating),
        b(c.pseudo),
        c.monitor,
        c.class,
        c.title,
        c.initial_class,
        c.initial_title,
        c.pid,
        b(c.xwayland),
        b(c.pinned),
        c.fullscreen,
        c.fullscreen_client,
        c.grouped.join(","),
        c.tags.join(","),
        c.swallowing,
        c.focus_history_id,
        b(c.inhibiting_idle),
        c.xdg_tag,
        c.xdg_description,
    )
}

fn workspace_text(w: &WorkspaceInfo) -> String {
    format!(
        "workspace ID {} ({}) on monitor {}:\n\tmonitorID: {}\n\twindows: {}\n\thasfullscreen: {}\n\tlastwindow: {}\n\tlastwindowtitle: {}\n\tispersistent: {}\n\ttiledLayout: {}\n\n",
        w.id,
        w.name,
        w.monitor,
        w.monitor_id,
        w.windows,
        b(w.has_fullscreen),
        w.last_window,
        w.last_window_title,
        b(w.is_persistent),
        w.tiled_layout,
    )
}

fn monitor_text(m: &MonitorInfo) -> String {
    format!(
        "Monitor {} (ID {}):\n\t{}x{}@{:.5} at {}x{}\n\tdescription: {}\n\tmake: {}\n\tmodel: {}\n\tserial: {}\n\tactive workspace: {} ({})\n\tspecial workspace: {} ({})\n\treserved: {} {} {} {}\n\tscale: {:.2}\n\ttransform: {}\n\tfocused: {}\n\tdpmsStatus: {}\n\tvrr: {}\n\tsolitary: {}\n\tactivelyTearing: {}\n\tdirectScanoutTo: {}\n\tdisabled: {}\n\tcurrentFormat: {}\n\tmirrorOf: {}\n\tavailableModes: {}\n\n",
        m.name,
        m.id,
        m.width,
        m.height,
        m.refresh_rate,
        m.x,
        m.y,
        m.description,
        m.make,
        m.model,
        m.serial,
        m.active_workspace.id,
        m.active_workspace.name,
        m.special_workspace.id,
        m.special_workspace.name,
        m.reserved[0],
        m.reserved[1],
        m.reserved[2],
        m.reserved[3],
        m.scale,
        m.transform,
        if m.focused { "yes" } else { "no" },
        b(m.dpms_status),
        b(m.vrr),
        m.solitary,
        m.actively_tearing,
        m.direct_scanout_to,
        m.disabled,
        m.current_format,
        m.mirror_of,
        m.available_modes.join(" "),
    )
}

fn bind_text(b: &BindInfo) -> String {
    format!(
        "bind\n\tmodmask: {}\n\tsubmap: {}\n\tkey: {}\n\tkeycode: {}\n\tcatch_all: {}\n\tdescription: {}\n\tdispatcher: {}\n\targ: {}\n\n",
        b.modmask, b.submap, b.key, b.keycode, b.catch_all, b.description, b.dispatcher, b.arg,
    )
}

fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

pub fn version_info() -> VersionInfo {
    VersionInfo {
        branch: "main".into(),
        commit: String::new(),
        version: env!("CARGO_PKG_VERSION").into(),
        dirty: false,
        commit_message: String::new(),
        commit_date: String::new(),
        tag: String::new(),
        commits: "0".into(),
        flags: Vec::new(),
    }
}

/// Answers a read-only query. `None` means the command is not a query.
pub fn query(snap: &Snapshot, cmd: &str, json_out: bool) -> Option<String> {
    let name = cmd.split_whitespace().next()?;
    Some(match (name, json_out) {
        ("version", true) => json(&version_info()),
        ("version", false) => {
            let v = version_info();
            format!(
                "Hyprland {} (hyprland-rust) built from branch {} at commit {}.\nflags: \n",
                v.version, v.branch, v.commit
            )
        }
        ("clients", true) => json(&snap.clients),
        ("clients", false) => snap.clients.iter().map(client_text).collect(),
        ("workspaces", true) => json(&snap.workspaces),
        ("workspaces", false) => snap.workspaces.iter().map(workspace_text).collect(),
        ("monitors", true) => json(&snap.monitors),
        ("monitors", false) => snap.monitors.iter().map(monitor_text).collect(),
        ("binds", true) => json(&snap.binds),
        ("binds", false) => snap.binds.iter().map(bind_text).collect(),
        ("activeworkspace", _) => {
            let ws = snap
                .workspaces
                .iter()
                .find(|w| w.id == snap.active_workspace.id);
            match (ws, json_out) {
                (Some(w), true) => json(w),
                (Some(w), false) => workspace_text(w),
                (None, true) => "{}".into(),
                (None, false) => "Invalid".into(),
            }
        }
        ("activewindow", _) => {
            let c = snap
                .clients
                .iter()
                .find(|c| Some(&c.address) == snap.active_window.as_ref());
            match (c, json_out) {
                (Some(c), true) => json(c),
                (Some(c), false) => client_text(c),
                (None, true) => "{}".into(),
                (None, false) => "Invalid".into(),
            }
        }
        _ => return None,
    })
}

/// Name and argument of a dispatcher as `hyprctl binds` shows them.
pub fn dispatcher_name_arg(d: &Dispatcher) -> (&'static str, String) {
    let dir = |d: &Direction| {
        match d {
            Direction::Left => "l",
            Direction::Right => "r",
            Direction::Up => "u",
            Direction::Down => "d",
        }
        .to_string()
    };
    match d {
        Dispatcher::Exec(c) => ("exec", c.clone()),
        Dispatcher::KillActive => ("killactive", String::new()),
        Dispatcher::Exit => ("exit", String::new()),
        Dispatcher::Workspace(n) => ("workspace", n.to_string()),
        Dispatcher::MoveToWorkspace(n) => ("movetoworkspace", n.to_string()),
        Dispatcher::ToggleFloating => ("togglefloating", String::new()),
        Dispatcher::Fullscreen => ("fullscreen", String::new()),
        Dispatcher::MoveFocus(d) => ("movefocus", dir(d)),
        Dispatcher::CycleNext => ("cyclenext", String::new()),
        Dispatcher::ToggleSplit => ("togglesplit", String::new()),
        other => other.name_arg(),
    }
}

// ----------------------------------------------------------- Smithay part

/// Stable per-window id, stored in the window's user data.
struct WinId(u64);
static NEXT_WIN_ID: AtomicU64 = AtomicU64::new(0x5580_0000);

pub fn window_addr(w: &Window) -> String {
    let id = w
        .user_data()
        .get_or_insert(|| WinId(NEXT_WIN_ID.fetch_add(0x40, Ordering::Relaxed)));
    format!("0x{:x}", id.0)
}

pub fn class_title(w: &Window) -> (String, String) {
    let Some(t) = w.toplevel() else {
        return Default::default();
    };
    with_states(t.wl_surface(), |s| {
        let d = s
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .unwrap()
            .lock()
            .unwrap();
        (
            d.app_id.clone().unwrap_or_default(),
            d.title.clone().unwrap_or_default(),
        )
    })
}

pub struct Ipc {
    dir: Option<PathBuf>,
    events: Vec<UnixStream>,
    last_active: Option<String>,
}

impl Ipc {
    /// Sends `EVENT>>DATA\n` to all event clients, dropping dead ones.
    pub fn emit(&mut self, event: &str, data: &str) {
        let line = format!("{event}>>{data}\n");
        self.events
            .retain_mut(|s| s.write_all(line.as_bytes()).is_ok());
    }

    /// Emits `activewindow`/`activewindowv2` if the focused window changed.
    pub fn emit_active(&mut self, w: Option<&Window>) {
        let addr = w.map(window_addr);
        if addr == self.last_active {
            return;
        }
        let (class, title) = w.map(class_title).unwrap_or_default();
        self.emit("activewindow", &format!("{class},{title}"));
        self.emit(
            "activewindowv2",
            addr.as_deref()
                .map(|a| a.trim_start_matches("0x"))
                .unwrap_or(""),
        );
        self.last_active = addr;
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn bind_socket(path: PathBuf) -> std::io::Result<UnixListener> {
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(path)?;
    l.set_nonblocking(true)?;
    Ok(l)
}

/// Creates the socket dir, exports `HYPRLAND_INSTANCE_SIGNATURE` and registers
/// both listeners in the event loop. Failures disable IPC (logged).
pub fn init(handle: &LoopHandle<'_, CalloopData>) -> Ipc {
    let mut ipc = Ipc {
        dir: None,
        events: Vec::new(),
        last_active: None,
    };
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let sig = format!("{:x}_{}_{}", now, now, std::process::id());
    let dir = runtime.join("hypr").join(&sig);
    let setup = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let req = bind_socket(dir.join(".socket.sock"))?;
        let ev = bind_socket(dir.join(".socket2.sock"))?;
        handle
            .insert_source(
                Generic::new(req, Interest::READ, Mode::Level),
                |_, l, data| {
                    serve_requests(&mut data.state, l);
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|_| std::io::Error::other("insert_source failed"))?;
        handle
            .insert_source(
                Generic::new(ev, Interest::READ, Mode::Level),
                |_, l, data| {
                    while let Ok((s, _)) = l.accept() {
                        if s.set_nonblocking(true).is_ok() {
                            data.state.ipc.events.push(s);
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|_| std::io::Error::other("insert_source failed"))?;
        Ok(())
    };
    match setup() {
        Ok(()) => {
            std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", &sig);
            tracing::info!("IPC at {}", dir.display());
            ipc.dir = Some(dir);
        }
        Err(e) => {
            tracing::warn!("IPC disabled: {e}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    ipc
}

// shortcut: requests are served synchronously inside the event loop with a
// short read timeout; move to a per-client calloop source if a client stalls.
fn serve_requests(h: &mut Hypr, l: &UnixListener) {
    while let Ok((mut s, _)) = l.accept() {
        let _ = s.set_nonblocking(false);
        let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
        let mut buf = vec![0u8; 65536];
        let n = s.read(&mut buf).unwrap_or(0);
        if n == 0 {
            continue;
        }
        let reply = handle_request(h, &String::from_utf8_lossy(&buf[..n]));
        let _ = s.write_all(reply.as_bytes());
    }
}

pub fn handle_request(h: &mut Hypr, raw: &str) -> String {
    match parse_batch(raw) {
        Some(reqs) => reqs.iter().map(|r| handle_one(h, r)).collect(),
        None => handle_one(h, &parse_request(raw)),
    }
}

fn handle_one(h: &mut Hypr, req: &Request) -> String {
    if let Some(args) = req.cmd.strip_prefix("dispatch") {
        let args = args.trim();
        let (name, arg) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
        return match Dispatcher::parse(name, arg) {
            Ok(d) => {
                h.dispatch(&d);
                "ok".into()
            }
            Err(e) => e,
        };
    }
    query(&snapshot(h), &req.cmd, req.json).unwrap_or_else(|| "unknown request".into())
}

fn snapshot(h: &Hypr) -> Snapshot {
    let focused = h.focused_window();
    let output = h.space.outputs().next();
    let mon_name = output
        .map(|o| o.name())
        .unwrap_or_else(|| "HEADLESS-1".into());
    let active = WsRef {
        id: h.workspaces[h.active_workspace].id,
        name: h.workspaces[h.active_workspace].name.clone(),
    };

    let mut clients = Vec::new();
    let mut workspaces = Vec::new();
    for (i, ws) in h.workspaces.iter().enumerate() {
        let wins = ws.windows();
        let id = ws.id;
        let shown = i == h.active_workspace || Some(id) == h.wm.special_shown;
        if wins.is_empty() && !shown {
            continue;
        }
        for (n, w) in wins.iter().enumerate() {
            let (class, title) = class_title(w);
            let (at, size) = match h.space.element_location(w) {
                Some(l) if shown => ([l.x, l.y], [w.geometry().size.w, w.geometry().size.h]),
                _ => ([0, 0], [0, 0]),
            };
            let pid = w
                .toplevel()
                .and_then(|t| t.wl_surface().client())
                .and_then(|c| c.get_credentials(&h.display_handle).ok())
                .map_or(-1, |c| c.pid);
            clients.push(ClientInfo {
                address: window_addr(w),
                mapped: true,
                hidden: false,
                at,
                size,
                workspace: WsRef {
                    id,
                    name: ws.name.clone(),
                },
                floating: ws.is_floating(w),
                pseudo: false,
                monitor: 0,
                initial_class: class.clone(),
                initial_title: title.clone(),
                class,
                title,
                pid,
                xwayland: false,
                pinned: false,
                fullscreen: if ws.fullscreen.as_ref() == Some(w) {
                    2
                } else {
                    0
                },
                fullscreen_client: 0,
                grouped: Vec::new(),
                tags: Vec::new(),
                swallowing: "0x0".into(),
                focus_history_id: if focused.as_ref() == Some(w) {
                    0
                } else {
                    n as i32 + 1
                },
                inhibiting_idle: false,
                xdg_tag: String::new(),
                xdg_description: String::new(),
            });
        }
        let last = ws.last_focused.as_ref().filter(|w| ws.contains(w));
        workspaces.push(WorkspaceInfo {
            id,
            name: ws.name.clone(),
            monitor: mon_name.clone(),
            monitor_id: 0,
            windows: wins.len() as i32,
            has_fullscreen: ws.fullscreen.is_some(),
            last_window: last.map_or("0x0".into(), window_addr),
            last_window_title: last.map(|w| class_title(w).1).unwrap_or_default(),
            is_persistent: false,
            tiled_layout: "dwindle".into(),
        });
    }

    let monitors = output
        .map(|o| {
            let mode = o.current_mode();
            let (w, hh, hz) = mode.map_or((0, 0, 60.0), |m| {
                (m.size.w, m.size.h, m.refresh as f64 / 1000.0)
            });
            let loc = o.current_location();
            MonitorInfo {
                id: 0,
                name: mon_name.clone(),
                description: String::new(),
                make: String::new(),
                model: String::new(),
                serial: String::new(),
                width: w,
                height: hh,
                physical_width: 0,
                physical_height: 0,
                refresh_rate: hz,
                x: loc.x,
                y: loc.y,
                active_workspace: active.clone(),
                special_workspace: WsRef {
                    id: 0,
                    name: String::new(),
                },
                reserved: [0; 4],
                scale: o.current_scale().fractional_scale(),
                transform: 0,
                focused: true,
                dpms_status: true,
                vrr: false,
                solitary: "0".into(),
                actively_tearing: false,
                direct_scanout_to: "0".into(),
                disabled: false,
                current_format: "XRGB8888".into(),
                mirror_of: "none".into(),
                available_modes: vec![format!("{w}x{hh}@{hz:.2}Hz")],
            }
        })
        .into_iter()
        .collect();

    let binds = h
        .config
        .binds
        .iter()
        .map(|bd| {
            let (dispatcher, arg) = dispatcher_name_arg(&bd.dispatcher);
            let m = &bd.mods;
            BindInfo {
                locked: bd.flags.locked,
                mouse: bd.flags.mouse,
                release: bd.flags.release,
                repeat: bd.flags.repeat,
                long_press: bd.flags.long_press,
                non_consuming: bd.flags.non_consuming,
                has_description: bd.flags.has_description,
                modmask: m.shift as u32
                    | (m.ctrl as u32) << 2
                    | (m.alt as u32) << 3
                    | (m.logo as u32) << 6,
                submap: bd.submap.clone(),
                key: bd.key.clone(),
                keycode: 0,
                catch_all: false,
                description: bd.description.clone(),
                dispatcher: dispatcher.into(),
                arg,
            }
        })
        .collect();

    Snapshot {
        clients,
        workspaces,
        monitors,
        binds,
        active_workspace: active,
        active_window: focused.as_ref().map(window_addr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_flags() {
        assert_eq!(
            parse_request("j/clients"),
            Request {
                json: true,
                cmd: "clients".into()
            }
        );
        assert_eq!(
            parse_request("/dispatch workspace 2"),
            Request {
                json: false,
                cmd: "dispatch workspace 2".into()
            }
        );
        assert_eq!(
            parse_request("dispatch exec foo/bar"),
            Request {
                json: false,
                cmd: "dispatch exec foo/bar".into()
            }
        );
        assert_eq!(parse_request("version").cmd, "version");
    }

    #[test]
    fn batch() {
        let r = parse_batch("[[BATCH]]j/clients; dispatch workspace 2;").unwrap();
        assert_eq!(r.len(), 2);
        assert!(r[0].json);
        assert_eq!(r[1].cmd, "dispatch workspace 2");
        assert!(parse_batch("clients").is_none());
    }

    fn client() -> ClientInfo {
        ClientInfo {
            address: "0x55800000".into(),
            mapped: true,
            hidden: false,
            at: [10, 20],
            size: [300, 200],
            workspace: WsRef {
                id: 2,
                name: "2".into(),
            },
            floating: false,
            pseudo: false,
            monitor: 0,
            class: "foot".into(),
            title: "t".into(),
            initial_class: "foot".into(),
            initial_title: "t".into(),
            pid: 42,
            xwayland: false,
            pinned: false,
            fullscreen: 0,
            fullscreen_client: 0,
            grouped: vec![],
            tags: vec![],
            swallowing: "0x0".into(),
            focus_history_id: 0,
            inhibiting_idle: false,
            xdg_tag: String::new(),
            xdg_description: String::new(),
        }
    }

    #[test]
    fn client_json_shape() {
        let v: serde_json::Value = serde_json::from_str(&json(&vec![client()])).unwrap();
        let c = &v[0];
        assert_eq!(c["address"], "0x55800000");
        assert_eq!(c["at"], serde_json::json!([10, 20]));
        assert_eq!(c["workspace"], serde_json::json!({"id": 2, "name": "2"}));
        assert_eq!(c["initialClass"], "foot");
        assert_eq!(c["focusHistoryID"], 0);
        assert_eq!(c["fullscreenClient"], 0);
        assert!(c["xwayland"] == false);
    }

    #[test]
    fn query_dispatch() {
        let snap = Snapshot {
            clients: vec![client()],
            workspaces: vec![],
            monitors: vec![],
            binds: vec![],
            active_workspace: WsRef {
                id: 1,
                name: "1".into(),
            },
            active_window: Some("0x55800000".into()),
        };
        assert!(query(&snap, "clients", true).unwrap().starts_with("[{"));
        assert!(query(&snap, "activewindow", false)
            .unwrap()
            .starts_with("Window 55800000 -> t:"));
        assert_eq!(query(&snap, "activeworkspace", true).unwrap(), "{}");
        assert!(query(&snap, "bogus", false).is_none());
    }
}
