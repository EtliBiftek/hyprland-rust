#!/bin/sh
# protocol-probe test: winit backend inside Xvfb (it needs a renderer).
#  - hyprland-toplevel-mapping + hyprland-toplevel-export capture a red window
#  - ext-image-copy-capture of a toplevel source and an output source
#  - hyprland-focus-grab: a click outside the whitelisted window sends `cleared`
#  - wlr-screencopy and toplevel-export into a dmabuf (a udmabuf, so no GPU is needed)
# Needs: Xvfb, xdotool, libxkbcommon-x11-0, Mesa.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
PROBE=${PROBE:-./target/debug/protocol-probe}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
echo "# empty" > "$T/test.conf"

Xvfb :96 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
    pkill -x protocol-probe 2>/dev/null || true
}
trap cleanup EXIT
unset HYPRLAND_INSTANCE_SIGNATURE
export DISPLAY=:96 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 60 "$T/log"
    exit 1
}

RUST_LOG=debug "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
WL=
for _ in $(seq 75); do
    WL=$(ls "$T" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -n 1 || true)
    [ -n "$WL" ] && break
    sleep 0.2
done
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"
sleep 2

timeout 60 "$PROBE" export || fail "probe export"
pkill -f weston 2>/dev/null || true
timeout 60 "$PROBE" copycap || fail "probe copycap"
timeout 30 "$PROBE" layerpopup || fail "probe layerpopup"
# linux-dmabuf imports are queued and answered by the backend's renderer.
timeout 30 "$PROBE" dmabuf || fail "probe dmabuf"
grep -q "linux-dmabuf: " "$T/log" || fail "dmabuf global not announced in the log"
# Screencopy renders the output straight into the client's dmabuf.
sudo modprobe udmabuf 2>/dev/null || true
sudo chmod a+rw /dev/udmabuf 2>/dev/null || true
timeout 60 "$PROBE" scdmabuf || fail "probe scdmabuf"
timeout 60 "$PROBE" exportdma || fail "probe exportdma"

# misc:focus_on_activate: the activated window takes the focus.
"$CTL" keyword misc:focus_on_activate true >/dev/null
"$PROBE" activate >"$T/act.out" 2>&1 &
for _ in $(seq 100); do
    grep -q activated "$T/act.out" && break
    sleep 0.2
done
grep -q activated "$T/act.out" || { cat "$T/act.out"; fail "no activation"; }
sleep 1
"$CTL" -j activewindow | grep -q '"class": *"probe-a"' || fail "focus_on_activate: $("$CTL" -j activewindow | head -c 300)"
pkill -x protocol-probe 2>/dev/null || true
sleep 0.5

# Focus grab: two windows (left and right half), grab the first, click the right half.
"$PROBE" focusgrab >"$T/grab.out" 2>&1 &
for _ in $(seq 100); do
    grep -q grabbed "$T/grab.out" && break
    sleep 0.2
done
grep -q grabbed "$T/grab.out" || { cat "$T/grab.out"; fail "focus grab not created"; }
xdotool mousemove 1100 400
sleep 0.5
xdotool click 1
for _ in $(seq 50); do
    grep -q "OK cleared" "$T/grab.out" && break
    sleep 0.2
done
grep -q "OK cleared" "$T/grab.out" || { cat "$T/grab.out"; fail "no cleared after outside click"; }
kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK"
