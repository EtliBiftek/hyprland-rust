//! Dual-kawase blur on offscreen textures.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTexProgram, GlesTexture, Uniform},
            Bind, Color32F, Frame, Offscreen, Renderer, Texture,
        },
    },
    utils::{Physical, Rectangle, Size, Transform},
};

use super::{fxtex::draw_tex, FxElement};

pub fn new_tex(r: &mut GlesRenderer, w: i32, h: i32) -> Result<GlesTexture, String> {
    r.create_buffer(Fourcc::Abgr8888, (w.max(1), h.max(1)).into())
        .map_err(|e| format!("create_buffer: {e:?}"))
}

/// Renders `elements` (front to back, output-local) into a new texture.
pub fn render_scene(
    r: &mut GlesRenderer,
    w: i32,
    h: i32,
    elements: &[&FxElement],
    clear: [f32; 4],
) -> Result<GlesTexture, String> {
    let mut tex = new_tex(r, w, h)?;
    {
        let mut target = r.bind(&mut tex).map_err(|e| format!("bind: {e:?}"))?;
        let mut dt = OutputDamageTracker::new((w, h), 1.0, Transform::Normal);
        dt.render_output(r, &mut target, 0, elements, clear)
            .map_err(|e| format!("render scene: {e:?}"))?;
    }
    Ok(tex)
}

/// Draws all of `src` over all of `dst` with `prog`.
fn pass(
    r: &mut GlesRenderer,
    src: &GlesTexture,
    dst: &GlesTexture,
    prog: &GlesTexProgram,
    uniforms: &[Uniform<'_>],
) -> Result<(), String> {
    let size = dst.size();
    let phys: Size<i32, Physical> = (size.w, size.h).into();
    let full = Rectangle::from_size(phys);
    let mut dst = dst.clone();
    let mut target = r.bind(&mut dst).map_err(|e| format!("bind: {e:?}"))?;
    let mut frame = r
        .render(&mut target, phys, Transform::Normal)
        .map_err(|e| format!("render: {e:?}"))?;
    frame
        .clear(Color32F::TRANSPARENT, &[full])
        .map_err(|e| format!("clear: {e:?}"))?;
    draw_tex(
        &mut frame,
        prog,
        src,
        None,
        Rectangle::from_size(src.size().to_f64()),
        full,
        &[full],
        1.0,
        uniforms,
    )
    .map_err(|e| format!("blur pass: {e:?}"))?;
    let _sync = frame.finish().map_err(|e| format!("finish: {e:?}"))?;
    Ok(())
}

/// Blurs `scene` in place: `passes` downsamples, then the same number of
/// upsamples (an even number of draws keeps the row order).
pub fn blur(
    r: &mut GlesRenderer,
    scene: &GlesTexture,
    down: &GlesTexProgram,
    up: &GlesTexProgram,
    passes: i32,
    size: i32,
) -> Result<(), String> {
    let full = scene.size();
    let offset = (size as f32 * 0.5).max(1.0);
    let mut chain = vec![scene.clone()];
    for i in 1..=passes.max(1) {
        chain.push(new_tex(r, full.w >> i, full.h >> i)?);
    }
    let hp = |t: &GlesTexture| {
        let s = t.size();
        Uniform::new("halfpixel", (0.5 / s.w as f32, 0.5 / s.h as f32))
    };
    for i in 1..chain.len() {
        let u = [hp(&chain[i - 1]), Uniform::new("offset", offset)];
        pass(r, &chain[i - 1], &chain[i], down, &u)?;
    }
    for i in (1..chain.len()).rev() {
        let u = [hp(&chain[i]), Uniform::new("offset", offset)];
        pass(r, &chain[i], &chain[i - 1], up, &u)?;
    }
    Ok(())
}
