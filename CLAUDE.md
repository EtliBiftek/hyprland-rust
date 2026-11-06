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

- Backends: `winit` (nested), `tty` (DRM/KMS + libinput + libseat; compiles in
  CI, never run on real hardware), `headless` (virtual output, no rendering,
  for CI). Selected with `--backend`; default is winit if `WAYLAND_DISPLAY` or
  `DISPLAY` is set, else tty.
- Target Hyprland release: **v0.57.1** (IPC JSON, rules, master layout and
  dispatcher names were checked against that tag's source/wiki). **Caveat:**
  v0.57 reads only `hyprland.lua` (Lua 5.5, `src/config/lua/`); v0.55-0.56
  read Lua plus a `legacy` hyprlang parser; v0.54.3 is the last
  hyprlang-only release. Decision (owner): support **both**.
- Lua config (`lua_config.rs`, mlua with vendored Lua 5.4): `hyprland.lua`
  runs once per (re)load against an `hl.*` API mirroring v0.57.1
  (`hl.config`, `bind`, `unbind`, `dsp.*`, `window_rule`, `layer_rule`,
  `workspace_rule`, `device`, `env`, `monitor`, `curve`, `animation`,
  `on("hyprland.start")`, `exec_cmd`, `define_submap`, `load`, `require`).
  Each call becomes a hyprlang line fed to the normal parser. `hyprland.lua`
  wins over `hyprland.conf` in the default path; `hyprctl dispatch
  'hl.dsp.exit()'` is evaluated as Lua. No Lua state survives the load, so
  function binds, other `hl.on` events, timers and queries are config errors.
- Parity table: `docs/PARITY.md` (per config option, keyword, dispatcher,
  hyprctl command, IPC event, Wayland protocol).
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
  flags `l r e n m t i o d`, `unbind`, submaps, `exec(-once)`, `source`,
  colors, `input { }` / `touchpad { }` / `device { }`, `windowrule` /
  `windowrulev2` / `layerrule` (`rules.rs`, `wm/window_rules.rs`). Unknown but
  well-known options are ignored with a debug log.
- Config reload: `hyprctl reload`, mtime-polling autoreload (incl. `source`d
  files), `hyprctl keyword`, `getoption` (`reload.rs`, `options.rs`).
- Dispatchers: exec, killactive, forcekillactive, closewindow, exit,
  workspace, movetoworkspace(silent), togglespecialworkspace, togglefloating,
  setfloating, settiled, fullscreen, pin, movefocus, movewindow, swapwindow,
  resizeactive, moveactive, centerwindow, cyclenext, swapnext,
  focuscurrentorlast, focusurgentorlast, focuswindow, splitratio,
  togglesplit, layoutmsg, submap, pass, tagwindow, group dispatchers,
  renameworkspace, bringactivetotop, alterzorder, movecursor(tocorner),
  togglepseudo, setprop (stored only), killwindow, signal, event, dpms,
  monitor dispatchers as single-monitor no-ops.
- Protocols (`handlers/protocols.rs`): primary selection, wlr/ext
  data-control, viewporter, fractional-scale (1.0), presentation-time,
  single-pixel-buffer, content-type, cursor-shape, idle-inhibit,
  ext-idle-notify (hand-written timers), ext-session-lock,
  keyboard-shortcuts-inhibit, relative-pointer, pointer-constraints,
  xdg-foreign, xdg-activation.
- IPC (`ipc.rs`, `ipc_cmds.rs`): async request socket `.socket.sock` and
  event socket `.socket2.sock` under
  `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/`. Requests include
  clients, workspaces, monitors, activewindow, layers, binds, devices,
  getoption, keyword, reload, configerrors, dispatch, `[[BATCH]]`.
  `src/bin/hyprctl.rs` is a std-only client.
- Tests: unit tests for all pure modules plus nine shell tests in `tests/`
  run by CI: keybinds (Xvfb + xdotool), headless smoke, IPC, layer-shell
  (waybar), window rules, master layout + groups, protocols (wl-clipboard),
  dispatchers, Lua config.
- Smoke-tested in Xvfb with 3 `weston-terminal`s (`docs/screenshot.png`).

## Roadmap

The full plan is `docs/ROADMAP.md` (Turkish, phases 0-9). Short form:
0 infrastructure (TTY on hardware, idle redraw, workspace split, headless
tests, PARITY.md), 1 daily-usable single monitor (multi-monitor, input,
cursor, protocols, XWayland), 2 IPC/hyprctl parity, 3 window-management parity
(dispatchers, binds, rules, layouts), 4 config parity, 5 visual effects,
6 advanced output/render, 7 protocol completeness, 8 plugins, 9 hardening and
release.

## Future work

Prioritized checklist (condensed from `docs/ROADMAP.md` plus known code gaps).

Phase 0, infrastructure
- [ ] Run the TTY backend on real hardware and fix what breaks.
- [ ] Split into a cargo workspace: `hypr-config`, `hypr-layout`, `hypr-ipc`,
      `hyprctl`, `compositor`.
- [ ] Keep `docs/PARITY.md` current (it predates Lua support).
- [ ] Lua runtime state: function binds, `hl.on` events, `hl.timer`, query
      API (`hl.get_*`), `hl.dsp.send_shortcut`/`fullscreen_state`/`global`.

TTY backend known gaps
- [ ] Never run on real hardware.
- [ ] Single GPU only; first connected monitor only; no udev hotplug.
- [ ] No `monitor=` rules (resolution, refresh, position, scale, transform,
      disable); one `DrmOutput` per CRTC and per-monitor active workspace.
- [ ] Cursor is an 8x8 square: xcursor theme, client cursor surface,
      cursor-shape, hardware cursor plane.
- [ ] No linux-dmabuf global.

IPC known gaps
- [ ] JSON shapes were checked against the v0.57.1 source, not a running
      Hyprland.
- [ ] Many placeholder fields (list in the `src/ipc.rs` header comment).
- [ ] Missing events: `monitoradded/removed` (single monitor), and other
      v0.57.1 events not emitted in `ipc.rs`.
- [ ] `seterror` / `notify` are not drawn on screen; `rollinglog` is empty.

Not started
- [ ] XWayland.
- [ ] Remaining dispatchers (`movecurrentworkspacetomonitor`, `dpms`,
      `alterzorder`, `sendshortcut`, `global`, `setprop`, ...).
- [ ] Bind flags `l`, `o`, `t` behave like plain binds (no lock screen,
      long-press or shortcut inhibit); `bindm` has no CI test.
- [ ] Master: no per-window weights (`resizeactive` only moves `mfact`).
      Groupbar has no titles and reserves no layout space; drag-into-group
      and group window rules are missing.
- [ ] Per-device `kb_*` overrides are ignored (one keyboard per seat).
- [ ] A toplevel unmapped with a null buffer keeps its layout slot (Hyprland
      removes it until it maps again); needs an "unmapped windows" list.
- [ ] Config error overlay; empty `is_known_unimplemented`; inotify instead
      of mtime polling.
- [ ] Animations (Bézier), rounded corners, gradient borders, blur, shadows.
- [ ] Remaining protocols (see `docs/PARITY.md`): wlr-screencopy /
      ext-image-copy-capture, output management, foreign-toplevel, gamma,
      tablet, input-method/text-input, drm-lease, ...
- [ ] Protocol shortcuts: pointer lock/confine not enforced; presentation
      feedback not fed; fractional scale fixed at 1.0; no lock-screen test.
- [ ] Dispatchers missing: `sendshortcut`, `sendkeystate`,
      `fullscreenstate`; `setprop` values stored but not applied;
      `alterzorder bottom` approximated.
- [ ] Fractional scale, VRR, multi-GPU, explicit sync, HDR (phase 6).
- [ ] Screen-sharing protocols for xdg-desktop-portal-hyprland (phase 7).
- [ ] Fuzzing (config parser, IPC), packaging, migration guide (phase 9).

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
| `src/lua_config.rs` | hyprland.lua: `hl.*` API translated to hyprlang (pure, unit-tested) |
| `src/reload.rs` | config reload, autoreload, `keyword` |
| `src/options.rs` | `getoption` lookup and formatting |
| `src/ipc_cmds.rs` | non-snapshot hyprctl requests (devices, notify, ...) |
| `src/handlers/layer_shell.rs` | wlr-layer-shell and xdg-decoration |
| `src/handlers/protocols.rs` | clipboard, idle, session-lock, pointer and misc protocols |
| `docs/PARITY.md` | parity table against Hyprland v0.57.1 |
| `src/workspace.rs` | `Workspace<W>`: tiled tree + floating stack + fullscreen + last focus |
| `src/wm.rs` | `impl Hypr`: relayout, focus, workspaces, dispatchers, border rects |
| `src/state.rs` | `Hypr` compositor state (Smithay states, seat, config, workspaces) |
| `src/input.rs` | keyboard bind matching (xkb keysym names, lowercased), VT switch, relative pointer motion, click-to-focus |
| `src/winit.rs` | winit backend, rendering (space + SolidColorRenderElement borders) |
| `src/tty.rs` | DRM/KMS + libinput + libseat backend (simplified from anvil) |
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
| `docs/ROADMAP.md` | Turkish roadmap, phases 0-9 |
| `.github/workflows/ci.yml` | fmt, build, test, clippy, check-config, the 9 shell tests |

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
- Borders are drawn as 4 solid rects outside each window, on top of all
  windows (overlapping floating windows can show a lower window's border).
  Rounded/gradient borders need a custom shader later; gradients currently
  use only the first color.
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
  run with the full stdlib but also under limits.
- The pure modules (config, selectors, rules, layout, master, tiling, groups,
  options, lua_config) build on Windows: copy them into a scratch lib crate
  (deps: tracing, bitflags, mlua, serde, serde_json, regex) to run their unit
  tests or fuzz them locally.
- The owner does not want subagents (Pro plan limits); if one is
  unavoidable, use Haiku.

## Conventions

- Keep pure logic (layout, config, IPC formatting) free of Smithay types and
  unit-tested.
- Run `cargo fmt`, `cargo clippy` and `cargo test` before committing.
- Unknown Hyprland options: add the prefix to `is_known_unimplemented` in
  `config.rs` instead of erroring, until the option is implemented.
- Known limits of a shortcut get a `shortcut: <limit>, <upgrade path>` comment.
