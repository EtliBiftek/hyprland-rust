#!/bin/sh
# xdg_popup.grab test: weston-terminal's right-click menu grabs the seat, and a
# click outside it must close it (`popup_done`, or the client closes it after the release it got through the grab).
# Needs: Xvfb, xdotool, weston, Mesa (llvmpipe).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
exec-once = WAYLAND_DEBUG=1 weston-terminal 2>$T/client
CONF

Xvfb :98 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill weston-terminal 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
export DISPLAY=:98 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- client log ---"
    tail -n 30 "$T/client" || true
    echo "--- compositor log ---"
    tail -n 40 "$T/log"
    exit 1
}

wait_log() {
    for _ in $(seq 25); do
        grep -q "$1" "$T/client" 2>/dev/null && return 0
        sleep 0.2
    done
    return 1
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
wait_log 'xdg_toplevel@.*configure' || fail "weston-terminal did not start"
sleep 2

xdotool mousemove --window "$WIN" 300 300
sleep 0.5
xdotool click 3
wait_log 'xdg_popup@.*\.grab(' || fail "the menu did not ask for a grab"

xdotool mousemove --window "$WIN" 1000 700
sleep 0.5
xdotool click 1
wait_log 'xdg_popup@[0-9]*\.\(popup_done\|destroy\)(' || fail "outside click did not dismiss the popup"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: xdg_popup grab, dismissed by an outside click"
