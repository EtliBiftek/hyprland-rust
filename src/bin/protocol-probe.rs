//! Test client for the protocols no CLI tool covers (used by `tests/probe.sh`).
//!
//! `protocol-probe export`    hyprland-toplevel-mapping + hyprland-toplevel-export of a red window
//! `protocol-probe copycap`   ext-image-copy-capture of a toplevel source and an output source
//! `protocol-probe layerpopup` a layer popup placed off screen comes back on screen
//! `protocol-probe focusgrab` hyprland-focus-grab: prints `grabbed`, then `cleared` when a click
//!                            outside the whitelisted window clears it
//! Prints `OK ...` and exits 0, or prints `FAIL ...` and exits 1.

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::Write,
    os::{fd::AsFd, unix::fs::FileExt},
    time::{Duration, Instant},
};

use wayland_client::{
    event_created_child,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{
        wl_buffer::WlBuffer,
        wl_compositor::WlCompositor,
        wl_output::WlOutput,
        wl_registry,
        wl_shm::{self, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::{
    ext::{
        foreign_toplevel_list::v1::client::{
            ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
            ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
        },
        image_capture_source::v1::client::{
            ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
            ext_image_capture_source_v1::ExtImageCaptureSourceV1,
            ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
        },
        image_copy_capture::v1::client::{
            ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
            ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
            ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
        },
    },
    xdg::shell::client::{
        xdg_popup::{self, XdgPopup},
        xdg_positioner::{self, XdgPositioner},
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    },
};

use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

macro_rules! hypr_client {
    ($name:ident, $xml:literal, [$($dep:path),*]) => {
        mod $name {
            #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_imports)]
            #![allow(non_upper_case_globals, non_snake_case, missing_docs, clippy::all)]
            use wayland_client;
            use wayland_client::protocol::*;
            $(use $dep::{client::*};)*

            pub mod __interfaces {
                use wayland_client::protocol::__interfaces::*;
                $(use $dep::{client::__interfaces::*};)*
                wayland_scanner::generate_interfaces!($xml);
            }
            use self::__interfaces::*;

            wayland_scanner::generate_client_code!($xml);
        }
    };
}

hypr_client!(focus_grab, "protocols/hyprland-focus-grab-v1.xml", []);
hypr_client!(
    toplevel_export,
    "protocols/hyprland-toplevel-export-v1.xml",
    [wayland_protocols_wlr::foreign_toplevel::v1]
);
hypr_client!(
    toplevel_mapping,
    "protocols/hyprland-toplevel-mapping-v1.xml",
    [
        wayland_protocols::ext::foreign_toplevel_list::v1,
        wayland_protocols_wlr::foreign_toplevel::v1
    ]
);

use focus_grab::{
    hyprland_focus_grab_manager_v1::HyprlandFocusGrabManagerV1,
    hyprland_focus_grab_v1::{self, HyprlandFocusGrabV1},
};
use toplevel_export::{
    hyprland_toplevel_export_frame_v1::{self, HyprlandToplevelExportFrameV1},
    hyprland_toplevel_export_manager_v1::HyprlandToplevelExportManagerV1,
};
use toplevel_mapping::{
    hyprland_toplevel_mapping_manager_v1::HyprlandToplevelMappingManagerV1,
    hyprland_toplevel_window_mapping_handle_v1::{self, HyprlandToplevelWindowMappingHandleV1},
};

const RED: u32 = 0xFFFF_0000;

#[derive(Default)]
struct P {
    app_ids: HashMap<u32, String>,
    handles: Vec<ExtForeignToplevelHandleV1>,
    map_addr: Option<u64>,
    map_failed: bool,
    export_buf: Option<(u32, u32, u32)>,
    export_buffer_done: bool,
    /// Some(true) = ready, Some(false) = failed.
    result: Option<bool>,
    cleared: bool,
    sess_size: Option<(u32, u32)>,
    sess_done: bool,
    sess_stopped: bool,
    popup_x: Option<i32>,
}

fn bind_err(n: &str) -> ! {
    fail(&format!("global {n} missing"))
}

fn fail(msg: &str) -> ! {
    println!("FAIL {msg}");
    std::process::exit(1);
}

struct Ctx {
    qh: QueueHandle<P>,
    shm: WlShm,
    compositor: WlCompositor,
    wm: XdgWmBase,
    n: u32,
}

/// Window data: the buffer that is attached on the first configure.
struct WinData {
    surface: WlSurface,
    buffer: WlBuffer,
}

impl Ctx {
    fn buffer(&mut self, w: u32, h: u32, color: Option<u32>) -> (WlBuffer, File) {
        self.n += 1;
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let path = format!("{dir}/probe-shm-{}-{}", std::process::id(), self.n);
        let mut f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap_or_else(|e| fail(&format!("shm file: {e}")));
        let _ = std::fs::remove_file(&path);
        let px = color.unwrap_or(0).to_ne_bytes();
        let data: Vec<u8> = (0..w * h).flat_map(|_| px).collect();
        f.write_all(&data).unwrap();
        let pool = self
            .shm
            .create_pool(f.as_fd(), (w * h * 4) as i32, &self.qh, ());
        let buf = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            (w * 4) as i32,
            wl_shm::Format::Xrgb8888,
            &self.qh,
            (),
        );
        pool.destroy();
        (buf, f)
    }

    /// Opens a w x h window with a solid colour and the given app id.
    fn window(&mut self, app_id: &str, w: u32, h: u32, color: u32) -> WlSurface {
        let (buffer, _file) = self.buffer(w, h, Some(color));
        let surface = self.compositor.create_surface(&self.qh, ());
        let xs = self.wm.get_xdg_surface(
            &surface,
            &self.qh,
            WinData {
                surface: surface.clone(),
                buffer,
            },
        );
        let top = xs.get_toplevel(&self.qh, ());
        top.set_app_id(app_id.into());
        surface.commit();
        // Keep the objects alive for the life of the process.
        std::mem::forget((xs, top));
        surface
    }
}

fn wait(p: &mut P, q: &mut EventQueue<P>, secs: u64, cond: impl Fn(&P) -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        q.roundtrip(p)
            .unwrap_or_else(|e| fail(&format!("roundtrip: {e}")));
        if cond(p) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Handle of the toplevel announced with `app_id`.
fn handle_of(p: &P, app_id: &str) -> Option<ExtForeignToplevelHandleV1> {
    p.handles
        .iter()
        .find(|h| p.app_ids.get(&h.id().protocol_id()).map(String::as_str) == Some(app_id))
        .cloned()
}

/// Share of pixels (Xrgb8888, little endian B G R X) that are mostly red.
fn red_share(f: &File, w: u32, h: u32) -> f64 {
    let mut v = vec![0u8; (w * h * 4) as usize];
    f.read_exact_at(&mut v, 0).unwrap();
    let red = v
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[2] > 200 && p[1] < 50 && p[0] < 50)
        .count();
    red as f64 / (w * h) as f64
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let conn = Connection::connect_to_env().unwrap_or_else(|e| fail(&format!("connect: {e}")));
    let (globals, mut q) = registry_queue_init::<P>(&conn).unwrap();
    let qh = q.handle();
    let mut ctx = Ctx {
        shm: globals
            .bind(&qh, 1..=1, ())
            .unwrap_or_else(|_| bind_err("wl_shm")),
        compositor: globals
            .bind(&qh, 1..=4, ())
            .unwrap_or_else(|_| bind_err("wl_compositor")),
        wm: globals
            .bind(&qh, 1..=1, ())
            .unwrap_or_else(|_| bind_err("xdg_wm_base")),
        qh: qh.clone(),
        n: 0,
    };
    let mut p = P::default();

    match mode.as_str() {
        "layerpopup" => {
            let shell: ZwlrLayerShellV1 = globals
                .bind(&qh, 1..=4, ())
                .unwrap_or_else(|_| bind_err("zwlr_layer_shell_v1"));
            let (buffer, _file) = ctx.buffer(100, 30, Some(RED));
            let surface = ctx.compositor.create_surface(&qh, ());
            let layer = shell.get_layer_surface(
                &surface,
                None,
                zwlr_layer_shell_v1::Layer::Top,
                "probe".into(),
                &qh,
                WinData {
                    surface: surface.clone(),
                    buffer,
                },
            );
            layer.set_size(100, 30);
            layer.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left,
            );
            surface.commit();
            wait(&mut p, &mut q, 2, |_| false);

            let positioner = ctx.wm.create_positioner(&qh, ());
            positioner.set_size(400, 100);
            positioner.set_anchor_rect(0, 0, 10, 10);
            positioner.set_offset(-500, 0);
            positioner.set_constraint_adjustment(
                xdg_positioner::ConstraintAdjustment::SlideX
                    | xdg_positioner::ConstraintAdjustment::SlideY,
            );
            let (buffer, _popup_file) = ctx.buffer(400, 100, Some(RED));
            let popup_surface = ctx.compositor.create_surface(&qh, ());
            let xs = ctx.wm.get_xdg_surface(
                &popup_surface,
                &qh,
                WinData {
                    surface: popup_surface.clone(),
                    buffer,
                },
            );
            let popup = xs.get_popup(None, &positioner, &qh, ());
            layer.get_popup(&popup);
            popup_surface.commit();
            if !wait(&mut p, &mut q, 5, |p| p.popup_x.is_some()) {
                fail("no popup configure");
            }
            match p.popup_x {
                Some(x) if x >= 0 => println!("OK layer popup at x={x}"),
                x => fail(&format!("layer popup off screen: {x:?}")),
            }
        }
        "focusgrab" => {
            let mgr: HyprlandFocusGrabManagerV1 = globals
                .bind(&qh, 1..=1, ())
                .unwrap_or_else(|_| bind_err("hyprland_focus_grab_manager_v1"));
            let a = ctx.window("probe-a", 200, 200, 0xFF00_FF00);
            let _b = ctx.window("probe-b", 200, 200, 0xFF00_00FF);
            // Let both windows map and get their first configure.
            wait(&mut p, &mut q, 2, |_| false);
            let grab = mgr.create_grab(&qh, ());
            grab.add_surface(&a);
            grab.commit();
            q.roundtrip(&mut p).unwrap();
            println!("grabbed");
            let _ = std::io::stdout().flush();
            if wait(&mut p, &mut q, 40, |p| p.cleared) {
                println!("OK cleared");
            } else {
                fail("no cleared event");
            }
        }
        "export" | "copycap" => {
            let list: ExtForeignToplevelListV1 = globals
                .bind(&qh, 1..=1, ())
                .unwrap_or_else(|_| bind_err("ext_foreign_toplevel_list_v1"));
            let _ = &list;
            ctx.window("probe-red", 200, 200, RED);
            if !wait(&mut p, &mut q, 15, |p| handle_of(p, "probe-red").is_some()) {
                fail("toplevel not announced");
            }
            let handle = handle_of(&p, "probe-red").unwrap();
            // Let the window commit its buffer before capturing.
            wait(&mut p, &mut q, 1, |_| false);
            if mode == "export" {
                export(&mut ctx, &globals, &mut p, &mut q, &handle);
            } else {
                copycap(&mut ctx, &globals, &mut p, &mut q, &handle);
            }
        }
        _ => fail("usage: protocol-probe export|copycap|focusgrab"),
    }
}

fn export(
    ctx: &mut Ctx,
    globals: &wayland_client::globals::GlobalList,
    p: &mut P,
    q: &mut EventQueue<P>,
    handle: &ExtForeignToplevelHandleV1,
) {
    let qh = ctx.qh.clone();
    let map: HyprlandToplevelMappingManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .unwrap_or_else(|_| fail("mapping global missing"));
    let exp: HyprlandToplevelExportManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .unwrap_or_else(|_| fail("export global missing"));
    let _m = map.get_window_for_toplevel(handle, &qh, ());
    if !wait(p, q, 5, |p| p.map_addr.is_some() || p.map_failed) || p.map_failed {
        fail("toplevel mapping failed");
    }
    let addr = p.map_addr.unwrap();
    let _f = exp.capture_toplevel(0, addr as u32, &qh, ());
    if !wait(p, q, 5, |p| {
        p.export_buf.is_some() || p.result == Some(false)
    }) {
        fail("no buffer event");
    }
    let (_fmt, w, h) = p
        .export_buf
        .unwrap_or_else(|| fail("frame failed before buffer"));
    if !wait(p, q, 2, |p| p.export_buffer_done) {
        fail("no buffer_done");
    }
    let (buf, file) = ctx.buffer(w, h, None);
    _f.copy(&buf, 0);
    if !wait(p, q, 10, |p| p.result.is_some()) || p.result != Some(true) {
        fail("export copy failed");
    }
    let share = red_share(&file, w, h);
    if share < 0.9 {
        fail(&format!("export pixels not red ({share:.2}, {w}x{h})"));
    }
    println!("OK export {w}x{h} red {share:.2}");
}

fn copycap(
    ctx: &mut Ctx,
    globals: &wayland_client::globals::GlobalList,
    p: &mut P,
    q: &mut EventQueue<P>,
    handle: &ExtForeignToplevelHandleV1,
) {
    let qh = ctx.qh.clone();
    let tl_mgr: ExtForeignToplevelImageCaptureSourceManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .unwrap_or_else(|_| fail("toplevel source manager missing"));
    let out_mgr: ExtOutputImageCaptureSourceManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .unwrap_or_else(|_| fail("output source manager missing"));
    let cap: ExtImageCopyCaptureManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .unwrap_or_else(|_| fail("copy capture manager missing"));
    let output: WlOutput = globals
        .bind(&qh, 1..=1, ())
        .unwrap_or_else(|_| fail("wl_output missing"));

    for (name, src) in [
        ("toplevel", tl_mgr.create_source(handle, &qh, ())),
        ("output", out_mgr.create_source(&output, &qh, ())),
    ] {
        *p = P {
            app_ids: std::mem::take(&mut p.app_ids),
            handles: std::mem::take(&mut p.handles),
            ..P::default()
        };
        let sess = cap.create_session(
            &src,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &qh,
            (),
        );
        if !wait(p, q, 5, |p| p.sess_done || p.sess_stopped) || p.sess_stopped {
            fail(&format!("{name}: session constraints missing"));
        }
        let (w, h) = p.sess_size.unwrap();
        let (buf, file) = ctx.buffer(w, h, None);
        let frame = sess.create_frame(&qh, ());
        frame.attach_buffer(&buf);
        frame.damage_buffer(0, 0, w as i32, h as i32);
        frame.capture();
        if !wait(p, q, 10, |p| p.result.is_some()) || p.result != Some(true) {
            fail(&format!("{name}: capture failed"));
        }
        if name == "toplevel" {
            let share = red_share(&file, w, h);
            if share < 0.9 {
                fail(&format!("toplevel pixels not red ({share:.2})"));
            }
        } else {
            let mut v = vec![0u8; (w * h * 4) as usize];
            file.read_exact_at(&mut v, 0).unwrap();
            if v.iter().all(|b| *b == 0) {
                fail("output capture is all zero");
            }
        }
        println!("OK copycap {name} {w}x{h}");
    }
}

//
// Event handling.
//

macro_rules! noop {
    ($($iface:ty),*) => {$(
        impl Dispatch<$iface, ()> for P {
            fn event(
                _: &mut Self,
                _: &$iface,
                _: <$iface as Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }
    )*};
}
noop!(
    WlShm,
    WlShmPool,
    WlBuffer,
    WlCompositor,
    WlSurface,
    WlOutput,
    XdgToplevel,
    XdgPositioner,
    ZwlrLayerShellV1,
    ExtImageCaptureSourceV1,
    ExtForeignToplevelImageCaptureSourceManagerV1,
    ExtOutputImageCaptureSourceManagerV1,
    ExtImageCopyCaptureManagerV1,
    HyprlandFocusGrabManagerV1,
    HyprlandToplevelExportManagerV1,
    HyprlandToplevelMappingManagerV1
);

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for P {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgWmBase, ()> for P {
    fn event(
        _: &mut Self,
        wm: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<XdgPopup, ()> for P {
    fn event(
        p: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_popup::Event::Configure { x, .. } = event {
            p.popup_x = Some(x);
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, WinData> for P {
    fn event(
        _: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        data: &WinData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            layer.ack_configure(serial);
            data.surface.attach(Some(&data.buffer), 0, 0);
            data.surface.commit();
        }
    }
}

impl Dispatch<XdgSurface, WinData> for P {
    fn event(
        _: &mut Self,
        xs: &XdgSurface,
        event: xdg_surface::Event,
        data: &WinData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xs.ack_configure(serial);
            data.surface.attach(Some(&data.buffer), 0, 0);
            data.surface.commit();
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for P {
    fn event(
        p: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            p.handles.push(toplevel);
        }
    }

    event_created_child!(P, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for P {
    fn event(
        p: &mut Self,
        h: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_handle_v1::Event::AppId { app_id } = event {
            p.app_ids.insert(h.id().protocol_id(), app_id);
        }
    }
}

impl Dispatch<HyprlandToplevelWindowMappingHandleV1, ()> for P {
    fn event(
        p: &mut Self,
        _: &HyprlandToplevelWindowMappingHandleV1,
        event: hyprland_toplevel_window_mapping_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            hyprland_toplevel_window_mapping_handle_v1::Event::WindowAddress {
                address_hi,
                address,
            } => p.map_addr = Some(((address_hi as u64) << 32) | address as u64),
            hyprland_toplevel_window_mapping_handle_v1::Event::Failed => p.map_failed = true,
        }
    }
}

impl Dispatch<HyprlandToplevelExportFrameV1, ()> for P {
    fn event(
        p: &mut Self,
        _: &HyprlandToplevelExportFrameV1,
        event: hyprland_toplevel_export_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use hyprland_toplevel_export_frame_v1::Event as E;
        match event {
            E::Buffer {
                format,
                width,
                height,
                ..
            } => {
                let f = match format {
                    WEnum::Value(v) => v as u32,
                    WEnum::Unknown(u) => u,
                };
                p.export_buf = Some((f, width, height));
            }
            E::BufferDone => p.export_buffer_done = true,
            E::Ready { .. } => p.result = Some(true),
            E::Failed => p.result = Some(false),
            _ => {}
        }
    }
}

impl Dispatch<HyprlandFocusGrabV1, ()> for P {
    fn event(
        p: &mut Self,
        _: &HyprlandFocusGrabV1,
        event: hyprland_focus_grab_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let hyprland_focus_grab_v1::Event::Cleared = event;
        p.cleared = true;
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for P {
    fn event(
        p: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event as E;
        match event {
            E::BufferSize { width, height } => p.sess_size = Some((width, height)),
            E::Done => p.sess_done = true,
            E::Stopped => p.sess_stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for P {
    fn event(
        p: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event as E;
        match event {
            E::Ready => p.result = Some(true),
            E::Failed { .. } => p.result = Some(false),
            _ => {}
        }
    }
}
