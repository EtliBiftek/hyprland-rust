#!/bin/sh
# Keybinding test: runs the compositor (winit backend) inside Xvfb, presses
# keys with xdotool (XTEST) and checks that the binds fire.
# Needs: Xvfb, xdotool, libxkbcommon-x11-0, Mesa (llvmpipe).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
bind = SUPER, Return, exec, touch $T/exec-fired
bind = SUPER SHIFT, E, exit,
CONF

Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
export DISPLAY=:99 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    cat "$T/log"
    exit 1
}

RUST_LOG=debug "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

WIN=
for _ in $(seq 50); do
    WIN=$(xdotool search --pid "$COMP" 2>/dev/null | head -n1 || true)
    [ -n "$WIN" ] && break
    sleep 0.2
done
[ -n "$WIN" ] || fail "compositor window did not appear"
xdotool windowfocus --sync "$WIN"
xdotool mousemove --window "$WIN" 100 100
sleep 1

xdotool key super+Return
for _ in $(seq 25); do
    [ -e "$T/exec-fired" ] && break
    sleep 0.2
done
[ -e "$T/exec-fired" ] || fail "SUPER+Return did not run exec"
echo "ok: SUPER+Return -> exec"

xdotool key super+shift+e
for _ in $(seq 25); do
    kill -0 "$COMP" 2>/dev/null || break
    sleep 0.2
done
kill -0 "$COMP" 2>/dev/null && fail "SUPER+SHIFT+E did not exit"
wait "$COMP" || fail "compositor exited with an error"
COMP=
echo "ok: SUPER+SHIFT+E -> exit"
