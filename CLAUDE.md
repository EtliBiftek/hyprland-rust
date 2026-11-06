# CLAUDE.md — hyprland-rust

Context for Claude Code sessions working on this repository.

## Project goal

Rewrite Hyprland (the C++ dynamic tiling Wayland compositor) in Rust, aiming
for compatibility with Hyprland's behaviour: same `hyprland.conf` syntax
(hyprlang), same dispatchers, eventually a `hyprctl`-compatible IPC. It is an
original Rust implementation built on Smithay, not a line-by-line port. The
owner writes in Turkish; reply in Turkish.

## Current status (milestone 1 done)

- Nested compositor only (winit backend: runs as a window inside an X11 or
  Wayland session). No TTY/DRM backend yet.
- Dwindle tiling (`src/layout.rs`), 10 workspaces, floating and fullscreen
  windows, borders (solid color, active/inactive), gaps.
- hyprlang config subset (`src/config.rs`): `$vars`, `category { }`,
  `cat:key` shorthand, `bind`, `exec-once`, `exec`, `source`, colors. Unknown
  but well-known Hyprland options are ignored with a debug log, not errors.
- Dispatchers: exec, killactive, exit, workspace, movetoworkspace,
  togglefloating, fullscreen, movefocus, cyclenext, togglesplit.
- 14 unit tests (config, layout, workspace), all passing.
- Smoke-tested in Xvfb with 3 `weston-terminal`s: tiling, gaps and borders
  render correctly (`docs/screenshot.png`).
- **Keybindings have NOT been tested with real key presses yet.**

## Roadmap (next milestones)

2. TTY backend: DRM/KMS + libinput + libseat (udev session), multi-monitor,
   `monitor=` rules. Use Smithay's `anvil` example as reference.
3. `hyprctl`-style IPC over a UNIX socket (`$XDG_RUNTIME_DIR/hypr/...`).
4. wlr-layer-shell (waybar, rofi), `windowrulev2`, master layout.
5. Animations with Bézier curves, rounded corners, blur, shadows (custom GLES
   shaders / render elements).
6. XWayland, input options (`input { kb_layout = tr }`), gestures.

## Code map

| Path | Purpose |
|------|---------|
| `src/main.rs` | CLI args (`-c`, `--check-config`), config loading, exec-once |
| `src/config.rs` | hyprlang parser, `Config`, `Bind`, `Dispatcher`, `DEFAULT_CONFIG` |
| `src/layout.rs` | generic dwindle binary tree `Dwindle<T>`, `Rect`, `find_in_direction` |
| `src/workspace.rs` | `Workspace<W>`: tiled tree + floating stack + fullscreen + last focus |
| `src/wm.rs` | `impl Hypr`: relayout, focus, workspaces, dispatchers, border rects |
| `src/state.rs` | `Hypr` compositor state (Smithay states, seat, config, workspaces) |
| `src/input.rs` | keyboard bind matching (xkb keysym names, lowercased), click-to-focus |
| `src/winit.rs` | winit backend, rendering (space + SolidColorRenderElement borders) |
| `src/handlers/` | Wayland protocol handlers (compositor, xdg-shell, seat, data device) |
| `src/grabs/` | interactive move/resize grabs (only allowed for floating windows) |

`state.rs`, `winit.rs`, `input.rs`, `grabs/`, `handlers/` started from
Smithay's `smallvil` example (MIT, see `docs/SMITHAY-LICENSE`). The project
itself is BSD-3-Clause like Hyprland.

## Build and test

System packages (Debian/Ubuntu):

```sh
apt-get install -y libwayland-dev libxkbcommon-dev libudev-dev libinput-dev \
  libseat-dev libgbm-dev libegl-dev libdrm-dev libgles-dev pkg-config
```

```sh
cargo build
cargo test
cargo clippy
./target/debug/hyprland-rust --check-config -c examples/hyprland.conf
```

Headless smoke test (no real display), e.g. in a cloud container:

```sh
apt-get install -y xvfb weston libxkbcommon-x11-0 libgl1-mesa-dri imagemagick
Xvfb :99 -screen 0 1280x800x24 &
mkdir -p /tmp/xdg && chmod 700 /tmp/xdg
XDG_RUNTIME_DIR=/tmp/xdg DISPLAY=:99 WAYLAND_DISPLAY= \
  timeout 12 ./target/debug/hyprland-rust -c some-test.conf &
sleep 8; DISPLAY=:99 import -window root shot.png
```

Use a test config with `exec-once = weston-terminal` lines to open windows.
Key presses could be injected with `xdotool` on the Xvfb display (untried).

## Gotchas learned so far

- Smithay version is **0.7.0**. `winit::init` needs an explicit renderer type:
  `winit::init::<GlesRenderer>()`.
- `Space::map_element(loc)` positions the window's *geometry*; Space subtracts
  the client-side decoration offset itself. Do NOT subtract
  `window.geometry().loc` again (this caused misaligned windows once).
  `space.element_location()` already returns the geometry position.
- The CSD geometry is only known after the client commits, so
  `CompositorHandler::commit` re-runs `relayout()` when `window.geometry()`
  changes.
- Only call `send_pending_configure()` after `is_initial_configure_sent()`;
  the initial configure is sent from `handle_commit` and picks up the size
  set in pending state by `relayout()`.
- Without `XDG_RUNTIME_DIR` the Wayland socket bind panics; winit on X11
  needs `libxkbcommon-x11.so.0` at runtime.
- Gaps: tiling area = output shrunk by `gaps_out - gaps_in`, then each window
  rect shrunk by `gaps_in`, which gives `gaps_out` at screen edges and
  `2 * gaps_in` between windows (matches Hyprland).
- Borders are drawn as 4 solid rects outside each window, on top of all
  windows (overlapping floating windows can show a lower window's border).
  Rounded/gradient borders need a custom shader later; gradients currently
  use only the first color.
- Spawned child processes are not reaped yet (zombies); add SIGCHLD handling.

## Conventions

- Keep pure logic (layout, config) free of Smithay types and unit-tested.
- Run `cargo fmt`, `cargo clippy` and `cargo test` before committing.
- Unknown Hyprland options: add the prefix to `is_known_unimplemented` in
  `config.rs` instead of erroring, until the option is implemented.
