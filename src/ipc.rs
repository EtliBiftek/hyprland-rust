//! Hyprland-compatible IPC: request socket (`.socket.sock`) and event socket
//! (`.socket2.sock`) under `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`.
//!
//! The top half (request parsing, info structs, JSON/text rendering) is pure
//! and free of Smithay types so it can be unit-tested. The bottom half builds
//! a `Snapshot` from `Hypr` and runs the sockets. Extra requests (`reload`,
//! `keyword`, `devices`, ...) live in `ipc_cmds.rs`.
//!
//! Output shapes were checked against Hyprland v0.57.1
//! (`src/ipc/s1/Commands.cpp`, `src/helpers/SystemInfo.cpp`). JSON is compact
//! here, Hyprland pretty-prints it; field names, order and types match.
//!
//! Placeholder fields (always a constant): clients `pinned`,
//! `pinFullscreened`, `fullscreenHandler`, `allowedOverFullscreen`,
//! `tags`, `swallowing`, `inhibitingIdle`, `xdgTag`, `xdgDescription`,
//! `contentType`, `tearingHint`, `initialClass`/`initialTitle` (current
//! values); workspaces `ispersistent`; monitors `serial`,
//! `transform`, `vrr`, `solitary*`,
//! `activelyTearing`, `tearingBlockedBy`, `directScanout*`, `disabled`,
//! `currentFormat`, `mirrorOf`, `colorManagementPreset`, `sdr*`,
//! `hardwareDetails`; version `commit*`, `tag`, `build*`/`system*`, `abiHash`,
//! `flags`; binds `keycode` and all boolean flags.

use std::{
    io::{ErrorKind, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use serde::Serialize;
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{layer_map_for_output, Window},
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_server::protocol::wl_surface::WlSurface,
    },
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

use crate::{
    config::{Direction, Dispatcher},
    state::Hypr,
    CalloopData,
};

// ---------------------------------------------------------------- pure part

#[derive(Debug, PartialEq, Eq, Default)]
pub struct Request {
    pub json: bool,
    /// `r`: refresh state after the command.
    pub refresh: bool,
    /// `a`: include unmapped windows / inactive monitors.
    pub all: bool,
    pub cmd: String,
}

/// Parses `[flags/]command args`. Flags are `j` (JSON), `r` (refresh), `a`
/// (all); `c` and `f` are accepted and ignored.
pub fn parse_request(s: &str) -> Request {
    let s = s.trim();
    if let Some((flags, rest)) = s.split_once('/') {
        if flags.chars().all(|c| "jarcf".contains(c)) {
            return Request {
                json: flags.contains('j'),
                refresh: flags.contains('r'),
                all: flags.contains('a'),
                cmd: rest.trim().to_string(),
            };
        }
    }
    Request {
        cmd: s.to_string(),
        ..Default::default()
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

/// Replies of a batch are joined with this (Hyprland's `BATCH_DELIMITER`).
pub const BATCH_DELIMITER: &str = "\n\n\n";

/// Workspace reference as nested in clients/monitors and flattened into
/// workspace objects. `id` is absent for named/special workspaces.
#[derive(Serialize, Clone, Debug)]
pub struct WsRef {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
}

impl WsRef {
    pub fn of(id: i32, name: &str) -> Self {
        let kind = if name.starts_with("special") {
            "special"
        } else {
            "normal"
        };
        Self {
            kind: kind.into(),
            name: name.into(),
            ..Self::numbered(id)
        }
    }

    pub fn numbered(id: i32) -> Self {
        Self {
            address: id.to_string(),
            id: Some(id),
            kind: "normal".into(),
            name: id.to_string(),
        }
    }

    pub fn empty() -> Self {
        Self {
            address: String::new(),
            id: None,
            kind: String::new(),
            name: String::new(),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub address: String,
    pub mapped: bool,
    pub hidden: bool,
    pub visible: bool,
    pub accepts_input: bool,
    pub at: [i32; 2],
    pub size: [i32; 2],
    pub workspace: WsRef,
    pub floating: bool,
    pub monitor: i32,
    pub class: String,
    pub title: String,
    pub initial_class: String,
    pub initial_title: String,
    pub pid: i32,
    pub xwayland: bool,
    pub pinned: bool,
    pub pin_fullscreened: bool,
    pub fullscreen: i32,
    pub fullscreen_client: i32,
    pub fullscreen_handler: String,
    pub allowed_over_fullscreen: bool,
    pub grouped: Vec<String>,
    pub tags: Vec<String>,
    pub swallowing: String,
    #[serde(rename = "focusHistoryID")]
    pub focus_history_id: i32,
    pub inhibiting_idle: bool,
    pub xdg_tag: String,
    pub xdg_description: String,
    pub content_type: String,
    pub tearing_hint: bool,
    pub stable_id: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct WorkspaceInfo {
    #[serde(flatten)]
    pub ws: WsRef,
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
pub struct HardwareDetails {
    pub backend: String,
    pub hdr: bool,
    pub chroma: bool,
    pub bt2020: bool,
    pub vrr_capable: bool,
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
    pub solitary_blocked_by: Option<Vec<String>>,
    pub actively_tearing: bool,
    pub tearing_blocked_by: Option<Vec<String>>,
    pub direct_scanout_to: String,
    pub direct_scanout_blocked_by: Option<Vec<String>>,
    pub disabled: bool,
    pub current_format: String,
    pub mirror_of: String,
    pub available_modes: Vec<String>,
    pub color_management_preset: String,
    pub sdr_brightness: f64,
    pub sdr_saturation: f64,
    pub sdr_min_luminance: f64,
    pub sdr_max_luminance: f64,
    pub hardware_cursors_in_use: bool,
    pub hardware_details: HardwareDetails,
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
    pub auto_consuming: bool,
    pub has_description: bool,
    pub modmask: u32,
    pub submap: String,
    /// Hyprland prints this one as a JSON string ("true"/"false").
    pub submap_universal: String,
    pub key: String,
    pub keycode: i32,
    pub catch_all: bool,
    pub description: String,
    pub allow_input_capture: bool,
    pub dispatcher: String,
    pub arg: String,
    /// Comma separated flag names, shown by the text output only.
    #[serde(skip)]
    pub flags: String,
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
    #[serde(rename = "buildAquamarine")]
    pub build_aquamarine: String,
    #[serde(rename = "buildHyprlang")]
    pub build_hyprlang: String,
    #[serde(rename = "buildHyprutils")]
    pub build_hyprutils: String,
    #[serde(rename = "buildHyprcursor")]
    pub build_hyprcursor: String,
    #[serde(rename = "buildHyprgraphics")]
    pub build_hyprgraphics: String,
    #[serde(rename = "systemAquamarine")]
    pub system_aquamarine: String,
    #[serde(rename = "systemHyprlang")]
    pub system_hyprlang: String,
    #[serde(rename = "systemHyprutils")]
    pub system_hyprutils: String,
    #[serde(rename = "systemHyprcursor")]
    pub system_hyprcursor: String,
    #[serde(rename = "systemHyprgraphics")]
    pub system_hyprgraphics: String,
    #[serde(rename = "abiHash")]
    pub abi_hash: String,
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
    let grouped = if c.grouped.is_empty() {
        "0".to_string()
    } else {
        c.grouped
            .iter()
            .map(|g| g.trim_start_matches("0x"))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "Window {} -> {}:\n\tmapped: {}\n\thidden: {}\n\tvisible: {}\n\tacceptsInput: {}\n\tat: {},{}\n\tsize: {},{}\n\tworkspace: {} ({})\n\tfloating: {}\n\tmonitor: {}\n\tclass: {}\n\ttitle: {}\n\tinitialClass: {}\n\tinitialTitle: {}\n\tpid: {}\n\txwayland: {}\n\tpinned: {}\n\tpinFullscreened: {}\n\tfullscreen: {}\n\tfullscreenClient: {}\n\tfullscreenHandler: {}\n\tallowedOverFullscreen: {}\n\tgrouped: {}\n\ttags: {}\n\tswallowing: {}\n\tfocusHistoryID: {}\n\tinhibitingIdle: {}\n\txdgTag: {}\n\txdgDescription: {}\n\tcontentType: {}\n\ttearingHint: {}\n\tstableID: {}\n\n",
        c.address.trim_start_matches("0x"),
        c.title,
        b(c.mapped),
        b(c.hidden),
        b(c.visible),
        b(c.accepts_input),
        c.at[0],
        c.at[1],
        c.size[0],
        c.size[1],
        c.workspace.address,
        c.workspace.name,
        b(c.floating),
        c.monitor,
        c.class,
        c.title,
        c.initial_class,
        c.initial_title,
        c.pid,
        b(c.xwayland),
        b(c.pinned),
        b(c.pin_fullscreened),
        c.fullscreen,
        c.fullscreen_client,
        c.fullscreen_handler,
        b(c.allowed_over_fullscreen),
        grouped,
        c.tags.join(", "),
        c.swallowing.trim_start_matches("0x"),
        c.focus_history_id,
        b(c.inhibiting_idle),
        c.xdg_tag,
        c.xdg_description,
        c.content_type,
        b(c.tearing_hint),
        c.stable_id,
    )
}

fn workspace_text(w: &WorkspaceInfo) -> String {
    format!(
        "workspace {} ({}) on monitor {}:\n\tmonitorID: {}\n\twindows: {}\n\thasfullscreen: {}\n\tlastwindow: {}\n\tlastwindowtitle: {}\n\tispersistent: {}\n\ttiledLayout: {}\n\n",
        w.ws.address,
        w.ws.name,
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

fn reasons_text(r: &Option<Vec<String>>) -> String {
    r.as_ref().map_or("null".into(), |v| v.join(","))
}

fn monitor_text(m: &MonitorInfo) -> String {
    let hd = &m.hardware_details;
    let yn = |v: bool| if v { "yes" } else { "no" };
    format!(
        "Monitor {} (ID {}):\n\t{}x{}@{:.5} at {}x{}\n\tdescription: {}\n\tmake: {}\n\tmodel: {}\n\tphysical size (mm): {}x{}\n\tserial: {}\n\tactive workspace: {} ({})\n\tspecial workspace: {} ({})\n\treserved: {} {} {} {}\n\tscale: {}\n\ttransform: {}\n\tfocused: {}\n\tdpmsStatus: {}\n\tvrr: {}\n\tsolitary: {}\n\tsolitaryBlockedBy: {}\n\tactivelyTearing: {}\n\ttearingBlockedBy: {}\n\tdirectScanoutTo: {}\n\tdirectScanoutBlockedBy: {}\n\tdisabled: {}\n\tcurrentFormat: {}\n\tmirrorOf: {}\n\tavailableModes: {} \n\tcolorManagementPreset: {}\n\tsdrBrightness: {}\n\tsdrSaturation: {}\n\tsdrMinLuminance: {}\n\tsdrMaxLuminance: {}\n\thardwareCursorsInUse: {}\n\thardwareDetails:\n\t\tbackend: {}\n\t\thdr: {}\n\t\tchroma: {}\n\t\tbt2020: {}\n\t\tvrrCapable: {}\n\n",
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
        m.physical_width,
        m.physical_height,
        m.serial,
        m.active_workspace.address,
        m.active_workspace.name,
        m.special_workspace.address,
        m.special_workspace.name,
        m.reserved[0],
        m.reserved[1],
        m.reserved[2],
        m.reserved[3],
        m.scale,
        m.transform,
        yn(m.focused),
        b(m.dpms_status),
        m.vrr,
        m.solitary,
        reasons_text(&m.solitary_blocked_by),
        m.actively_tearing,
        reasons_text(&m.tearing_blocked_by),
        m.direct_scanout_to,
        reasons_text(&m.direct_scanout_blocked_by),
        m.disabled,
        m.current_format,
        m.mirror_of,
        m.available_modes.join(" "),
        m.color_management_preset,
        m.sdr_brightness,
        m.sdr_saturation,
        m.sdr_min_luminance,
        m.sdr_max_luminance,
        m.hardware_cursors_in_use,
        hd.backend,
        yn(hd.hdr),
        yn(hd.chroma),
        yn(hd.bt2020),
        yn(hd.vrr_capable),
    )
}

fn bind_text(b: &BindInfo) -> String {
    format!(
        "bind\n\tflags: {}\n\tmodmask: {}\n\tsubmap: {}\n\tkey: {}\n\tkeycode: {}\n\tcatchall: {}\n\tdescription: {}\n\tdispatcher: {}\n\targ: {}\n\n",
        b.flags, b.modmask, b.submap, b.key, b.keycode, b.catch_all, b.description, b.dispatcher, b.arg,
    )
}

pub fn json<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

pub fn version_info() -> VersionInfo {
    let unknown = || String::new();
    VersionInfo {
        branch: "main".into(),
        commit: String::new(),
        version: env!("CARGO_PKG_VERSION").into(),
        dirty: false,
        commit_message: String::new(),
        commit_date: String::new(),
        tag: String::new(),
        commits: "0".into(),
        build_aquamarine: unknown(),
        build_hyprlang: unknown(),
        build_hyprutils: unknown(),
        build_hyprcursor: unknown(),
        build_hyprgraphics: unknown(),
        system_aquamarine: unknown(),
        system_hyprlang: unknown(),
        system_hyprutils: unknown(),
        system_hyprcursor: unknown(),
        system_hyprgraphics: unknown(),
        abi_hash: unknown(),
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
                "Hyprland {} built from branch {} at commit {} {} ({}).\nDate: {}\nTag: {}, commits: {}\n\nno flags were set\n",
                v.version,
                v.branch,
                v.commit,
                if v.dirty { "dirty" } else { "clean" },
                v.commit_message,
                v.commit_date,
                v.tag,
                v.commits
            )
        }
        ("clients", true) => json(&snap.clients),
        ("clients", false) if snap.clients.is_empty() => "no open windows".into(),
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
                .find(|w| w.ws.id == snap.active_workspace.id);
            match (ws, json_out) {
                (Some(w), true) => json(w),
                (Some(w), false) => workspace_text(w),
                (None, _) => "internal error".into(),
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

/// Set once `openwindow` was emitted for the window.
struct Mapped(AtomicBool);

pub fn window_addr(w: &Window) -> String {
    let id = w
        .user_data()
        .get_or_insert(|| WinId(NEXT_WIN_ID.fetch_add(0x40, Ordering::Relaxed)));
    format!("0x{:x}", id.0)
}

fn mapped_flag(w: &Window) -> &Mapped {
    w.user_data()
        .get_or_insert(|| Mapped(AtomicBool::new(false)))
}

/// Forgets that `openwindow` was sent (the window unmapped).
pub fn clear_mapped(w: &Window) {
    mapped_flag(w).0.store(false, Ordering::Relaxed);
}

/// True once `openwindow` was sent for this window.
pub fn is_mapped(w: &Window) -> bool {
    mapped_flag(w).0.load(Ordering::Relaxed)
}

pub fn class_title(w: &Window) -> (String, String) {
    let Some(t) = w.toplevel() else {
        // X11: WM_CLASS class (like Hyprland) and the window title.
        return w
            .x11_surface()
            .map(|x| (x.class(), x.title()))
            .unwrap_or_default();
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
    /// Events since the last loop iteration, for `hl.on` (`lua_host.rs`).
    pub lua_events: Vec<(String, String)>,
    /// Backend name shown in `monitors` (`winit`, `tty`, `headless`).
    pub backend: String,
}

impl Ipc {
    /// Sends `EVENT>>DATA\n` to all event clients, dropping dead ones.
    pub fn emit(&mut self, event: &str, data: &str) {
        self.lua_events.push((event.into(), data.into()));
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
        self.emit_activewindow(w);
        self.emit(
            "activewindowv2",
            addr.as_deref()
                .map(|a| a.trim_start_matches("0x"))
                .unwrap_or(""),
        );
        self.last_active = addr;
    }

    /// `activewindow>>CLASS,TITLE` only (class/title may have changed).
    fn emit_activewindow(&mut self, w: Option<&Window>) {
        let (class, title) = w.map(class_title).unwrap_or_default();
        self.emit("activewindow", &format!("{class},{title}"));
    }

    /// Records backend and Wayland socket; writes `hyprland.lock` (pid and
    /// socket name, read by `hyprctl instances`).
    pub fn set_instance_info(&mut self, backend: &str, wl_socket: &str) {
        self.backend = backend.to_string();
        if let Some(dir) = &self.dir {
            let _ = std::fs::write(
                dir.join("hyprland.lock"),
                format!("{}\n{}\n", std::process::id(), wl_socket),
            );
        }
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Hypr {
    /// Name of the focused output, `HEADLESS-1` if there is none yet.
    pub fn monitor_name(&self) -> String {
        let f = self.focused_mon_name();
        if !f.is_empty() {
            return f;
        }
        self.space
            .outputs()
            .next()
            .map_or_else(|| "HEADLESS-1".into(), |o| o.name())
    }

    /// `focusedmon>>MON,WSNAME` / `focusedmonv2>>MON,WSID`.
    pub fn emit_focused_mon(&mut self) {
        let mon = self.monitor_name();
        let ws = &self.workspaces[self.active_workspace];
        let (id, name) = (ws.id, ws.name.clone());
        self.ipc.emit("focusedmon", &format!("{mon},{name}"));
        self.ipc.emit("focusedmonv2", &format!("{mon},{id}"));
    }

    /// Called after every commit of a toplevel: emits `openwindow` the first
    /// time the window has a buffer, when app_id and title are known.
    pub fn check_window_mapped(&mut self, w: &Window) {
        let Some(surface) = crate::xwayland::wl_surface_of(w) else {
            return;
        };
        let has_buffer =
            with_renderer_surface_state(&surface, |s| s.buffer().is_some()).unwrap_or(false);
        if !has_buffer || mapped_flag(w).0.swap(true, Ordering::Relaxed) {
            return;
        }
        // `openwindow>>ADDR,WORKSPACENAME,CLASS,TITLE`.
        let ws = self
            .workspaces
            .iter()
            .find(|ws| ws.contains(w))
            .unwrap_or(&self.workspaces[self.active_workspace])
            .name
            .clone();
        let (class, title) = class_title(w);
        let addr = window_addr(w);
        self.ipc.emit(
            "openwindow",
            &format!("{},{ws},{class},{title}", addr.trim_start_matches("0x")),
        );
        if self.focused_window().as_ref() == Some(w) {
            // The focus event fired before class/title were known.
            self.ipc.emit_activewindow(Some(w));
        }
    }

    /// xdg_toplevel title change: `windowtitle` and `windowtitlev2`.
    pub fn on_title_changed(&mut self, surface: &WlSurface) {
        if let Some((_, w)) = self.window_for_surface(surface) {
            self.on_title_changed_win(&w);
        }
    }

    pub fn on_title_changed_win(&mut self, w: &Window) {
        let w = w.clone();
        if !is_mapped(&w) {
            return; // `openwindow` will carry the title
        }
        let addr = window_addr(&w);
        let addr = addr.trim_start_matches("0x");
        let (_, title) = class_title(&w);
        self.ipc.emit("windowtitle", addr);
        self.ipc.emit("windowtitlev2", &format!("{addr},{title}"));
        if self.focused_window().as_ref() == Some(&w) {
            self.ipc.emit_activewindow(Some(&w));
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
        lua_events: Vec::new(),
        backend: String::new(),
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
        let h = handle.clone();
        handle
            .insert_source(
                Generic::new(req, Interest::READ, Mode::Level),
                move |_, l, _| {
                    while let Ok((s, _)) = l.accept() {
                        add_client(&h, s);
                    }
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

/// Largest request accepted from one client.
const MAX_REQUEST: usize = 1 << 20;

/// Registers one request connection as its own event source: read the
/// request (non-blocking), reply, close. A client that stalls only keeps its
/// own fd open; it never blocks the compositor.
fn add_client(handle: &LoopHandle<'_, CalloopData>, stream: UnixStream) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let h = handle.clone();
    let mut buf = Vec::new();
    let _ = handle.insert_source(
        Generic::new(stream, Interest::READ, Mode::Level),
        move |_, s, data| {
            let mut chunk = [0u8; 4096];
            let eof = loop {
                match (&**s).read(&mut chunk) {
                    Ok(0) => break true,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.len() > MAX_REQUEST {
                            return Ok(PostAction::Remove);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break false,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(_) => return Ok(PostAction::Remove),
                }
            };
            // A request is one write: complete once the socket runs dry.
            if buf.is_empty() {
                return Ok(if eof {
                    PostAction::Remove
                } else {
                    PostAction::Continue
                });
            }
            let reply = handle_request(&mut data.state, &String::from_utf8_lossy(&buf));
            let out = reply.into_bytes();
            let mut off = 0;
            while off < out.len() {
                match (&**s).write(&out[off..]) {
                    Ok(n) => off += n,
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(_) => return Ok(PostAction::Remove),
                }
            }
            if off < out.len() {
                if let Ok(w) = s.try_clone() {
                    let _ = h.insert_source(
                        Generic::new(w, Interest::WRITE, Mode::Level),
                        move |_, w, _| {
                            while off < out.len() {
                                match (&**w).write(&out[off..]) {
                                    Ok(n) => off += n,
                                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                                        return Ok(PostAction::Continue)
                                    }
                                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                                    Err(_) => break,
                                }
                            }
                            Ok(PostAction::Remove)
                        },
                    );
                }
            }
            Ok(PostAction::Remove)
        },
    );
}

pub fn handle_request(h: &mut Hypr, raw: &str) -> String {
    match parse_batch(raw) {
        Some(reqs) => reqs
            .iter()
            .map(|r| handle_one(h, r))
            .collect::<Vec<_>>()
            .join(BATCH_DELIMITER),
        None => handle_one(h, &parse_request(raw)),
    }
}

fn handle_one(h: &mut Hypr, req: &Request) -> String {
    if req.cmd.trim() == "layers" {
        return h.layers_reply(req.json);
    }
    let reply = if let Some(args) = req.cmd.strip_prefix("dispatch") {
        let args = args.trim();
        // Quickshell sends `dispatch hl.config({...})`: same as `hyprctl eval`.
        if args.starts_with("hl.") && !args.starts_with("hl.dsp.") {
            return crate::ipc_cmds::eval(h, args);
        }
        // Hyprland >= 0.57 takes Lua here: `hyprctl dispatch 'hl.dsp.exit()'`.
        let lua;
        let (name, arg) = if args.starts_with("hl.") {
            match crate::lua_config::eval_dispatch(args) {
                Ok(p) => {
                    lua = p;
                    (lua.0.as_str(), lua.1.as_str())
                }
                Err(e) => return e,
            }
        } else {
            args.split_once(char::is_whitespace).unwrap_or((args, ""))
        };
        match Dispatcher::parse(name, arg) {
            Ok(d) => {
                h.dispatch(&d);
                "ok".into()
            }
            Err(e) => e,
        }
    } else if let Some(r) = crate::ipc_cmds::handle(h, req) {
        r
    } else {
        query(&snapshot(h), &req.cmd, req.json).unwrap_or_else(|| "unknown request".into())
    };
    if req.refresh {
        h.relayout();
    }
    reply
}

fn snapshot(h: &Hypr) -> Snapshot {
    let focused = h.focused_window();
    let visible = h.visible_ws();
    let mon_id = |name: &str| {
        h.wm.mons
            .iter()
            .find(|m| m.name == name)
            .map_or(0, |m| m.id as i32)
    };
    let active = WsRef::of(
        h.workspaces[h.active_workspace].id,
        &h.workspaces[h.active_workspace].name,
    );

    let mut clients = Vec::new();
    let mut workspaces = Vec::new();
    for (i, ws) in h.workspaces.iter().enumerate() {
        let wins = ws.windows();
        let id = ws.id;
        let shown = visible.contains(&i);
        let ws_mon = h.ws_monitor(i);
        if wins.is_empty() && !shown {
            continue;
        }
        for (n, w) in wins.iter().enumerate() {
            let (class, title) = class_title(w);
            // A hidden group member reports the place of the visible one.
            let rep = ws.groups.rep(w);
            let (at, size) = match h.space.element_location(rep) {
                Some(l) if shown => ([l.x, l.y], [rep.geometry().size.w, rep.geometry().size.h]),
                _ => ([0, 0], [0, 0]),
            };
            let pid = h.window_pid(w);
            let fs = if ws.fullscreen.as_ref() == Some(w) {
                2
            } else {
                0
            };
            let addr = window_addr(w);
            let hidden = ws.groups.is_hidden(w);
            let grouped = ws
                .groups
                .group_of(w)
                .map(|g| g.members.iter().map(window_addr).collect())
                .unwrap_or_default();
            let (initial_class, initial_title) = h.window_initial(w);
            clients.push(ClientInfo {
                mapped: is_mapped(w),
                hidden,
                visible: shown,
                accepts_input: true,
                at,
                size,
                workspace: WsRef::of(id, &ws.name),
                floating: ws.is_floating(w),
                monitor: mon_id(&ws_mon),
                initial_class,
                initial_title,
                class,
                title,
                pid,
                xwayland: w.is_x11(),
                pinned: h.wm.pinned.contains(w),
                pin_fullscreened: false,
                fullscreen: fs,
                fullscreen_client: fs,
                fullscreen_handler: "none".into(),
                allowed_over_fullscreen: false,
                grouped,
                tags: h.window_tags(w),
                swallowing: "0x0".into(),
                focus_history_id: if focused.as_ref() == Some(w) {
                    0
                } else {
                    n as i32 + 1
                },
                inhibiting_idle: false,
                xdg_tag: String::new(),
                xdg_description: String::new(),
                content_type: "none".into(),
                tearing_hint: false,
                stable_id: addr.trim_start_matches("0x").into(),
                address: addr,
            });
        }
        let last = ws.last_focused.as_ref().filter(|w| ws.contains(w));
        workspaces.push(WorkspaceInfo {
            ws: WsRef::of(id, &ws.name),
            monitor: ws_mon.clone(),
            monitor_id: mon_id(&ws_mon),
            windows: wins.len() as i32,
            has_fullscreen: ws.fullscreen.is_some(),
            last_window: last.map_or("0x0".into(), window_addr),
            last_window_title: last.map(|w| class_title(w).1).unwrap_or_default(),
            is_persistent: false,
            tiled_layout: ws.tiled.kind.name().into(),
        });
    }

    let monitors =
        h.wm.mons
            .iter()
            .enumerate()
            .filter_map(|(mi, mon)| {
                let o = &h.output_named(&mon.name)?;
                let mon_name = &mon.name;
                let is_focused = mi == h.wm.focused_mon;
                let mode = o.current_mode();
                let (w, hh, hz) = mode.map_or((0, 0, 60.0), |m| {
                    (m.size.w, m.size.h, m.refresh as f64 / 1000.0)
                });
                let loc = o.current_location();
                let phys = o.physical_properties();
                // shortcut: one special workspace at a time, on the focused monitor.
                let special =
                    h.wm.special_shown
                        .filter(|_| is_focused)
                        .and_then(|id| h.ws_index(id))
                        .map_or_else(WsRef::empty, |i| {
                            WsRef::of(h.workspaces[i].id, &h.workspaces[i].name)
                        });
                let shown = h.ws_index(mon.ws).map_or_else(WsRef::empty, |i| {
                    WsRef::of(h.workspaces[i].id, &h.workspaces[i].name)
                });
                // [left, top, right, bottom] taken by layer-shell exclusive zones.
                let reserved = {
                    let z = layer_map_for_output(o).non_exclusive_zone();
                    [
                        z.loc.x,
                        z.loc.y,
                        w - z.loc.x - z.size.w,
                        hh - z.loc.y - z.size.h,
                    ]
                };
                Some(MonitorInfo {
                    id: mon.id as i32,
                    name: mon_name.clone(),
                    description: format!("{} {} ({})", phys.make, phys.model, mon_name),
                    make: phys.make.clone(),
                    model: phys.model.clone(),
                    serial: String::new(),
                    width: w,
                    height: hh,
                    physical_width: phys.size.w,
                    physical_height: phys.size.h,
                    refresh_rate: hz,
                    x: loc.x,
                    y: loc.y,
                    active_workspace: shown,
                    special_workspace: special,
                    reserved,
                    scale: o.current_scale().fractional_scale(),
                    transform: crate::monitors::transform_index(o.current_transform()) as i32,
                    focused: is_focused,
                    dpms_status: h.wm.dpms_on,
                    vrr: false,
                    solitary: "0".into(),
                    solitary_blocked_by: None,
                    actively_tearing: false,
                    tearing_blocked_by: None,
                    direct_scanout_to: "0".into(),
                    direct_scanout_blocked_by: None,
                    disabled: false,
                    current_format: "XRGB8888".into(),
                    mirror_of: "none".into(),
                    available_modes: vec![format!("{w}x{hh}@{hz:.2}Hz")],
                    color_management_preset: "srgb".into(),
                    sdr_brightness: 1.0,
                    sdr_saturation: 1.0,
                    sdr_min_luminance: 0.005,
                    sdr_max_luminance: 80.0,
                    hardware_cursors_in_use: false,
                    hardware_details: HardwareDetails {
                        backend: h.ipc.backend.clone(),
                        hdr: false,
                        chroma: false,
                        bt2020: false,
                        vrr_capable: false,
                    },
                })
            })
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
                auto_consuming: false,
                has_description: bd.flags.has_description,
                modmask: m.shift as u32
                    | (m.ctrl as u32) << 2
                    | (m.alt as u32) << 3
                    | (m.logo as u32) << 6,
                submap: bd.submap.clone(),
                submap_universal: bd.flags.universal.to_string(),
                key: bd.key.clone(),
                keycode: 0,
                catch_all: false,
                description: bd.description.clone(),
                allow_input_capture: false,
                dispatcher: dispatcher.into(),
                arg,
                flags: String::new(),
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
                cmd: "clients".into(),
                ..Default::default()
            }
        );
        let r = parse_request("jr/reload");
        assert!(r.json && r.refresh && !r.all);
        assert_eq!(
            parse_request("/dispatch workspace 2"),
            Request {
                cmd: "dispatch workspace 2".into(),
                ..Default::default()
            }
        );
        assert_eq!(
            parse_request("dispatch exec foo/bar"),
            Request {
                cmd: "dispatch exec foo/bar".into(),
                ..Default::default()
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
            visible: true,
            accepts_input: true,
            at: [10, 20],
            size: [300, 200],
            workspace: WsRef::numbered(2),
            floating: false,
            monitor: 0,
            class: "foot".into(),
            title: "t".into(),
            initial_class: "foot".into(),
            initial_title: "t".into(),
            pid: 42,
            xwayland: false,
            pinned: false,
            pin_fullscreened: false,
            fullscreen: 0,
            fullscreen_client: 0,
            fullscreen_handler: "none".into(),
            allowed_over_fullscreen: false,
            grouped: vec![],
            tags: vec![],
            swallowing: "0x0".into(),
            focus_history_id: 0,
            inhibiting_idle: false,
            xdg_tag: String::new(),
            xdg_description: String::new(),
            content_type: "none".into(),
            tearing_hint: false,
            stable_id: "55800000".into(),
        }
    }

    #[test]
    fn client_json_shape() {
        let v: serde_json::Value = serde_json::from_str(&json(&vec![client()])).unwrap();
        let c = &v[0];
        assert_eq!(c["address"], "0x55800000");
        assert_eq!(c["at"], serde_json::json!([10, 20]));
        assert_eq!(
            c["workspace"],
            serde_json::json!({"address": "2", "id": 2, "type": "normal", "name": "2"})
        );
        assert_eq!(c["initialClass"], "foot");
        assert_eq!(c["focusHistoryID"], 0);
        assert_eq!(c["fullscreenClient"], 0);
        assert_eq!(c["acceptsInput"], true);
        assert_eq!(c["stableId"], "55800000");
        assert!(c["xwayland"] == false);
        assert!(c.get("pseudo").is_none());
    }

    #[test]
    fn client_json_field_order() {
        let s = json(&client());
        let keys = [
            "address",
            "mapped",
            "hidden",
            "visible",
            "acceptsInput",
            "at",
            "size",
            "workspace",
            "floating",
            "monitor",
            "class",
            "title",
            "initialClass",
            "initialTitle",
            "pid",
            "xwayland",
            "pinned",
            "pinFullscreened",
            "fullscreen",
            "fullscreenClient",
            "fullscreenHandler",
            "allowedOverFullscreen",
            "grouped",
            "tags",
            "swallowing",
            "focusHistoryID",
            "inhibitingIdle",
            "xdgTag",
            "xdgDescription",
            "contentType",
            "tearingHint",
            "stableId",
        ];
        let mut last = 0;
        for k in keys {
            let p = s
                .find(&format!("\"{k}\":"))
                .unwrap_or_else(|| panic!("{k}"));
            assert!(p >= last, "{k} out of order");
            last = p;
        }
    }

    #[test]
    fn workspace_json_flattens_ref() {
        let w = WorkspaceInfo {
            ws: WsRef::numbered(3),
            monitor: "M".into(),
            monitor_id: 0,
            windows: 1,
            has_fullscreen: false,
            last_window: "0x0".into(),
            last_window_title: String::new(),
            is_persistent: false,
            tiled_layout: "dwindle".into(),
        };
        assert!(json(&w).starts_with(
            r#"{"address":"3","id":3,"type":"normal","name":"3","monitor":"M","monitorID":0"#
        ));
        assert!(workspace_text(&w).starts_with("workspace 3 (3) on monitor M:\n"));
    }

    #[test]
    fn query_dispatch() {
        let snap = Snapshot {
            clients: vec![client()],
            workspaces: vec![],
            monitors: vec![],
            binds: vec![],
            active_workspace: WsRef::numbered(1),
            active_window: Some("0x55800000".into()),
        };
        assert!(query(&snap, "clients", true).unwrap().starts_with("[{"));
        assert!(query(&snap, "activewindow", false)
            .unwrap()
            .starts_with("Window 55800000 -> t:"));
        assert_eq!(
            query(&snap, "activeworkspace", true).unwrap(),
            "internal error"
        );
        assert!(query(&snap, "bogus", false).is_none());
    }
}
