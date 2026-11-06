#!/bin/sh
# Layer-shell test: headless backend, a waybar bar (exclusive zone, top layer)
# and a weston-simple-shm window. Checks that `hyprctl -j layers` lists the
# bar and that the tiled window is placed below the bar's exclusive zone.
# Needs: waybar, weston (weston-simple-shm).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
BAR_H=40
cat > "$T/waybar.json" <<CONF
{"layer": "top", "position": "top", "height": $BAR_H, "exclusive": true,
 "modules-center": ["clock"]}
CONF
: > "$T/waybar.css"
cat > "$T/test.conf" <<CONF
exec-once = GDK_BACKEND=wayland NO_AT_BRIDGE=1 dbus-run-session -- waybar -l debug -c $T/waybar.json -s $T/waybar.css >$T/waybar.log 2>&1
exec-once = weston-simple-shm
CONF

COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -f weston-simple-shm 2>/dev/null || true
    pkill -x waybar 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    cat "$T/log"
    echo "--- waybar log ---"
    cat "$T/waybar.log" 2>/dev/null || true
    exit 1
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

# Wait until the bar is listed and the window sits below it.
ok=
for _ in $(seq 100); do
    kill -0 "$COMP" 2>/dev/null || fail "compositor died"
    if "$HYPRCTL" -j layers >"$T/layers.json" 2>/dev/null &&
        "$HYPRCTL" -j clients >"$T/clients.json" 2>/dev/null &&
        python3 - "$T/layers.json" "$T/clients.json" "$BAR_H" <<'PY'
import json, sys
layers = json.load(open(sys.argv[1]))
clients = json.load(open(sys.argv[2]))
bar_h = int(sys.argv[3])
bars = [l for m in layers.values() for l in m["levels"]["2"] if l["namespace"] == "waybar"]
assert bars, "waybar not listed in level 2"
assert len(clients) == 1, "expected 1 client"
assert clients[0]["at"][1] >= bar_h, f"window y={clients[0]['at'][1]} overlaps bar"
PY
    then
        ok=1
        break
    fi
    sleep 0.3
done
[ -n "$ok" ] || fail "layer/window check never passed: layers=$(cat "$T/layers.json" 2>/dev/null) clients=$(cat "$T/clients.json" 2>/dev/null)"
"$HYPRCTL" layers | grep -q "namespace: waybar" || fail "text layers output lacks waybar"
echo "OK: waybar listed by hyprctl layers, window below its exclusive zone"
