//! `hyprctl getoption`: option lookup and Hyprland-shaped output.
//!
//! To expose a new config option, add a match arm to `get_option`.

use crate::{
    config::Config,
    master::{NewOnActive, NewStatus},
};

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
pub(crate) fn gradient(c: [f32; 4]) -> String {
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
        "general:layout" => Str(cfg.general.layout.clone()),
        "dwindle:force_split" => Int(cfg.dwindle.force_split.into()),
        "dwindle:smart_split" => Bool(cfg.dwindle.smart_split),
        "dwindle:smart_resizing" => Bool(cfg.dwindle.smart_resizing),
        "dwindle:permanent_direction_override" => Bool(cfg.dwindle.permanent_direction_override),
        "dwindle:special_scale_factor" => Float(cfg.dwindle.special_scale_factor.into()),
        "dwindle:split_width_multiplier" => Float(cfg.dwindle.split_width_multiplier.into()),
        "dwindle:use_active_for_splits" => Bool(cfg.dwindle.use_active_for_splits),
        "master:allow_small_split" => Bool(cfg.master.allow_small_split),
        "master:special_scale_factor" => Float(cfg.master.special_scale_factor.into()),
        "master:mfact" => Float(cfg.master.mfact.into()),
        "master:new_status" => Str(match cfg.master.new_status {
            NewStatus::Master => "master",
            NewStatus::Slave => "slave",
            NewStatus::Inherit => "inherit",
        }
        .into()),
        "master:new_on_top" => Bool(cfg.master.new_on_top),
        "master:new_on_active" => Str(match cfg.master.new_on_active {
            NewOnActive::None => "none",
            NewOnActive::Before => "before",
            NewOnActive::After => "after",
        }
        .into()),
        "master:orientation" => Str(cfg.master.orientation.name().into()),
        "master:slave_count_for_center_master" => {
            Int(cfg.master.slave_count_for_center_master.into())
        }
        "master:center_master_fallback" => Str(cfg.master.center_master_fallback.name().into()),
        "master:smart_resizing" => Bool(cfg.master.smart_resizing),
        "master:drop_at_cursor" => Bool(cfg.master.drop_at_cursor),
        "master:always_keep_position" => Bool(cfg.master.always_keep_position),
        "master:focus_master_on_close" => Bool(cfg.master.focus_master_on_close),
        "group:auto_group" => Bool(cfg.group.auto_group),
        "group:insert_after_current" => Bool(cfg.group.insert_after_current),
        "group:focus_removed_window" => Bool(cfg.group.focus_removed_window),
        "group:col.border_active" => Custom(gradient(cfg.group.border_active)),
        "group:col.border_inactive" => Custom(gradient(cfg.group.border_inactive)),
        "group:col.border_locked_active" => Custom(gradient(cfg.group.border_locked_active)),
        "group:col.border_locked_inactive" => Custom(gradient(cfg.group.border_locked_inactive)),
        "group:groupbar:enabled" => Bool(cfg.group.groupbar.enabled),
        "group:groupbar:height" => Int(cfg.group.groupbar.height.into()),
        "group:groupbar:gaps_in" => Int(cfg.group.groupbar.gaps_in.into()),
        "group:groupbar:disable_when_only" => Bool(cfg.group.groupbar.disable_when_only),
        "group:groupbar:col.active" => Custom(gradient(cfg.group.groupbar.col_active)),
        "group:groupbar:col.inactive" => Custom(gradient(cfg.group.groupbar.col_inactive)),
        "group:groupbar:col.locked_active" => {
            Custom(gradient(cfg.group.groupbar.col_locked_active))
        }
        "group:groupbar:col.locked_inactive" => {
            Custom(gradient(cfg.group.groupbar.col_locked_inactive))
        }
        "binds:ignore_group_lock" => Bool(cfg.ignore_group_lock),
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
    fn getoption_layout_and_group_options() {
        let (cfg, _) = Config::parse_str(
            "general:layout = master
master:orientation = top
master:new_status = master
group:groupbar:height = 9
group:auto_group = false",
        );
        let get = |k: &str| format_option(k, &get_option(&cfg, k).unwrap(), false, false);
        assert_eq!(
            get("general:layout"),
            "str: master
set: false"
        );
        assert_eq!(
            get("master:orientation"),
            "str: top
set: false"
        );
        assert_eq!(
            get("master:new_status"),
            "str: master
set: false"
        );
        assert_eq!(
            get("master:mfact"),
            "float: 0.550000
set: false"
        );
        assert_eq!(
            get("group:groupbar:height"),
            "int: 9
set: false"
        );
        assert_eq!(
            get("group:auto_group"),
            "bool: false
set: false"
        );
        assert_eq!(
            get("group:col.border_active"),
            "custom type: 66ffff00 0deg
set: false"
        );
        for k in [
            "dwindle:force_split",
            "binds:ignore_group_lock",
            "master:slave_count_for_center_master",
        ] {
            assert!(get_option(&cfg, k).is_some(), "{k}");
        }
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
