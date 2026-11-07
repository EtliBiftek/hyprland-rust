//! Cursor images for backends that draw the cursor themselves (TTY): xcursor
//! themes (`XCURSOR_THEME`, `XCURSOR_SIZE`, Hyprland's `HYPRCURSOR_*` are not
//! read), named cursors from cursor-shape, and client cursor surfaces.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    utils::{Logical, Physical, Point, Transform},
    wayland::compositor::with_states,
};

pub enum CursorElement {
    Image(MemoryRenderBufferRenderElement<GlesRenderer>),
    Surface(WaylandSurfaceRenderElement<GlesRenderer>),
}

struct Image {
    buffer: MemoryRenderBuffer,
    hotspot: (i32, i32),
}

pub struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    cache: HashMap<String, Option<Image>>,
    /// Shown when the theme has no image: a small white square.
    fallback: Image,
}

impl Cursors {
    pub fn new() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(24);
        let px = vec![0xffu8; 8 * 8 * 4];
        Cursors {
            theme: xcursor::CursorTheme::load(&name),
            size,
            cache: HashMap::new(),
            fallback: Image {
                buffer: MemoryRenderBuffer::from_slice(
                    &px,
                    Fourcc::Abgr8888,
                    (8, 8),
                    1,
                    Transform::Normal,
                    None,
                ),
                hotspot: (0, 0),
            },
        }
    }

    fn load(&self, name: &str) -> Option<Image> {
        let path = self.theme.load_icon(name)?;
        let data = std::fs::read(path).ok()?;
        let images = xcursor::parser::parse_xcursor(&data)?;
        let img = images
            .iter()
            .min_by_key(|i| (i.size as i64 - self.size as i64).abs())?;
        Some(Image {
            buffer: MemoryRenderBuffer::from_slice(
                &img.pixels_rgba,
                Fourcc::Abgr8888,
                (img.width as i32, img.height as i32),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: (img.xhot as i32, img.yhot as i32),
        })
    }

    /// Elements for the cursor at `pos` (output-local logical position).
    pub fn elements(
        &mut self,
        renderer: &mut GlesRenderer,
        status: &CursorImageStatus,
        pos: Point<f64, Logical>,
        scale: f64,
    ) -> Vec<CursorElement> {
        match status {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(surface, |s| {
                    s.data_map
                        .get::<CursorImageSurfaceData>()
                        .map(|d| d.lock().unwrap().hotspot)
                        .unwrap_or_default()
                });
                let loc: Point<i32, Physical> =
                    (pos - hotspot.to_f64()).to_physical(scale).to_i32_round();
                render_elements_from_surface_tree(renderer, surface, loc, scale, 1.0, Kind::Cursor)
                    .into_iter()
                    .map(CursorElement::Surface)
                    .collect()
            }
            CursorImageStatus::Named(icon) => {
                let name = icon.name().to_string();
                if !self.cache.contains_key(&name) {
                    // xcursor themes name the arrow `left_ptr`.
                    let img = self
                        .load(&name)
                        .or_else(|| (name == "default").then(|| self.load("left_ptr")).flatten());
                    self.cache.insert(name.clone(), img);
                }
                let img = self.cache[&name].as_ref().unwrap_or(&self.fallback);
                let loc = (pos - Point::from((img.hotspot.0 as f64, img.hotspot.1 as f64)))
                    .to_physical(scale);
                match MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    loc,
                    &img.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    Ok(e) => vec![CursorElement::Image(e)],
                    Err(e) => {
                        tracing::warn!("cursor upload failed: {e:?}");
                        Vec::new()
                    }
                }
            }
        }
    }
}
