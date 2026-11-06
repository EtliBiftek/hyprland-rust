#!/bin/sh
# Headless backend smoke test: starts the compositor with --backend headless,
# launches weston-simple-shm via exec-once and checks the log for a mapped window.
# Needs: weston (weston-simple-shm).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
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

RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

for _ in $(seq 50); do
    grep -q "window mapped" "$T/log" && break
    sleep 0.2
done
sleep 2

grep -q "backend: headless" "$T/log" || fail "headless backend not started"
grep -q "window mapped" "$T/log" || fail "no window mapped"
kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: headless backend mapped a window and stayed alive"
