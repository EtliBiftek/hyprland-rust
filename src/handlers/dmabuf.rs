//! zwp_linux_dmabuf_v1.
//!
//! winit and tty call `Hypr::init_dmabuf_renderer`: the global carries the
//! renderer's formats, with default feedback (v4, main device = the EGL
//! render node) when EGL names one, else version 3. Imports are queued and
//! answered by the backend, which tests them against the renderer
//! (`test_dmabuf_imports`). Headless advertises a fixed linear ARGB/XRGB list
//! so the global exists for tests and accepts everything (it never draws).
//! The global is created once.
//! shortcuts: no per-surface feedback and no scanout tranches; no `wl_drm`
//! (needs smithay's `use_system_lib`); wlr-screencopy and toplevel-export render
//! into client dmabufs, ext-image-copy-capture still offers shm only.

use smithay::{
    backend::{
        allocator::{dmabuf::Dmabuf, format::FormatSet, Format, Fourcc, Modifier},
        drm::DrmNode,
        egl::EGLDevice,
        renderer::{gles::GlesRenderer, ImportDma},
    },
    delegate_dmabuf,
    wayland::dmabuf::{
        DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier,
    },
};

use crate::Hypr;

pub struct DmabufHolder {
    state: DmabufState,
    global: Option<DmabufGlobal>,
    /// The backend has a renderer and answers imports itself.
    tested: bool,
    /// The renderer can draw into an Xrgb8888 dmabuf (screencopy `linux_dmabuf`).
    export: bool,
    pending: Vec<(Dmabuf, ImportNotifier)>,
}

impl DmabufHolder {
    pub fn can_export(&self) -> bool {
        self.export
    }
}

impl Default for DmabufHolder {
    fn default() -> Self {
        Self {
            state: DmabufState::new(),
            global: None,
            tested: false,
            export: false,
            pending: Vec::new(),
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
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if self.extra.dmabuf.tested {
            self.extra.dmabuf.pending.push((dmabuf, notifier));
        } else if notifier.successful::<Hypr>().is_err() {
            tracing::debug!("dmabuf client vanished during import");
        }
    }
}

delegate_dmabuf!(Hypr);

impl Hypr {
    /// Creates the dmabuf global with `formats` (no-op if it already exists);
    /// with a render node it also sends default feedback (version 4).
    fn init_dmabuf(&mut self, formats: FormatSet, node: Option<DrmNode>) {
        let d = &mut self.extra.dmabuf;
        if d.global.is_some() {
            return;
        }
        let dh = self.display_handle.clone();
        let feedback = node.and_then(|n| {
            DmabufFeedbackBuilder::new(n.dev_id(), formats.iter().copied())
                .build()
                .ok()
        });
        tracing::info!(
            "linux-dmabuf: {} formats, render node {node:?}, feedback: {}",
            formats.iter().count(),
            feedback.is_some()
        );
        d.global = Some(match &feedback {
            Some(f) => d.state.create_global_with_default_feedback::<Hypr>(&dh, f),
            None => d
                .state
                .create_global::<Hypr>(&dh, formats.iter().copied().collect::<Vec<Format>>()),
        });
    }

    /// Global for a backend that draws with `renderer`; the backend must call
    /// `test_dmabuf_imports` from its loop.
    pub fn init_dmabuf_renderer(&mut self, renderer: &GlesRenderer) {
        let node = EGLDevice::device_for_display(renderer.egl_context().display())
            .ok()
            .and_then(|d| d.try_get_render_node().ok().flatten());
        self.extra.dmabuf.tested = true;
        self.extra.dmabuf.export = renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .any(|f| f.code == Fourcc::Xrgb8888);
        self.init_dmabuf(renderer.dmabuf_formats(), node);
    }

    /// Answers the queued imports: a buffer the renderer cannot import fails
    /// at creation instead of when it is drawn.
    pub fn test_dmabuf_imports(&mut self, renderer: &mut GlesRenderer) {
        for (dmabuf, notifier) in std::mem::take(&mut self.extra.dmabuf.pending) {
            match renderer.import_dmabuf(&dmabuf, None) {
                Ok(_) => {
                    if notifier.successful::<Hypr>().is_err() {
                        tracing::debug!("dmabuf client vanished during import");
                    }
                }
                Err(e) => {
                    tracing::debug!("dmabuf import refused: {e:?}");
                    notifier.failed();
                }
            }
        }
    }

    /// For backends without a renderer (headless).
    pub fn init_dmabuf_fallback(&mut self) {
        let formats = [Fourcc::Argb8888, Fourcc::Xrgb8888].map(|code| Format {
            code,
            modifier: Modifier::Linear,
        });
        self.init_dmabuf(FormatSet::from_iter(formats), None);
    }
}
