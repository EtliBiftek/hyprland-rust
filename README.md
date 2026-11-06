# hyprland-rust

An original reimplementation of [Hyprland](https://hyprland.org) (the dynamic
tiling Wayland compositor) in Rust, built on
[Smithay](https://github.com/Smithay/smithay). It aims at compatibility with
Hyprland's `hyprland.conf` syntax, dispatchers and `hyprctl` IPC. It is not a
line-by-line port and is not affiliated with the Hyprland project.

![screenshot](docs/screenshot.png)

## Status

Early. Keybindings and IPC are covered by CI tests; the TTY backend compiles in
CI but has **not yet been run on real hardware**.

What works now:

- Backends: `winit` (nested window in an X11/Wayland session), `tty`
  (DRM/KMS + libinput + libseat; single GPU, first connected monitor only),
  `headless` (virtual output without rendering, for CI).
- Dwindle tiling, 10 workspaces, floating and fullscreen windows, solid-color
  active/inactive borders, `gaps_in`/`gaps_out`, click-to-focus, interactive
  move/resize of floating windows.
- hyprlang subset: `$variables`, `category { }` blocks, `category:key`
  shorthand, `bind`, `exec-once`, `exec`, `source`, colors. Known Hyprland
  options that are not implemented yet are ignored with a debug log.
- Dispatchers: `exec`, `killactive`, `exit`, `workspace`, `movetoworkspace`,
  `togglefloating`, `fullscreen`, `movefocus`, `cyclenext`, `togglesplit`.
- IPC: Hyprland-style request and event sockets under
  `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`, and a small `hyprctl`
  binary with a subset of commands (JSON via `-j`, `--batch`). Some JSON fields
  are placeholders.

Not done yet: multi-monitor and `monitor=` rules, layer-shell, XWayland, input
options, window rules, master layout, animations and visual effects. See
[`docs/ROADMAP.md`](docs/ROADMAP.md) (in Turkish) for the plan.

## Building

Debian/Ubuntu packages:

```sh
sudo apt install libwayland-dev libxkbcommon-dev libudev-dev libinput-dev \
  libseat-dev libgbm-dev libegl-dev libdrm-dev libgles-dev pkg-config
cargo build
```

This produces `hyprland-rust` and `hyprctl` in `target/debug/`.

## Usage

```sh
hyprland-rust [-c FILE] [--backend winit|tty|headless] [--check-config]
```

- `-c, --config FILE`: config file (default
  `$XDG_CONFIG_HOME/hypr/hyprland.conf`; built-in defaults if none exists).
- `--backend`: default is `winit` if `WAYLAND_DISPLAY` or `DISPLAY` is set,
  otherwise `tty`.
- `--check-config`: parse the config, print the error count and exit.
- `HYPR_HEADLESS_SIZE=WxH`: output size for the headless backend (default
  1920x1080).
- `RUST_LOG=debug`: verbose logs.

Nested: run it from inside a desktop session and it opens as a window (the host
may intercept `SUPER` shortcuts). On a TTY: switch to a free VT, log in and run
it with no display variables set (or `--backend tty`).

```sh
hyprland-rust -c examples/hyprland.conf
hyprctl -j clients
hyprctl dispatch workspace 2
```

## Testing

```sh
cargo test
cargo clippy --all-targets -- -D warnings
```

Integration scripts in `tests/` (run from the repository root after
`cargo build`; they need Xvfb, xdotool, weston and python3 as listed in
`.github/workflows/ci.yml`):

- `tests/keybinds-xvfb.sh`: nested compositor in Xvfb, key presses via xdotool.
- `tests/headless-smoke.sh`: headless backend with two clients, layout checked
  through `hyprctl -j clients`.
- `tests/ipc-xvfb.sh`: `hyprctl` requests and events on the sockets.

GitHub Actions runs all of the above on every push.

## License

BSD-3-Clause, like Hyprland (see `LICENSE`). Parts of the compositor core are
derived from Smithay's `smallvil` and `anvil` examples (MIT); see
`docs/SMITHAY-LICENSE`.
