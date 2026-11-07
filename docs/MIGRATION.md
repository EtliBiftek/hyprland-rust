# Moving from Hyprland to hyprland-rust

hyprland-rust reads the same config files and speaks the same IPC, so most
setups work unchanged. This page lists what to check.

## Install

- Arch: `cd packaging && makepkg -si` (installs `hyprland-rust`, `hyprctl-rust`
  and a `Hyprland (Rust)` session for display managers).
- Elsewhere: `cargo build --release`, then copy `target/release/hyprland-rust`
  and `target/release/hyprctl` to your `PATH` and
  `packaging/hyprland-rust.desktop` to `/usr/share/wayland-sessions/`.

`hyprctl` here is a drop-in client: scripts calling `hyprctl` work against
either compositor (the socket path and protocol are the same). Keep only one
of the two binaries named `hyprctl` in your `PATH`.

## Config

- `~/.config/hypr/hyprland.lua` wins over `hyprland.conf`, as in Hyprland
  0.55+. Both formats are read; `hyprctl configerrors` lists what was not
  understood.
- Check your config without starting a session:
  `hyprland-rust --check-config -c ~/.config/hypr/hyprland.conf`.
- `source =` globs, `$ENV` fallback, `# hyprlang if/noerror`, window/layer
  rules (all syntax generations), gestures, monitor rules and Lua callbacks are
  supported. Options that have no effect yet are accepted and listed in
  `docs/PARITY.md`.

## Known differences

- Not yet run on real GPUs (CI runs the DRM/KMS backend on the kernel's
  virtual VKMS device). Report what breaks on your hardware.
- No plugins (`hyprpm`), no HDR/VRR, fractional scale is limited, one special
  workspace at a time.
- `hyprctl notify` and config error bars are not drawn on screen.
- Gamma/CTM (hyprsunset), surface opacity and `setprop` values are stored but
  not applied yet.

The full list of gaps is the "Future work" section of `CLAUDE.md`; the
per-option status is `docs/PARITY.md`.
