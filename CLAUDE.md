# CLAUDE.md — hyprland-rust

Context for Claude Code sessions working on this repository.

## Project goal

Rewrite Hyprland (the C++ dynamic tiling Wayland compositor) in Rust, aiming
for compatibility with Hyprland's behaviour: same `hyprland.conf` syntax
(hyprlang), same dispatchers, `hyprctl`-compatible IPC. It is an original Rust
implementation built on Smithay, not a line-by-line port. The owner writes in
Turkish; reply in Turkish. Code, docs, commits and this file stay in English
(except `docs/ROADMAP.md`, which is Turkish).

## Current status

- Backends: `winit` (nested), `tty` (DRM/KMS + libinput + libseat; one
  `DrmOutput` per connector, udev hotplug, modes from `monitor =` rules,
  xcursor cursor, GPU from `AQ_DRM_DEVICES`/`WLR_DRM_DEVICES`; runs in CI on
  the kernel's VKMS device with seatd, `tests/tty-vkms.sh`, never on real
  hardware), `headless` (`HYPR_HEADLESS_OUTPUTS=N` virtual outputs, no
  rendering, for CI). Selected with `--backend`; default is winit if
  `WAYLAND_DISPLAY` or `DISPLAY` is set, else tty.
- Multi-monitor (`monitors.rs`): every output shows its own workspace
  (`WmState::mons`), `active_workspace` is the focused monitor's; `monitor =`
  rules (mode, position incl. auto-*, scale, transform, disable, `desc:`),
  `workspace = N, monitor:X, default:true`, focusmonitor,
  (move)currentworkspacetomonitor, moveworkspacetomonitor,
  swapactiveworkspaces, focusworkspaceoncurrentmonitor, `movewindow mon:`,
  pointer focus follows monitors, `hyprctl output create headless|remove`,
  monitoradded/removed(v2), moveworkspace(v2), focusedmon only on monitor
  change. shortcut: one special workspace at a time (on the focused monitor).
- Touchpad gestures (`gestures.rs`, pure): libinput swipe/pinch run
  `gesture =` actions (workspace, dispatcher incl. Lua callbacks, special,
  close, float, fullscreen, live move/resize); unclaimed gestures go to
  clients through pointer-gestures-v1.
- Visual effects (`src/render/`, `animation.rs`): rounded corners, gradient
  borders with angle, shadows, dual-kawase blur (windows, layerrule `blur`),
  opacity/dim, Bézier animations (windows, fade, border, workspaces).
- Target Hyprland release: **v0.57.1** (IPC JSON, rules, master layout and
  dispatcher names were checked against that tag's source/wiki). **Caveat:**
  v0.57 reads only `hyprland.lua` (Lua 5.5, `src/config/lua/`); v0.55-0.56
  read Lua plus a `legacy` hyprlang parser; v0.54.3 is the last
  hyprlang-only release. Decision (owner): support **both**.
- Lua config (`lua_config.rs`, mlua with vendored Lua 5.4): `hyprland.lua`
  runs once per (re)load against an `hl.*` API mirroring v0.57.1; each
  load-time call becomes a hyprlang line fed to the normal parser. The Lua
  state then lives on in `Config::lua` (`lua_config::Runtime`), driven by
  `lua_host.rs`: function binds (dispatcher `__lua, N`), `hl.on` events (fed
  from the socket2 events once per loop iteration, plus `hyprland.start` /
  `shutdown`), `hl.timer` (calloop), `hl.dispatch`, `hl.get_*` (tables
  built from the hyprctl JSON), runtime `hl.config`/`hl.bind` (as keywords),
  rule objects with `set_enabled`, `hyprctl eval`. `hyprland.lua` wins over
  `hyprland.conf` in the default path; `hyprctl dispatch 'hl.dsp.exit()'`
  is sandboxed Lua.
- Dotfiles: Hyprland's v0.54.3 example, JaKooLit (hyprlang) and end-4,
  ML4W, HyDE (Lua) at pinned commits load and boot with zero config errors
  (`tests/dots-conf.sh`, `tests/dots-lua.sh`).
- Parity table: `docs/PARITY.md` (per config option, keyword, dispatcher,
  hyprctl command, IPC event, Wayland protocol).
- XWayland (`xwayland.rs`, Smithay `X11Wm`): started at launch unless
  `xwayland:enabled = false`, `DISPLAY` exported before `exec-once`. Managed X11
  windows live in workspaces (`Window::new_x11_window`), float by
  `should_float` (dialog/utility/splash/transient/fixed-size), class is
  WM_CLASS class, `clients` has `xwayland: true`; override-redirect windows
  are mapped in `Space` only (`XState::or_windows`); clipboard and primary
  selection are bridged; X11 positions are synced from the `Space` once per
  loop iteration (`x11_sync`). Client move/resize requests start the `bindm`
  grab; no lazy start, no scaling.
- Layouts: dwindle (`layout.rs`) and master (`master.rs`), chosen per
  workspace (`general:layout`, workspace rule `layout:`), wrapped in
  `tiling.rs`; `layoutmsg` for both. Groups (tabbed) with a solid-color
  groupbar (`groups.rs`, `wm_groups.rs`).
- Dynamic workspaces (ids, `name:`, special/scratchpad), selectors
  (`selectors.rs`): `N`, `+1`, `e+1`, `m+1`, `r+1`, `previous`, `empty`.
- Floating, fullscreen (modes 0/1), pin, borders (solid; window-rule and
  group colors), gaps, wlr-layer-shell (exclusive zones, layer focus),
  xdg-decoration (always server side).
- hyprlang config (`config.rs`): `$vars`, categories, `cat:key`, `bind` with
  flags `l r e n m t i o d p u`, `unbind`, submaps, `exec(-once)`, `source`
  (globs), `$ENV` fallback, `# hyprlang if/endif/noerror`, `gesture`
  (run from libinput), `env =`, colors, `input { }` / `touchpad { }` / `device { }`, `windowrule`
  / `windowrulev2` / `layerrule` / `windowrule[NAME]:enable` (`rules.rs`,
  `wm/window_rules.rs`). Unknown but well-known options are ignored with a
  debug log.
- Config reload: `hyprctl reload`, mtime-polling autoreload (incl. `source`d
  files), `hyprctl keyword`, `getoption` (`reload.rs`, `options.rs`).
- Dispatchers: exec, killactive, forcekillactive, closewindow, exit,
  workspace, movetoworkspace(silent), togglespecialworkspace, togglefloating,
  setfloating, settiled, fullscreen, pin, movefocus, movewindow, swapwindow,
  resizeactive, moveactive, centerwindow, cyclenext, swapnext,
  focuscurrentorlast, focusurgentorlast, focuswindow, splitratio,
  togglesplit, layoutmsg, submap, pass, tagwindow, group dispatchers,
  renameworkspace, bringactivetotop, alterzorder, movecursor(tocorner),
  togglepseudo/pseudo, fullscreenstate, sendshortcut, sendkeystate, setprop, killwindow, signal, event,
  dpms, global, monitor dispatchers (see Multi-monitor).
- Protocols (`handlers/protocols.rs`): primary selection, wlr/ext
  data-control, viewporter, fractional-scale (1.0), presentation-time,
  single-pixel-buffer, content-type, cursor-shape, idle-inhibit,
  ext-idle-notify (hand-written timers), ext-session-lock,
  keyboard-shortcuts-inhibit, relative-pointer, pointer-constraints,
  xdg-foreign, xdg-activation. Own files in `handlers/`: wlr-screencopy
  (shm, winit/tty; headless replies `failed`), wlr-foreign-toplevel-management
  + ext-foreign-toplevel-list, wlr-output-management (read-only),
  hyprland-global-shortcuts-v1 (real XML, untested with a real client).
- IPC (`ipc.rs`, `ipc_cmds.rs`): async request socket `.socket.sock` and
  event socket `.socket2.sock` under
  `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`. Requests include
  clients, workspaces, monitors, activewindow, layers, binds, devices,
  getoption, keyword, reload, configerrors, dispatch, getprop, eval, submap,
  locked, status, rollinglog, setcursor (validated only), `[[BATCH]]`; hyprpaper/hyprsunset are
  forwarded to their own sockets. `src/bin/hyprctl.rs` is a std-only client
  (flags anywhere on the command line, like Hyprland's).
- Tests: unit tests for all pure modules plus twenty shell tests in
  `tests/` run by CI: keybinds (Xvfb + xdotool), headless smoke, IPC,
  IPC as used by dotfile scripts, layer-shell (waybar), window rules, master
  layout + groups, protocols (wl-clipboard), dispatchers, Lua config,
  dotfiles (conf and Lua), screencopy (grim), foreign toplevel (wlrctl).
- Smoke-tested in Xvfb with 3 `weston-terminal`s (`docs/screenshot.png`).

- Protocols batch 2 (`handlers/extra.rs` owns the state): hyprland focus-grab,
  toplevel-export/mapping, ctm-control, surface, lock-notify,
  global-shortcuts (real XML), wlr-gamma, ext-image-copy-capture,
  linux-dmabuf v3, xdg-dialog/toplevel-icon/system-bell, alpha-modifier,
  tearing-control, fifo/commit-timing (unmanaged), security-context, tablet,
  text-input/input-method/virtual-keyboard, wlr-virtual-pointer, ext-workspace.
  Capture jobs go through `handlers/capture.rs`. Tests: `tests/globals.sh`
  (wayland-info, wlsunset, wtype+wev), `tests/probe.sh` (`src/bin/protocol-probe.rs`).
  Missing: drm-lease, input-capture (libei). The tty backend writes gamma and
  the CTM diagonal to the CRTCs; surface opacity is applied to windows only.

## Roadmap

The full plan is `docs/ROADMAP.md` (Turkish, phases 0-9). Short form:
0 infrastructure (TTY on hardware, idle redraw, workspace split, headless
tests, PARITY.md), 1 daily-usable single monitor (multi-monitor, input,
cursor, protocols, XWayland), 2 IPC/hyprctl parity, 3 window-management parity
(dispatchers, binds, rules, layouts), 4 config parity, 5 visual effects,
6 advanced output/render, 7 protocol completeness, 8 plugins, 9 hardening and
release.

## In progress: OsmiumOS gap list

Branch `osmium` (unmerged). Source: `docs/hyprland-rust-gaps.md` in
EtliBiftek/osmium-os (written against d9010fb, so many rows are stale). Rules:
one commit and one test per item, update `docs/PARITY.md`.

- CI is red only on `tests/dots-conf.sh` (m3ww): `unknown dispatcher
  fakefullscreen` (`keybinds.conf:258`) and `invalid float 1200px`
  (`hyprland.conf:42`). Fix these first.
- Done on the branch: Phase 0, #9, #19, #24, #30. #1, #2, #4-6, #10, #12, #13,
  #16, #31 and #32-38 were done in main (some partially, see the lists below).
- Next: #26, #7, #8, #14, #17, #18, #20, #21, #22, #27, #29, #39-43, then the
  partial ones (#3, #11, #15, #23, #28).

## Future work

Prioritized checklist (condensed from `docs/ROADMAP.md` plus known code gaps).

Phase 0, infrastructure
- [ ] Run the TTY backend on real hardware and fix what breaks.
- [ ] Split into a cargo workspace: `hypr-config`, `hypr-layout`, `hypr-ipc`,
      `hyprctl`, `compositor`.
- [ ] Keep `docs/PARITY.md` current.
- [ ] Lua gaps: `hl.on` `window.update_state`/`*_early` (no IPC
      counterpart), `get_urgent_window`/`get_last_workspace` are nil,
      runtime `hl.window_rule` blocks (multi-line keywords) unsupported.
- [ ] Gesture `cursorZoom` and `scroll_move` do nothing.

TTY backend known gaps
- [ ] Never run on real hardware (VKMS in CI only).
- [ ] Single GPU only; no DRM leasing; `monitor =` mode changes apply on
      reconnect; borders are not scaled on fractional-scale outputs.
- [ ] No hardware cursor plane.

IPC known gaps
- [ ] JSON shapes were checked against the v0.57.1 source, not a running
      Hyprland.
- [ ] Many placeholder fields (list in the `src/ipc.rs` header comment).
- [ ] Missing events: other v0.57.1 events not emitted in `ipc.rs`.
- [ ] `seterror` / `notify` are not drawn on screen (no text rendering yet).

Not started
- [ ] XWayland extras: lazy start, HiDPI scaling (`force_zero_scaling`),
      `WM_NORMAL_HINTS` limits while resizing.
- [ ] `dpms off` shows black frames on tty but keeps the CRTC on.
- [ ] Bind flag `t` is implicit (all matching binds run); `bindm` on a
      keyboard key has no CI test (needs a held key + pointer motion).
- [ ] Master: no per-window weights (`resizeactive` only moves `mfact`).
      Groupbar has no titles and reserves no layout space; drag-into-group
      and group window rules are missing.
- [ ] Per-device `kb_*`: the keymap switches when a keyboard with a `device { kb_* }` block types (one seat keymap, as Hyprland).
- [ ] Config error overlay; empty `is_known_unimplemented`; inotify instead
      of mtime polling.
- [ ] Effects shortcuts: see `shortcut:` comments in `src/render/` (blur cost, no damage tracking of effects).
- [ ] Remaining protocols (see `docs/PARITY.md`): drm-lease, input-capture,
      gamma/CTM on winit/headless, screencopy dmabuf/cursor/scale and
      headless rendering, output-management apply, foreign-toplevel minimize (only the `minimized` event).
- [ ] Protocol shortcuts: pointer lock/confine not enforced; presentation
      feedback not fed; fractional scale fixed at 1.0; no lock-screen test.
- [ ] `alterzorder bottom` approximated; `sendshortcut` presses modifiers by
      fixed keycodes (Super_L/Shift_L/Control_L/Alt_L).
- [ ] Fractional scale, VRR, multi-GPU, explicit sync, HDR (phase 6).
- [ ] Screen-sharing protocols for xdg-desktop-portal-hyprland (phase 7).
- [ ] Fuzzing with an external fuzzer (cargo-fuzz); `fuzz_tests.rs` runs randomized parser and layout checks in `cargo test`. Packaging: `packaging/` (PKGBUILD, session file); migration guide: `docs/MIGRATION.md`.

Open decisions for the owner
- Plugin approach: no plugins / Rust dylib API / WASM (decide after phase 3).
- Access to a Linux machine for hardware testing.

## Code map

| Path | Purpose |
|------|---------|
| `src/main.rs` | CLI args (`-c`, `--backend`, `--check-config`), config loading, exec-once |
| `src/config.rs` | hyprlang parser, `Config`, `Bind`, `Dispatcher`, `DEFAULT_CONFIG` |
| `src/layout.rs` | generic dwindle binary tree `Dwindle<T>`, `Rect`, `find_in_direction` |
| `src/master.rs` | master layout geometry and `layoutmsg` (pure) |
| `src/tiling.rs` | per-workspace tiled layout: dwindle or master |
| `src/groups.rs` | group membership logic (pure) |
| `src/wm_groups.rs` | group dispatchers, `layoutmsg`, groupbar elements |
| `src/selectors.rs` | workspace/window selector parsing and resolution (pure) |
| `src/rules.rs` | windowrule/layerrule parsing and matching (pure) |
| `src/wm/window_rules.rs` | applying window rules to windows (static + dynamic) |
| `src/xwayland.rs` | Xwayland start, `XwmHandler` (map, configure, selections), window-type heuristic (pure, unit-tested) |
| `src/lua_config.rs` | hyprland.lua: `hl.*` API, load-time translation to hyprlang, live `Runtime` (callbacks, events, timers, queries) behind the `Host` trait (pure, unit-tested) |
| `src/lua_host.rs` | `Host` for `Hypr`: runs Lua binds, `hl.on` events, timers, `hyprctl eval` |
| `src/reload.rs` | config reload, autoreload, `keyword` |
| `src/options.rs` | `getoption` lookup and formatting |
| `src/ipc_cmds.rs` | non-snapshot hyprctl requests (devices, notify, ...) |
| `src/handlers/layer_shell.rs` | wlr-layer-shell and xdg-decoration |
| `src/handlers/protocols.rs` | clipboard, idle, session-lock, pointer and misc protocols |
| `src/handlers/screencopy.rs` | wlr-screencopy (shm, offscreen GLES render) |
| `src/handlers/foreign_toplevel.rs` | wlr-foreign-toplevel-management, ext-foreign-toplevel-list |
| `src/handlers/output_management.rs` | wlr-output-management (read-only) |
| `src/handlers/global_shortcuts.rs` | hyprland-global-shortcuts-v1, `global` dispatcher |
| `docs/PARITY.md` | parity table against Hyprland v0.57.1 |
| `src/workspace.rs` | `Workspace<W>`: tiled tree + floating stack + fullscreen + last focus |
| `src/wm.rs` | `impl Hypr`: relayout, focus, workspaces, dispatchers, border rects |
| `src/state.rs` | `Hypr` compositor state (Smithay states, seat, config, workspaces) |
| `src/input.rs` | keyboard bind matching (xkb keysym names, lowercased), VT switch, relative pointer motion, click-to-focus |
| `src/winit.rs` | winit backend, rendering (space + SolidColorRenderElement borders) |
| `src/tty.rs` | DRM/KMS + libinput + libseat backend: one output per connector, udev hotplug (from anvil) |
| `src/monitors.rs` | multi-monitor: per-monitor workspaces, `monitor =` rules, monitor dispatchers, virtual outputs |
| `src/gestures.rs` | touchpad gesture recognition (pure, unit-tested) |
| `src/cursor.rs` | xcursor theme, named and client cursors for the tty backend |
| `src/render/` | effects: rounding, gradient borders, shadows, blur, opacity (GLES shaders) |
| `src/animation.rs` | Bézier curves and the `animation =` tree (pure) |
| `src/handlers/extra.rs` | state of the second protocol batch (hyprland protocols, capture, input method, ...) |
| `src/headless.rs` | headless backend, virtual output, size from `HYPR_HEADLESS_SIZE=WxH` (default 1920x1080) |
| `src/ipc.rs` | IPC: pure request/JSON/text half (unit tested) + sockets and events |
| `src/bin/hyprctl.rs` | std-only `hyprctl` client |
| `src/handlers/` | Wayland protocol handlers (compositor, xdg-shell, seat, data device) |
| `src/grabs/` | interactive move/resize grabs (only allowed for floating windows) |
| `tests/keybinds-xvfb.sh` | Xvfb + xdotool key press test |
| `tests/headless-smoke.sh` | headless backend + 2 clients, layout via `hyprctl -j clients` |
| `tests/ipc-xvfb.sh` | hyprctl requests and `.socket2.sock` events in Xvfb |
| `tests/layer-shell.sh` | waybar exclusive zone + `hyprctl layers` (headless) |
| `tests/window-rules.sh` | float/size/center, workspace silent, regex matchers |
| `tests/master-groups.sh` | master geometry, layoutmsg, groups |
| `tests/protocols.sh` | wl-copy/wl-paste incl. primary selection (headless) |
| `tests/lua-config.sh` | hyprland.lua options, binds, rules, exec-once, Lua dispatch |
| `tests/dispatchers.sh` | renameworkspace, togglepseudo, movecursor, event, dpms |
| `tests/ipc-dots.sh` | hyprctl flags, getprop, eval, submap, events used by dotfile scripts |
| `tests/dots-conf.sh` | Hyprland v0.54.3 example + JaKooLit load with 0 errors |
| `tests/dots-lua.sh` | end-4, ML4W, HyDE (pinned) load and boot with 0 errors |
| `tests/xwayland.sh` | Xwayland: xterm/xeyes tiled, dialog floats, killactive, fullscreen, override-redirect, clipboard bridge |
| `tests/x11-win.py` | helper: one X11 window of a given kind (python3-xlib) |
| `tests/screencopy.sh` | grim against winit in Xvfb |
| `tests/foreign-toplevel.sh` | wlrctl toplevel list/focus/close, wlr-randr |
| `tests/multimon.sh` | 2 headless outputs: rules, monitor dispatchers, output create/remove |
| `tests/tty-vkms.sh` | tty backend on VKMS + seatd: 2 connectors, render, hotplug |
| `tests/render-fx.sh` | rounding, gradient, shadow, blur, animation pixels (grim) |
| `tests/globals.sh` / `tests/probe.sh` | protocol globals; toplevel-export and focus-grab probe |
| `docs/ROADMAP.md` | Turkish roadmap, phases 0-9 |
| `.github/workflows/ci.yml` | fmt, build, test, clippy, check-config, the 20 shell tests |

`state.rs`, `winit.rs`, `input.rs`, `grabs/`, `handlers/` started from
Smithay's `smallvil` example (MIT, see `docs/SMITHAY-LICENSE`); `tty.rs` is
simplified from `anvil`. The project itself is BSD-3-Clause like Hyprland.

## Development workflow

The owner is on Windows without WSL, so builds and tests run only on GitHub
Actions: push a branch, then `gh run watch` and `gh run view --log-failed`.

- Commit with
  `git -c user.name=Pifo -c user.email=97687807+EtliBiftek@users.noreply.github.com commit ...`
  and end the message with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Never `git add -A` without checking that `.claude/worktrees/` is excluded.

## Build and test

System packages (Debian/Ubuntu), as in CI:

```sh
apt-get install -y libwayland-dev libxkbcommon-dev libudev-dev libinput-dev \
  libseat-dev libgbm-dev libegl-dev libdrm-dev libgles-dev pkg-config \
  xvfb xdotool weston libxkbcommon-x11-0 libgl1-mesa-dri
```

```sh
cargo fmt --check
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
./target/debug/hyprland-rust --check-config -c examples/hyprland.conf
for t in tests/*.sh; do sh "$t"; done
```

Manual headless smoke test (no real display), e.g. in a cloud container:

```sh
apt-get install -y xvfb weston libxkbcommon-x11-0 libgl1-mesa-dri imagemagick
Xvfb :99 -screen 0 1280x800x24 &
mkdir -p /tmp/xdg && chmod 700 /tmp/xdg
XDG_RUNTIME_DIR=/tmp/xdg DISPLAY=:99 WAYLAND_DISPLAY= \
  timeout 12 ./target/debug/hyprland-rust -c some-test.conf &
sleep 8; DISPLAY=:99 import -window root shot.png
```

Use a test config with `exec-once = weston-terminal` lines to open windows.

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
- Borders, rounding, shadows and blur are drawn by `render::output_elements`
  per output, in output-local coordinates; backends only call it.
- `spawn` runs `sh -c 'eval "$1" &' sh <cmd>` and waits for that `sh`: the
  program is re-parented to init, so no SIGCHLD handler is needed. Do not
  ignore SIGCHLD globally (children would inherit `SIG_IGN`).
- Agent worktrees live in `.claude/worktrees/`; keep that path out of git
  (it is in `.git/info/exclude` locally) or `git add -A` commits them.
- `openwindow` IPC event fires before the client sets app_id/title, so CLASS
  and TITLE are usually empty there.
- `render_elements!` with a generic `R` needs `ImportMemWl + ImportDmaWl`
  bounds for wayland surface elements; `tty.rs` uses the concrete form
  `pub TtyElement<=GlesRenderer>;` instead.
- Storing a `LoopHandle` in `CalloopData` needs `EventLoop<'static, _>`.
- Keybind test in Xvfb: there is no WM, so focus the winit window with
  `xdotool windowfocus` (found via `xdotool search --pid`) before `key`.
- TTY renders only after a vblank or a 16 ms retry timer (when a frame had no
  damage). Borders/groupbar/cursor use persistent `SolidColorBuffer`s, so
  idle frames carry no damage; winit skips the swap when there is none.
- Never call `space.element_geometry()` (or anything else that locks surface
  data) inside `toplevel.with_pending_state(..)`: it deadlocks.
- waybar in CI needs `dbus-run-session`.
- `lua_config.rs` only needs mlua + std, so it can be unit-tested on Windows:
  copy it as `src/lib.rs` into a scratch crate with the same mlua dependency
  and run `cargo test` / `cargo clippy`. mlua `bool` from Lua is truthiness
  (nil = false), so check for `Value::Nil` when a missing field must default
  to true; Lua table iteration order is unspecified (sort output lines).
- While `session_locked`, `focus_window`, layer focus and `forward_key` are
  no-ops so keys never reach a hidden window; the lock surface gets focus
  directly in `new_surface`.
- Look windows up with `window_for_surface` (all workspaces), not
  `space.elements()`: windows on hidden workspaces are not in `Space`, but
  their commits still need `on_commit()`, rules and the initial configure (a
  `workspace N silent` rule once left such a window unmapped forever).
- Never `unwrap()` a `Space` lookup in a client request handler: a client can
  name a surface that is on a hidden workspace (move/resize once crashed).
- `hyprctl dispatch 'hl...'` is sandboxed Lua (`lua_config::eval_dispatch`:
  no os/io/package/file loading, instruction + memory limits); config scripts
  run with the full stdlib but also under limits (each callback gets a fresh
  instruction budget).
- Lua callbacks reach the compositor only through a scoped host function in
  the Lua registry (`with_host`), so nothing outlives the call; `hl.on`
  handlers run from the main-loop closure (`lua_drain`), never inside a
  dispatcher. Hyprlang generated from Lua escapes `#` as `##` (inline
  comments) and multi-line dispatchers go through callbacks. mlua's safe
  stdlib has no `debug`; a minimal `debug.getinfo` is provided (HyDE).
- Third-party dotfile Lua (`os.execute`, `io.popen`) must not run on the
  owner's machine; parse `.conf` dotfiles locally, run Lua ones in CI only.
- The pure modules (config, selectors, rules, layout, master, tiling, groups,
  options, lua_config) build on Windows: copy them into a scratch lib crate
  (deps: tracing, bitflags, mlua, serde, serde_json, regex) to run their unit
  tests or fuzz them locally.
- Default is no subagents (Pro plan limits). Use one only when the owner asks
  in that request, with the model the owner names (else Haiku).

## Conventions

- Keep pure logic (layout, config, IPC formatting) free of Smithay types and
  unit-tested.
- Run `cargo fmt`, `cargo clippy` and `cargo test` before committing.
- Unknown Hyprland options: add the prefix to `is_known_unimplemented` in
  `config.rs` instead of erroring, until the option is implemented.
- Known limits of a shortcut get a `shortcut: <limit>, <upgrade path>` comment.
