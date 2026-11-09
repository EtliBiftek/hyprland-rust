#!/bin/sh
# `hyprctl kill`: the next click kills the client under the pointer, and
# Escape cancels the mode.
# Needs: Xvfb, xdotool, weston, Mesa (llvmpipe).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
echo 'exec-once = weston-terminal' > "$T/test.conf"

Xvfb :97 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill weston-terminal 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
export DISPLAY=:97 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    tail -n 30 "$T/log"
    exit 1
}

count() { timeout 2 "$CTL" -j clients | grep -c '"address"' || true; }

RUST_LOG=info "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
WIN=
for _ in $(seq 50); do
    WIN=$(xdotool search --pid "$COMP" 2>/dev/null | head -n1 || true)
    [ -n "$WIN" ] && break
    sleep 0.2
done
[ -n "$WIN" ] || fail "compositor window did not appear"
xdotool windowfocus --sync "$WIN"
for _ in $(seq 50); do
    [ "$(count)" = 1 ] && break
    sleep 0.2
done
[ "$(count)" = 1 ] || fail "no client"
xdotool mousemove --window "$WIN" 300 300
sleep 1

"$CTL" kill >/dev/null
xdotool key Escape
sleep 0.5
xdotool click 1
sleep 1
[ "$(count)" = 1 ] || fail "Escape did not cancel kill mode"
echo "ok: Escape cancels"

"$CTL" kill >/dev/null
xdotool click 1
for _ in $(seq 25); do
    [ "$(count)" = 0 ] && break
    sleep 0.2
done
[ "$(count)" = 0 ] || fail "click did not kill the client"
kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: hyprctl kill"
