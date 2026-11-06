#!/bin/sh
# Headless backend smoke test: starts the compositor with --backend headless,
# launches two weston-simple-shm windows via exec-once, checks the log and
# checks the tiled layout through `hyprctl -j clients`.
# Needs: weston (weston-simple-shm).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
exec-once = weston-simple-shm
exec-once = weston-simple-shm
CONF

COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -f weston-simple-shm 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    cat "$T/log"
    exit 1
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

for _ in $(seq 50); do
    [ "$(grep -c "window mapped" "$T/log")" -ge 2 ] && break
    sleep 0.2
done
sleep 2

grep -q "backend: headless" "$T/log" || fail "headless backend not started"
grep -q "window mapped" "$T/log" || fail "no window mapped"
kill -0 "$COMP" 2>/dev/null || fail "compositor died"

# Two tiled windows must not overlap and must lie inside the 1000x800 output.
"$HYPRCTL" -j clients >"$T/clients.json" || fail "hyprctl clients failed"
python3 - "$T/clients.json" <<'PY' || fail "unexpected layout: $(cat "$T/clients.json")"
import json, sys
c = json.load(open(sys.argv[1]))
assert len(c) == 2, f"expected 2 clients, got {len(c)}"
(ax, ay), (bx, by) = c[0]["at"], c[1]["at"]
assert (ax, ay) != (bx, by), "windows at the same position"
for w in c:
    x, y = w["at"]
    assert 0 <= x < 1000 and 0 <= y < 800, f"window outside output: {w['at']}"
print("layout:", [w["at"] for w in c])
PY
echo "OK: headless backend tiled two windows (checked via hyprctl)"
