#!/bin/sh
# Visual effects test: winit backend in Xvfb, a tiny shm client (tests/fx-client.c)
# and grim (wlr-screencopy) to read pixels back. Checks rounded corners,
# gradient borders, shadows, blur behind a translucent window and the timing of
# the window-open animation.
# Needs: Xvfb, gcc, wayland-scanner, wayland-protocols, grim, python3, Mesa.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"

XML=$(pkg-config --variable=pkgdatadir wayland-protocols)/stable/xdg-shell/xdg-shell.xml
wayland-scanner client-header "$XML" "$T/xdg-shell-client-protocol.h"
wayland-scanner private-code "$XML" "$T/xdg-shell.c"
gcc -o "$T/fx-client" -I"$T" tests/fx-client.c "$T/xdg-shell.c" -lwayland-client

cat > "$T/test.conf" <<'CONF'
animations {
    enabled = false
}
general {
    border_size = 0
}
decoration {
    rounding = 0
    shadow {
        enabled = false
    }
    blur {
        enabled = false
    }
}
windowrule = match:class fx.*, float on, size 400 300, move 100 100
windowrule = match:class fxa, size 200 150, move 200 150, opacity 0.5
bindm = SUPER, mouse:272, movewindow
CONF

cat > "$T/px.py" <<'PY'
import sys
def load(p):
    d = open(p, "rb").read()
    parts = d.split(b"\n", 3)
    w, h = map(int, parts[1].split())
    return w, h, parts[3]
def px(img, x, y):
    w, h, d = img
    i = (y * w + x) * 3
    return tuple(d[i:i + 3])
def near(p, q, tol):
    return all(abs(a - b) <= tol for a, b in zip(p, q))
PY

Xvfb :96 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    pkill -f "$T/fx-client" 2>/dev/null || true
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
unset HYPRLAND_INSTANCE_SIGNATURE
export DISPLAY=:96 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 40 "$T/log"
    exit 1
}

RUST_LOG=debug "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
WL=
for _ in $(seq 50); do
    WL=$(ls "$T" | grep -E '^wayland-[0-9]+$' | head -n 1 || true)
    [ -n "$WL" ] && break
    sleep 0.2
done
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"

nclients() { "$CTL" -j clients 2>/dev/null | grep -o '"stableId"' | wc -l || true; }
wait_clients() {
    n=0
    for _ in $(seq 75); do
        n=$(nclients)
        [ "$n" = "$1" ] && return 0
        sleep 0.2
    done
    fail "expected $1 clients, have $n"
}
start() { "$T/fx-client" "$@" >/dev/null 2>&1 & }
stop_all() {
    pkill -f "$T/fx-client" || true
    wait_clients 0
}
shot() { timeout 20 grim -t ppm "$1" || fail "grim failed"; }
kw() { "$CTL" keyword "$1" "$2" >/dev/null || fail "keyword $1"; }

# 1. rounded corners -------------------------------------------------------
start fx solid ff00ff00 400 300
wait_clients 1
kw decoration:rounding 30
sleep 1.5
shot "$T/round.ppm"
kw decoration:rounding 0
sleep 1
shot "$T/square.ppm"
python3 - "$T" <<'PY' || fail "rounded corner check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
r = load(sys.argv[1] + "/round.ppm")
s = load(sys.argv[1] + "/square.ppm")
bg = (26, 26, 26)
print("rounded corner", px(r, 101, 101), "center", px(r, 300, 250), "square corner", px(s, 101, 101))
assert near(px(r, 101, 101), bg, 4), "corner of a rounded window must show the background"
assert near(px(r, 300, 250), (0, 255, 0), 6), "window content missing"
assert near(px(s, 101, 101), (0, 255, 0), 6), "unrounded corner must be window content"
PY
echo "ok: rounded corners"

# 1b. cursor zoom ----------------------------------------------------------
"$CTL" dispatch movecursor "300 250" >/dev/null
kw cursor:zoom_factor 2
sleep 1
shot "$T/zoom.ppm"
kw cursor:zoom_factor 1
sleep 1
python3 - "$T" <<'PY' || fail "cursor zoom check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
z = load(sys.argv[1] + "/zoom.ppm")
print("zoom", px(z, 650, 500), px(z, 750, 500))
green = [(x, y) for y in range(0, z[1], 10) for x in range(0, z[0], 10) if near(px(z, x, y), (0, 255, 0), 6)]
print("green bbox", z[0], z[1], min(g[0] for g in green), min(g[1] for g in green), max(g[0] for g in green), max(g[1] for g in green)) if green else print("no green")
assert near(px(z, 650, 500), (0, 255, 0), 6), "window not scaled around the cursor"
assert near(px(z, 750, 500), (26, 26, 26), 6), "zoomed window too large"
PY
echo "ok: cursor zoom"

# 1c. output scale: the whole scene fills the output ------------------------
"$CTL" keyword monitor "winit, preferred, 0x0, 2" >/dev/null
sleep 1
shot "$T/scaled.ppm"
"$CTL" keyword monitor "winit, preferred, 0x0, 1" >/dev/null
sleep 1
python3 - "$T" <<'PY' || fail "scale check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
z = load(sys.argv[1] + "/scaled.ppm")
print("scaled", z[0], z[1], px(z, z[0] // 2, z[1] // 2))
green = [(x, y) for y in range(0, z[1], 10) for x in range(0, z[0], 10) if near(px(z, x, y), (0, 255, 0), 6)]
print("green bbox", min(g[0] for g in green), min(g[1] for g in green), max(g[0] for g in green), max(g[1] for g in green)) if green else print("no green")
assert z[0] == 1280, "scaled shot has the wrong size"
assert near(px(z, 600, 500), (0, 255, 0), 6), "window not stretched by the output scale"
PY
echo "ok: output scale"

# 1d. misc:background_color --------------------------------------------------
kw misc:background_color "rgb(0000ff)"
sleep 1
shot "$T/bg.ppm"
kw misc:background_color "rgb(1a1a1a)"
sleep 1
python3 - "$T" <<'PY' || fail "background color check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
z = load(sys.argv[1] + "/bg.ppm")
print("background", px(z, 1000, 700))
assert near(px(z, 1000, 700), (0, 0, 255), 6), "background_color not drawn"
PY
echo "ok: background color"

# 2. gradient border -------------------------------------------------------
kw general:border_size 10
kw general:col.active_border "rgb(ff0000) rgb(0000ff) 0deg"
sleep 1.5
shot "$T/border.ppm"
python3 - "$T" <<'PY' || fail "gradient border check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
b = load(sys.argv[1] + "/border.ppm")
l, r = px(b, 95, 250), px(b, 505, 250)
print("border left", l, "right", r)
assert l[0] > 200 and l[2] < 60, "left side of the border must be the first color"
assert r[2] > 200 and r[0] < 60, "right side of the border must be the last color"
PY
echo "ok: gradient border"
kw general:border_size 0

# 3. shadow ------------------------------------------------------------------
kw decoration:shadow:enabled true
kw decoration:shadow:range 30
kw decoration:shadow:color "rgba(000000ff)"
sleep 1.5
shot "$T/shadow.ppm"
python3 - "$T" <<'PY' || fail "shadow check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
s = load(sys.argv[1] + "/shadow.ppm")
near_px, far_px = px(s, 95, 250), px(s, 50, 250)
print("shadow near", near_px, "far", far_px)
assert near_px[0] < 20, "pixel next to the window must be darkened by the shadow"
assert near(far_px, (26, 26, 26), 4), "pixel far from the window must be background"
PY
echo "ok: shadow"
kw decoration:shadow:enabled false
stop_all

# 4. blur behind a translucent window ---------------------------------------
start fxb checker ffffffff 400 300
wait_clients 1
start fxa solid ffffffff 200 150
wait_clients 2
sleep 1.5
shot "$T/noblur.ppm"
kw decoration:blur:enabled true
kw decoration:blur:size 8
kw decoration:blur:passes 3
kw decoration:blur:noise 0
kw decoration:blur:brightness 1
kw decoration:blur:contrast 1
kw decoration:blur:vibrancy 0
sleep 2
shot "$T/blur.ppm"
python3 - "$T" <<'PY' || fail "blur check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
def rough(img):
    row = [px(img, x, 225)[0] for x in range(230, 370)]
    return sum(abs(a - b) for a, b in zip(row, row[1:])) / (len(row) - 1)
a = rough(load(sys.argv[1] + "/noblur.ppm"))
b = rough(load(sys.argv[1] + "/blur.ppm"))
print("roughness without blur", a, "with blur", b)
assert a > 5, "the pattern behind the translucent window is not visible"
assert b < a * 0.5, "blur must smooth the pattern behind the translucent window"
PY
echo "ok: blur"
stop_all

# 5. open animation (popin 50%, linear, 3 s) ---------------------------------
kw animations:enabled true
kw animation "windowsIn, 1, 30, linear, popin 50%"
kw animation "fadeIn, 0"
start fx solid ff00ff00 400 300
wait_clients 1
sleep 1
shot "$T/anim_mid.ppm"
sleep 4
shot "$T/anim_end.ppm"
python3 - "$T" <<'PY' || fail "animation check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
def width(img):
    xs = [x for x in range(0, 1280) if near(px(img, x, 250), (0, 255, 0), 8)]
    return (xs[-1] - xs[0] + 1) if xs else 0
m = width(load(sys.argv[1] + "/anim_mid.ppm"))
e = width(load(sys.argv[1] + "/anim_end.ppm"))
print("window width mid-animation", m, "after", e)
assert 200 <= m <= 385, "window must be partway to its size while animating"
assert e >= 396, "window must reach its full size when the animation ends"
PY
echo "ok: open animation"

# 6. misc:animate_mouse_windowdragging (windowsMove linear, 3 s) --------------
stop_all
kw animation "windowsIn, 0"
kw animation "windowsMove, 1, 30, linear"
start fx solid ff00ff00 400 300
wait_clients 1
sleep 1
XWIN=$(xdotool search --pid "$COMP" 2>/dev/null | head -n1 || true)
[ -n "$XWIN" ] || fail "no X11 window of the compositor"
xdotool windowfocus --sync "$XWIN"
# drag_shot FILE: drags the window 300 px to the right, shoots while it is
# held, then puts it back.
drag_shot() {
    xdotool mousemove --window "$XWIN" 200 200
    sleep 0.3
    xdotool keydown super mousedown 1
    sleep 0.3
    xdotool mousemove --window "$XWIN" 500 200
    sleep 0.3
    shot "$1"
    xdotool mousemove --window "$XWIN" 200 200
    sleep 0.3
    xdotool mouseup 1 keyup super
    sleep 4
}
drag_shot "$T/drag_off.ppm"
kw misc:animate_mouse_windowdragging true
drag_shot "$T/drag_on.ppm"
python3 - "$T" <<'PY' || fail "drag animation check failed"
import sys
sys.path.insert(0, sys.argv[1])
from px import *
def left(img):
    xs = [x for x in range(0, 1280) if near(px(img, x, 350), (0, 255, 0), 8)]
    return xs[0] if xs else -1
off = left(load(sys.argv[1] + "/drag_off.ppm"))
on = left(load(sys.argv[1] + "/drag_on.ppm"))
print("left edge while dragging: option off", off, "on", on)
assert 398 <= off <= 402, "a dragged window must follow the pointer at once"
assert 104 <= on <= 385, "with animate_mouse_windowdragging the window must lag behind"
PY
echo "ok: animate_mouse_windowdragging"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
grep -q "panicked" "$T/log" && fail "compositor panicked"
grep -q "failed to compile\|blur failed" "$T/log" && fail "effect error in the log"
echo "OK: render effects"
