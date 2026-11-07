#!/bin/sh
# XWayland test: headless backend + Xwayland. X11 clients (xterm, xeyes and
# tests/x11-win.py windows) must show up in `hyprctl -j clients` with
# `xwayland: true`, tile side by side, float when they are dialogs or fixed
# size, close with killactive / forcekillactive, bridge the clipboard both
# ways, and override-redirect windows must stay out of the client list.
# Needs: xwayland xterm x11-apps xclip xdotool python3-xlib wl-clipboard.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d)
chmod 700 "$T"
: > "$T/test.conf"

COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -f x11-win.py 2>/dev/null || true
    pkill -x xterm 2>/dev/null || true
    pkill -x xeyes 2>/dev/null || true
    pkill -x xclip 2>/dev/null || true
    pkill -x wl-copy 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
export NO_COLOR=1
unset DISPLAY WAYLAND_DISPLAY

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 80 "$T/log"
    exit 1
}

clients() { timeout 2 "$HYPRCTL" -j clients; }
hc() { timeout 5 "$HYPRCTL" "$@" >/dev/null; }

# Waits until `hyprctl -j clients` lists exactly $1 windows.
wait_clients() {
    n=-1
    for _ in $(seq 25); do
        n=$(clients 2>/dev/null | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))' 2>/dev/null || echo -1)
        [ "$n" = "$1" ] && return 0
        sleep 0.2
    done
    clients || true
    fail "expected $1 clients, have $n"
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

# The log says which X display Xwayland got and when the X11 WM is up.
for _ in $(seq 100); do
    grep -q "X11 window manager started" "$T/log" && break
    kill -0 "$COMP" 2>/dev/null || fail "compositor died"
    sleep 0.2
done
grep -q "X11 window manager started" "$T/log" || fail "Xwayland / X11 WM did not start"
XD=$(grep -o 'DISPLAY=:[0-9]*' "$T/log" | head -n 1 | cut -d= -f2)
[ -n "$XD" ] || fail "no DISPLAY in the log"
echo "Xwayland on $XD"
for _ in $(seq 25); do
    clients >/dev/null 2>&1 && break
    sleep 0.2
done

# 1. Two tiled X11 windows, side by side.
hc dispatch exec "xterm -T xtermwin -e sleep 600"
wait_clients 1
hc dispatch exec "xeyes"
wait_clients 2
sleep 1.5
clients >"$T/c1.json"
python3 - "$T/c1.json" <<'PY' || fail "tiling: $(cat "$T/c1.json")"
import json, sys
c = json.load(open(sys.argv[1]))
assert len(c) == 2, c
for w in c:
    assert w["xwayland"] is True, w
    assert not w["floating"], w
    assert w["pid"] > 0, w
    assert w["size"][0] > 200 and w["size"][1] > 200, w
by = {w["class"]: w for w in c}
assert set(by) == {"XTerm", "XEyes"}, list(by)
assert by["XTerm"]["title"] == "xtermwin", by["XTerm"]["title"]
a, b = sorted(c, key=lambda w: w["at"][0])
assert a["at"][0] + a["size"][0] <= b["at"][0], (a, b)
assert abs(a["at"][1] - b["at"][1]) <= 2, (a, b)
print("tiled:", [(w["class"], w["at"], w["size"]) for w in (a, b)])
PY

# 2. A dialog-type window and a fixed-size one float.
hc dispatch exec "python3 $HERE/x11-win.py dialog dlg"
wait_clients 3
hc dispatch exec "python3 $HERE/x11-win.py fixed fix"
wait_clients 4
sleep 1.5
clients >"$T/c2.json"
python3 - "$T/c2.json" <<'PY' || fail "floating: $(cat "$T/c2.json")"
import json, sys
by = {w["class"]: w for w in json.load(open(sys.argv[1]))}
for k in ("dlg", "fix"):
    w = by[k]
    assert w["xwayland"] is True and w["floating"], w
    assert w["size"] == [300, 200], w["size"]
assert not by["XTerm"]["floating"] and not by["XEyes"]["floating"]
print("dialog and fixed-size windows float at their own size")
PY

# 3. killactive closes the focused window (the last one opened: `fix`).
timeout 2 "$HYPRCTL" -j activewindow | grep -q '"class": *"fix"' || fail "fix is not focused: $(timeout 2 "$HYPRCTL" -j activewindow)"
hc dispatch killactive
wait_clients 3
clients | grep -q '"fix"' && fail "fix still listed after killactive"
pgrep -f "x11-win.py fixed" >/dev/null && fail "fix process still running"

# 4. forcekillactive: focus xeyes by class, kill it.
hc dispatch focuswindow "class:XEyes"
hc dispatch forcekillactive
wait_clients 2
clients | grep -q '"XEyes"' && fail "XEyes still listed after forcekillactive"

# 5. Fullscreen request from the client (_NET_WM_STATE_FULLSCREEN).
hc dispatch exec "python3 $HERE/x11-win.py fullscreen fs"
wait_clients 3
sleep 2.5
clients >"$T/c3.json"
python3 - "$T/c3.json" <<'PY' || fail "fullscreen: $(cat "$T/c3.json")"
import json, sys
by = {w["class"]: w for w in json.load(open(sys.argv[1]))}
w = by["fs"]
assert w["fullscreen"] == 2, w
assert w["size"] == [1000, 800] and w["at"] == [0, 0], w
print("client fullscreen ok")
PY
hc dispatch killactive
wait_clients 2

# 6. Override-redirect window: drawn, but never a client.
hc dispatch exec "python3 $HERE/x11-win.py or menu"
sleep 3
pgrep -f "x11-win.py or" >/dev/null || fail "override-redirect client died"
wait_clients 2
clients | grep -q '"menu"' && fail "override-redirect window is listed as a client"
grep -q "panicked" "$T/log" && fail "compositor panicked"
pkill -f "x11-win.py or" || true

# 7. Clipboard bridge. Wayland -> X11, then X11 -> Wayland.
WL=$(ls "$T" | grep -E '^wayland-[0-9]+$' | head -n 1)
[ -n "$WL" ] || fail "no wayland socket"
echo from-wayland | WAYLAND_DISPLAY="$WL" wl-copy || fail "wl-copy"
got=$(DISPLAY="$XD" timeout 5 xclip -selection clipboard -o -t UTF8_STRING 2>/dev/null || DISPLAY="$XD" timeout 5 xclip -selection clipboard -o -t STRING) || fail "xclip -o failed"
[ "$got" = "from-wayland" ] || fail "wayland->x11 clipboard: got '$got'"
pkill -x wl-copy || true
printf from-x11 | DISPLAY="$XD" xclip -selection clipboard -i || fail "xclip -i"
sleep 1
got=$(WAYLAND_DISPLAY="$WL" timeout 5 wl-paste -n) || fail "wl-paste failed"
[ "$got" = "from-x11" ] || fail "x11->wayland clipboard: got '$got'"
echo "clipboard bridge ok"

kill -0 "$COMP" 2>/dev/null || fail "compositor died during the test"
grep -q "panicked" "$T/log" && fail "compositor panicked"
echo "xwayland test passed"
