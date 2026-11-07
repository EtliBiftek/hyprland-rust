//! GLSL ES 1.00 shaders and their compilation.
//!
//! Orientation: offscreen textures hold GL-natural rows (image top at the last
//! row), so shaders that read one flip `v_coords.y`.

use smithay::backend::renderer::gles::{
    GlesError, GlesPixelProgram, GlesRenderer, GlesTexProgram, UniformName, UniformType,
};

use crate::animation::MAX_GRADIENT;

/// Signed distance to a rounded box (negative inside). `n` is the corner
/// exponent (`rounding_power`, 2 = circle).
const SDF: &str = r#"
float box_dist(vec2 p, vec2 hs, float r, float n) {
    vec2 q = abs(p) - (hs - vec2(r));
    if (q.x > 0.0 && q.y > 0.0) {
        return pow(pow(q.x, n) + pow(q.y, n), 1.0 / n) - r;
    }
    return max(q.x, q.y) - r;
}
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}
"#;

const PRECISION: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
"#;

fn tex_header() -> String {
    format!(
        "#version 100\n//_DEFINES_\n#if defined(EXTERNAL)\n#extension GL_OES_EGL_image_external : require\n#endif\n{PRECISION}\n\
#if defined(EXTERNAL)\nuniform samplerExternalOES tex;\n#else\nuniform sampler2D tex;\n#endif\n\
uniform float alpha;\nvarying vec2 v_coords;\n#if defined(DEBUG_FLAGS)\nuniform float tint;\n#endif\n"
    )
}

/// Window / layer surface: rounded mask, opacity, dim, optional blurred
/// backdrop (`tex2`, full output, sampled at `rect + pixel`).
pub fn window_tex_src() -> String {
    format!(
        "{}{SDF}{}",
        tex_header(),
        r#"
uniform sampler2D tex2;
uniform vec2 size;
uniform float radius;
uniform float power;
uniform float opacity;
uniform float dim;
uniform float blur_on;
uniform float thr;
uniform float ignore_opacity;
uniform vec2 rect_xy;
uniform vec2 scr;
uniform float brightness;
uniform float contrast;
uniform float noise;
uniform float vibrancy;
void main() {
    vec4 c = texture2D(tex, vec2(v_coords.x, 1.0 - v_coords.y));
#if defined(NO_ALPHA)
    c = vec4(c.rgb, 1.0);
#endif
    vec2 p = v_coords * size;
    float d = box_dist(p - size * 0.5, size * 0.5, radius, power);
    float m = clamp(0.5 - d, 0.0, 1.0);
    c.rgb *= (1.0 - dim);
    vec4 o = c * opacity;
    if (blur_on > 0.5 && c.a > thr) {
        vec2 sp = rect_xy + p;
        vec3 b = texture2D(tex2, vec2(sp.x / scr.x, 1.0 - sp.y / scr.y)).rgb;
        float lum = dot(b, vec3(0.299, 0.587, 0.114));
        b = mix(vec3(lum), b, 1.0 + vibrancy);
        b = (b - 0.5) * contrast + 0.5;
        b *= brightness;
        b += (hash(sp) - 0.5) * noise;
        b = clamp(b, 0.0, 1.0);
        float ba = ignore_opacity > 0.5 ? 1.0 : opacity;
        o = o + (1.0 - o.a) * vec4(b * ba, ba);
    }
    gl_FragColor = o * m * alpha;
}
"#
    )
}

/// One blur pass. `down` = dual-kawase downsample, else upsample.
pub fn blur_src(down: bool) -> String {
    let body = if down {
        r#"
    vec4 sum = texture2D(tex, uv) * 4.0;
    sum += texture2D(tex, uv - halfpixel * offset);
    sum += texture2D(tex, uv + halfpixel * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset);
    sum += texture2D(tex, uv - vec2(halfpixel.x, -halfpixel.y) * offset);
    gl_FragColor = sum / 8.0;
"#
    } else {
        r#"
    vec4 sum = texture2D(tex, uv + vec2(-halfpixel.x * 2.0, 0.0) * offset);
    sum += texture2D(tex, uv + vec2(-halfpixel.x, halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, halfpixel.y * 2.0) * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(halfpixel.x * 2.0, 0.0) * offset);
    sum += texture2D(tex, uv + vec2(halfpixel.x, -halfpixel.y) * offset) * 2.0;
    sum += texture2D(tex, uv + vec2(0.0, -halfpixel.y * 2.0) * offset);
    sum += texture2D(tex, uv + vec2(-halfpixel.x, -halfpixel.y) * offset) * 2.0;
    gl_FragColor = sum / 12.0;
"#
    };
    format!(
        "{}uniform vec2 halfpixel;\nuniform float offset;\nvoid main() {{\n    vec2 uv = v_coords;{body}}}\n",
        tex_header()
    )
}

fn pixel_header() -> String {
    format!(
        "{PRECISION}\nvarying vec2 v_coords;\nuniform vec2 size;\nuniform float alpha;\n#if defined(DEBUG_FLAGS)\nuniform float tint;\n#endif\n"
    )
}

fn color_uniforms() -> String {
    let mut s = String::new();
    for i in 0..MAX_GRADIENT {
        s += &format!("uniform vec4 c{i};\n");
    }
    s
}

/// Gradient color at position `s` (0..1) from `c0..c7`, `ncol` colors.
fn gradient_fn() -> String {
    let mut s = String::from("vec4 col_at(int i) {\n");
    for i in 0..MAX_GRADIENT {
        s += &format!("    if (i == {i}) return c{i};\n");
    }
    s += "    return c0;\n}\n";
    s += r#"
vec4 grad(float t) {
    if (ncol < 1.5) return c0;
    float x = clamp(t, 0.0, 1.0) * (ncol - 1.0);
    int i = int(floor(x));
    float f = x - float(i);
    vec4 a = col_at(i);
    vec4 b = col_at(i + 1);
    if (f <= 0.0) return a;
    return mix(a, b, f);
}
"#;
    s
}

/// Border ring: outer rounded box (the whole element) minus the inner one,
/// colored by a gradient at `angle` degrees.
pub fn border_src() -> String {
    format!(
        "{}{SDF}{}uniform float ncol;\nuniform float angle;\nuniform float radius;\nuniform float bsize;\nuniform float power;\n{}{}",
        pixel_header(),
        color_uniforms(),
        "",
        gradient_fn()
    ) + r#"
void main() {
    vec2 p = v_coords * size;
    vec2 c = p - size * 0.5;
    float dout = box_dist(c, size * 0.5, radius + bsize, power);
    float din = box_dist(c, size * 0.5 - vec2(bsize), radius, power);
    float m = clamp(0.5 - dout, 0.0, 1.0) * clamp(0.5 + din, 0.0, 1.0);
    float a = radians(angle);
    vec2 dir = vec2(cos(a), -sin(a));
    float ext = max(abs(dir.x) * size.x + abs(dir.y) * size.y, 0.000001);
    float t = dot(c, dir) / ext + 0.5;
    vec4 col = grad(t);
    gl_FragColor = vec4(col.rgb * col.a, col.a) * (m * alpha);
}
"#
}

/// Drop shadow around the window rect `wrect` (x, y, w, h inside the element).
pub fn shadow_src() -> String {
    format!(
        "{}{SDF}uniform vec4 color;\nuniform vec4 wrect;\nuniform float wradius;\nuniform float range;\nuniform float rpower;\nuniform float sharp;\nuniform float ignore_window;\nuniform float power;\n",
        pixel_header()
    ) + r#"
void main() {
    vec2 p = v_coords * size;
    vec2 c = p - (wrect.xy + wrect.zw * 0.5);
    float d = box_dist(c, wrect.zw * 0.5, wradius, power);
    float a;
    if (sharp > 0.5) {
        a = clamp(0.5 - d, 0.0, 1.0);
    } else {
        a = d <= 0.0 ? 1.0 : pow(clamp(1.0 - d / max(range, 1.0), 0.0, 1.0), rpower);
    }
    if (ignore_window > 0.5) {
        a *= clamp(0.5 + d, 0.0, 1.0);
    }
    float al = color.a * a;
    gl_FragColor = vec4(color.rgb * al, al) * alpha;
}
"#
}

pub struct Programs {
    pub window: GlesTexProgram,
    pub blur_down: GlesTexProgram,
    pub blur_up: GlesTexProgram,
    pub border: GlesPixelProgram,
    pub shadow: GlesPixelProgram,
}

fn f1(n: &'static str) -> UniformName<'static> {
    UniformName::new(n, UniformType::_1f)
}
fn f2(n: &'static str) -> UniformName<'static> {
    UniformName::new(n, UniformType::_2f)
}

impl Programs {
    pub fn compile(r: &mut GlesRenderer) -> Result<Programs, GlesError> {
        let window = r.compile_custom_texture_shader(
            window_tex_src(),
            &[
                UniformName::new("tex2", UniformType::_1i),
                f2("size"),
                f1("radius"),
                f1("power"),
                f1("opacity"),
                f1("dim"),
                f1("blur_on"),
                f1("thr"),
                f1("ignore_opacity"),
                f2("rect_xy"),
                f2("scr"),
                f1("brightness"),
                f1("contrast"),
                f1("noise"),
                f1("vibrancy"),
            ],
        )?;
        let blur_u = [f2("halfpixel"), f1("offset")];
        let blur_down = r.compile_custom_texture_shader(blur_src(true), &blur_u)?;
        let blur_up = r.compile_custom_texture_shader(blur_src(false), &blur_u)?;
        let mut bu = vec![f1("ncol"), f1("angle"), f1("radius"), f1("bsize"), f1("power")];
        let names: Vec<&'static str> = ["c0", "c1", "c2", "c3", "c4", "c5", "c6", "c7"]
            .into_iter()
            .take(MAX_GRADIENT)
            .collect();
        bu.extend(
            names
                .iter()
                .map(|n| UniformName::new(*n, UniformType::_4f)),
        );
        let border = r.compile_custom_pixel_shader(border_src(), &bu)?;
        let shadow = r.compile_custom_pixel_shader(
            shadow_src(),
            &[
                UniformName::new("color", UniformType::_4f),
                UniformName::new("wrect", UniformType::_4f),
                f1("wradius"),
                f1("range"),
                f1("rpower"),
                f1("sharp"),
                f1("ignore_window"),
                f1("power"),
            ],
        )?;
        Ok(Programs {
            window,
            blur_down,
            blur_up,
            border,
            shadow,
        })
    }
}
