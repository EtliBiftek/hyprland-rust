//! Animations: cubic beziers, the `animation = NAME, ...` inheritance tree,
//! styles and scalar tweens, plus border gradients. Pure (no Smithay), unit
//! tested. Times are seconds as `f64`.

use std::collections::HashMap;
use std::sync::Arc;

const SAMPLES: usize = 255;

/// Cubic bezier from (0,0) to (1,1), sampled like Hyprland (255 points).
#[derive(Debug, Clone, PartialEq)]
pub struct Bezier {
    pts: Vec<(f32, f32)>,
}

impl Bezier {
    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let pts = (0..SAMPLES)
            .map(|i| {
                let t = i as f32 / (SAMPLES - 1) as f32;
                let u = 1.0 - t;
                let b = |a: f32, b: f32| 3.0 * u * u * t * a + 3.0 * u * t * t * b + t * t * t;
                (b(x0, x1), b(y0, y1))
            })
            .collect();
        Self { pts }
    }

    /// Eased value for linear progress `x` in 0..=1.
    pub fn y(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        // Points are monotonic in x for sane curves: binary search the segment.
        let i = self.pts.partition_point(|p| p.0 < x).clamp(1, SAMPLES - 1);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        if (b.0 - a.0).abs() < 1e-9 {
            return b.1;
        }
        a.1 + (b.1 - a.1) * ((x - a.0) / (b.0 - a.0)).clamp(0.0, 1.0)
    }
}

/// Named curves; `default` and `linear` are built in.
#[derive(Debug, Clone, PartialEq)]
pub struct Curves(HashMap<String, Arc<Bezier>>);

impl Default for Curves {
    fn default() -> Self {
        let mut m = HashMap::new();
        m.insert("default".into(), Arc::new(Bezier::new(0.0, 0.75, 0.15, 1.0)));
        m.insert("linear".into(), Arc::new(Bezier::new(0.0, 0.0, 1.0, 1.0)));
        Curves(m)
    }
}

impl Curves {
    /// `bezier = name, x0, y0, x1, y1`.
    pub fn add(&mut self, value: &str) -> Result<(), String> {
        let p: Vec<&str> = value.split(',').map(str::trim).collect();
        if p.len() != 5 || p[0].is_empty() {
            return Err("bezier: expected `name, x0, y0, x1, y1`".into());
        }
        let f = |s: &str| s.parse::<f32>().map_err(|_| format!("bezier: bad number `{s}`"));
        let (x0, y0, x1, y1) = (f(p[1])?, f(p[2])?, f(p[3])?, f(p[4])?);
        self.0
            .insert(p[0].to_string(), Arc::new(Bezier::new(x0.clamp(0.0, 1.0), y0, x1.clamp(0.0, 1.0), y1)));
        Ok(())
    }

    pub fn get(&self, name: &str) -> Arc<Bezier> {
        self.0.get(name).unwrap_or(&self.0["default"]).clone()
    }
}

/// Animation style (`popin 80%`, `slide left`, ...).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Style {
    /// Node default: popin for windows, slide for workspaces, fade for layers.
    Default,
    /// Scale from this fraction (0.8 = 80%) while fading.
    Popin(f32),
    Slide(Option<Side>),
    SlideVert,
    Gnomed,
    Fade,
    /// Slide + fade, fraction of the size to travel (1.0 = full).
    SlideFade(f32),
    SlideFadeVert(f32),
    Once,
    Loop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Style {
    pub fn parse(s: &str) -> Style {
        let mut it = s.split_whitespace();
        let name = it.next().unwrap_or("").to_ascii_lowercase();
        let arg = it.next();
        let pct = |d: f32| {
            arg.and_then(|a| a.trim_end_matches('%').parse::<f32>().ok())
                .map_or(d, |v| v / 100.0)
        };
        match name.as_str() {
            "popin" => Style::Popin(pct(0.8)),
            "slide" => Style::Slide(arg.and_then(|a| match a {
                "left" => Some(Side::Left),
                "right" => Some(Side::Right),
                "top" => Some(Side::Top),
                "bottom" => Some(Side::Bottom),
                _ => None,
            })),
            "slidevert" => Style::SlideVert,
            "gnomed" | "gnome" => Style::Gnomed,
            "fade" => Style::Fade,
            "slidefade" => Style::SlideFade(pct(1.0)),
            "slidefadevert" => Style::SlideFadeVert(pct(1.0)),
            "once" => Style::Once,
            "loop" => Style::Loop,
            _ => Style::Default,
        }
    }
}

/// Resolved settings of one animation node.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub enabled: bool,
    /// Seconds.
    pub duration: f64,
    pub curve: Arc<Bezier>,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq)]
struct Setting {
    enabled: bool,
    speed: f32,
    curve: String,
    style: String,
}

/// Parent of each node (global is the root).
const TREE: &[(&str, &str)] = &[
    ("windows", "global"),
    ("windowsIn", "windows"),
    ("windowsOut", "windows"),
    ("windowsMove", "windows"),
    ("layers", "global"),
    ("layersIn", "layers"),
    ("layersOut", "layers"),
    ("fade", "global"),
    ("fadeIn", "fade"),
    ("fadeOut", "fade"),
    ("fadeSwitch", "fade"),
    ("fadeShadow", "fade"),
    ("fadeDim", "fade"),
    ("fadeLayers", "fade"),
    ("fadeLayersIn", "fadeLayers"),
    ("fadeLayersOut", "fadeLayers"),
    ("fadePopups", "fade"),
    ("fadePopupsIn", "fadePopups"),
    ("fadePopupsOut", "fadePopups"),
    ("fadeDpms", "fade"),
    ("border", "global"),
    ("borderangle", "global"),
    ("workspaces", "global"),
    ("workspacesIn", "workspaces"),
    ("workspacesOut", "workspaces"),
    ("specialWorkspace", "workspaces"),
    ("specialWorkspaceIn", "specialWorkspace"),
    ("specialWorkspaceOut", "specialWorkspace"),
    ("zoomFactor", "global"),
    ("monitorAdded", "global"),
];

/// `animations { enabled }`, `bezier` and `animation` lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Animations {
    pub enabled: bool,
    pub curves: Curves,
    set: HashMap<String, Setting>,
}

impl Default for Animations {
    fn default() -> Self {
        let mut set = HashMap::new();
        set.insert(
            "global".to_string(),
            Setting {
                enabled: true,
                speed: 8.0,
                curve: "default".into(),
                style: String::new(),
            },
        );
        Self {
            enabled: true,
            curves: Curves::default(),
            set,
        }
    }
}

impl Animations {
    /// `animation = NAME, ONOFF, SPEED, CURVE[, STYLE]`.
    pub fn add(&mut self, value: &str) -> Result<(), String> {
        let p: Vec<&str> = value.split(',').map(str::trim).collect();
        let name = p[0];
        if name != "global" && !TREE.iter().any(|(n, _)| *n == name) {
            return Err(format!("animation: unknown animation `{name}`"));
        }
        let on = match p.get(1).copied() {
            Some("1") | Some("true") | Some("yes") | Some("on") => true,
            Some("0") | Some("false") | Some("no") | Some("off") => false,
            _ => return Err("animation: expected ONOFF (0/1)".into()),
        };
        let speed = match p.get(2) {
            Some(s) => s.parse::<f32>().map_err(|_| format!("animation: bad speed `{s}`"))?,
            None if !on => 8.0,
            None => return Err("animation: missing speed".into()),
        };
        let curve = p.get(3).copied().unwrap_or("default").to_string();
        let style = p[4.min(p.len())..].join(",");
        self.set.insert(
            name.to_string(),
            Setting {
                enabled: on,
                speed,
                curve,
                style,
            },
        );
        Ok(())
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Settings of `name`, inherited from the nearest ancestor that was set.
    pub fn resolve(&self, name: &str) -> Resolved {
        let mut n = name;
        loop {
            if let Some(s) = self.set.get(n) {
                // A node without its own style inherits the style too.
                let style = if s.style.is_empty() {
                    self.inherited_style(n)
                } else {
                    s.style.clone()
                };
                return Resolved {
                    enabled: self.enabled && s.enabled,
                    duration: (s.speed.max(0.0) * 0.1) as f64,
                    curve: self.curves.get(&s.curve),
                    style: Style::parse(&style),
                };
            }
            match TREE.iter().find(|(c, _)| *c == n) {
                Some((_, parent)) => n = parent,
                None => n = "global",
            }
        }
    }

    fn inherited_style(&self, mut n: &str) -> String {
        while let Some((_, parent)) = TREE.iter().find(|(c, _)| *c == n) {
            n = parent;
            if let Some(s) = self.set.get(n) {
                if !s.style.is_empty() {
                    return s.style.clone();
                }
            }
        }
        String::new()
    }
}

/// A scalar tween.
#[derive(Debug, Clone)]
pub struct Anim {
    from: f32,
    to: f32,
    start: f64,
    dur: f64,
    curve: Arc<Bezier>,
}

impl Anim {
    pub fn new(from: f32, to: f32, now: f64, r: &Resolved) -> Self {
        Self {
            from,
            to,
            start: now,
            dur: if r.enabled { r.duration } else { 0.0 },
            curve: r.curve.clone(),
        }
    }

    pub fn still(v: f32) -> Self {
        Self {
            from: v,
            to: v,
            start: 0.0,
            dur: 0.0,
            curve: Arc::new(Bezier::new(0.0, 0.0, 1.0, 1.0)),
        }
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn done(&self, now: f64) -> bool {
        self.dur <= 0.0 || now >= self.start + self.dur
    }

    /// Eased progress 0..=1.
    pub fn progress(&self, now: f64) -> f32 {
        if self.done(now) {
            1.0
        } else {
            self.curve.y(((now - self.start) / self.dur) as f32)
        }
    }

    pub fn value(&self, now: f64) -> f32 {
        self.from + (self.to - self.from) * self.progress(now)
    }

    /// Retarget from the current value (keeps the motion continuous).
    pub fn retarget(&mut self, to: f32, now: f64, r: &Resolved) {
        *self = Anim::new(self.value(now), to, now, r);
    }
}

// ----------------------------------------------------------------- gradients

pub const MAX_GRADIENT: usize = 8;

/// `rgba(..) rgba(..) 45deg`: colors (premultiplication not applied) and angle.
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub colors: Vec<[f32; 4]>,
    /// Degrees, 0 = left to right, counter-clockwise like Hyprland.
    pub angle: f32,
}

impl Default for Gradient {
    fn default() -> Self {
        Gradient::solid([0.0; 4])
    }
}

impl Gradient {
    pub fn solid(c: [f32; 4]) -> Self {
        Gradient {
            colors: vec![c],
            angle: 0.0,
        }
    }

    /// Parses colors with `parse` and an optional trailing `Ndeg`.
    pub fn parse(
        value: &str,
        parse: impl Fn(&str) -> Result<[f32; 4], String>,
    ) -> Result<Gradient, String> {
        let mut colors = Vec::new();
        let mut angle = 0.0;
        for tok in value.split_whitespace() {
            if let Some(d) = tok.strip_suffix("deg") {
                angle = d.parse::<f32>().map_err(|_| format!("invalid angle `{tok}`"))?;
            } else {
                colors.push(parse(tok)?);
            }
        }
        if colors.is_empty() {
            return Err(format!("invalid color `{value}`"));
        }
        colors.truncate(MAX_GRADIENT);
        Ok(Gradient { colors, angle })
    }

    /// Linear blend; the shorter gradient is padded with its last color.
    pub fn mix(&self, other: &Gradient, t: f32) -> Gradient {
        let n = self.colors.len().max(other.colors.len());
        let at = |g: &Gradient, i: usize| g.colors[i.min(g.colors.len() - 1)];
        let colors = (0..n)
            .map(|i| {
                let (a, b) = (at(self, i), at(other, i));
                [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * t)
            })
            .collect();
        Gradient {
            colors,
            angle: self.angle + (other.angle - self.angle) * t,
        }
    }

    /// Color at position `s` (0..=1) along the gradient line.
    pub fn at(&self, s: f32) -> [f32; 4] {
        let n = self.colors.len();
        if n == 1 {
            return self.colors[0];
        }
        let p = s.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (p.floor() as usize).min(n - 2);
        let f = p - i as f32;
        let (a, b) = (self.colors[i], self.colors[i + 1]);
        [0, 1, 2, 3].map(|k| a[k] + (b[k] - a[k]) * f)
    }

    /// Hyprland's `getoption` format: `aarrggbb [...] 45deg`.
    pub fn to_hypr(&self) -> String {
        let mut s = String::new();
        for c in &self.colors {
            let [r, g, b, a] = c.map(|v| (v * 255.0).round() as u32);
            s.push_str(&format!("{:x} ", a << 24 | r << 16 | g << 8 | b));
        }
        s.push_str(&format!("{}deg", self.angle.round() as i32));
        s
    }
}

/// Position (0..=1) of pixel `(x, y)` of a `w` by `h` box along a gradient of
/// `angle` degrees (0 = left to right, 90 = bottom to top).
pub fn gradient_pos(x: f32, y: f32, w: f32, h: f32, angle: f32) -> f32 {
    let a = angle.to_radians();
    let (dx, dy) = (a.cos(), -a.sin());
    // Project the centered point on the direction; normalize by the box extent.
    let ext = (w * dx.abs() + h * dy.abs()).max(1e-6);
    (((x - w / 2.0) * dx + (y - h / 2.0) * dy) / ext + 0.5).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_endpoints_and_linear() {
        let b = Bezier::new(0.0, 0.0, 1.0, 1.0);
        assert!(b.y(0.0).abs() < 1e-3);
        assert!((b.y(1.0) - 1.0).abs() < 1e-3);
        assert!((b.y(0.5) - 0.5).abs() < 0.01);
        let ease = Bezier::new(0.05, 0.9, 0.1, 1.05);
        assert!(ease.y(0.3) > 0.6, "fast start");
        let mut c = Curves::default();
        c.add("myb, 0.05, 0.9, 0.1, 1.05").unwrap();
        assert!(c.add("bad, 1, 2").is_err());
        assert!((c.get("myb").y(0.3) - ease.y(0.3)).abs() < 1e-6);
        assert_eq!(c.get("nope"), c.get("default"));
    }

    #[test]
    fn tree_inheritance() {
        let mut a = Animations::default();
        a.curves.add("c, 0, 0, 1, 1").unwrap();
        let r = a.resolve("windowsIn");
        assert!(r.enabled);
        assert!((r.duration - 0.8).abs() < 1e-6);
        a.add("windows, 1, 4, c, popin 50%").unwrap();
        a.add("windowsOut, 0").unwrap();
        a.add("fade, 1, 2, default").unwrap();
        let r = a.resolve("windowsIn");
        assert!((r.duration - 0.4).abs() < 1e-6);
        assert_eq!(r.style, Style::Popin(0.5));
        assert!(!a.resolve("windowsOut").enabled);
        // Not set: falls to its parent (windows) for move, fade for fadeIn.
        assert_eq!(a.resolve("windowsMove").style, Style::Popin(0.5));
        assert!((a.resolve("fadeIn").duration - 0.2).abs() < 1e-6);
        // global applies to the rest.
        assert!((a.resolve("border").duration - 0.8).abs() < 1e-6);
        a.enabled = false;
        assert!(!a.resolve("windowsIn").enabled);
        assert!(a.add("nonsense, 1, 2, default").is_err());
    }

    #[test]
    fn styles() {
        assert_eq!(Style::parse("slide"), Style::Slide(None));
        assert_eq!(Style::parse("slide left"), Style::Slide(Some(Side::Left)));
        assert_eq!(Style::parse("slidefade 20%"), Style::SlideFade(0.2));
        assert_eq!(Style::parse("popin"), Style::Popin(0.8));
        assert_eq!(Style::parse("loop"), Style::Loop);
        assert_eq!(Style::parse(""), Style::Default);
    }

    #[test]
    fn tween() {
        let mut a = Animations::default();
        a.add("global, 1, 10, linear").unwrap();
        let r = a.resolve("windowsMove");
        let t = Anim::new(0.0, 100.0, 5.0, &r);
        assert!((t.value(5.5) - 50.0).abs() < 1.0);
        assert!(!t.done(5.9));
        assert!(t.done(6.0));
        assert_eq!(t.value(7.0), 100.0);
        let mut t = t;
        t.retarget(0.0, 5.5, &r);
        assert!((t.value(5.5) - 50.0).abs() < 1.0);
        a.enabled = false;
        assert!(Anim::new(0.0, 1.0, 0.0, &a.resolve("windowsMove")).done(0.0));
    }

    fn col(s: &str) -> Result<[f32; 4], String> {
        match s {
            "r" => Ok([1.0, 0.0, 0.0, 1.0]),
            "b" => Ok([0.0, 0.0, 1.0, 1.0]),
            _ => Err("bad".into()),
        }
    }

    #[test]
    fn gradient_math() {
        let g = Gradient::parse("r b 45deg", col).unwrap();
        assert_eq!(g.angle, 45.0);
        assert_eq!(g.colors.len(), 2);
        assert!(Gradient::parse("45deg", col).is_err());
        assert_eq!(g.at(0.0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(g.at(1.0), [0.0, 0.0, 1.0, 1.0]);
        let m = g.at(0.5);
        assert!((m[0] - 0.5).abs() < 1e-6 && (m[2] - 0.5).abs() < 1e-6);
        // 0deg: left is the start, right the end. 90deg: bottom to top.
        assert!(gradient_pos(0.0, 5.0, 100.0, 10.0, 0.0) < 0.01);
        assert!(gradient_pos(100.0, 5.0, 100.0, 10.0, 0.0) > 0.99);
        assert!(gradient_pos(50.0, 10.0, 10.0, 10.0, 90.0) < 0.01);
        assert!(gradient_pos(50.0, 0.0, 100.0, 10.0, 90.0) > 0.99);
        let half = g.mix(&Gradient::solid([0.0, 1.0, 0.0, 1.0]), 0.5);
        assert_eq!(half.colors.len(), 2);
        assert!((half.angle - 22.5).abs() < 1e-6);
        assert_eq!(g.to_hypr(), "ffff0000 ff0000ff 45deg");
    }
}

// ------------------------------------------------------------ style geometry

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl FRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn lerp(a: FRect, b: FRect, t: f32) -> FRect {
        let l = |p: f32, q: f32| p + (q - p) * t;
        FRect::new(l(a.x, b.x), l(a.y, b.y), l(a.w, b.w), l(a.h, b.h))
    }

    /// Scaled about its center.
    pub fn scaled(self, sx: f32, sy: f32) -> FRect {
        let (w, h) = (self.w * sx, self.h * sy);
        FRect::new(self.x + (self.w - w) / 2.0, self.y + (self.h - h) / 2.0, w, h)
    }

    pub fn moved(self, dx: f32, dy: f32) -> FRect {
        FRect::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// Screen edge nearest to `r`.
pub fn nearest_side(r: FRect, scr: FRect) -> Side {
    let d = [
        (r.x - scr.x, Side::Left),
        (scr.x + scr.w - (r.x + r.w), Side::Right),
        (r.y - scr.y, Side::Top),
        (scr.y + scr.h - (r.y + r.h), Side::Bottom),
    ];
    d.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap().1
}

/// Window open/close geometry. `q` is how present the window is (0 = gone,
/// 1 = fully there). Returns the rect to draw and, for styles that fade by
/// themselves, an alpha that replaces the `fade` animation.
pub fn window_style(style: Style, q: f32, r: FRect, scr: FRect) -> (FRect, Option<f32>) {
    let slide = |side: Side, frac: f32| {
        let off = match side {
            Side::Left => (scr.x - (r.x + r.w), 0.0),
            Side::Right => (scr.x + scr.w - r.x, 0.0),
            Side::Top => (0.0, scr.y - (r.y + r.h)),
            Side::Bottom => (0.0, scr.y + scr.h - r.y),
        };
        r.moved(off.0 * (1.0 - q) * frac, off.1 * (1.0 - q) * frac)
    };
    match style {
        Style::Popin(m) => (r.scaled(m + (1.0 - m) * q, m + (1.0 - m) * q), None),
        Style::Default | Style::Once | Style::Loop => {
            (r.scaled(0.8 + 0.2 * q, 0.8 + 0.2 * q), None)
        }
        Style::Gnomed => (r.scaled(1.0, 0.05 + 0.95 * q), None),
        Style::Fade => (r, Some(q)),
        Style::Slide(side) => (slide(side.unwrap_or_else(|| nearest_side(r, scr)), 1.0), None),
        Style::SlideVert => (slide(Side::Bottom, 1.0), None),
        Style::SlideFade(f) => (slide(nearest_side(r, scr), f), Some(q)),
        Style::SlideFadeVert(f) => (slide(Side::Bottom, f), Some(q)),
    }
}

/// Workspace switch: window offset `(dx, dy)` and an alpha override. `q`:
/// presence 0..=1; `dir` +1 when the window comes from the right/bottom.
pub fn ws_style(
    style: Style,
    q: f32,
    dir: f32,
    special: bool,
    scr: FRect,
) -> (f32, f32, Option<f32>) {
    let k = 1.0 - q;
    match style {
        Style::Fade => (0.0, 0.0, Some(q)),
        Style::SlideVert => (0.0, dir * k * scr.h, None),
        Style::SlideFadeVert(f) => (0.0, dir * k * scr.h * f, Some(q)),
        Style::SlideFade(f) if special => (0.0, dir * k * scr.h * f, Some(q)),
        Style::SlideFade(f) => (dir * k * scr.w * f, 0.0, Some(q)),
        _ if special => (0.0, dir * k * scr.h, None),
        _ => (dir * k * scr.w, 0.0, None),
    }
}

#[cfg(test)]
mod style_tests {
    use super::*;

    #[test]
    fn styles_geometry() {
        let scr = FRect::new(0.0, 0.0, 1000.0, 800.0);
        let r = FRect::new(100.0, 100.0, 200.0, 100.0);
        assert_eq!(window_style(Style::Popin(0.5), 1.0, r, scr).0, r);
        let (h, a) = window_style(Style::Popin(0.5), 0.0, r, scr);
        assert_eq!((h.w, h.h, a), (100.0, 50.0, None));
        assert!((h.x + h.w / 2.0 - 200.0).abs() < 1e-4, "stays centered");
        let (s, _) = window_style(Style::Slide(None), 0.0, r, scr);
        assert!(s.x + s.w <= 1e-4);
        assert_eq!(window_style(Style::Slide(None), 1.0, r, scr).0, r);
        assert_eq!(window_style(Style::Fade, 0.25, r, scr).1, Some(0.25));
        let (dx, dy, a) = ws_style(Style::Default, 0.0, 1.0, false, scr);
        assert_eq!((dx, dy, a), (1000.0, 0.0, None));
        let (dx, dy, a) = ws_style(Style::SlideFade(0.5), 0.5, -1.0, false, scr);
        assert_eq!((dx, dy, a), (-250.0, 0.0, Some(0.5)));
        assert_eq!(ws_style(Style::Default, 0.0, 1.0, true, scr).1, 800.0);
        assert_eq!(nearest_side(FRect::new(900.0, 300.0, 90.0, 50.0), scr), Side::Right);
    }
}
