//! zwp_linux_dmabuf_v1 (version 3, formats only; no per-surface feedback).
//!
//! The backends call `Hypr::init_dmabuf` once they know the renderer's dmabuf
//! formats (winit, tty); headless advertises a fixed linear ARGB/XRGB list so
//! the global exists for tests. The global is created once.
//! shortcuts: imports are accepted without being tested against the renderer
//! (a bad buffer only fails when it is drawn); headless never draws them;
//! no `default_feedback` (v4), no main-device or scanout tranches; screencopy
//! and the capture protocols still write shm only.

use smithay::{
    backend::allocator::{dmabuf::Dmabuf, format::FormatSet, Format, Fourcc, Modifier},
    delegate_dmabuf,
    wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
};

use crate::Hypr;

pub struct DmabufHolder {
    state: DmabufState,
    global: Option<DmabufGlobal>,
}

impl Default for DmabufHolder {
    fn default() -> Self {
        Self {
            state: DmabufState::new(),
            global: None,
        }
    }
}

impl DmabufHandler for Hypr {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.extra.dmabuf.state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        _dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if notifier.successful::<Hypr>().is_err() {
            tracing::debug!("dmabuf client vanished during import");
        }
    }
}

delegate_dmabuf!(Hypr);

impl Hypr {
    /// Creates the dmabuf global with `formats` (no-op if it already exists).
    pub fn init_dmabuf(&mut self, formats: FormatSet) {
        let d = &mut self.extra.dmabuf;
        if d.global.is_some() {
            return;
        }
        let dh = self.display_handle.clone();
        d.global = Some(
            d.state
                .create_global::<Hypr>(&dh, formats.iter().copied().collect::<Vec<Format>>()),
        );
    }

    /// For backends without a renderer (headless).
    pub fn init_dmabuf_fallback(&mut self) {
        let formats = [Fourcc::Argb8888, Fourcc::Xrgb8888].map(|code| Format {
            code,
            modifier: Modifier::Linear,
        });
        self.init_dmabuf(FormatSet::from_iter(formats));
    }
}
