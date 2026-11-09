# Hyprland v0.57.1 parity table

Parity of `hyprland-rust` against Hyprland **v0.57.1**, checked against Hyprland's own source.

## Sources

- Hyprland tag `v0.57.1`, commit `acc1aa0bfd4bd575dcb0f76c35ccf12b660a1ab9`, fetched with `gh api repos/hyprwm/Hyprland/tarball/v0.57.1`.
- Config options: `src/config/values/ConfigValues.cpp`, 407 `MS<Type>("name", ...)` entries.
- Config file discovery: `src/config/supplementary/jeremy/Jeremy.cpp`; config manager: `src/config/ConfigManager.cpp`, `src/config/lua/`.
- Lua API: `src/config/lua/bindings/LuaBindingsRegistration.cpp`, `LuaBindingsConfigRules.cpp`, `LuaBindingsToplevel.cpp`.
- Dispatchers: `src/config/lua/bindings/LuaBindingsDispatchers.cpp` (`hl.dsp.*`).
- hyprctl: `src/ipc/s1/Commands.cpp` (`registerBuiltinCommands`).
- IPC events: every `postEvent` and `ipc->emit` call under `src/`, sent through `src/ipc/s2/S2.cpp`.
- Protocols: `src/managers/ProtocolManager.cpp`.
- Repo side: branch `parity` from `origin/main` (`ddb7e08`). Status is read from `src/config.rs` (`apply_inner`, `Dispatcher::parse`, `is_known_unimplemented`, `BindFlags`), `src/input.rs`, `src/options.rs`, `src/ipc.rs`, `src/ipc_cmds.rs`, the `delegate_*!` calls and `OutputManagerState` setup, plus `tests/*.sh` and the `#[cfg(test)]` modules.

## Important: v0.57.1 is Lua-first

In v0.57.1 the config file is `hyprland.lua`. `Jeremy.cpp` searches only for `hyprland.lua`, and the default config is `example/hyprland.lua`. The hyprlang keyword parser is gone: `bind =`, `windowrule =`, `monitor =`, `exec-once =`, `$var` lines and `hyprctl keyword` are not read by v0.57.1. hyprlang is still linked, but only as the value-storage library for the config options (`ConfigValue.hpp`). hyprctl dispatcher names such as `killactive` and `movefocus` are gone too: `hyprctl dispatch` evaluates Lua (`hl.dispatch(...)`).

The option names (`general:gaps_in` and so on) did not change, so the Config options section still applies. For Keywords and Dispatchers, the repo implements the **pre-Lua hyprlang names**. Those rows show the repo's status, and their Notes give the v0.57.1 Lua equivalent. A `hyprland.conf` written for this repo is not a valid v0.57.1 config.

Status meaning: `none` = not supported (for a config option, a config load error); `parsed` = accepted, stored or ignored, no effect; `partial` = implemented with known gaps; `full` = matches Hyprland with no known gaps (no row was verified at this level, so none is marked full); `tested` = implemented and covered by a `#[cfg(test)]` unit test or a `tests/*.sh` shell test. For config options, `tested` comes from a text match of the option's leaf name and category in the test code. Treat it as a hint.

## Summary

| Section | none | parsed | partial | full | tested | Total |
|---------|-----:|-------:|--------:|-----:|-------:|------:|
| Config options (v0.57.1 registry, 407 options) | 63 | 282 | 19 | 0 | 43 | 407 |
| Keywords: v0.57.1 Lua API (`hl.*`) | 0 | 6 | 4 | 1 | 14 | 25 |
| Keywords: hyprlang (repo only, not parsed by v0.57.1) | 2 | 6 | 4 | 0 | 8 | 20 |
| Dispatchers: v0.57.1 Lua (`hl.dsp.*`) | 24 | 0 | 20 | 0 | 7 | 51 |
| Dispatchers: hyprlang names (repo only) | 0 | 0 | 34 | 0 | 9 | 43 |
| hyprctl commands (v0.57.1) | 12 | 8 | 11 | 0 | 9 | 40 |
| IPC events (socket2, v0.57.1) | 16 | 0 | 23 | 0 | 6 | 45 |
| Wayland protocols (v0.57.1 ProtocolManager) | 60 | 0 | 6 | 0 | 5 | 71 |

Repo inconsistencies outside the tables: `input:tap-to-click` (no touchpad prefix) is accepted, but Hyprland has no such option; only `input:touchpad:tap-to-click` exists. `dwindle:smart_resizing` and `decoration:rounding` are readable through `hyprctl getoption` but have no effect.

`hyprctl getoption` and `hl.get_config` answer every option in the v0.57.1 table (`src/option_defaults.rs`): options without a `Config` field report the configured text or the default, typed like Hyprland. "parsed" rows are still not applied.

Window rules applied beyond the tables: toplevels with a parent or a fixed size float on their own, `keep_aspect_ratio` (resize grab), `idle_inhibit` (`always`, `focus`, `fullscreen`). Also `no_focus`, `stay_focused`, `min_size`, `max_size` (resize grab). Still stored only: `immediate`.

## Config options (v0.57.1 registry, 407 options)

Every option in the v0.57.1 registry. Types are Hyprland's. Status is for the repo's config parser, which uses the same option names.

| Item | Status | Notes |
|------|--------|-------|
| `animations:enabled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `animations`). |
| `animations:workspace_wraparound` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `animations`). |
| `binds:allow_pin_fullscreen` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:allow_workspace_cycles` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:client_requested_fs_uses_layout_handler` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:disable_keybind_grabbing` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:drag_center_window` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:drag_threshold` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:focus_preferred_method` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:hide_special_on_workspace_change` | tested | Bool. `tests/dispatchers.sh`. |
| `binds:ignore_group_lock` | tested | Bool. Used by group dispatchers. |
| `binds:movefocus_cycles_fullscreen` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:movefocus_cycles_groupfirst` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:pass_mouse_when_bound` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `binds:scroll_event_delay` | tested | Int, default 300 ms. Gates `mouse_up/down/left/right` binds; `bind = MODS, mouse:N` works too (`tests/keybinds-xvfb.sh`). |
| `binds:window_direction_monitor_fallback` | tested | Bool. `movefocus`, `movewindow` (`tests/multimon.sh`). |
| `binds:workspace_back_and_forth` | tested | Int. `tests/dispatchers.sh`. |
| `binds:workspace_center_on` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `binds:`). |
| `cursor:default_monitor` | parsed | String. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:enable_hyprcursor` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:hide_on_key_press` | partial | Bool. Hides the cursor until motion; no test. |
| `cursor:hide_on_tablet` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:hide_on_touch` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:hotspot_padding` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:inactive_timeout` | tested | Float. `tests/screencopy.sh`. |
| `cursor:invisible` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:min_refresh_rate` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:no_break_fs_vrr` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:no_hardware_cursors` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:no_warps` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:persistent_warps` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:sync_gsettings_theme` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:use_cpu_buffer` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:warp_back_after_non_mouse_input` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:warp_on_change_workspace` | partial | Int. Warps to the focused window after a workspace switch; no test. |
| `cursor:warp_on_monitor_change` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:warp_on_toggle_special` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:zoom_detached_camera` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:zoom_disable_aa` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `cursor:`). |
| `cursor:zoom_factor` | tested | Float. Scene scaled around the cursor (`tests/render-fx.sh`); the cursor image is not scaled. |
| `cursor:zoom_rigid` | tested | Bool. Cursor kept centered. |
| `debug:colored_stdout_logs` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:damage_blink` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:damage_tracking` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:disable_logs` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:disable_scale_checks` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:disable_time` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:ds_handle_same_buffer` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:ds_handle_same_buffer_fifo` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:enable_stdout_logs` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:error_limit` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:error_position` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:full_cm_proto` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:gl_debugging` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:invalidate_buffers` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:invalidate_fp16` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:log_damage` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:manual_crash` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:overlay` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:pass` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:render_solitary_wo_damage` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:suppress_errors` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `debug:vfr` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `decoration:active_opacity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:acrylic:aberration` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:acrylic:bulb` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:acrylic:clarity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:acrylic:refraction` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:acrylic:tint` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:aurora:color1` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:aurora:color2` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:aurora:intensity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:aurora:speed` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:brightness` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:contrast` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:drops:speed` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:enabled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:color` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:distortion` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:fill_amount` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:mass` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:precision` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:speed` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:fluid_jar:turbulence` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:glass:refraction` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:glass:roughness` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:glass:size` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:haze:intensity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:haze:iridescence` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:heat_shimmer:speed` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:ignore_opacity` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:input_methods` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:input_methods_ignorealpha` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:new_optimizations` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:noise` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:passes` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:popups` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:popups_ignorealpha` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:proportional` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:ripple:duration` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:ripple:radius` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:ripple:strength` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:ripple:width` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:size` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:special` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:variant` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:vibrancy` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:vibrancy_darkness` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:water:damping` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:water:duration` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:water:radius` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:water:speed` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:water:strength` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:blur:xray` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:border_part_of_window` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:dim_around` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:dim_inactive` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:dim_modal` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:dim_special` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:dim_strength` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:fullscreen_opacity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:glow:color` | parsed | Gradient. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:glow:color_inactive` | parsed | Gradient. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:glow:enabled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:glow:range` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:glow:render_power` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:inactive_opacity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:motion_blur:enabled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:motion_blur:samples` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:rounding` | parsed | Int. Parsed and returned by getoption, but no rounded corners are drawn. |
| `decoration:rounding_power` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:screen_shader` | tested | String. GLSL ES 1.00 or 3.00 fragment shader (`tex`, `v_texcoord`, `time`), `tests/screen-shader.sh`. |
| `decoration:shadow:color` | parsed | Gradient. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:color_inactive` | parsed | Gradient. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:enabled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:offset` | parsed | Vec2. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:range` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:render_power` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:scale` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:shadow:sharp` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:damping` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:enabled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:intensity` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:mass` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:mesh` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:stiffness` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:value_epsilon` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `decoration:wobble:velocity_epsilon` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `decoration:`). |
| `dwindle:default_split_ratio` | tested | Float. Clamped to 0.1-1.9. |
| `dwindle:force_split` | tested | Int. Clamped to 0-2. |
| `dwindle:permanent_direction_override` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `dwindle:precise_mouse_move` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `dwindle:precise_mouse_move`). |
| `dwindle:preserve_split` | partial | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `dwindle:smart_resizing` | parsed | Bool. Parsed and returned by getoption, no effect on resizing. |
| `dwindle:smart_split` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `dwindle:special_scale_factor` | partial | Float. Clamped to 0.0-1.0. |
| `dwindle:split_bias` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `dwindle:split_bias`). |
| `dwindle:split_width_multiplier` | tested | Float. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `dwindle:use_active_for_splits` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `ecosystem:enforce_permissions` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `ecosystem:no_donation_nag` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `ecosystem:no_update_news` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `experimental:wp_cm_1_2` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `general:allow_tearing` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:border_size` | partial | Int. Solid-colour borders drawn as 4 rects outside each window; no rounding. |
| `general:col.active_border` | tested | Gradient. Only the first colour of a gradient is used. |
| `general:col.inactive_border` | partial | Gradient. Only the first colour of a gradient is used. |
| `general:col.nogroup_border` | parsed | Gradient. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:col.nogroup_border_active` | parsed | Gradient. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:extend_border_grab_area` | done | Int. Extra grab pixels outside the border. |
| `general:float_gaps` | parsed | CssGap. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:gaps_in` | tested | CssGap. Int only; CSS form (`a b c d`) not parsed. Used by layout and tiling. |
| `general:gaps_out` | tested | CssGap. Int only; CSS form not parsed. |
| `general:gaps_workspaces` | tested | Int. Extra distance of the workspace slide (unit test). |
| `general:hover_icon_on_border` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:layout` | tested | String. Only `dwindle` and `master` are implemented. |
| `general:locale` | parsed | String. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:modal_parent_blocking` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:no_focus_fallback` | tested | Bool. Closing the focused window focuses nothing (`tests/dispatchers.sh`). Directional focus never wraps anyway. |
| `general:resize_corner` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `general:`). |
| `general:resize_on_border` | done | Bool. Left press in the border band around a floating window starts a resize; tiled windows not resizable this way. |
| `general:snap:border_overlap` | tested | Bool. Unit test. |
| `general:snap:enabled` | tested | Bool. Move grab of floating windows (`tests/snap.sh`). |
| `general:snap:monitor_gap` | tested | Int. |
| `general:snap:respect_gaps` | tested | Bool. Unit test. |
| `general:snap:window_gap` | tested | Int. |
| `gestures:close_max_timeout` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:scrolling:move_snap_cursor` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:scrolling:move_snap_to_grid` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_cancel_ratio` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_create_new` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_direction_lock` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_direction_lock_threshold` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_distance` | tested | Int, an old `px` suffix (`1200px`) is accepted. Used by the workspace swipe gesture. |
| `gestures:workspace_swipe_forever` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_invert` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_min_speed_to_force` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_touch` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_touch_invert` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `gestures:workspace_swipe_use_r` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `gestures:`). |
| `group:auto_group` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:col.border_active` | tested | Gradient. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:col.border_inactive` | partial | Gradient. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:col.border_locked_active` | partial | Gradient. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:col.border_locked_inactive` | partial | Gradient. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:drag_into_group` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:focus_removed_window` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:group_on_movetoworkspace` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:blur` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:col.active` | tested | Gradient. Groupbar colour (first gradient colour). |
| `group:groupbar:col.inactive` | partial | Gradient. Groupbar colour (first gradient colour). |
| `group:groupbar:col.locked_active` | partial | Gradient. Groupbar colour (first gradient colour). |
| `group:groupbar:col.locked_inactive` | partial | Gradient. Groupbar colour (first gradient colour). |
| `group:groupbar:disable_when_only` | partial | Bool. Groupbar is solid rectangles only, no titles. |
| `group:groupbar:enabled` | tested | Bool. Groupbar is solid rectangles only, no titles. |
| `group:groupbar:font_family` | parsed | String. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:font_size` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:font_weight_active` | parsed | FontWeight. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:font_weight_inactive` | parsed | FontWeight. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:gaps_in` | tested | Int. Groupbar is solid rectangles only, no titles. |
| `group:groupbar:gaps_out` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:gradient_round_only_edges` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:gradient_rounding` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:gradient_rounding_power` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:gradients` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:height` | tested | Int. Groupbar is solid rectangles only, no titles. |
| `group:groupbar:indicator_gap` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:indicator_height` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:keep_upper_gap` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:middle_click_close` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:priority` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:render_titles` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:round_only_edges` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:rounding` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:rounding_power` | parsed | Float. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:scrolling` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:stacked` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:text_color` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:text_color_inactive` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:text_color_locked_active` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:text_color_locked_inactive` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:text_offset` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:groupbar:text_padding` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:insert_after_current` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `group:merge_floated_into_tiled_on_groupbar` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:merge_groups_on_drag` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `group:merge_groups_on_groupbar` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `group:`). |
| `input-capture:capture_modifiers` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `input-capture:enforce_barriers` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `input:accel_profile` | tested | String. Only `flat` and `adaptive`; `custom` ignored. |
| `input:emulate_discrete_scroll` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:float_switch_override_focus` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:focus_on_close` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:follow_mouse` | tested | Int. 0-3 mapped; 2 behaves like 0 (no hover focus split). |
| `input:follow_mouse_shrink` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:follow_mouse_threshold` | parsed | Float. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:force_no_accel` | tested | Bool. Flat profile. |
| `input:kb_file` | parsed | String. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:kb_layout` | tested | String. Keyboard layout via xkb; no per-device layouts. |
| `input:kb_model` | partial | String. xkb model. |
| `input:kb_options` | tested | String. xkb options. |
| `input:kb_rules` | partial | String. xkb rules. |
| `input:kb_variant` | partial | String. xkb variant. |
| `input:left_handed` | tested | Bool. Pointer device option (libinput). |
| `input:mouse_refocus` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:natural_scroll` | tested | Bool. Pointer device option (libinput). |
| `input:numlock_by_default` | tested | Bool. Numlock on at start. |
| `input:off_window_axis_events` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:repeat_delay` | tested | Int. Key repeat delay. |
| `input:repeat_rate` | tested | Int. Key repeat rate. |
| `input:resolve_binds_by_sym` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:rotation` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:scroll_button` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:scroll_button_lock` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:scroll_factor` | tested | Float. Pointer device option (libinput). |
| `input:scroll_method` | tested | String. `2fg`, `edge`, `on_button_down`, `no_scroll` (libinput). |
| `input:scroll_points` | parsed | String. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:sensitivity` | tested | Float. Pointer device option (libinput). |
| `input:special_fallthrough` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:swallow_focus_click` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:absolute_region_position` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:active_area_position` | parsed | Vec2. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:active_area_size` | parsed | Vec2. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:enabled` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:left_handed` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:output` | parsed | String. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:region_position` | parsed | Vec2. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:region_size` | parsed | Vec2. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:relative_input` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablet:transform` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablettool:eraser_button_mode` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablettool:eraser_button_override` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablettool:pressure_range_max` | parsed | Float. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:tablettool:pressure_range_min` | parsed | Float. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchdevice:enabled` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchdevice:output` | parsed | String. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchdevice:transform` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchpad:clickfinger_behavior` | tested | Bool. libinput click method. |
| `input:touchpad:disable_while_typing` | partial | Bool. Touchpad device option (default on for touchpads). |
| `input:touchpad:drag_3fg` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchpad:drag_lock` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchpad:flip_x` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchpad:flip_y` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:touchpad:middle_button_emulation` | tested | Bool. libinput. |
| `input:touchpad:natural_scroll` | tested | Bool. Touchpad device option. |
| `input:touchpad:scroll_factor` | tested | Float. Used for finger scrolling. |
| `input:touchpad:tap-and-drag` | tested | Bool. libinput. |
| `input:touchpad:tap-to-click` | tested | Bool. Touchpad device option (default on for touchpads). |
| `input:touchpad:tap_button_map` | parsed | String. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:virtualkeyboard:release_pressed_on_close` | parsed | Bool. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `input:virtualkeyboard:share_states` | parsed | Int. Accepted, ignored: `Input::apply` logs and skips unknown `input:` keys. |
| `layers:allow_new_top_layers_over_existing_fullscreen` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `layers:layers_hog_keyboard_focus` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `layout:single_window_aspect_ratio` | none | Vec2. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `layout:single_window_aspect_ratio_tolerance` | none | Float. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `master:allow_small_split` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:always_keep_position` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:center_ignores_reserved` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `master:center_ignores_reserved`). |
| `master:center_master_fallback` | tested | String. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:drop_at_cursor` | partial | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:focus_master_on_close` | partial | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:mfact` | tested | Float. Clamped to 0.0-1.0. |
| `master:new_on_active` | tested | String. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:new_on_top` | tested | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:new_status` | tested | String. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:orientation` | tested | String. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:slave_count_for_center_master` | tested | Int. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:smart_resizing` | partial | Bool. Applied in `config.rs` `apply_inner`; read by the layout or group code. |
| `master:special_scale_factor` | partial | Float. Clamped to 0.0-1.0. |
| `misc:allow_session_lock_restore` | tested | Bool. A new locker may replace a crashed one (`tests/session-lock.sh`). |
| `misc:always_follow_on_dnd` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:animate_manual_resizes` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:animate_mouse_windowdragging` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:anr_missed_pings` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:background_color` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:bell_sound` | parsed | String. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:close_special_on_empty` | partial | Bool. Gates closing an emptied special workspace; no test for off. |
| `misc:col.splash` | parsed | Color. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:disable_autoreload` | tested | Bool. Controls config file watching (`reload.rs`). |
| `misc:disable_hyprland_guiutils_check` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:disable_hyprland_logo` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:disable_scale_notification` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:disable_splash_rendering` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:disable_watchdog_warning` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:disable_xdg_env_checks` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:enable_anr_dialog` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:enable_swallow` | tested | Bool. `tests/swallow.sh`. The hidden terminal is not listed by `clients`. |
| `misc:exit_window_retains_fullscreen` | partial | Int. The next focused window of the workspace becomes fullscreen; no test. |
| `misc:float_force_onscreen` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:focus_on_activate` | tested | Bool, also the `focus_on_activate` rule (`tests/probe.sh`). |
| `misc:font_family` | parsed | String. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:force_default_wallpaper` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:initial_workspace_token_timeout` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:initial_workspace_tracking` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:key_press_enables_dpms` | tested | Bool. `tests/keybinds-xvfb.sh`. |
| `misc:layers_hog_keyboard_focus` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:lockdead_screen_delay` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:middle_click_paste` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:mouse_move_enables_dpms` | tested | Bool. `tests/keybinds-xvfb.sh`. |
| `misc:mouse_move_focuses_monitor` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:name_vk_after_proc` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:new_float_force_onscreen` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:on_focus_under_fullscreen` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:render_unfocused_fps` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:screencopy_force_8b` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:session_lock_blur` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:session_lock_xray` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:size_limits_tiled` | parsed | Bool. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:splash_font_family` | parsed | String. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `misc:swallow_exception_regex` | partial | String, full match on the title; no test. |
| `misc:swallow_regex` | tested | String, full match on the class. |
| `misc:vrr` | parsed | Int. Accepted, ignored (`is_known_unimplemented` prefix `misc:`). |
| `opengl:nvidia_anti_flicker` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `quirks:prefer_hdr` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `quirks:skip_non_kms_dmabuf_formats` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:async_commit` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:cm_auto_hdr` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:cm_enabled` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:cm_sdr_eotf` | none | String. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:commit_timing_enabled` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:ctm_animation` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:direct_scanout` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:expand_undersized_textures` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:fp16_sdr_tf` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:icc_vcgt_enabled` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:keep_unmodified_copy` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:new_render_scheduling` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:non_shader_cm` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:non_shader_cm_interop` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:not_shown_fifo_lock` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:send_content_type` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:use_fp16` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:use_shader_blur_blend` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `render:xp_mode` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:column_width` | none | Float. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:direction` | none | String. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:explicit_column_widths` | none | String. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:focus_fit_method` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:follow_focus` | none | Int. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:follow_min_visible` | none | Float. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:fullscreen_on_one_column` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:wrap_focus` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `scrolling:wrap_swapcol` | none | Bool. Not accepted: unknown option, config load error (`config.rs` `apply_inner`). |
| `xwayland:create_abstract_socket` | implemented | Bool, passed to Xwayland (`xwayland.rs`). |
| `xwayland:enabled` | implemented | Bool. Xwayland starts at launch (not lazily) and `DISPLAY` is exported before `exec-once`. |
| `xwayland:force_zero_scaling` | parsed | Bool. Stored only (no output scaling yet). |
| `xwayland:use_nearest_neighbor` | parsed | Bool. Stored only (no output scaling yet). |

## Keywords: v0.57.1 Lua API (`hl.*`)

`hyprland.lua` runs in mlua (Lua 5.4) through `src/lua_config.rs`; the Lua state stays alive after the load (`Config::lua`) and callbacks run against the compositor through `src/lua_host.rs`. While loading, each `hl.*` call becomes a hyprlang line for the normal parser; inside a callback it applies like `hyprctl keyword`. Unit tests in `lua_config.rs`; `tests/lua-config.sh`; `tests/dots-lua.sh` loads and boots end-4, ML4W and HyDE with zero errors.

| Item | Status | Notes |
|------|--------|-------|
| `hl.bind(keys, dispatcher, opts)` | tested | `hl.dsp.*` or a Lua function (dispatcher `__lua, N`). Opts `locked`, `release`, `repeating`, `non_consuming`, `mouse`, `transparent`, `ignore_mods`, `long_press`, `description`. |
| `hl.unbind(keys)` | tested | As `unbind`. |
| `hl.config(table)` | tested | Nested tables flatten to `cat:key`; also works inside callbacks. |
| `hl.get_config(name)` | tested | From `getoption`; returns `nil, msg` for unknown keys. |
| `hl.define_submap(name, fn)` | tested | As `submap =` ... `submap = reset`. |
| `hl.device(table)` | partial | As `device { }`. |
| `hl.env(name, value)` | tested | Sets the variable and emits `env =`. |
| `hl.monitor(table)` | parsed | As `monitor =` (monitor rules are not applied yet). |
| `hl.window_rule(table)` | tested | As a named `windowrule { }`; returns an object whose `set_enabled(bool)` toggles it (`windowrule[NAME]:enable`). |
| `hl.layer_rule(table)` | parsed | As `layerrule =`. |
| `hl.workspace_rule(table)` | partial | As `workspace =`. |
| `hl.gesture(table)` | parsed | As `gesture =`; a function `action` becomes `dispatcher, __lua, N`. Gestures are not executed yet. |
| `hl.animation(table) / hl.curve(name, points)` | parsed | As `animation =` / `bezier =`; no animations are drawn. |
| `hl.on(event, fn)` | tested | Fed from the socket2 events once per loop iteration (`window.open/close/active/title/urgent/pin/move_to_workspace`, `workspace.active/created/removed/special_active`, `monitor.focused/added/removed`, `keybinds.submap`, `layer.opened/closed`, `config.reloaded`), plus `hyprland.start` and `hyprland.shutdown`. Handlers get object tables. Returns a subscription with `remove()`. No `window.fullscreen`, `window.update_state`, `*_early`. |
| `hl.timer(fn, { timeout, type })` | tested | calloop timers, `oneshot` or `repeat`; `stop()`/`cancel()`; dropped on reload. |
| `hl.notification.create(table)` | partial | Becomes `hyprctl notify` (logged, not drawn). |
| `hl.permission(table)` | parsed | Accepted, ignored. |
| `hl.dispatch(dsp)` | tested | Runs at once inside a callback; while loading it runs once the compositor is up. |
| `hl.exec_cmd(cmd)` | tested | Inside a callback runs at once; while loading it is `exec =`. |
| `hl.get_windows / get_window / get_active_window / get_last_window / get_workspaces / get_workspace / get_active_workspace / get_active_special_workspace / get_workspace_windows / get_monitors / get_monitor / get_active_monitor / get_monitor_at / get_monitor_at_cursor / get_cursor_pos / get_layers / get_devices / get_current_submap` | tested | Tables built from the hyprctl JSON with the field names of Hyprland's Lua objects (`at.x`, `workspace.monitor.name`, ...); `nil` while loading. `get_urgent_window`, `get_last_workspace` return nil. |
| `hl.load(path)` / `require` | tested | Relative to the config directory (`?.lua`, `?/init.lua`). |
| `debug.getinfo` / `debug.traceback` | partial | mlua's safe stdlib has no `debug`; a minimal safe one is provided for config scripts. |
| `hl.version()` | full | Crate version. |
| `hl.is_key_down(key)` | parsed | Always false. |
| `hyprctl eval <lua>` | tested | Runs in the config's live Lua state. |

## Keywords: hyprlang (repo only, not parsed by v0.57.1)

| Item | Status | Notes |
|------|--------|-------|
| `bind`, `bindl`, `bindr`, `binde`, `bindn`, `bindm`, `bindt`, `bindi`, `bindo`, `bindd` (flags l r e n m t i o d) | tested | Flags parsed in `BindFlags::from_keyword`, matched in `input.rs`. Covered by `tests/keybinds-xvfb.sh`. v0.57.1 equivalent: `hl.bind`. |
| `bind flag letters `s`, `c`, `g`, `k`, `p`` | parsed | Accepted, no effect. |
| `unbind` | partial | Unit test `submaps_and_unbind`. v0.57.1 equivalent: `hl.unbind`. |
| `submap` | tested | `tests/keybinds-xvfb.sh`. v0.57.1 equivalent: `hl.define_submap`. |
| `windowrule` / `windowrulev2` / `windowrule { }` | tested | Matchers and effects in `rules.rs`, covered by `tests/window-rules.sh`. v0.57.1 equivalent: `hl.window_rule`. |
| `layerrule` | parsed | Parsed and matched; results are only logged (`handlers/layer_shell.rs`). v0.57.1 equivalent: `hl.layer_rule`. |
| `workspace =` (workspace rules) | partial | Stored in `workspace_rules` and read by `wm.rs`. Only the fields `wm.rs` reads take effect. |
| `exec-once` | tested | `tests/headless-smoke.sh`, `tests/ipc-xvfb.sh`. v0.57.1: `hl.exec_cmd` at startup. |
| `exec` | partial | Runs after each config load (`main.rs`). No test. |
| `source` | tested | Globs (`*`, `?`, `[..]`); no match is an error; `$ENVVAR` expanded; depth limit 16. Unit test `source_glob_and_env`, `tests/dots-conf.sh`. |
| `$var` variables | tested | Unit tests `variables_longest_first`, `dotfile_syntax`: an undefined `$NAME` falls back to the environment like hyprlang. |
| `# hyprlang noerror/if/endif`, `##` | tested | A line starting with `#` (also `####`) is a comment; `##` inside a value is a literal `#`. Unit test `dotfile_syntax`. |
| `gesture =`, `gesturep =` | parsed | Validated and stored in `Config::gestures`. shortcut: touchpad gestures are not executed, needs libinput swipe/pinch events in tty/input. |
| dispatcher `pseudo` | tested | Hyprland's name for `togglepseudo`; both accepted. |
| `category { }` blocks and `cat:key` shorthand | tested | Unit test `categories_and_shorthand`. |
| `monitor =` | tested | Modes, positions, scale, transform, disable, `desc:`, `addreserved` (`tests/multimon.sh`). Scale is integer on the render side. |
| `env =` | tested | Exported before `exec(-once)` and on reload, after XDG_CURRENT_DESKTOP/XDG_SESSION_TYPE/XDG_SESSION_DESKTOP; `tests/session-env.sh`. v0.57.1 equivalent: `hl.env`. |
| `device { }` blocks | tested | `Input` device overrides in `config.rs`, unit test `input_config`. v0.57.1 equivalent: `hl.device`. |
| `animations { }`, `bezier =`, `animation =` | parsed | Ignored (prefixes `animations`, `bezier`). v0.57.1 equivalent: `hl.animation`, `hl.curve`. |
| `gestures { }` category | parsed | Ignored (prefix `gestures:`). v0.57.1 equivalent: `hl.gesture`. |
| `gesture =` keyword | none | Not a known keyword: config error. |
| `permission =` | none | Not accepted. v0.57.1 equivalent: `hl.permission`. |
| `keyword` (`hyprctl keyword`) | tested | `tests/window-rules.sh`, `tests/master-groups.sh`, `tests/ipc-xvfb.sh`. Removed in v0.57.1 (no `keyword` command in its registry). |

## Dispatchers: v0.57.1 Lua (`hl.dsp.*`)

| Item | Status | Notes |
|------|--------|-------|
| `hl.dsp.cursor.move_to_corner` | none | No cursor dispatchers in the repo. |
| `hl.dsp.cursor.move` | none | No cursor dispatchers in the repo. |
| `hl.dsp.group.toggle` | tested | Legacy `togglegroup`, `tests/master-groups.sh`. |
| `hl.dsp.group.next` / `prev` / `active` | tested | Legacy `changegroupactive`, `tests/master-groups.sh`. |
| `hl.dsp.group.move_window` | partial | Legacy `movegroupwindow`. No test. |
| `hl.dsp.group.lock` | partial | Legacy `lockgroups`. No test. |
| `hl.dsp.group.lock_active` | partial | Legacy `lockactivegroup`. No test. |
| `hl.dsp.window.close` | partial | Legacy `closewindow` (selector form). Legacy `killactive` also maps here. |
| `hl.dsp.window.kill` | partial | Legacy `forcekillactive`. No test. |
| `hl.dsp.window.signal` | none | Not implemented. |
| `hl.dsp.window.float` | partial | Legacy `togglefloating`, `setfloating`, `settiled`. No test. |
| `hl.dsp.window.fullscreen` | partial | Legacy `fullscreen`; modes 0, 1 (maximize), 2 only. |
| `hl.dsp.window.fullscreen_state` | none | Not implemented. |
| `hl.dsp.window.pseudo` | none | dwindle pseudotile not implemented. |
| `hl.dsp.window.move` | partial | Legacy `movewindow` (direction). Workspace and monitor targets not supported. |
| `hl.dsp.window.swap` | partial | Legacy `swapwindow`. No test. |
| `hl.dsp.window.center` | partial | Legacy `centerwindow`. No test. |
| `hl.dsp.window.cycle_next` | partial | Legacy `cyclenext`. No test. |
| `hl.dsp.window.tag` | tested | Legacy `tagwindow`, `tests/window-rules.sh`. |
| `hl.dsp.window.clear_tags` | none | Not implemented. |
| `hl.dsp.window.toggle_swallow` | none | Not implemented. |
| `hl.dsp.window.pin` | tested | Legacy `pin`, honours `window`; so do `float`, `fullscreen`, `fullscreen_state`, `move` and `resize`. |
| `hl.dsp.window.bring_to_top` | none | Not implemented. |
| `hl.dsp.window.alter_zorder` | none | Not implemented. |
| `hl.dsp.window.set_prop` | none | Not implemented. |
| `hl.dsp.window.deny_from_group` | partial | Legacy `denywindowfromgroup`. No test. |
| `hl.dsp.window.drag` | partial | `bindm` with `movewindow` (`MouseMove`). No test. |
| `hl.dsp.window.resize` | partial | Legacy `resizeactive`, and `bindm` with `resizewindow` (`MouseResize`). No test. |
| `hl.dsp.workspace.rename` | none | Not implemented. |
| `hl.dsp.workspace.change_id` | none | Not implemented. |
| `hl.dsp.workspace.move` | none | Moving a whole workspace to a monitor is not implemented. |
| `hl.dsp.workspace.swap_monitors` | none | Not implemented. |
| `hl.dsp.workspace.toggle_special` | partial | Legacy `togglespecialworkspace`; name only, no selector. No test. |
| `hl.dsp.exec_cmd` | tested | Legacy `exec`, `tests/keybinds-xvfb.sh`. |
| `hl.dsp.exec_raw` | partial | Legacy `exec`. The repo strips a `[rules]` prefix, otherwise runs the string with `sh -c`. |
| `hl.dsp.exit` | tested | Legacy `exit`, `tests/keybinds-xvfb.sh`. |
| `hl.dsp.reload_config` | partial | Repo has `hyprctl reload`, not a dispatcher. |
| `hl.dsp.submap` | tested | Legacy `submap`, `tests/keybinds-xvfb.sh`. |
| `hl.dsp.pass` | partial | Legacy `pass`. No test. |
| `hl.dsp.send_shortcut` | none | Not implemented. |
| `hl.dsp.send_key_state` | none | Not implemented. |
| `hl.dsp.layout` | tested | Legacy `layoutmsg`, `tests/master-groups.sh`. |
| `hl.dsp.dpms` | none | Not implemented. |
| `hl.dsp.event` | none | Not implemented (no `custom` IPC event either). |
| `hl.dsp.global` | none | No global shortcuts. |
| `hl.dsp.force_renderer_reload` | none | Not implemented. |
| `hl.dsp.force_idle` | none | Not implemented. |
| `hl.dsp.release_input_capture` | none | Not implemented. |
| `hl.dsp.focus` | partial | Legacy `movefocus`, `focuswindow`, `focuscurrentorlast`, `focusurgentorlast` and `workspace`. No monitor focus. |
| `hl.dsp.no_op` | none | Not implemented. |
| `hl.dsp.request_release` | none | Not implemented. |

## Dispatchers: hyprlang names (repo only)

| Item | Status | Notes |
|------|--------|-------|
| `exec` | tested | Runs `sh -c`, detached. `tests/keybinds-xvfb.sh` and `exec-once`. v0.57.1: `hl.dsp.exec_cmd`. |
| `killactive` | partial | Closes the active window. v0.57.1: `hl.dsp.window.close`. |
| `forcekillactive` | partial | v0.57.1: `hl.dsp.window.kill`. |
| `exit` | tested | `tests/keybinds-xvfb.sh`. |
| `workspace` | tested | `tests/keybinds-xvfb.sh` and `tests/ipc-xvfb.sh`. Selectors: number, `+N`, `name:`, `previous`. v0.57.1: `hl.dsp.focus`. |
| `movetoworkspace` | tested | Selectors as `workspace`, optional `,WINDOW`. v0.57.1: `hl.dsp.window.move`. |
| `movetoworkspacesilent` | tested | Selectors as `workspace`, optional `,WINDOW` (`tests/dispatchers.sh`). |
| `movewindowpixel` / `resizewindowpixel` | partial | `moveactive` / `resizeactive` with a required `,WINDOW`. |
| `togglefloating` | partial | v0.57.1: `hl.dsp.window.float`. |
| `setfloating` | partial | v0.57.1: `hl.dsp.window.float`. |
| `settiled` | partial | v0.57.1: `hl.dsp.window.float`. |
| `fullscreen` | partial | Modes 0, 1 (maximize), 2 only. v0.57.1: `hl.dsp.window.fullscreen`. |
| `fakefullscreen` | tested | Pre-0.41 name, same as `fullscreenstate -1 2 toggle`; `clients` reports `fullscreenClient` (`tests/dispatchers.sh`). |
| `movefocus` | partial | Direction-based focus. v0.57.1: `hl.dsp.focus`. |
| `cyclenext` | partial | v0.57.1: `hl.dsp.window.cycle_next`. |
| `togglesplit` | partial | dwindle split toggle. No v0.57.1 Lua entry. |
| `movewindow` | partial | Direction. `mon:` and workspace targets not implemented. v0.57.1: `hl.dsp.window.move`. |
| `swapwindow` | partial | v0.57.1: `hl.dsp.window.swap`. |
| `resizeactive` | partial | Pixel deltas. v0.57.1: `hl.dsp.window.resize`. |
| `moveactive` | partial | Pixel deltas. v0.57.1: `hl.dsp.window.move`. |
| `centerwindow` | partial | v0.57.1: `hl.dsp.window.center`. |
| `pin` | partial | v0.57.1: `hl.dsp.window.pin`. |
| `focuscurrentorlast` | partial | v0.57.1: `hl.dsp.focus`. |
| `focusurgentorlast` | partial | Urgent state set by xdg-activation. v0.57.1: `hl.dsp.focus`. |
| `splitratio` | partial | dwindle. No v0.57.1 Lua entry. |
| `layoutmsg` | tested | `tests/master-groups.sh` (`swapwithmaster`, `mfact`, `orientation*`). v0.57.1: `hl.dsp.layout`. |
| `togglegroup` | tested | `tests/master-groups.sh`. v0.57.1: `hl.dsp.group.toggle`. |
| `changegroupactive` | tested | `tests/master-groups.sh`. v0.57.1: `hl.dsp.group.next` / `prev` / `active`. |
| `moveintogroup` | tested | `tests/master-groups.sh`. v0.57.1: `hl.dsp.window.move` with a group target. |
| `moveoutofgroup` | partial | No test. |
| `movewindoworgroup` | partial | No test. |
| `movegroupwindow` | partial | v0.57.1: `hl.dsp.group.move_window`. No test. |
| `lockgroups` | partial | v0.57.1: `hl.dsp.group.lock`. No test. |
| `lockactivegroup` | partial | v0.57.1: `hl.dsp.group.lock_active`. No test. |
| `denywindowfromgroup` | partial | v0.57.1: `hl.dsp.window.deny_from_group`. No test. |
| `setignoregrouplock` | partial | Sets the same flag as `binds:ignore_group_lock`. No test. |
| `swapnext` | partial | No test. |
| `closewindow` | partial | Selector form. v0.57.1: `hl.dsp.window.close`. |
| `focuswindow` | partial | Selector form. v0.57.1: `hl.dsp.focus`. |
| `pass` | partial | Selector form. v0.57.1: `hl.dsp.pass`. |
| `togglespecialworkspace` | partial | Name only. v0.57.1: `hl.dsp.workspace.toggle_special`. |
| `submap` | tested | `tests/keybinds-xvfb.sh`. v0.57.1: `hl.dsp.submap`. |
| `tagwindow` | tested | `tests/window-rules.sh`. v0.57.1: `hl.dsp.window.tag`. |
| `MouseMove` (`bindm` + `movewindow`) | partial | No test. v0.57.1: `hl.dsp.window.drag`. |
| `MouseResize` (`bindm` + `resizewindow`) | partial | No test. v0.57.1: `hl.dsp.window.resize`. |

## hyprctl commands (v0.57.1)

| Item | Status | Notes |
|------|--------|-------|
| `workspaces` | partial | Query, text and JSON. No test. |
| `workspacerules` | parsed | Stub: JSON `[]`, text empty. |
| `activeworkspace` | tested | `tests/ipc-xvfb.sh`. |
| `clients` | tested | `tests/headless-smoke.sh` (layout check). |
| `kill` | tested | The next click kills the client under the pointer, Escape cancels (`tests/hyprctl-kill.sh`). |
| `activewindow` | partial | Query. No test. |
| `layers` | tested | `tests/layer-shell.sh`. |
| `version` | tested | `tests/ipc-xvfb.sh`, which checks only the name. Version fields are placeholders. |
| `devices` | partial | Fake device addresses and names. |
| `splash` | parsed | Stub: replies `Rust never sleeps.` |
| `cursorpos` | partial | Query. No test. |
| `binds` | partial | Query, text and JSON. No test. |
| `globalshortcuts` | parsed | Stub: replies `none` or `[]`. No global shortcuts protocol. |
| `systeminfo` | partial | Query. Placeholder fields. |
| `animations` | parsed | Stub: empty animation list, no beziers. |
| `rollinglog` | parsed | Stub: empty log. |
| `configerrors` | partial | Lists load errors from `reload.rs`. |
| `locked` | tested | `tests/ipc-dots.sh`. |
| `descriptions` | parsed | Stub: replies `[]` (no option description table). `tests/ipc-dots.sh`. |
| `submap` | tested | `default` or the active submap; JSON string. `tests/ipc-dots.sh`. |
| `status` | tested | `configProvider` (`lua`/`hyprlang`) and `configErrors`; `systeminfo` shows the same provider. `tests/lua-config.sh`, `tests/ipc-dots.sh`. |
| `deprecated-config` | none | Not implemented. |
| `reloadshaders` | tested | Recompiles `decoration:screen_shader`. |
| `monitors` | partial | One output. JSON and text; description, make, model, physical size, `specialWorkspace` and `reserved` are real. `tests/ipc-dots.sh`. |
| `reload` | tested | `tests/ipc-xvfb.sh` (`reload` and autoreload). |
| `plugin` | parsed | `list` replies `no plugins loaded`, others an error. No plugin system. |
| `notify` | partial | Returns `ok`; no notification overlay. |
| `dismissnotify` | parsed | Argument check only; no notifications exist. |
| `getprop` | tested | Window selectors `class:`, `title:`, `address:`, `pid:`, `active`; values are the `setprop` store plus Hyprland defaults. `tests/ipc-dots.sh`. |
| `seterror` | partial | Sets the error shown by `configerrors`. |
| `switchxkblayout` | tested | `tests/ipc-xvfb.sh`. |
| `output` | parsed | Replies with an error: no output create or remove. |
| `dispatch` | tested | Legacy dispatcher names, `tests/ipc-xvfb.sh` and `tests/window-rules.sh`. The v0.57.1 Lua form is not implemented. |
| `setcursor` | parsed | Validates the size, replies `ok`; the cursor theme is not applied. |
| `getoption` | tested | `tests/ipc-xvfb.sh`. Only the options in `options.rs` are answered. |
| `decorations` | parsed | Stub: replies `none`. |
| `eval` | partial | Also reached by `dispatch hl.<non-dsp>`. `hl.dsp.*` runs as a dispatcher, other code runs like a config script and its settings are applied as keywords. `tests/ipc-dots.sh`. |
| `repl` | none | No Lua runtime. |
| `keyword` (repo only) | tested | Not in v0.57.1: `hyprctl keyword` was removed. The repo keeps it for hyprlang. |
| `layouts` (repo only) | partial | Not in the v0.57.1 registration list. Returns dwindle and master. |

## IPC events (socket2, v0.57.1)

| Item | Status | Notes |
|------|--------|-------|
| `activelayout` | partial | Emitted by `hyprctl switchxkblayout`; not on keyboard-triggered layout switches. `tests/ipc-dots.sh`. |
| `activespecial` | partial | Emitted from `wm.rs` on special workspace toggle. |
| `activespecialv2` | tested | `id,name,monitor` or `,,monitor`. `tests/ipc-dots.sh`. |
| `activewindow` | partial | `class,title`; emitted from `ipc.rs`. |
| `activewindowv2` | partial | Address only. |
| `bell` | none | Not emitted. |
| `changefloatingmode` | partial | Emitted from `wm.rs`. No test. |
| `changeworkspaceid` | none | Not emitted. |
| `closelayer` | partial | Emitted from `handlers/layer_shell.rs`. No test. |
| `closewindow` | partial | Emitted from `ipc.rs`. No test. |
| `configreloaded` | tested | `tests/ipc-xvfb.sh`. |
| `createworkspace` | partial | Emitted from `wm.rs`. No test. |
| `createworkspacev2` | partial | Emitted from `wm.rs`. No test. |
| `custom` | none | `hl.dsp.event` not implemented. |
| `destroyworkspace` | partial | Emitted from `wm.rs`. No test. |
| `destroyworkspacev2` | partial | Emitted from `wm.rs`. No test. |
| `focusedmon` | tested | `tests/ipc-xvfb.sh`. |
| `focusedmonv2` | partial | Emitted from `ipc.rs`. No test. |
| `fullscreen` | partial | Only from the dispatcher, not from client requests (known gap in CLAUDE.md). |
| `kill` | tested | See the hyprctl row. |
| `lockgroups` | partial | Emitted from `wm_groups.rs`. No test. |
| `minimized` | none | Not emitted. |
| `monitoradded` | none | No hotplug. |
| `monitoraddedv2` | none | No hotplug. |
| `monitorremoved` | none | No hotplug. |
| `monitorremovedv2` | none | No hotplug. |
| `moveintogroup` | partial | Emitted from `wm_groups.rs`. No test. |
| `moveoutofgroup` | partial | Emitted from `wm_groups.rs`. No test. |
| `movewindow` | partial | Emitted from `wm.rs`. No test. |
| `movewindowv2` | partial | Emitted from `wm.rs`. No test. |
| `moveworkspace` | none | Not emitted. |
| `moveworkspacev2` | none | Not emitted. |
| `openlayer` | partial | Emitted from `handlers/layer_shell.rs`. No test. |
| `openwindow` | tested | `tests/ipc-xvfb.sh` checks that class and title are present (CLAUDE.md lists this as a gap). |
| `pin` | partial | Emitted from `wm.rs`. No test. |
| `renameworkspace` | none | Not emitted. |
| `screencast` | none | No screencast. |
| `screencastv2` | none | No screencast. |
| `submap` | partial | Emitted from `wm.rs`. No test. |
| `togglegroup` | partial | Emitted from `wm_groups.rs`. No test. |
| `urgent` | partial | Emitted from `handlers/mod.rs` (xdg-activation). No test. |
| `windowtitle` | tested | `tests/ipc-xvfb.sh`. |
| `windowtitlev2` | tested | `tests/ipc-xvfb.sh`. |
| `workspace` | tested | `tests/ipc-xvfb.sh`. |
| `workspacev2` | partial | Emitted from `wm.rs`. No test. |

## Wayland protocols (v0.57.1 ProtocolManager)

| Item | Status | Notes |
|------|--------|-------|
| `ext_background_effect_manager_v1` | none | Not registered by the repo. |
| `ext_data_control_manager_v1` | none | Not registered by the repo. |
| `ext_foreign_toplevel_list_v1` | partial | `handlers/foreign_toplevel.rs` (Smithay list, synced every frame). No test client. |
| `ext_idle_notifier_v1` | none | Not registered by the repo. |
| `ext_image_copy_capture_manager_v1` | tested | `handlers/image_copy_capture.rs` (output + foreign-toplevel sources, shm). `tests/probe.sh` (`protocol-probe copycap`). Cursor sessions inert, no dmabuf. |
| `ext_session_lock_manager_v1` | tested | `locked` after the first lock surface commit, per-output sizes, a dead locker stays locked (`misc:allow_session_lock_restore`). `tests/session-lock.sh`. |
| `ext_workspace_manager_v1` | partial | `handlers/ext_workspace.rs`: one group, normal workspaces, activate only. Global listed by `tests/globals.sh`. |
| `hyprland_ctm_control_manager_v1` | partial | `handlers/ctm_control.rs`: validated and stored per output, not applied to the screen. `tests/globals.sh`. |
| `hyprland_focus_grab_manager_v1` | tested | `handlers/focus_grab.rs`: `cleared` on an outside click, keyboard focus on commit. `tests/probe.sh`. Pointer focus not restricted. |
| `xdg_popup.grab` | tested | Keyboard and pointer popup grabs; an outside click dismisses (`tests/popup-grab.sh`). |
| `hyprland_global_shortcuts_manager_v1` | partial | `handlers/global_shortcuts.rs` + `global` dispatcher; XML is now the real hyprland-protocols v0.8.0 one (duplicate id posts `already_taken`). No test client. |
| `hyprland_input_capture_manager_v1` | none | Needs libei (EIS fd); not implemented. |
| `hyprland_lock_notifier_v1` | partial | `handlers/lock_notify.rs`: locked/unlocked from ext-session-lock. Global in `tests/globals.sh`. |
| `hyprland_surface_manager_v1` | partial | `handlers/hyprland_surface.rs`: opacity validated and stored, not applied; visible region ignored. |
| `hyprland_toplevel_export_manager_v1` | tested | `handlers/toplevel_export.rs` (shm, v2 buffer_done, wlr handle variant). `tests/probe.sh` checks the pixels. No dmabuf. |
| `hyprland_toplevel_mapping_manager_v1` | tested | `handlers/toplevel_mapping.rs`, used by `tests/probe.sh` to find the export address. |
| `org_kde_kwin_server_decoration_manager` | none | Not registered by the repo. |
| `vicinae_hotkey_manager_v1` | none | Not registered by the repo. |
| `wl_compositor` | tested | Smithay `CompositorState`; used by every test client. |
| `wl_data_device_manager` | partial | Clipboard only (`delegate_data_device`); no primary selection. No test. |
| `wl_drm` | none | Not registered by the repo. |
| `wl_output` | partial | `OutputManagerState`; one output. No test. |
| `wl_seat` | tested | Keyboard input in `tests/keybinds-xvfb.sh`. |
| `wl_shm` | tested | `weston-simple-shm` in the shell tests. |
| `wl_subcompositor` | partial | Smithay `CompositorState`. No test. |
| `wp_alpha_modifier_v1` | partial | `handlers/alpha_modifier.rs`: Smithay state, multiplier not applied when drawing. |
| `wp_color_manager_v1` | none | Not registered by the repo. |
| `wp_commit_timing_manager_v1` | partial | `handlers/fifo.rs`: unmanaged, accepted but commits are never held. |
| `wp_content_type_manager_v1` | none | Not registered by the repo. |
| `wp_cursor_shape_manager_v1` | none | Not registered by the repo. |
| `wp_drm_lease_device_v1` | none | Not registered by the repo. |
| `wp_fifo_manager_v1` | partial | `handlers/fifo.rs`: unmanaged, barriers have no effect. |
| `wp_fractional_scale_manager_v1` | none | Not registered by the repo. |
| `wp_linux_drm_syncobj_manager_v1` | none | Not registered by the repo. |
| `wp_pointer_warp_v1` | none | Not registered by the repo. |
| `wp_presentation` | none | Not registered by the repo. |
| `wp_security_context_manager_v1` | partial | `handlers/security_context.rs`: sandboxed clients are tagged; only virtual keyboard, input method and this global are hidden from them. |
| `wp_single_pixel_buffer_manager_v1` | none | Not registered by the repo. |
| `wp_tearing_control_manager_v1` | partial | `handlers/tearing_control.rs`: hint accepted and ignored. |
| `wp_viewporter` | none | Not registered by the repo. |
| `xdg_activation_v1` | partial | `delegate_xdg_activation`; marks windows urgent. No test. |
| `xdg_system_bell_v1` | partial | `handlers/system_bell.rs`: ring is logged only. |
| `xdg_toplevel_tag_manager_v1` | none | Not registered by the repo. |
| `xdg_toplevel_icon_manager_v1` | partial | `handlers/toplevel_icon.rs`: Smithay state, icons not drawn. `tests/globals.sh`. |
| `ext_output_image_capture_source_manager_v1` / `ext_foreign_toplevel_image_capture_source_manager_v1` | tested | With `ext_image_copy_capture_manager_v1` (`tests/probe.sh`). |
| `xdg_wm_base` | tested | `delegate_xdg_shell`; every test client. |
| `xdg_wm_dialog_v1` | partial | `handlers/xdg_dialog.rs`: modal state ignored. |
| `xwayland_shell_v1` | implemented | Smithay `XWaylandShellState`, visible to the Xwayland client only (`xwayland.rs`). |
| `xx_hotkey_manager_v1` | none | Not registered by the repo. |
| `zwlr_data_control_manager_v1` | tested | `tests/protocols.sh` (wl-clipboard). |
| `zwlr_foreign_toplevel_manager_v1` | tested | `handlers/foreign_toplevel.rs`, `tests/foreign-toplevel.sh` (wlrctl list/fullscreen/close). Minimize and set_rectangle ignored. |
| `zwlr_gamma_control_manager_v1` | tested | `handlers/gamma_control.rs`: ramps validated and stored, not applied on any backend. `tests/globals.sh` runs wlsunset. |
| `zwlr_layer_shell_v1` | tested | `tests/layer-shell.sh` (waybar). Layer popups are constrained to the output (`tests/probe.sh`). |
| `zwlr_output_manager_v1` | tested | `handlers/output_management.rs`: heads at bind (`wlr-randr`), `apply` of position, scale and transform (`tests/foreign-toplevel.sh`); other modes and disabling answer `failed`. |
| `zwlr_output_power_manager_v1` | none | Not registered by the repo. |
| `zwlr_screencopy_manager_v1` | tested | `overlay_cursor` drawn (`tests/screencopy.sh`). `handlers/screencopy.rs`, `tests/screencopy.sh` (grim, winit in Xvfb). shm only, no cursor overlay, headless answers `failed`. |
| `zwlr_virtual_pointer_manager_v1` | partial | `handlers/virtual_pointer.rs`: motion, absolute motion, button, axis. No client test (global in `tests/globals.sh`). |
| `zwp_idle_inhibit_manager_v1` | none | Not registered by the repo. |
| `zwp_input_method_manager_v2` | partial | `handlers/input_method.rs`: popups tracked but not drawn. No client test. |
| `zwp_keyboard_shortcuts_inhibit_manager_v1` | none | Not registered by the repo. |
| `zwp_linux_dmabuf_v1` | partial | `handlers/dmabuf.rs`: v3, renderer formats (winit, tty) or linear ARGB/XRGB (headless); imports accepted untested; no feedback, capture stays shm. |
| `zwp_pointer_constraints_v1` | none | Not registered by the repo. |
| `zwp_pointer_gestures_v1` | none | Not registered by the repo. |
| `zwp_primary_selection_device_manager_v1` | tested | Smithay primary selection; `tests/protocols.sh`. |
| `zwp_relative_pointer_manager_v1` | none | Not registered by the repo. |
| `zwp_tablet_manager_v2` | partial | `handlers/tablet.rs`: global only, no tablet events. |
| `zwp_text_input_manager_v1` | none | Not registered by the repo. |
| `zwp_text_input_manager_v3` | partial | `handlers/text_input.rs`: focus follows keyboard focus. No client test. |
| `zwp_virtual_keyboard_manager_v1` | tested | `handlers/virtual_keyboard.rs`; `tests/globals.sh` types with wtype into wev. |
| `zxdg_decoration_manager_v1` | partial | `delegate_xdg_decoration`. No test. |
| `zxdg_exporter_v2` | none | Not registered by the repo. |
| `zxdg_importer_v2` | none | Not registered by the repo. |
| `zxdg_output_manager_v1` | partial | `OutputManagerState::new_with_xdg_output`. No test. |

## hyprctl command line (dotfile compatibility)

`src/bin/hyprctl.rs` accepts `-j -r -a -c --config -f --follow -q --quiet -i --instance --batch --` in any position (`hyprctl clients -j`, `hyprctl getoption x -j`, `hyprctl dispatch x -q`), like Hyprland's hyprctl. `hyprctl hyprpaper ...` and `hyprctl hyprsunset ...` go to `.hyprpaper.sock` / `.hyprsunset.sock` and fail with `Couldn't connect to ... (3)` when those daemons are not there. Tested by `tests/ipc-dots.sh`.
