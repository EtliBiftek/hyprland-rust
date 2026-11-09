#!/bin/sh
# wlr-screencopy test: winit backend inside Xvfb, one floating weston-simple-shm
# window at the top left, captured with grim (full output, region, ppm).
# Checks size, orientation (window at the top, plain background at the bottom)
# and that the compositor survives. The headless backend has no renderer, so it
# is not used here.
# Needs: Xvfb, weston (weston-simple-shm), grim, python3, libxkbcommon-x11-0, Mesa.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
windowrule = match:class .*, float on, size 400 300, move 20 20
exec-once = weston-simple-shm
CONF

Xvfb :97 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
    pkill -f weston-simple-shm 2>/dev/null || true
}
trap cleanup EXIT
unset HYPRLAND_INSTANCE_SIGNATURE
export DISPLAY=:97 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 60 "$T/log"
    exit 1
}

RUST_LOG=debug "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

N=0
for _ in $(seq 75); do
    N=$("$CTL" -j clients 2>/dev/null | grep -o '"stableId"' | wc -l || true)
    [ "$N" = 1 ] && break
    sleep 0.2
done
[ "$N" = 1 ] || fail "window did not map (clients: $N)"
sleep 2

WL=$(ls "$T" | grep -E '^wayland-[0-9]+$' | head -n 1)
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"

SIZE=$("$CTL" -j monitors | python3 -c 'import json,sys; m=json.load(sys.stdin)[0]; print(m["width"], m["height"])')
echo "output size: $SIZE"

timeout 20 grim -t ppm "$T/full.ppm" || fail "grim (full output) failed"
python3 - "$T/full.ppm" "$SIZE" <<'PY' || fail "full capture check failed"
import sys
data = open(sys.argv[1], "rb").read()
w, h = map(int, sys.argv[2].split())
parts = data.split(b"\n", 3)
assert parts[0] == b"P6", parts[0]
assert tuple(map(int, parts[1].split())) == (w, h), f"size {parts[1]} != {w}x{h}"
px = parts[3]
assert len(px) == w * h * 3, len(px)
def pixel(x, y):
    i = (y * w + x) * 3
    return tuple(px[i:i + 3])
def far_from_bg(p):
    return any(abs(c - 26) > 3 for c in p)
assert any(far_from_bg(pixel(x, y)) for x in range(w) for y in range(0, h, 7)), "all background"
top = sum(far_from_bg(pixel(x, y)) for x in range(30, 400, 5) for y in range(30, 300, 5))
bottom = sum(far_from_bg(pixel(x, y)) for x in range(0, w, 5) for y in range(h // 2 + 100, h, 5))
total = len(range(30, 400, 5)) * len(range(30, 300, 5))
print(f"window area non-background: {top}/{total}, bottom area: {bottom}")
assert top > total // 2, "window not found at the top: image missing or upside down"
assert bottom == 0, "content in the bottom half: image upside down"
PY
echo "ok: full output capture"

timeout 20 grim -t ppm -g "10,10 100x50" "$T/region.ppm" || fail "grim (region) failed"
head -c 20 "$T/region.ppm" | head -n 2 | tail -n 1 | grep -qx "100 50" || fail "region size is not 100x50"
echo "ok: region capture"

timeout 20 grim "$T/shot.png" || fail "grim (png) failed"
[ "$(head -c 4 "$T/shot.png" | tail -c 3)" = PNG ] || fail "not a PNG"
echo "ok: png capture"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
grep -q "panicked" "$T/log" && fail "compositor panicked"
# overlay_cursor: `grim -c` differs from a plain capture around the pointer.
"$CTL" dispatch movecursor "900 600" >/dev/null
sleep 0.5
timeout 20 grim -t ppm -g "880,580 60x60" "$T/plain.ppm" || fail "grim plain failed"
timeout 20 grim -c -t ppm -g "880,580 60x60" "$T/cursor.ppm" || fail "grim -c failed"
cmp -s "$T/plain.ppm" "$T/cursor.ppm" && fail "grim -c shows no cursor"
echo "ok: overlay_cursor"

echo "OK: wlr-screencopy through grim"
