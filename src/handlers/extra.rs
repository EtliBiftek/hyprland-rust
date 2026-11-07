//! State of the protocols added after the first batch (`protocols2`): hyprland
//! protocols, capture, gamma, dmabuf, input method stack and friends. Each lives
//! in its own file under `handlers/`; this only owns their state and registers
//! the globals so `state.rs` needs one field.

use std::any::Any;

use smithay::reexports::wayland_server::DisplayHandle;

use super::{
    alpha_modifier, capture::CaptureState, ctm_control::CtmState, dmabuf::DmabufHolder, fifo,
    focus_grab::FocusGrabState, gamma_control::GammaState, input_method,
    lock_notify::LockNotifyState, security_context, system_bell, tablet, text_input, toplevel_icon,
    virtual_keyboard,
};

pub struct ExtraState {
    pub capture: CaptureState,
    pub focus_grab: FocusGrabState,
    pub ctm: CtmState,
    pub lock_notify: LockNotifyState,
    pub gamma: GammaState,
    pub dmabuf: DmabufHolder,
    /// Smithay states that only need to stay alive.
    _keep: Vec<Box<dyn Any>>,
}

impl ExtraState {
    pub fn new(dh: &DisplayHandle) -> Self {
        super::focus_grab::init(dh);
        super::toplevel_export::init(dh);
        super::toplevel_mapping::init(dh);
        super::ctm_control::init(dh);
        super::hyprland_surface::init(dh);
        super::lock_notify::init(dh);
        super::gamma_control::init(dh);
        super::image_copy_capture::init(dh);
        super::tearing_control::init(dh);
        super::xdg_dialog::init(dh);
        let (fifo_state, timing_state) = fifo::init(dh);
        let keep: Vec<Box<dyn Any>> = vec![
            Box::new(alpha_modifier::init(dh)),
            Box::new(system_bell::init(dh)),
            Box::new(toplevel_icon::init(dh)),
            Box::new(fifo_state),
            Box::new(timing_state),
            Box::new(tablet::init(dh)),
            Box::new(security_context::init(dh)),
            Box::new(text_input::init(dh)),
            Box::new(input_method::init(dh)),
            Box::new(virtual_keyboard::init(dh)),
        ];
        Self {
            capture: Default::default(),
            focus_grab: Default::default(),
            ctm: Default::default(),
            lock_notify: Default::default(),
            gamma: Default::default(),
            dmabuf: Default::default(),
            _keep: keep,
        }
    }
}
