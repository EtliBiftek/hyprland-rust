//! Render element drawing a texture (an offscreen copy of a surface) with a
//! custom shader and an optional second texture (the blurred backdrop).
//! Unlike Smithay's `TextureShaderElement` it keeps its id across frames, so
//! the damage tracker only redraws what really changed.

use smithay::{
    backend::renderer::{
        element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
        gles::{
            ffi, GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
        },
        utils::CommitCounter,
        Texture,
    },
    utils::{Buffer, Logical, Physical, Rectangle, Scale},
};

#[derive(Debug, Clone)]
pub struct FxTex {
    id: Id,
    commit: CommitCounter,
    tex: GlesTexture,
    tex2: Option<GlesTexture>,
    prog: GlesTexProgram,
    pub area: Rectangle<i32, Logical>,
    uniforms: Vec<Uniform<'static>>,
    alpha: f32,
}

impl FxTex {
    pub fn new(prog: GlesTexProgram, tex: GlesTexture) -> Self {
        Self {
            id: Id::new(),
            commit: CommitCounter::default(),
            tex,
            tex2: None,
            prog,
            area: Rectangle::default(),
            uniforms: Vec::new(),
            alpha: 1.0,
        }
    }

    /// Replaces everything that affects the drawn pixels; bumps the commit
    /// counter only when something differs (or `force`: new texture contents).
    pub fn update(
        &mut self,
        tex: &GlesTexture,
        tex2: Option<&GlesTexture>,
        area: Rectangle<i32, Logical>,
        uniforms: Vec<Uniform<'static>>,
        alpha: f32,
        force: bool,
    ) {
        let changed = force
            || self.area != area
            || self.uniforms != uniforms
            || (self.alpha - alpha).abs() > 1e-4
            || self.tex2.as_ref().map(|t| t.tex_id()) != tex2.map(|t| t.tex_id())
            || self.tex.tex_id() != tex.tex_id();
        if changed {
            self.tex = tex.clone();
            self.tex2 = tex2.cloned();
            self.area = area;
            self.uniforms = uniforms;
            self.alpha = alpha;
            self.commit.increment();
        }
    }
}

impl Element for FxTex {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.tex.size().to_f64())
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.area.to_physical_precise_round(scale)
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

/// Makes `id` (bound to `unit`) a complete NPOT texture: clamped and linear.
pub fn prepare_unit(gl: &ffi::Gles2, unit: u32, id: u32) {
    unsafe {
        gl.ActiveTexture(unit);
        gl.BindTexture(ffi::TEXTURE_2D, id);
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE as i32);
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE as i32);
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
        gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
        gl.ActiveTexture(ffi::TEXTURE0);
    }
}

/// Draws `tex` (plus `tex2` on unit 1, sampler uniform `tex2` = 1) with `prog`.
#[allow(clippy::too_many_arguments)]
pub fn draw_tex(
    frame: &mut GlesFrame<'_, '_>,
    prog: &GlesTexProgram,
    tex: &GlesTexture,
    tex2: Option<&GlesTexture>,
    src: Rectangle<f64, Buffer>,
    dst: Rectangle<i32, Physical>,
    damage: &[Rectangle<i32, Physical>],
    alpha: f32,
    uniforms: &[Uniform<'_>],
) -> Result<(), GlesError> {
    let (id1, id2) = (tex.tex_id(), tex2.map(|t| t.tex_id()));
    frame.with_context(|gl| {
        prepare_unit(gl, ffi::TEXTURE0, id1);
        if let Some(id2) = id2 {
            prepare_unit(gl, ffi::TEXTURE1, id2);
        }
    })?;
    frame.render_texture_from_to(
        tex,
        src,
        dst,
        damage,
        &[],
        smithay::utils::Transform::Normal,
        alpha,
        Some(prog),
        uniforms,
    )
}

impl RenderElement<GlesRenderer> for FxTex {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        draw_tex(
            frame,
            &self.prog,
            &self.tex,
            self.tex2.as_ref(),
            src,
            dst,
            damage,
            self.alpha,
            &self.uniforms,
        )
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
