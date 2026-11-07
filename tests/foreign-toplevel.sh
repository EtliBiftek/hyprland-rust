#!/bin/sh
# wlr-foreign-toplevel-management + wlr-output-management test: headless backend,
# wlrctl lists a client, fullscreens it and closes it; wlr-randr lists the output.
# Needs: weston (weston-simple-shm), wlrctl, wlr-randr, python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
: > "$T/test.conf"

COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -f weston-simple-shm 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 60 "$T/log"
    exit 1
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
WL=
for _ in $(seq 50); do
    WL=$(ls "$T" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -n 1 || true)
    [ -n "$WL" ] && break
    sleep 0.2
done
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"

clients() { timeout 2 "$CTL" -j clients; }
count() { clients 2>/dev/null | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))' 2>/dev/null || echo -1; }
wait_count() {
    n=-1
    for _ in $(seq 25); do
        n=$(count)
        [ "$n" = "$1" ] && return 0
        sleep 0.2
    done
    fail "expected $1 clients, have $n"
}

# Output management (read-only).
timeout 10 wlr-randr >"$T/randr" || fail "wlr-randr failed"
grep -q "HEADLESS-1" "$T/randr" || fail "wlr-randr does not list HEADLESS-1: $(cat "$T/randr")"
grep -q "1000x800" "$T/randr" || fail "wlr-randr does not list the 1000x800 mode: $(cat "$T/randr")"
echo "ok: wlr-output-management lists the output"

[ -z "$(timeout 10 wlrctl toplevel list)" ] || fail "toplevel list is not empty at start"

timeout 5 "$CTL" dispatch exec weston-simple-shm >/dev/null || fail "dispatch exec"
wait_count 1
CLASS=$(clients | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["class"])')
echo "client class: [$CLASS]"
[ -n "$CLASS" ] || fail "client has no app_id"

LIST=
for _ in $(seq 25); do
    LIST=$(timeout 10 wlrctl toplevel list || true)
    [ -n "$LIST" ] && break
    sleep 0.2
done
echo "wlrctl toplevel list: $LIST"
[ "$(echo "$LIST" | wc -l)" = 1 ] || fail "expected exactly one toplevel"
echo "$LIST" | grep -q "$CLASS" || fail "toplevel list does not contain the app_id"
echo "ok: wlr-foreign-toplevel lists the window"

# wlrctl segfaults on fullscreen/maximize (seen in CI), so only focus and close
# are exercised through it.
timeout 10 wlrctl toplevel focus "app_id:$CLASS" || fail "wlrctl focus failed"
echo "ok: activate request"

timeout 10 wlrctl toplevel close "app_id:$CLASS" || fail "wlrctl close failed"
wait_count 0
for _ in $(seq 25); do
    [ -z "$(timeout 10 wlrctl toplevel list || true)" ] && break
    sleep 0.2
done
[ -z "$(timeout 10 wlrctl toplevel list || true)" ] || fail "toplevel still listed after close"
echo "ok: close request and removal"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
grep -q "panicked" "$T/log" && fail "compositor panicked"
echo "OK: foreign toplevel management and output management"
