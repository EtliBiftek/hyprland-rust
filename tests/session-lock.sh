#!/bin/sh
# ext-session-lock test with swaylock on 2 headless outputs of different sizes:
# `locked` after a surface commit, per-output configure, a killed locker stays
# locked, misc:allow_session_lock_restore.
# Needs: swaylock, python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
monitor = HEADLESS-2, 1280x720, auto, 1
CONF

COMP=
cleanup() {
    pkill -9 swaylock 2>/dev/null || true
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY

fail() {
    echo "FAIL: $1"
    echo "--- swaylock logs ---"
    tail -n 15 "$T"/s* || true
    echo "--- compositor log ---"
    tail -n 40 "$T/log"
    exit 1
}

ctl() { timeout 5 "$HYPRCTL" "$@"; }

wait_in() { # FILE PATTERN
    for _ in $(seq 40); do
        grep -q "$2" "$1" 2>/dev/null && return 0
        sleep 0.2
    done
    return 1
}

locker() { ctl dispatch exec "WAYLAND_DEBUG=1 swaylock -c 000000 2>$T/$1" >/dev/null; }

HYPR_HEADLESS_OUTPUTS=2 HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=info \
    "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
for _ in $(seq 50); do
    ctl -j monitors >/dev/null 2>&1 && break
    sleep 0.2
done

locker s1
wait_in "$T/s1" 'ext_session_lock_v1@.*\.locked()' || fail "no locked event"
[ "$(ctl locked)" = true ] || fail "hyprctl locked is not true"
grep -q 'ext_session_lock_surface_v1@.*configure(.*, 1000, 800)' "$T/s1" || fail "first output size"
grep -q 'ext_session_lock_surface_v1@.*configure(.*, 1280, 720)' "$T/s1" || fail "second output size"
echo "ok: locked, per-output sizes"

pkill -9 swaylock
sleep 1
[ "$(ctl locked)" = true ] || fail "killing the locker unlocked the session"
echo "ok: a dead locker keeps the session locked"

locker s2
wait_in "$T/s2" 'ext_session_lock_v1@.*\.finished()' || fail "second locker was not refused"
echo "ok: no restore by default"

ctl keyword misc:allow_session_lock_restore true >/dev/null
locker s3
wait_in "$T/s3" 'ext_session_lock_v1@.*\.locked()' || fail "restore did not lock"
echo "ok: misc:allow_session_lock_restore"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: session lock"
