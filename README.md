# hyprland-rust

A Hyprland-inspired dynamic tiling Wayland compositor written in Rust, built on
[Smithay](https://github.com/Smithay/smithay).

![screenshot](docs/screenshot.png)

> **Status:** early stage (milestone 1). It runs **nested** inside an existing
> X11 or Wayland session (winit backend). Running directly on a TTY (DRM/KMS +
> libinput) is not implemented yet.

## Features so far

- **Dwindle tiling layout** – every new window splits the focused one along its
  longer side, like Hyprland's default layout. `togglesplit` and
  `dwindle:preserve_split` are supported.
- **10 workspaces** – `workspace N`, `movetoworkspace N`
- **Floating and fullscreen windows** – `togglefloating`, `fullscreen` (plus
  client fullscreen requests). Floating windows can be dragged and resized.
- **Borders** with active and inactive colors, **gaps** (`gaps_in`, `gaps_out`)
- **Focus**: click to focus, `movefocus l/r/u/d`, `cyclenext`
- **hyprlang config parser**: `$variables`, `category { }` blocks, the
  `category:key` shorthand, `bind`, `exec-once`, `exec`, `source`, colors
  (`rgba()`, `rgb()`, `0xAARRGGBB`). Options that aren't implemented yet are
  ignored rather than rejected, so an existing `hyprland.conf` loads.

### Supported dispatchers

`exec`, `killactive`, `exit`, `workspace`, `movetoworkspace`,
`togglefloating`, `fullscreen`, `movefocus`, `cyclenext`, `togglesplit`

## Building

System dependencies (Debian/Ubuntu):

```sh
sudo apt install libwayland-dev libxkbcommon-dev libudev-dev libinput-dev \
    libseat-dev libgbm-dev libegl-dev libdrm-dev libgles-dev pkg-config
```

```sh
cargo build --release
cargo test
```

## Running

Start it from inside an existing desktop session; it opens as a window:

```sh
./target/release/hyprland-rust                       # uses ~/.config/hypr/hyprland.conf
./target/release/hyprland-rust -c examples/hyprland.conf
./target/release/hyprland-rust --check-config        # only validate the config
```

Without a config file, the built-in defaults apply (`SUPER+Return` opens
`kitty`, `SUPER+Q` closes a window, `SUPER+1..0` switch workspaces, and so on;
see `DEFAULT_CONFIG` in `src/config.rs`). Use `RUST_LOG=debug` for verbose
logs.

When running nested, the host desktop may intercept `SUPER` shortcuts.

## Roadmap

1. ~~Nested compositor, dwindle tiling, workspaces, config, keybinds~~
2. TTY backend (DRM/KMS, libinput, libseat), multi-monitor, `monitor=` rules
3. `hyprctl`-compatible IPC socket
4. Layer shell (waybar, rofi, …), `windowrulev2`, master layout
5. Animations and Bézier curves, rounded corners, blur, shadows
6. XWayland, input config (`kb_layout`, …), gestures

## Layout

| Path | Purpose |
|------|---------|
| `src/config.rs` | hyprlang config parser |
| `src/layout.rs` | dwindle binary-tree layout (pure, unit tested) |
| `src/workspace.rs` | per-workspace window lists |
| `src/wm.rs` | window management and dispatchers |
| `src/input.rs` | keyboard bindings and pointer input |
| `src/winit.rs` | nested backend and rendering |
| `src/handlers/`, `src/grabs/` | Wayland protocol handlers, move/resize grabs |

## License

BSD-3-Clause, like Hyprland. Parts of the compositor core are derived from
Smithay's `smallvil` example (MIT); see `LICENSE` and `docs/SMITHAY-LICENSE`.
