#!/bin/sh
# misc:enable_swallow: a client started from weston-terminal takes its place,
# and the terminal comes back when the client exits.
# Needs: weston (weston-terminal, weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
misc {
    enable_swallow = true
    swallow_regex = .*terminal.*
}
CONF
printf '#!/bin/sh\nweston-simple-shm\nsleep 60\n' > "$T/child.sh"
chmod +x "$T/child.sh"

COMP=
cleanup() {
    pkill -f weston-simple-shm 2>/dev/null || true
    pkill weston-terminal 2>/dev/null || true
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY

fail() {
    echo "FAIL: $1"
    timeout 2 "$HYPRCTL" -j clients || true
    tail -n 40 "$T/log"
    exit 1
}

# classes: sorted class list of the open windows.
classes() { timeout 2 "$HYPRCTL" -j clients | python3 -c 'import json,sys; print(",".join(sorted(c["class"] for c in json.load(sys.stdin))))'; }
wait_classes() { # grep pattern on the class list
    for _ in $(seq 60); do
        classes 2>/dev/null | grep -Eq "$1" && return 0
        sleep 0.2
    done
    return 1
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=info "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
for _ in $(seq 50); do
    timeout 2 "$HYPRCTL" -j clients >/dev/null 2>&1 && break
    sleep 0.2
done

timeout 5 "$HYPRCTL" dispatch exec "weston-terminal --shell=$T/child.sh" >/dev/null || fail "exec"
# Only the client is listed once it has swallowed the terminal.
wait_classes '^[^,]+$' || fail "no window"
for _ in $(seq 60); do
    c=$(classes)
    case "$c" in *,*|*terminal*|"") sleep 0.2 ;; *) break ;; esac
done
case "$c" in *,*|*terminal*|"") fail "terminal was not swallowed: [$c]" ;; esac
echo "ok: swallowed, clients: [$c]"

pkill -f weston-simple-shm
wait_classes '^[^,]*terminal[^,]*$' || fail "terminal did not come back: [$(classes)]"
kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: swallow"
