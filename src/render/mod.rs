//! Visual effects: turns the windows, layers and config of one output into
//! render elements (rounded corners, gradient borders, shadows, blur,
//! opacity, dim, animations). The backends only call `output_elements`.
//!
//! Windows that need an effect (or any window while animations are on) are
//! drawn into an offscreen texture that is refreshed only when the client
//! commits, then composited by one custom shader element. Closing windows and
//! windows leaving on a workspace switch live on as "ghosts" using that cached
//! texture.
//! shortcuts: layer surfaces get blur but no open/close animation; the offscreen
//! copy is full-window (no direct scanout); `dim_around`, `dim_special` and
//! per-rule `animation` styles are parsed but not drawn; rule border colors are
//! solid, not gradients.

pub mod blur;
pub mod fxtex;
pub mod shaders;

use std::{cell::RefCell, collections::HashMap};

use smithay::{
    backend::renderer::{
        damage::OutputDamageTracker,
        element::{
            solid::SolidColorRenderElement,
            surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement},
            Element, Id, Kind,
        },
        gles::{
            element::PixelShaderElement, GlesPixelProgram, GlesRenderer, GlesTexProgram,
            GlesTexture, Uniform, UniformName, UniformType,
        },
        utils::CommitCounter,
        Bind,
    },
    desktop::{layer_map_for_output, PopupManager, Window},
    output::Output,
    utils::{Logical, Physical, Point, Rectangle, Scale, Transform},
    wayland::{seat::WaylandFocus, shell::wlr_layer::Layer},
};

use crate::{
    animation::{
        window_style, ws_style, Anim, Animations, FRect, Gradient, Resolved, Style, MAX_GRADIENT,
    },
    config::Decoration,
    rules::DynProps,
    state::Hypr,
};
use fxtex::FxTex;
use shaders::Programs;

smithay::backend::renderer::element::render_elements! {
    pub FxElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Solid=SolidColorRenderElement,
    Pixel=PixelShaderElement,
    Tex=FxTex,
    Cursor=smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement<GlesRenderer>,
}

const CLEAR: [f32; 4] = [0.1, 0.1, 0.1, 1.0];
type Key = Vec<(Id, CommitCounter)>;
type Surf = WaylandSurfaceRenderElement<GlesRenderer>;

// ------------------------------------------------------------------ state

/// Effect state of the compositor (all outputs).
#[derive(Default)]
pub struct FxState {
    progs: Option<Programs>,
    failed: bool,
    /// Windows drawn last frame.
    known: Vec<Window>,
    ws_key: Option<(i32, Option<i32>)>,
    ws_anim: Option<WsAnim>,
    ghosts: Vec<Ghost>,
    /// An animation was still running in the last `output_elements` call.
    pub animating: bool,
    /// `decoration:screen_shader`, compiled; `None` recompiles on next frame.
    pub shader: Option<ScreenShader>,
    /// Per output name: the last scene shown through the screen shader.
    screens: HashMap<String, ScreenSlot>,
}

pub struct ScreenShader {
    path: String,
    /// `None`: the file could not be read or compiled (logged once).
    prog: Option<GlesTexProgram>,
    uses_time: bool,
}

struct ScreenSlot {
    key: Vec<(Id, CommitCounter, Rectangle<i32, Physical>)>,
    tex: GlesTexture,
    el: FxTex,
}

struct WsAnim {
    anim: Anim,
    dir: f32,
    special: bool,
    style: Style,
}

struct Ghost {
    window: Window,
    anim: Anim,
    fade: Anim,
    style: Style,
    /// Workspace switch: (direction it leaves towards, special).
    ws: Option<(f32, bool)>,
}

#[derive(Clone, Default)]
struct Props {
    radius: f32,
    power: f32,
    opacity: f32,
    dim: f32,
    bsize: i32,
    grad: Gradient,
    shadow: Option<ShadowP>,
    /// Blur threshold on the surface alpha (-1: whole rect).
    blur: Option<f32>,
}

#[derive(Clone)]
struct ShadowP {
    range: f32,
    power: f32,
    sharp: bool,
    ignore_window: bool,
    color: [f32; 4],
    offset: (f32, f32),
    scale: f32,
}

#[derive(Default)]
struct PxSlot {
    el: Option<PixelShaderElement>,
    u: Vec<Uniform<'static>>,
    alpha: f32,
}

/// Per window (or layer surface) effect state, kept in its user data.
#[derive(Default)]
struct FxWin {
    seen: bool,
    dirty: bool,
    shown: bool,
    tex: Option<GlesTexture>,
    tex_size: (i32, i32),
    tex_key: Key,
    dt: Option<OutputDamageTracker>,
    el: Option<FxTex>,
    border: PxSlot,
    shadow: PxSlot,
    blur_key: Vec<(Id, CommitCounter, Rectangle<i32, Physical>)>,
    blur_cfg: (i32, i32),
    blur_tex: Option<GlesTexture>,
    from: FRect,
    to: FRect,
    mv: Option<Anim>,
    open: Option<(Anim, Anim, Style)>,
    ws_slide: bool,
    bfrom: Gradient,
    bto: Gradient,
    banim: Option<Anim>,
    props: Props,
    /// Displayed rect of the last live frame.
    last: FRect,
}

struct Info {
    w: Window,
    loc: (i32, i32),
    size: (i32, i32),
    focused: bool,
    fullscreen: bool,
    special: bool,
    dynp: DynProps,
    bsize: i32,
    grad: Gradient,
}

struct Ctx<'a> {
    progs: &'a Programs,
    deco: &'a Decoration,
    anim: &'a Animations,
    now: f64,
    scr: FRect,
    /// `scr` plus `general:gaps_workspaces`: how far a workspace slides.
    ws_scr: FRect,
    grabbed: bool,
    ws: Option<&'a WsAnim>,
    animating: bool,
}

impl Ctx<'_> {
    fn res(&self, name: &str) -> Resolved {
        self.anim.resolve(name)
    }
}

fn rect_i(r: FRect) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (r.x.round() as i32, r.y.round() as i32).into(),
        (r.w.round().max(1.0) as i32, r.h.round().max(1.0) as i32).into(),
    )
}

fn u<'a>(
    name: &'static str,
    v: impl Into<smithay::backend::renderer::gles::UniformValue>,
) -> Uniform<'a> {
    Uniform::new(name, v)
}

impl PxSlot {
    fn update(
        &mut self,
        prog: &GlesPixelProgram,
        area: Rectangle<i32, Logical>,
        uniforms: Vec<Uniform<'static>>,
        alpha: f32,
    ) -> PixelShaderElement {
        match &mut self.el {
            Some(el) if (self.alpha - alpha).abs() < 1e-4 => {
                el.resize(area, None);
                if self.u != uniforms {
                    el.update_uniforms(uniforms.clone());
                    self.u = uniforms;
                }
                el.clone()
            }
            _ => {
                let el = PixelShaderElement::new(
                    prog.clone(),
                    area,
                    None,
                    alpha,
                    uniforms.clone(),
                    Kind::Unspecified,
                );
                self.el = Some(el.clone());
                self.u = uniforms;
                self.alpha = alpha;
                el
            }
        }
    }
}

// ---------------------------------------------------------------- surfaces

fn surface_elems(
    r: &mut GlesRenderer,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    origin: (i32, i32),
) -> Vec<Surf> {
    render_elements_from_surface_tree(r, surface, origin, 1.0, 1.0, Kind::Unspecified)
}

fn popup_elems(
    r: &mut GlesRenderer,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    base: (i32, i32),
    geo_loc: Point<i32, Logical>,
) -> Vec<FxElement> {
    let mut out = Vec::new();
    for (popup, offset) in PopupManager::popups_for_surface(surface) {
        let o = geo_loc + offset - popup.geometry().loc;
        for e in surface_elems(r, popup.wl_surface(), (base.0 + o.x, base.1 + o.y)) {
            out.push(e.into());
        }
    }
    out
}

fn area_of(e: &Surf) -> i64 {
    e.opaque_regions(Scale::from(1.0))
        .iter()
        .map(|r| r.size.w as i64 * r.size.h as i64)
        .sum()
}

/// Redraws the offscreen copy when the surface changed. True if it did.
fn refresh_content(
    fw: &mut FxWin,
    r: &mut GlesRenderer,
    elems: &[Surf],
    size: (i32, i32),
) -> Result<bool, String> {
    let key: Key = elems
        .iter()
        .map(|e| (e.id().clone(), e.current_commit()))
        .collect();
    if fw.tex.is_some() && fw.tex_size == size && fw.tex_key == key {
        return Ok(false);
    }
    if fw.tex.is_none() || fw.tex_size != size {
        fw.tex = Some(blur::new_tex(r, size.0, size.1)?);
        fw.dt = Some(OutputDamageTracker::new(size, 1.0, Transform::Normal));
        fw.tex_size = size;
    }
    let mut tex = fw.tex.clone().unwrap();
    let mut target = r.bind(&mut tex).map_err(|e| format!("bind: {e:?}"))?;
    fw.dt
        .as_mut()
        .unwrap()
        .render_output(r, &mut target, 0, elems, [0.0, 0.0, 0.0, 0.0])
        .map_err(|e| format!("render window: {e:?}"))?;
    fw.tex_key = key;
    fw.dirty = true;
    Ok(true)
}

fn ensure_blur(
    ctx: &Ctx,
    fw: &mut FxWin,
    r: &mut GlesRenderer,
    below: &[&FxElement],
    out: (i32, i32),
) {
    let key: Vec<_> = below
        .iter()
        .map(|e| {
            (
                e.id().clone(),
                e.current_commit(),
                e.geometry(Scale::from(1.0)),
            )
        })
        .collect();
    let cfg = (ctx.deco.blur.size, ctx.deco.blur.passes);
    if fw.blur_tex.is_some() && fw.blur_key == key && fw.blur_cfg == cfg {
        return;
    }
    let p = ctx.progs;
    let res = blur::reuse_tex(r, fw.blur_tex.take(), out.0, out.1).and_then(|mut t| {
        blur::render_scene_into(r, &mut t, below, CLEAR)?;
        let chain = &mut p.blur_chain.borrow_mut();
        blur::blur(r, &t, &p.blur_down, &p.blur_up, cfg.1, cfg.0, chain)?;
        Ok(t)
    });
    match res {
        Ok(t) => {
            fw.blur_tex = Some(t);
            // Same texture id, new pixels: the element must count as changed.
            fw.dirty = true;
            fw.blur_key = key;
            fw.blur_cfg = cfg;
        }
        Err(e) => {
            tracing::warn!("blur failed: {e}");
            fw.blur_tex = None;
        }
    }
}

/// Content, border and shadow of a surface drawn at `rect` (front to back).
fn compose(ctx: &Ctx, fw: &mut FxWin, rect: FRect, alpha: f32, blur: bool) -> Vec<FxElement> {
    let mut out = Vec::new();
    let pr = fw.props.clone();
    let sc = if fw.to.w > 0.0 { rect.w / fw.to.w } else { 1.0 };
    let radius = pr.radius * sc;
    let area = rect_i(rect);
    let (w, h) = (area.size.w as f32, area.size.h as f32);
    let blur_tex = if blur && pr.blur.is_some() {
        fw.blur_tex.clone()
    } else {
        None
    };
    if let Some(tex) = fw.tex.clone() {
        let b = &ctx.deco.blur;
        let uniforms: Vec<Uniform<'static>> = vec![
            u("tex2", 1i32),
            u("size", (w, h)),
            u("radius", radius),
            u("power", pr.power),
            u("opacity", pr.opacity),
            u("dim", pr.dim),
            u("blur_on", if blur_tex.is_some() { 1.0f32 } else { 0.0 }),
            u("thr", pr.blur.unwrap_or(-1.0)),
            u(
                "ignore_opacity",
                if b.ignore_opacity { 1.0f32 } else { 0.0 },
            ),
            u("rect_xy", (area.loc.x as f32, area.loc.y as f32)),
            u("scr", (ctx.scr.w, ctx.scr.h)),
            u("brightness", b.brightness),
            u("contrast", b.contrast),
            u("noise", b.noise),
            u("vibrancy", b.vibrancy),
        ];
        let el = fw
            .el
            .get_or_insert_with(|| FxTex::new(ctx.progs.window.clone(), tex.clone()));
        let force = std::mem::take(&mut fw.dirty);
        el.update(&tex, blur_tex.as_ref(), area, uniforms, alpha, force);
        out.push(FxElement::Tex(el.clone()));
    }
    if pr.bsize > 0 {
        let b = pr.bsize as f32;
        let barea = Rectangle::new(
            (area.loc.x - pr.bsize, area.loc.y - pr.bsize).into(),
            (area.size.w + 2 * pr.bsize, area.size.h + 2 * pr.bsize).into(),
        );
        let mut g = pr.grad.clone();
        let n = g.colors.len().clamp(1, MAX_GRADIENT);
        let last = g.colors[n - 1];
        g.colors.resize(MAX_GRADIENT, last);
        let mut un: Vec<Uniform<'static>> = vec![
            u("ncol", n as f32),
            u("angle", g.angle),
            u("radius", radius),
            u("bsize", b),
            u("power", pr.power),
        ];
        const NAMES: [&str; 8] = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"];
        for (i, c) in g.colors.iter().take(MAX_GRADIENT).enumerate() {
            un.push(u(NAMES[i], *c));
        }
        let el = fw.border.update(&ctx.progs.border, barea, un, alpha);
        out.push(FxElement::Pixel(el));
    }
    if let Some(s) = &pr.shadow {
        let r = s.range;
        let ri = r.round() as i32;
        let sarea = Rectangle::new(
            (
                area.loc.x - ri + s.offset.0.round() as i32,
                area.loc.y - ri + s.offset.1.round() as i32,
            )
                .into(),
            (area.size.w + 2 * ri, area.size.h + 2 * ri).into(),
        );
        let ws = FRect::new(r, r, w, h).scaled(s.scale, s.scale);
        let un: Vec<Uniform<'static>> = vec![
            u("color", s.color),
            u("wrect", (ws.x, ws.y, ws.w, ws.h)),
            u("wradius", radius * s.scale),
            u("range", r),
            u("rpower", s.power),
            u("sharp", if s.sharp { 1.0f32 } else { 0.0 }),
            u("ignore_window", if s.ignore_window { 1.0f32 } else { 0.0 }),
            u("power", pr.power),
        ];
        let el = fw.shadow.update(&ctx.progs.shadow, sarea, un, alpha);
        out.push(FxElement::Pixel(el));
    }
    out
}

fn flatten(groups: &[Vec<FxElement>]) -> Vec<&FxElement> {
    groups.iter().rev().flatten().collect()
}

fn fw_of(cell: &smithay::utils::user_data::UserDataMap) -> &RefCell<FxWin> {
    cell.get_or_insert(|| RefCell::new(FxWin::default()))
}

// ------------------------------------------------------------------ windows

#[allow(clippy::too_many_arguments)]
fn window_group(
    ctx: &mut Ctx,
    r: &mut GlesRenderer,
    i: &Info,
    groups: &[Vec<FxElement>],
    lower_n: usize,
    out: (i32, i32),
) -> Vec<FxElement> {
    let deco = ctx.deco;
    let Some(surface) = i.w.wl_surface().map(|s| s.into_owned()) else {
        return Vec::new();
    };
    let geo = i.w.geometry();
    let cell = fw_of(i.w.user_data());
    let mut fw = cell.borrow_mut();
    let fw = &mut *fw;
    let now = ctx.now;
    let target = FRect::new(
        i.loc.0 as f32,
        i.loc.1 as f32,
        i.size.0 as f32,
        i.size.1 as f32,
    );

    // Geometry animation.
    let cur = |fw: &FxWin| match &fw.mv {
        Some(a) => FRect::lerp(fw.from, fw.to, a.value(now)),
        None => fw.to,
    };
    if !fw.shown {
        fw.from = target;
        fw.to = target;
        fw.mv = None;
    } else if fw.to != target {
        let res = ctx.res("windowsMove");
        if ctx.grabbed || !res.enabled {
            fw.from = target;
            fw.mv = None;
        } else {
            fw.from = cur(&*fw);
            fw.mv = Some(Anim::new(0.0, 1.0, now, &res));
        }
        fw.to = target;
    }
    let mut rect = cur(&*fw);
    if fw.mv.as_ref().is_some_and(|a| a.done(now)) {
        fw.mv = None;
        fw.from = fw.to;
    }
    ctx.animating |= fw.mv.is_some();

    // Open animation (first time shown).
    let mut alpha = 1.0f32;
    if !fw.seen {
        fw.seen = true;
        let r_in = ctx.res("windowsIn");
        if r_in.enabled {
            let f = ctx.res("fadeIn");
            fw.open = Some((
                Anim::new(0.0, 1.0, now, &r_in),
                Anim::new(0.0, 1.0, now, &f),
                r_in.style,
            ));
        }
    }
    if let Some((a, f, style)) = fw.open.clone() {
        if a.done(now) && f.done(now) {
            fw.open = None;
        } else {
            ctx.animating = true;
            let (rr, ao) = window_style(style, a.progress(now), rect, ctx.scr);
            rect = rr;
            alpha *= ao.unwrap_or_else(|| f.progress(now));
        }
    }
    // Workspace switch: windows arriving slide in.
    match ctx.ws {
        Some(wa) if fw.ws_slide => {
            let (dx, dy, ao) = ws_style(
                wa.style,
                wa.anim.progress(now),
                wa.dir,
                wa.special,
                ctx.ws_scr,
            );
            rect = rect.moved(dx, dy);
            alpha *= ao.unwrap_or(1.0);
        }
        _ => fw.ws_slide = false,
    }
    fw.shown = true;
    fw.last = rect;

    // Border color (animated on focus change / rule change).
    let br = ctx.res("border");
    if fw.bto.colors.is_empty() {
        fw.bfrom = i.grad.clone();
        fw.bto = i.grad.clone();
    } else if fw.bto != i.grad {
        let curg = match &fw.banim {
            Some(a) => fw.bfrom.mix(&fw.bto, a.value(now)),
            None => fw.bto.clone(),
        };
        fw.bfrom = curg;
        fw.bto = i.grad.clone();
        fw.banim = br.enabled.then(|| Anim::new(0.0, 1.0, now, &br));
    }
    let mut grad = match &fw.banim {
        Some(a) if !a.done(now) => {
            ctx.animating = true;
            fw.bfrom.mix(&fw.bto, a.value(now))
        }
        _ => {
            fw.banim = None;
            fw.bto.clone()
        }
    };
    let ba = ctx.res("borderangle");
    if ba.enabled && ba.style == Style::Loop && ba.duration > 0.0 {
        grad.angle += ((now % ba.duration) / ba.duration) as f32 * 360.0;
        ctx.animating = true;
    }

    // Effective properties.
    let d = &i.dynp;
    let fs = i.fullscreen;
    let rounding = if fs {
        0
    } else {
        d.rounding.unwrap_or(deco.rounding)
    };
    let power = d
        .other("rounding_power")
        .and_then(|a| a.trim().parse::<f32>().ok())
        .unwrap_or(deco.rounding_power);
    let base = if fs {
        deco.fullscreen_opacity
    } else if i.focused {
        deco.active_opacity
    } else {
        deco.inactive_opacity
    };
    let opacity = match d.opacity {
        Some(o) => {
            let (v, ov) = if fs {
                o.fullscreen
            } else if i.focused {
                o.active
            } else {
                o.inactive
            };
            if ov {
                v
            } else {
                base * v
            }
        }
        None => base,
    } * crate::handlers::hyprland_surface::surface_opacity(&i.w);
    let dim = if deco.dim_inactive && !i.focused && !d.flag("no_dim") {
        deco.dim_strength
    } else {
        0.0
    };
    let shadow = (deco.shadow.enabled && !fs && deco.shadow.range > 0 && !d.flag("no_shadow"))
        .then(|| {
            let s = &deco.shadow;
            let color = if i.focused {
                s.color
            } else {
                s.color_inactive.unwrap_or(s.color)
            };
            ShadowP {
                range: s.range as f32,
                power: s.render_power as f32,
                sharp: s.sharp,
                ignore_window: s.ignore_window,
                color,
                offset: s.offset,
                scale: s.scale,
            }
        });
    let anims_on = ctx.anim.enabled;
    let need_tex = rounding > 0 || opacity < 0.999 || dim > 0.0 || anims_on || ctx.animating;

    // Surface elements at the offscreen origin.
    let elems = surface_elems(r, &surface, (-geo.loc.x, -geo.loc.y));
    let opaque = elems.iter().map(area_of).sum::<i64>() >= (geo.size.w as i64 * geo.size.h as i64);
    let want_blur = deco.blur.enabled
        && !d.flag("no_blur")
        && !fs
        && (!i.special || deco.blur.special)
        && (opacity < 0.999 || !opaque);
    fw.props = Props {
        radius: rounding as f32,
        power,
        opacity,
        dim,
        bsize: if fs { 0 } else { i.bsize },
        grad,
        shadow,
        blur: want_blur.then_some(-1.0),
    };

    let mut items: Vec<FxElement> = popup_elems(
        r,
        &surface,
        (
            rect.x.round() as i32 - geo.loc.x,
            rect.y.round() as i32 - geo.loc.y,
        ),
        geo.loc,
    );
    if need_tex || want_blur {
        match refresh_content(fw, r, &elems, (geo.size.w.max(1), geo.size.h.max(1))) {
            Ok(_) => {}
            Err(e) => tracing::warn!("{e}"),
        }
        if want_blur {
            let below = flatten_below(groups, lower_n, deco.blur.xray);
            ensure_blur(ctx, fw, r, &below, out);
        }
        items.extend(compose(ctx, fw, rect, alpha, want_blur));
    } else {
        // Plain path: surfaces straight at the window position, borders around.
        let org = (
            rect.x.round() as i32 - geo.loc.x,
            rect.y.round() as i32 - geo.loc.y,
        );
        items.extend(
            surface_elems(r, &surface, org)
                .into_iter()
                .map(FxElement::from),
        );
        fw.tex = None;
        fw.el = None;
        let mut c = compose(ctx, fw, rect, alpha, false);
        items.append(&mut c);
    }
    items
}

fn flatten_below(groups: &[Vec<FxElement>], lower_n: usize, xray: bool) -> Vec<&FxElement> {
    flatten(
        &groups[..if xray {
            lower_n.min(groups.len())
        } else {
            groups.len()
        }],
    )
}

/// A ghost: the cached texture of a window that closed or left.
fn ghost_group(ctx: &mut Ctx, g: &Ghost) -> Vec<FxElement> {
    let cell = fw_of(g.window.user_data());
    let mut fw = cell.borrow_mut();
    let fw = &mut *fw;
    if fw.tex.is_none() {
        return Vec::new();
    }
    let now = ctx.now;
    let p = g.anim.progress(now);
    let q = 1.0 - p;
    let base = fw.last;
    let (rect, alpha) = match g.ws {
        Some((dir, special)) => {
            let (dx, dy, ao) = ws_style(g.style, q, dir, special, ctx.ws_scr);
            (base.moved(dx, dy), ao.unwrap_or(1.0))
        }
        None => {
            let (rr, ao) = window_style(g.style, q, base, ctx.scr);
            (rr, ao.unwrap_or(1.0 - g.fade.progress(now)))
        }
    };
    fw.to = base;
    compose(ctx, fw, rect, alpha, false)
}

// -------------------------------------------------------------------- entry

/// All elements of `output`, front to back, in output-local coordinates
/// (cursor excluded).
pub fn output_elements(h: &mut Hypr, r: &mut GlesRenderer, output: &Output) -> Vec<FxElement> {
    if h.fx.progs.is_none() && !h.fx.failed {
        match Programs::compile(r) {
            Ok(p) => h.fx.progs = Some(p),
            Err(e) => {
                tracing::error!("effect shaders failed to compile, effects off: {e:?}");
                h.fx.failed = true;
            }
        }
    }
    let Some(geo) = h.space.output_geometry(output) else {
        return Vec::new();
    };
    let origin = (geo.loc.x, geo.loc.y);
    let out = (geo.size.w, geo.size.h);
    let scr = FRect::new(0.0, 0.0, out.0 as f32, out.1 as f32);
    let ws_scr = scr.with_gap(
        match crate::options::get_option(&h.config, "general:gaps_workspaces") {
            Some(crate::options::OptValue::Int(g)) => g as f32,
            _ => 0.0,
        },
    );
    let now = h.start_time.elapsed().as_secs_f64();

    // Gather (immutable) data.
    let focused = h.focused_window();
    let mut infos = Vec::new();
    for w in h.space.elements() {
        let Some(l) = h.space.element_location(w) else {
            continue;
        };
        let size = w.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        let wsi = h.ws_of(w);
        let ws = wsi.map(|i| &h.workspaces[i]);
        let is_focus = focused.as_ref() == Some(w);
        let g = &h.config.general;
        let gc = &h.config.group;
        let (rule_size, rc) = h.rule_border(w);
        let rule = if is_focus { rc.active } else { rc.inactive };
        let grad = match rule {
            Some(c) => Gradient::solid(c),
            None => match (ws.and_then(|ws| ws.groups.group_of(w)), is_focus) {
                (None, true) => g.active_border.clone(),
                (None, false) => g.inactive_border.clone(),
                (Some(grp), true) if grp.locked => Gradient::solid(gc.border_locked_active),
                (Some(_), true) => Gradient::solid(gc.border_active),
                (Some(grp), false) if grp.locked => Gradient::solid(gc.border_locked_inactive),
                (Some(_), false) => Gradient::solid(gc.border_inactive),
            },
        };
        infos.push(Info {
            w: w.clone(),
            loc: (l.x - origin.0, l.y - origin.1),
            size: (size.w, size.h),
            focused: is_focus,
            fullscreen: ws.is_some_and(|ws| ws.fullscreen.as_ref() == Some(w)),
            special: ws.is_some_and(|ws| ws.is_special()),
            dynp: h.window_dynp(w),
            bsize: rule_size.unwrap_or(g.border_size),
            grad,
        });
    }
    let groupbar: Vec<FxElement> = h
        .groupbar_elements(geo.loc)
        .into_iter()
        .map(Into::into)
        .collect();
    let deco = h.config.decoration.clone();
    let mut anim = h.config.anim.clone();
    let grabbed = h.seat.get_pointer().is_some_and(|p| p.is_grabbed());
    let key = (h.workspaces[h.active_workspace].id, h.wm.special_shown);
    let layers: Vec<_> = {
        let map = layer_map_for_output(output);
        map.layers()
            .filter_map(|l| {
                let g = map.layer_geometry(l)?;
                Some((l.clone(), g, l.layer()))
            })
            .collect()
    };
    let layer_rules: Vec<Vec<(String, String)>> = layers
        .iter()
        .map(|(l, _, _)| {
            crate::rules::layer_effects(&h.config.rules.layer, l.namespace())
                .into_iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        })
        .collect();

    let fx = &mut h.fx;
    if fx.failed {
        anim.enabled = false;
    }
    // Workspace switch detection.
    if let Some(prev) = fx.ws_key {
        if prev != key && anim.enabled {
            let special = prev.0 == key.0;
            let (dir, name) = if special {
                (
                    if key.1.is_some() { -1.0 } else { 1.0 },
                    "specialWorkspaceIn",
                )
            } else {
                (if key.0 > prev.0 { 1.0 } else { -1.0 }, "workspacesIn")
            };
            let rin = anim.resolve(name);
            if rin.enabled {
                for i in &infos {
                    if !fx.known.contains(&i.w) {
                        let c = fw_of(i.w.user_data());
                        let mut fw = c.borrow_mut();
                        if fw.seen {
                            fw.ws_slide = true;
                        }
                    }
                }
                let ws_changed = true;
                let rout = anim.resolve(if special {
                    "specialWorkspaceOut"
                } else {
                    "workspacesOut"
                });
                if ws_changed && rout.enabled {
                    for w in fx.known.clone() {
                        if infos.iter().any(|i| i.w == w) {
                            continue;
                        }
                        {
                            fx.ghosts.push(Ghost {
                                window: w,
                                anim: Anim::new(0.0, 1.0, now, &rout),
                                fade: Anim::still(1.0),
                                style: rout.style,
                                ws: Some((-dir, special)),
                            });
                        }
                    }
                }
                fx.ws_anim = Some(WsAnim {
                    anim: Anim::new(0.0, 1.0, now, &rin),
                    dir,
                    special,
                    style: rin.style,
                });
            }
        }
    }
    fx.ws_key = Some(key);
    if fx.ws_anim.as_ref().is_some_and(|w| w.anim.done(now)) {
        fx.ws_anim = None;
    }
    fx.ghosts.retain(|g| !g.anim.done(now));
    let Some(progs) = fx.progs.take() else {
        // Shaders unavailable: plain windows and layers, no effects.
        let mut v = groupbar;
        for (l, g, _) in layers.iter().rev() {
            v.extend(
                surface_elems(r, l.wl_surface(), (g.loc.x, g.loc.y))
                    .into_iter()
                    .map(FxElement::from),
            );
        }
        for i in infos.iter().rev() {
            if let Some(s) = i.w.wl_surface() {
                let o = (
                    i.loc.0 - i.w.geometry().loc.x,
                    i.loc.1 - i.w.geometry().loc.y,
                );
                v.extend(surface_elems(r, &s, o).into_iter().map(FxElement::from));
            }
        }
        return v;
    };
    let ws_anim = fx.ws_anim.take();
    let mut ctx = Ctx {
        progs: &progs,
        deco: &deco,
        anim: &anim,
        now,
        scr,
        ws_scr,
        grabbed,
        ws: ws_anim.as_ref(),
        animating: ws_anim.is_some(),
    };

    // Back to front: lower layers, windows, ghosts, upper layers.
    let mut groups: Vec<Vec<FxElement>> = Vec::new();
    let layer_group = |ctx: &mut Ctx,
                       r: &mut GlesRenderer,
                       groups: &mut Vec<Vec<FxElement>>,
                       lower_n: usize,
                       idx: usize| {
        let (l, g, _) = &layers[idx];
        let surface = l.wl_surface();
        let org = (g.loc.x, g.loc.y);
        let mut items = popup_elems(r, surface, org, Point::from((0, 0)));
        let rules = &layer_rules[idx];
        let wants_blur = ctx.deco.blur.enabled
            && rules
                .iter()
                .any(|(k, v)| k == "blur" && !matches!(v.trim(), "off" | "0" | "false"));
        let cell = fw_of(l.user_data());
        let mut fw = cell.borrow_mut();
        let fw = &mut *fw;
        if wants_blur && g.size.w > 0 && g.size.h > 0 {
            let thr = rules
                .iter()
                .find(|(k, _)| k == "ignore_alpha")
                .map_or(-1.0, |(_, v)| v.trim().parse::<f32>().unwrap_or(0.0));
            let elems = surface_elems(r, surface, (0, 0));
            if let Err(e) = refresh_content(fw, r, &elems, (g.size.w, g.size.h)) {
                tracing::warn!("{e}");
            }
            let below = flatten_below(groups, lower_n, ctx.deco.blur.xray);
            ensure_blur(ctx, fw, r, &below, out);
            fw.props = Props {
                radius: 0.0,
                power: 2.0,
                opacity: 1.0,
                blur: Some(thr),
                ..Default::default()
            };
            fw.to = FRect::new(
                g.loc.x as f32,
                g.loc.y as f32,
                g.size.w as f32,
                g.size.h as f32,
            );
            let rect = fw.to;
            items.extend(compose(ctx, fw, rect, 1.0, true));
        } else {
            fw.tex = None;
            fw.el = None;
            items.extend(
                surface_elems(r, surface, org)
                    .into_iter()
                    .map(FxElement::from),
            );
        }
        groups.push(items);
    };
    for (idx, lay) in layers.iter().enumerate() {
        if matches!(lay.2, Layer::Background | Layer::Bottom) {
            layer_group(&mut ctx, r, &mut groups, 0, idx);
        }
    }
    let lower_n = groups.len();
    let mut seen_now: Vec<Window> = Vec::new();
    for i in &infos {
        seen_now.push(i.w.clone());
        let g = window_group(&mut ctx, r, i, &groups, lower_n, out);
        groups.push(g);
    }
    let ghosts = std::mem::take(&mut h.fx.ghosts);
    for g in &ghosts {
        ctx.animating = true;
        let items = ghost_group(&mut ctx, g);
        groups.push(items);
    }
    for (idx, lay) in layers.iter().enumerate() {
        if matches!(lay.2, Layer::Top | Layer::Overlay) {
            layer_group(&mut ctx, r, &mut groups, lower_n, idx);
        }
    }
    let animating = ctx.animating;

    // Windows that disappeared: closing windows become ghosts.
    let fx = &mut h.fx;
    fx.ghosts = ghosts;
    fx.ws_anim = ws_anim;
    fx.progs = Some(progs);
    fx.animating = animating;
    let gone: Vec<Window> = fx
        .known
        .iter()
        .filter(|w| !seen_now.contains(w))
        .cloned()
        .collect();
    fx.known = seen_now;
    if anim.enabled {
        let rout = anim.resolve("windowsOut");
        if rout.enabled {
            let fade = anim.resolve("fadeOut");
            for w in gone {
                let alive = h.workspaces.iter().any(|ws| ws.contains(&w));
                if !alive && !h.fx.ghosts.iter().any(|g| g.window == w) {
                    h.fx.ghosts.push(Ghost {
                        window: w,
                        anim: Anim::new(0.0, 1.0, now, &rout),
                        fade: Anim::new(0.0, 1.0, now, &fade),
                        style: rout.style,
                        ws: None,
                    });
                }
            }
        }
    }

    let mut v = groupbar;
    v.extend(groups.into_iter().rev().flatten());
    let v = cursor_zoom(h, r, output, geo, v);
    let v = screen_shader(h, r, output, out, now, v);
    output_scale_pass(h, r, output, out, v)
}

/// The effects are all built in logical pixels. On a scaled output the scene
/// is drawn at that size and stretched over the output.
/// shortcut: blurry on HiDPI; render the scene at physical size to sharpen it.
fn output_scale_pass(
    h: &mut Hypr,
    r: &mut GlesRenderer,
    output: &Output,
    out: (i32, i32),
    v: Vec<FxElement>,
) -> Vec<FxElement> {
    if (output.current_scale().fractional_scale() - 1.0).abs() < 1e-3 {
        return v;
    }
    let Some(prog) = h.fx.progs.as_ref().map(|p| p.plain.clone()) else {
        return v;
    };
    let area = Rectangle::new((0, 0).into(), out.into());
    let crop = vec![u("crop", (0.0f32, 0.0f32, 1.0f32, 1.0f32))];
    match post_pass(
        &mut h.fx.screens,
        r,
        format!("scale:{}", output.name()),
        prog,
        out,
        &v,
        area,
        crop,
    ) {
        Some(el) => vec![el],
        None => v,
    }
}

/// `decoration:screen_shader`: draws the finished scene into a texture and
/// returns one full-output element that shows it through the user's shader.
/// shortcut: the cursor is drawn above the shader and screenshots include it.
fn screen_shader(
    h: &mut Hypr,
    r: &mut GlesRenderer,
    output: &Output,
    out: (i32, i32),
    now: f64,
    v: Vec<FxElement>,
) -> Vec<FxElement> {
    let path = match crate::options::get_option(&h.config, "decoration:screen_shader") {
        Some(crate::options::OptValue::Str(p)) if !p.is_empty() && p != "[[EMPTY]]" => p,
        _ => {
            h.fx.shader = None;
            return v;
        }
    };
    let fx = &mut h.fx;
    if fx.shader.as_ref().is_none_or(|s| s.path != path) {
        fx.shader = Some(load_screen_shader(r, path));
        fx.screens.clear();
    }
    let Some((prog, time)) = fx
        .shader
        .as_ref()
        .and_then(|s| Some((s.prog.clone()?, s.uses_time)))
    else {
        return v;
    };
    let uniforms = if time {
        fx.animating = true;
        vec![u("time", now as f32)]
    } else {
        Vec::new()
    };
    let area = Rectangle::new((0, 0).into(), out.into());
    match post_pass(
        &mut fx.screens,
        r,
        output.name(),
        prog,
        out,
        &v,
        area,
        uniforms,
    ) {
        Some(el) => vec![el],
        None => v,
    }
}

/// `cursor:zoom_factor`: shows the finished scene scaled around the cursor.
/// shortcuts: the cursor itself is drawn by the backends and is not scaled;
/// `zoom_disable_aa` and `zoom_detached_camera` are ignored.
fn cursor_zoom(
    h: &mut Hypr,
    r: &mut GlesRenderer,
    output: &Output,
    geo: Rectangle<i32, Logical>,
    v: Vec<FxElement>,
) -> Vec<FxElement> {
    use crate::options::{get_option, OptValue};
    let zoom = match get_option(&h.config, "cursor:zoom_factor") {
        Some(OptValue::Float(f)) if f > 1.0 => f as f32,
        _ => return v,
    };
    let rigid = get_option(&h.config, "cursor:zoom_rigid") == Some(OptValue::Bool(true));
    let (Some(prog), Some(pointer)) = (
        h.fx.progs.as_ref().map(|p| p.plain.clone()),
        h.seat.get_pointer(),
    ) else {
        return v;
    };
    let c = pointer.current_location() - geo.loc.to_f64();
    let out = (geo.size.w, geo.size.h);
    let rect = crate::animation::zoom_rect(
        (out.0 as f32, out.1 as f32),
        (c.x as f32, c.y as f32),
        zoom,
        rigid,
    );
    h.fx.animating = true;
    let crop = (
        -rect.x / rect.w,
        -rect.y / rect.h,
        out.0 as f32 / rect.w,
        out.1 as f32 / rect.h,
    );
    let area = Rectangle::new((0, 0).into(), out.into());
    match post_pass(
        &mut h.fx.screens,
        r,
        format!("zoom:{}", output.name()),
        prog,
        out,
        &v,
        area,
        vec![u("crop", crop)],
    ) {
        Some(el) => vec![el],
        None => v,
    }
}

/// Draws `v` into a cached texture (redrawn only when `v` changed) and returns
/// one element showing it at `area` through `prog`.
#[allow(clippy::too_many_arguments)]
fn post_pass(
    slots: &mut HashMap<String, ScreenSlot>,
    r: &mut GlesRenderer,
    name: String,
    prog: GlesTexProgram,
    out: (i32, i32),
    v: &[FxElement],
    area: Rectangle<i32, Logical>,
    uniforms: Vec<Uniform<'static>>,
) -> Option<FxElement> {
    let refs: Vec<&FxElement> = v.iter().collect();
    let key: Vec<_> = refs
        .iter()
        .map(|e| {
            (
                e.id().clone(),
                e.current_commit(),
                e.geometry(Scale::from(1.0)),
            )
        })
        .collect();
    let force = slots.get(&name).is_none_or(|s| s.key != key);
    if force {
        let old = slots.get(&name).map(|s| s.tex.clone());
        let tex = blur::reuse_tex(r, old, out.0, out.1)
            .and_then(|mut t| {
                blur::render_scene_into(r, &mut t, &refs, CLEAR)?;
                Ok(t)
            })
            .map_err(|e| tracing::warn!("post pass: {e}"))
            .ok()?;
        let slot = slots.entry(name.clone()).or_insert_with(|| ScreenSlot {
            key: Vec::new(),
            tex: tex.clone(),
            el: FxTex::new(prog, tex.clone()),
        });
        slot.key = key;
        slot.tex = tex;
    }
    let slot = slots.get_mut(&name)?;
    slot.el.update(&slot.tex, None, area, uniforms, 1.0, force);
    Some(FxElement::Tex(slot.el.clone()))
}

fn load_screen_shader(r: &mut GlesRenderer, path: String) -> ScreenShader {
    let file = crate::config::expand_home(&path);
    let src = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("screen shader {}: {e}", file.display());
            return ScreenShader {
                path,
                prog: None,
                uses_time: false,
            };
        }
    };
    let uses_time = src.contains("time");
    let names = [UniformName::new("time", UniformType::_1f)];
    let prog = r
        .compile_custom_texture_shader(shaders::adapt_screen_shader(&src), &names)
        .map_err(|e| tracing::error!("screen shader {}: {e:?}", file.display()))
        .ok();
    ScreenShader {
        path,
        prog,
        uses_time,
    }
}
