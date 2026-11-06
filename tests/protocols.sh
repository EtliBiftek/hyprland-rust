#!/bin/sh
# Protocol test: headless backend + wl-clipboard. wl-copy/wl-paste need the
# data-control protocols (clipboard and primary selection) to work without a
# focused client.
# Needs: wl-clipboard (wl-copy, wl-paste).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
T=$(mktemp -d)
chmod 700 "$T"
echo "# empty" > "$T/test.conf"

COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -x wl-copy 2>/dev/null || true
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

HYPR_HEADLESS_SIZE=800x600 "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

# The compositor creates its socket as wayland-N inside $XDG_RUNTIME_DIR.
WL=
for _ in $(seq 50); do
    WL=$(ls "$T" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -n 1 || true)
    [ -n "$WL" ] && break
    sleep 0.2
done
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"
kill -0 "$COMP" 2>/dev/null || fail "compositor died"

echo hi | wl-copy || fail "wl-copy failed"
got=$(timeout 5 wl-paste -n) || fail "wl-paste failed (data-control missing?)"
[ "$got" = "hi" ] || fail "clipboard: expected 'hi', got '$got'"

echo prim | wl-copy --primary || fail "wl-copy --primary failed"
got=$(timeout 5 wl-paste --primary -n) || fail "wl-paste --primary failed"
[ "$got" = "prim" ] || fail "primary selection: expected 'prim', got '$got'"

kill -0 "$COMP" 2>/dev/null || fail "compositor died during the test"
grep -q "panicked" "$T/log" && fail "compositor panicked"
echo "OK: clipboard and primary selection through data-control"
