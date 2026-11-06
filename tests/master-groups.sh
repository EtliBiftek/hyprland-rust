#!/bin/sh
# Master layout and groups test: headless backend, three weston-simple-shm
# windows, everything checked through `hyprctl -j clients`.
#  - master layout geometry (mfact, orientation), `layoutmsg swapwithmaster`
#  - runtime switch with `keyword general:layout`
#  - groups: `moveintogroup` hides the other member, `changegroupactive`
#    swaps which one is visible, `togglegroup` dissolves the group.
# Needs: weston (weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
general {
    layout = master
    gaps_in = 0
    gaps_out = 0
}
exec-once = weston-simple-shm
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

# check DESCRIPTION 'python assertions on c (the clients list)'; retries for 10 s.
PRE='import json; c = json.load(open("'$T'/clients.json")); pos = sorted(tuple(w["at"]) for w in c if not w["hidden"])'
check() {
    for _ in $(seq 50); do
        if "$HYPRCTL" -j clients >"$T/clients.json" 2>/dev/null &&
            python3 -c "$PRE; $2" 2>"$T/err"; then
            echo "ok: $1"
            return 0
        fi
        sleep 0.2
    done
    cat "$T/err" || true
    fail "$1: $(cat "$T/clients.json")"
}

# Address of the visible window at X,Y.
addr_at() {
    "$HYPRCTL" -j clients | python3 -c \
        'import json,sys; print([w["address"] for w in json.load(sys.stdin) if w["at"]==[int(sys.argv[1]),int(sys.argv[2])] and not w["hidden"]][0])' "$1" "$2"
}

ctl() {
    R=$("$HYPRCTL" "$@")
    [ "$R" = ok ] || fail "hyprctl $* replied '$R'"
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

check "master left: master 55% wide, two slaves stacked" \
    'assert len(c) == 3, len(c); assert pos == [(0, 0), (550, 0), (550, 400)], pos'
kill -0 "$COMP" 2>/dev/null || fail "compositor died"

"$HYPRCTL" -j workspaces | python3 -c 'import json,sys; w=json.load(sys.stdin); assert w[0]["tiledLayout"]=="master", w' ||
    fail "tiledLayout is not master"

# swapwithmaster: the focused slave becomes the master.
SLAVE=$(addr_at 550 400)
MASTER=$(addr_at 0 0)
ctl dispatch focuswindow "address:$SLAVE"
ctl dispatch layoutmsg swapwithmaster
check "swapwithmaster moved the focused slave to the master slot" \
    'at = {w["address"]: w["at"] for w in c}; assert at["'"$SLAVE"'"] == [0, 0], at; assert at["'"$MASTER"'"] == [550, 400], at'

ctl dispatch layoutmsg mfact exact 0.7
check "mfact exact 0.7" \
    'assert pos == [(0, 0), (700, 0), (700, 400)], pos'
ctl dispatch layoutmsg mfact exact 0.55

ctl dispatch layoutmsg orientationtop
check "orientation top: master on top, slaves below" \
    'assert pos == [(0, 0), (0, 440), (500, 440)], pos'
ctl dispatch layoutmsg orientationleft

# Runtime layout switch.
R=$("$HYPRCTL" keyword general:layout dwindle)
[ "$R" = ok ] || fail "keyword general:layout replied '$R'"
check "keyword general:layout dwindle re-tiles the windows" \
    'assert len(c) == 3 and (550, 0) not in pos, pos'
"$HYPRCTL" getoption general:layout | grep -q dwindle || fail "getoption general:layout"
ctl keyword general:layout master
check "back to master" \
    'assert pos == [(0, 0), (550, 0), (550, 400)], pos'

# Groups: put the lower slave into the upper slave's group.
UPPER=$(addr_at 550 0)
LOWER=$(addr_at 550 400)
ctl dispatch focuswindow "address:$UPPER"
ctl dispatch togglegroup
ctl dispatch focuswindow "address:$LOWER"
ctl dispatch moveintogroup u
check "moveintogroup: group of two takes one slot, one member hidden" \
    'h = [w for w in c if w["hidden"]]; v = [w for w in c if not w["hidden"] and w["grouped"]]; assert len(c) == 3 and len(h) == 1 and len(v) == 1, c; assert v[0]["address"] == "'"$LOWER"'" and h[0]["address"] == "'"$UPPER"'", c; assert sorted(v[0]["grouped"]) == sorted(["'"$LOWER"'", "'"$UPPER"'"]), v; assert pos == [(0, 0), (550, 0)], pos'

ctl dispatch changegroupactive f
check "changegroupactive swaps the visible member" \
    'h = [w for w in c if w["hidden"]]; assert len(h) == 1 and h[0]["address"] == "'"$LOWER"'", c; assert pos == [(0, 0), (550, 0)], pos'

ctl dispatch togglegroup
check "togglegroup dissolves the group, all windows visible again" \
    'assert not any(w["hidden"] or w["grouped"] for w in c), c; assert len(pos) == 3, pos'

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: master layout and groups (checked via hyprctl)"
