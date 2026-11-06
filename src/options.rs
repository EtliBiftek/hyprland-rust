//! `hyprctl getoption`: option lookup and Hyprland-shaped output.
//!
//! To expose a new config option, add a match arm to `get_option`.

use crate::config::Config;

#[derive(Debug, Clone, PartialEq)]
pub enum OptValue {
    Int(i64),
    Bool(bool),
    Float(f64),
    Str(String),
    /// Complex value printed by `toString()` in Hyprland (gradients etc.).
    Custom(String),
}

/// Gradient as Hyprland prints it: `aarrggbb [aarrggbb ...] 0deg`.
fn gradient(c: [f32; 4]) -> String {
    let [r, g, b, a] = c.map(|v| (v * 255.0).round() as u32);
    format!("{:x} 0deg", a << 24 | r << 16 | g << 8 | b)
}

pub fn get_option(cfg: &Config, key: &str) -> Option<OptValue> {
    use OptValue::*;
    Some(match key {
        "general:gaps_in" => Int(cfg.general.gaps_in.into()),
        "general:gaps_out" => Int(cfg.general.gaps_out.into()),
        "general:border_size" => Int(cfg.general.border_size.into()),
        "general:col.active_border" => Custom(gradient(cfg.general.active_border_color)),
        "general:col.inactive_border" => Custom(gradient(cfg.general.inactive_border_color)),
        "decoration:rounding" => Int(cfg.decoration.rounding.into()),
        "dwindle:preserve_split" => Bool(cfg.dwindle.preserve_split),
        "dwindle:default_split_ratio" => Float(cfg.dwindle.default_split_ratio.into()),
        "misc:disable_autoreload" => Bool(cfg.misc.disable_autoreload),
        _ => return None,
    })
}

/// Output of `getoption`. Shapes (including the space before `}` and `{:2f}`
/// float formatting) follow Hyprland's `dispatchGetOption`.
pub fn format_option(key: &str, v: &OptValue, set: bool, json: bool) -> String {
    let (ty, val, quoted) = match v {
        OptValue::Int(i) => ("int", i.to_string(), false),
        OptValue::Bool(b) => ("bool", b.to_string(), false),
        OptValue::Float(f) => ("float", format!("{f:.6}"), false),
        OptValue::Str(s) => ("str", s.clone(), true),
        OptValue::Custom(s) => ("custom", s.clone(), true),
    };
    if json {
        let val = if quoted {
            serde_json::to_string(&val).unwrap_or_default()
        } else {
            val
        };
        format!(r#"{{"option": "{key}", "{ty}": {val}, "set": {set} }}"#)
    } else {
        let ty = if ty == "custom" { "custom type" } else { ty };
        format!("{ty}: {val}\nset: {set}")
    }
}

/// `keyword <key> <value>` -> (key, value).
pub fn parse_keyword(args: &str) -> Result<(&str, &str), &'static str> {
    let args = args.trim();
    match args.split_once(char::is_whitespace) {
        Some((k, v)) if !v.trim().is_empty() => Ok((k, v.trim())),
        _ => Err("not enough args"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getoption_shapes() {
        let mut cfg = Config::default();
        cfg.general.gaps_in = 7;
        let v = get_option(&cfg, "general:gaps_in").unwrap();
        assert_eq!(
            format_option("general:gaps_in", &v, true, true),
            r#"{"option": "general:gaps_in", "int": 7, "set": true }"#
        );
        assert_eq!(
            format_option("general:gaps_in", &v, false, false),
            "int: 7\nset: false"
        );
        let v = get_option(&cfg, "dwindle:default_split_ratio").unwrap();
        assert_eq!(
            format_option("k", &v, false, false),
            "float: 1.000000\nset: false"
        );
        let v = get_option(&cfg, "general:col.active_border").unwrap();
        assert_eq!(
            format_option("k", &v, false, true),
            r#"{"option": "k", "custom": "ffffffff 0deg", "set": false }"#
        );
        assert!(get_option(&cfg, "nope:nope").is_none());
    }

    #[test]
    fn keyword_args() {
        assert_eq!(
            parse_keyword("general:gaps_in 10"),
            Ok(("general:gaps_in", "10"))
        );
        assert_eq!(
            parse_keyword("bind SUPER,X,exec,foo bar"),
            Ok(("bind", "SUPER,X,exec,foo bar"))
        );
        assert!(parse_keyword("general:gaps_in").is_err());
    }
}
