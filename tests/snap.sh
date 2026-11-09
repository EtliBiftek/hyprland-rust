#!/bin/sh
# general:snap: dragging a floating window near the monitor corner snaps it.
# Needs: Xvfb, xdotool, weston, python3, Mesa.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
general {
    border_size = 0
    snap {
        enabled = true
        monitor_gap = 30
    }
}
animations {
    enabled = false
}
bindm = SUPER, mouse:272, movewindow
windowrule = match:class .*, float on, size 400 300, move 100 100
exec-once = weston-terminal
CONF

Xvfb :94 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill weston-terminal 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
export DISPLAY=:94 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    tail -n 30 "$T/log"
    exit 1
}

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
    timeout 2 "$CTL" -j clients 2>/dev/null | grep -q '"address"' && break
    sleep 0.2
done
sleep 2

at() { timeout 2 "$CTL" -j clients | python3 -c 'import json,sys; print(*json.load(sys.stdin)[0]["at"])'; }

[ "$(at)" = "100 100" ] || fail "window not at 100 100: $(at)"
xdotool mousemove --window "$WIN" 200 200
sleep 0.3
xdotool keydown super mousedown 1
sleep 0.3
xdotool mousemove --window "$WIN" 115 115
sleep 0.5
xdotool mouseup 1 keyup super
sleep 0.5
[ "$(at)" = "0 0" ] || fail "window did not snap to the corner: $(at)"
kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: snap"
