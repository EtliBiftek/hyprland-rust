use std::{ffi::OsString, sync::Arc};

use smithay::{
    backend::session::libseat::LibSeatSession,
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{pointer::CursorImageStatus, Seat, SeatState},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_protocols::ext::idle_notify::v1::server::ext_idle_notifier_v1::ExtIdleNotifierV1,
        wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        content_type::ContentTypeState,
        cursor_shape::CursorShapeManagerState,
        fractional_scale::FractionalScaleManagerState,
        idle_inhibit::IdleInhibitManagerState,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitState,
        output::OutputManagerState,
        pointer_constraints::PointerConstraintsState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        selection::{
            data_device::DataDeviceState, ext_data_control,
            primary_selection::PrimarySelectionState, wlr_data_control,
        },
        session_lock::{LockSurface, SessionLockManagerState},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{decoration::XdgDecorationState, XdgShellState},
        },
        shm::ShmState,
        single_pixel_buffer::SinglePixelBufferState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
        xdg_activation::XdgActivationState,
        xdg_foreign::XdgForeignState,
    },
};

use smithay::wayland::shell::wlr_layer::Layer;

use crate::{config::Config, handlers::protocols::IdleState, workspace::Workspace, CalloopData};

pub struct Hypr {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Smithay State
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub layer_shell_state: WlrLayerShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Hypr>,
    pub data_device_state: DataDeviceState,
    pub xdg_activation_state: XdgActivationState,
    pub popups: PopupManager,

    // Protocol states from `handlers/protocols.rs`.
    pub primary_selection_state: PrimarySelectionState,
    pub wlr_data_control_state: wlr_data_control::DataControlState,
    pub ext_data_control_state: ext_data_control::DataControlState,
    pub viewporter_state: ViewporterState,
    pub fractional_scale_state: FractionalScaleManagerState,
    pub presentation_state: PresentationState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub content_type_state: ContentTypeState,
    pub cursor_shape_state: CursorShapeManagerState,
    pub idle_inhibit_state: IdleInhibitManagerState,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub pointer_constraints_state: PointerConstraintsState,
    pub relative_pointer_state: RelativePointerManagerState,
    pub xdg_foreign_state: XdgForeignState,
    pub session_lock_state: SessionLockManagerState,
    /// ext-idle-notify timers and idle-inhibit surfaces.
    pub idle: IdleState,
    pub screencopy: crate::handlers::screencopy::ScreencopyState,
    /// wlr-foreign-toplevel-management and ext-foreign-toplevel-list.
    pub toplevels: crate::handlers::foreign_toplevel::ToplevelState,
    pub global_shortcuts: crate::handlers::global_shortcuts::GlobalShortcutsState,
    /// hyprland protocols, capture, gamma, dmabuf, input method (see `handlers/extra.rs`).
    pub extra: crate::handlers::extra::ExtraState,
    /// Last cursor image requested by a client (`cursor-shape` or a surface).
    pub cursor_status: CursorImageStatus,
    /// Session lock held: only `lock_surfaces` are drawn and get input.
    pub session_locked: bool,
    pub lock_surfaces: Vec<LockSurface>,

    pub seat: Seat<Self>,
    /// Layer surface that currently holds keyboard focus (exclusive or clicked).
    pub layer_focus: Option<WlSurface>,

    // Window management
    pub config: Config,
    pub workspaces: Vec<Workspace<Window>>,
    /// Index into `workspaces` (not the workspace id).
    pub active_workspace: usize,
    /// Focus history, special workspace, submap, key repeat, ...
    pub wm: crate::wm::WmState,

    /// libseat session on the TTY backend, used for VT switching.
    pub session: Option<LibSeatSession>,

    /// Hyprland-compatible IPC sockets and event broadcast.
    pub ipc: crate::ipc::Ipc,

    /// Config path, errors and autoreload bookkeeping.
    pub reload: crate::reload::ReloadState,
}

impl Hypr {
    pub fn new(
        event_loop: &mut EventLoop<'static, CalloopData>,
        display: Display<Self>,
        config: Config,
    ) -> Self {
        let start_time = std::time::Instant::now();

        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let mut seat_state = SeatState::new();
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let popups = PopupManager::default();

        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let wlr_data_control_state = wlr_data_control::DataControlState::new::<Self, _>(
            &dh,
            Some(&primary_selection_state),
            |_| true,
        );
        let ext_data_control_state = ext_data_control::DataControlState::new::<Self, _>(
            &dh,
            Some(&primary_selection_state),
            |_| true,
        );
        // Presentation clock: 1 is CLOCK_MONOTONIC on Linux.
        let presentation_state = PresentationState::new::<Self>(&dh, 1);
        let session_lock_state = SessionLockManagerState::new::<Self, _>(&dh, |_| true);
        dh.create_global::<Self, ExtIdleNotifierV1, _>(2, ());
        dh.create_global::<Self, ZwlrScreencopyManagerV1, _>(3, ());
        crate::handlers::output_management::init(&dh);
        crate::handlers::global_shortcuts::init(&dh);

        // A seat is a group of keyboards, pointer and touch devices.
        // A seat typically has a pointer and maintains a keyboard focus and a pointer focus.
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "seat0");

        // Notify clients that we have a keyboard, for the sake of the example we assume that keyboard is always present.
        // You may want to track keyboard hot-plug in real compositor.
        seat.add_keyboard(Default::default(), 200, 25).unwrap();

        // Notify clients that we have a pointer (mouse)
        // Here we assume that there is always pointer plugged in
        seat.add_pointer();

        // A space represents a two-dimensional plane. Windows and Outputs can be mapped onto it.
        //
        // Windows get a position and stacking order through mapping.
        // Outputs become views of a part of the Space and can be rendered via Space::render_output.
        let space = Space::default();

        let socket_name = Self::init_wayland_listener(display, event_loop);

        // Get the loop signal, used to stop the event loop
        let loop_signal = event_loop.get_signal();

        let mut hypr = Self {
            start_time,
            display_handle: dh.clone(),

            space,
            loop_signal,
            socket_name,

            compositor_state,
            xdg_shell_state,
            layer_shell_state,
            xdg_decoration_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            xdg_activation_state,
            popups,
            primary_selection_state,
            wlr_data_control_state,
            ext_data_control_state,
            viewporter_state: ViewporterState::new::<Self>(&dh),
            fractional_scale_state: FractionalScaleManagerState::new::<Self>(&dh),
            presentation_state,
            single_pixel_buffer_state: SinglePixelBufferState::new::<Self>(&dh),
            content_type_state: ContentTypeState::new::<Self>(&dh),
            cursor_shape_state: CursorShapeManagerState::new::<Self>(&dh),
            idle_inhibit_state: IdleInhibitManagerState::new::<Self>(&dh),
            keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState::new::<Self>(&dh),
            pointer_constraints_state: PointerConstraintsState::new::<Self>(&dh),
            relative_pointer_state: RelativePointerManagerState::new::<Self>(&dh),
            xdg_foreign_state: XdgForeignState::new::<Self>(&dh),
            session_lock_state,
            idle: IdleState::default(),
            screencopy: Default::default(),
            toplevels: crate::handlers::foreign_toplevel::ToplevelState::new(&dh),
            global_shortcuts: Default::default(),
            extra: crate::handlers::extra::ExtraState::new(&dh),
            cursor_status: CursorImageStatus::default_named(),
            session_locked: false,
            lock_surfaces: Vec::new(),
            seat,
            layer_focus: None,

            config,
            workspaces: vec![Workspace::new(1, "1".into())],
            active_workspace: 0,
            wm: crate::wm::WmState::new(event_loop.handle()),
            session: None,
            ipc: crate::ipc::init(&event_loop.handle()),
            reload: Default::default(),
        };
        hypr.apply_input_config();
        hypr
    }

    fn init_wayland_listener(
        display: Display<Hypr>,
        event_loop: &mut EventLoop<CalloopData>,
    ) -> OsString {
        // Creates a new listening socket, automatically choosing the next available `wayland` socket name.
        let listening_socket = ListeningSocketSource::new_auto().unwrap();

        // Get the name of the listening socket.
        // Clients will connect to this socket.
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                // Inside the callback, you should insert the client into the display.
                //
                // You may also associate some data with the client when inserting the client.
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        // You also need to add the display itself to the event loop, so that client events will be processed by wayland-server.
        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // Safety: we don't drop the display
                    unsafe {
                        display
                            .get_mut()
                            .dispatch_clients(&mut state.state)
                            .unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    /// Overlay/top layers, then windows, then bottom/background layers.
    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.session_locked {
            return self
                .lock_surfaces
                .iter()
                .find(|s| s.alive())
                .map(|s| (s.wl_surface().clone(), Point::from((0.0, 0.0))));
        }
        self.layer_at(pos, [Layer::Overlay, Layer::Top])
            .map(|(_, s, p)| (s, p))
            .or_else(|| {
                self.space
                    .element_under(pos)
                    .and_then(|(window, location)| {
                        window
                            .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                            .map(|(s, p)| (s, (p + location).to_f64()))
                    })
            })
            .or_else(|| {
                self.layer_at(pos, [Layer::Bottom, Layer::Background])
                    .map(|(_, s, p)| (s, p))
            })
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    /// Set for clients that connected through a wp-security-context listener.
    pub security_context: Option<smithay::wayland::security_context::SecurityContext>,
}

impl ClientState {
    /// False for sandboxed clients; used as the filter of privileged globals.
    pub fn trusted(client: &smithay::reexports::wayland_server::Client) -> bool {
        client
            .get_data::<ClientState>()
            .is_none_or(|d| d.security_context.is_none())
    }
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
