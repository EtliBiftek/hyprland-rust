# CLAUDE.md — hyprland-rust

Context for Claude Code sessions working on this repository.

## Project goal

Rewrite Hyprland (the C++ dynamic tiling Wayland compositor) in Rust, aiming
for compatibility with Hyprland's behaviour: same `hyprland.conf` syntax
(hyprlang), same dispatchers, eventually a `hyprctl`-compatible IPC. It is an
original Rust implementation built on Smithay, not a line-by-line port. The
owner writes in Turkish; reply in Turkish.

## Current status (milestone 1 done, milestone 2 started)

- Two backends, chosen with `--backend winit|tty` (default: winit if
  `WAYLAND_DISPLAY` or `DISPLAY` is set, else tty):
  - winit: nested window inside an X11 or Wayland session.
  - tty (`src/tty.rs`): DRM/KMS + GBM/EGL + libinput + libseat, simplified
    from anvil. Single GPU, **first connected monitor only**, no hotplug,
    cursor is an 8x8 white square, Ctrl+Alt+F<n> switches VT. **Compiles in
    CI but has NOT been run on real hardware yet.**
- Dwindle tiling (`src/layout.rs`), 10 workspaces, floating and fullscreen
  windows, borders (solid color, active/inactive), gaps.
- hyprlang config subset (`src/config.rs`): `$vars`, `category { }`,
  `cat:key` shorthand, `bind`, `exec-once`, `exec`, `source`, colors. Unknown
  but well-known Hyprland options are ignored with a debug log, not errors.
- Dispatchers: exec, killactive, exit, workspace, movetoworkspace,
  togglefloating, fullscreen, movefocus, cyclenext, togglesplit.
- 15 unit tests (config, layout, workspace, spawn reaping), all passing.
- Smoke-tested in Xvfb with 3 `weston-terminal`s: tiling, gaps and borders
  render correctly (`docs/screenshot.png`).
- Keybindings tested with real key presses: `tests/keybinds-xvfb.sh` runs the
  winit backend in Xvfb and presses SUPER+Return (exec) and SUPER+SHIFT+E
  (exit) with xdotool (XTEST). Runs in CI.
- `exec`/`exec-once` children are reaped (double fork via `sh`), no zombies.
- CI: `.github/workflows/ci.yml` on GitHub Actions (ubuntu-24.04): fmt,
  build, test, clippy `-D warnings`, `--check-config`, Xvfb keybind test.

## Roadmap (next milestones)

2. TTY backend (started): test on real hardware, then multi-monitor (one
   `DrmOutput` per CRTC, workspaces per monitor), udev hotplug, `monitor=`
   rules, real cursor images (xcursor / client cursor surfaces), dmabuf.
   Smithay's `anvil/src/udev.rs` (tag v0.7.0) is the reference.
3. `hyprctl`-style IPC over a UNIX socket (`$XDG_RUNTIME_DIR/hypr/...`).
4. wlr-layer-shell (waybar, rofi), `windowrulev2`, master layout.
5. Animations with Bézier curves, rounded corners, blur, shadows (custom GLES
   shaders / render elements).
6. XWayland, input options (`input { kb_layout = tr }`), gestures.

## Code map

| Path | Purpose |
|------|---------|
| `src/main.rs` | CLI args (`-c`, `--check-config`, `--backend`), config loading, exec-once |
| `src/config.rs` | hyprlang parser, `Config`, `Bind`, `Dispatcher`, `DEFAULT_CONFIG` |
| `src/layout.rs` | generic dwindle binary tree `Dwindle<T>`, `Rect`, `find_in_direction` |
| `src/workspace.rs` | `Workspace<W>`: tiled tree + floating stack + fullscreen + last focus |
| `src/wm.rs` | `impl Hypr`: relayout, focus, workspaces, dispatchers, border rects |
| `src/state.rs` | `Hypr` compositor state (Smithay states, seat, config, workspaces) |
| `src/input.rs` | keyboard bind matching (xkb keysym names, lowercased), VT switch, pointer motion, click-to-focus |
| `src/winit.rs` | winit backend, rendering (space + SolidColorRenderElement borders) |
| `src/tty.rs` | TTY backend: libseat session, DRM output, libinput, render loop on vblank |
| `tests/keybinds-xvfb.sh` | Xvfb + xdotool keybinding test (needs a built binary) |
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
Key presses: `sh tests/keybinds-xvfb.sh` (apt: `xvfb xdotool`).

The owner develops on Windows (no WSL): build and test by pushing a branch
and reading the GitHub Actions run (`gh run watch`, `gh run view --log-failed`).

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
- `spawn` runs `sh -c 'eval "$1" &' sh <cmd>` and waits for that `sh`: the
  program is re-parented to init, so no SIGCHLD handler is needed. Do not
  ignore SIGCHLD globally (children would inherit `SIG_IGN`).
- `render_elements!` with a generic `R` needs `ImportMemWl + ImportDmaWl`
  bounds for wayland surface elements; `tty.rs` uses the concrete form
  `pub TtyElement<=GlesRenderer>;` instead.
- Storing a `LoopHandle` in `CalloopData` needs `EventLoop<'static, _>`.
- Keybind test in Xvfb: there is no WM, so focus the winit window with
  `xdotool windowfocus` (found via `xdotool search --pid`) before `key`.
- TTY renders only after a vblank or a 16 ms retry timer (when a frame had no
  damage); borders/cursor use a fresh `Id` every frame, so every frame is
  currently treated as damaged.

## Conventions

- Keep pure logic (layout, config) free of Smithay types and unit-tested.
- Run `cargo fmt`, `cargo clippy` and `cargo test` before committing.
- Unknown Hyprland options: add the prefix to `is_known_unimplemented` in
  `config.rs` instead of erroring, until the option is implemented.
