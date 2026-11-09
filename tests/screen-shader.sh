#!/bin/sh
# decoration:screen_shader: an inverting GLSL 1.00 shader, then a GLSL 3.00 one
# after `hyprctl reloadshaders`. Pixels are read back with grim.
# Needs: Xvfb, grim, python3, Mesa.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"

cat > "$T/s.frag" <<'FRAG'
precision highp float;
varying vec2 v_texcoord;
uniform sampler2D tex;
void main() {
    gl_FragColor = vec4(1.0 - texture2D(tex, v_texcoord).rgb, 1.0);
}
FRAG
cat > "$T/test.conf" <<CONF
decoration {
    screen_shader = $T/s.frag
}
CONF

Xvfb :95 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
unset HYPRLAND_INSTANCE_SIGNATURE
export DISPLAY=:95 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    tail -n 40 "$T/log"
    exit 1
}

RUST_LOG=info "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
WL=
for _ in $(seq 50); do
    WL=$(ls "$T" | grep -E '^wayland-[0-9]+$' | head -n 1 || true)
    [ -n "$WL" ] && break
    sleep 0.2
done
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"
sleep 2

cat > "$T/px.py" <<'PY'
import sys
parts = open(sys.argv[1], "rb").read().split(b"\n", 3)
w, h = map(int, parts[1].split())
i = (h // 2 * w + w // 2) * 3
print(*parts[3][i:i + 3])
PY
pixel() {
    timeout 20 grim -t ppm "$T/shot.ppm" || fail "grim failed"
    python3 -I "$T/px.py" "$T/shot.ppm"
}

set -- $(pixel)
# The background is 0.1 grey: inverted it is about 230.
[ "$1" -ge 200 ] && [ "$2" -ge 200 ] && [ "$3" -ge 200 ] || fail "not inverted: $*"
echo "ok: inverted background ($*)"

cat > "$T/s.frag" <<'FRAG'
#version 300 es
precision highp float;
in vec2 v_texcoord;
uniform sampler2D tex;
out vec4 color;
void main() {
    color = vec4(0.0, texture(tex, v_texcoord).a, 0.0, 1.0);
}
FRAG
"$CTL" reloadshaders >/dev/null
sleep 1
set -- $(pixel)
[ "$1" -le 30 ] && [ "$2" -ge 200 ] && [ "$3" -le 30 ] || fail "not green after reload: $*"
echo "ok: GLSL 3.00 shader after reloadshaders ($*)"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: screen shader"
