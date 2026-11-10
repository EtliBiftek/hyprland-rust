#!/bin/sh
# misc:initial_workspace_tracking: a window opens on the workspace its `exec`
# ran on, even when another workspace is shown by the time it maps. Off (0)
# and an expired token (initial_workspace_token_timeout) open it on the
# current workspace.
# Needs: weston (weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
: > "$T/test.conf"

COMP=
cleanup() {
    pkill -f weston-simple-shm 2>/dev/null || true
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
ctl() { timeout 5 "$HYPRCTL" "$@" >/dev/null || fail "hyprctl $*"; }

# ws_list: sorted workspace ids of the open windows, e.g. "1,1,3".
ws_list() { timeout 2 "$HYPRCTL" -j clients | python3 -c 'import json,sys; print(",".join(sorted(str(c["workspace"]["id"]) for c in json.load(sys.stdin))))'; }
active() { timeout 2 "$HYPRCTL" -j activeworkspace | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])'; }
wait_ws() { # expected ws_list
    for _ in $(seq 60); do
        [ "$(ws_list 2>/dev/null)" = "$1" ] && return 0
        sleep 0.2
    done
    fail "windows on [$(ws_list)], want [$1]"
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=info "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
for _ in $(seq 50); do
    timeout 2 "$HYPRCTL" -j clients >/dev/null 2>&1 && break
    sleep 0.2
done

# The client starts late, after the workspace switch.
late() { ctl dispatch exec "sh -c 'sleep $1; exec weston-simple-shm'"; }

# 1. Default (1): started on 1, mapped while 2 is shown: opens on 1, silently.
late 1.5
ctl dispatch workspace 2
wait_ws 1
[ "$(active)" = 2 ] || fail "workspace followed the window: $(active)"
echo "ok: window opened on the workspace of its exec"

# 2. Off: the window opens on the workspace shown when it maps.
ctl keyword misc:initial_workspace_tracking 0
late 1.5
ctl dispatch workspace 3
wait_ws 1,3
echo "ok: tracking off"

# 3. The token expires after initial_workspace_token_timeout seconds.
ctl keyword misc:initial_workspace_tracking 1
ctl keyword misc:initial_workspace_token_timeout 1
late 3
ctl dispatch workspace 4
wait_ws 1,3,4
echo "ok: expired token"

# 4. A `workspace` window rule wins over the token.
ctl keyword misc:initial_workspace_token_timeout 10
ctl keyword windowrule "match:class .*, workspace 6 silent"
late 1.5
ctl dispatch workspace 5
wait_ws 1,3,4,6

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: initial workspace tracking"
