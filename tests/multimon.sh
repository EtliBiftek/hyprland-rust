#!/bin/sh
# Multi-monitor test (headless, 2 virtual outputs): monitor rules (mode,
# auto position), `workspace = N, monitor:X, default:true`, per-monitor
# workspaces, focusmonitor, movecurrentworkspacetomonitor,
# swapactiveworkspaces, movewindow mon:, `hyprctl output create/remove`
# and the focusedmon / monitoradded / monitorremoved / moveworkspace events.
# Needs: weston (weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<'CONF'
monitor = HEADLESS-2, 1280x720, auto, 1
monitor = TEST, 640x480, auto, 1
workspace = 5, monitor:HEADLESS-2, default:true
general {
    gaps_in = 0
    gaps_out = 0
    border_size = 0
}
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
    echo "--- monitors ---"
    ctl -j monitors || true
    echo "--- clients ---"
    ctl -j clients || true
    echo "--- events ---"
    cat "$T/events" 2>/dev/null || true
    echo "--- compositor log ---"
    tail -n 60 "$T/log"
    exit 1
}

ctl() { timeout 5 "$HYPRCTL" "$@"; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
mon() { ctl -j monitors | json "[m for m in d if m['name']=='$1'][0]$2"; }
clients_on() { ctl -j clients | json "len([c for c in d if c['monitor']==$1])"; }
wait_for() { # python-expr-on-monitors expected
    for _ in $(seq 50); do
        [ "$(ctl -j monitors | json "$1" 2>/dev/null)" = "$2" ] && return 0
        sleep 0.1
    done
    return 1
}

HYPR_HEADLESS_OUTPUTS=2 HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=info \
    "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

n=0
for _ in $(seq 50); do
    n=$(ctl -j clients 2>/dev/null | json 'len(d)' 2>/dev/null || echo 0)
    [ "$n" = 1 ] && break
    sleep 0.2
done
[ "$n" = 1 ] || fail "exec-once window did not open"

EV=$(ls -d "$T"/hypr/*/ | head -n1).socket2.sock
python3 - "$EV" >"$T/events" <<'PY' &
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.settimeout(1)
end = time.time() + 60
while time.time() < end:
    try:
        d = s.recv(4096)
    except socket.timeout:
        continue
    if not d:
        break
    sys.stdout.write(d.decode())
    sys.stdout.flush()
PY
sleep 0.3

# ---- layout of the outputs ---------------------------------------------------
[ "$(ctl -j monitors | json 'len(d)')" = 2 ] || fail "expected 2 monitors"
[ "$(mon HEADLESS-1 '["x"]')" = 0 ] || fail "HEADLESS-1 not at x=0"
[ "$(mon HEADLESS-2 '["x"]')" = 1000 ] || fail "HEADLESS-2 not auto-placed right of HEADLESS-1"
[ "$(mon HEADLESS-2 '["width"]')" = 1280 ] || fail "monitor rule mode 1280x720 not applied"
[ "$(mon HEADLESS-2 '["activeWorkspace"]["id"]')" = 5 ] || fail "default:true workspace 5 not on HEADLESS-2"
[ "$(mon HEADLESS-1 '["activeWorkspace"]["id"]')" = 1 ] || fail "HEADLESS-1 does not show workspace 1"
[ "$(mon HEADLESS-1 '["focused"]')" = True ] || fail "HEADLESS-1 not focused at start"
[ "$(clients_on 0)" = 1 ] || fail "first window not on monitor 0"
echo "ok: monitor rules, auto placement, default workspace per monitor"

# ---- focusmonitor -------------------------------------------------------------
[ "$(ctl dispatch focusmonitor r)" = ok ] || fail "focusmonitor r"
wait_for "[m['name'] for m in d if m['focused']][0]" HEADLESS-2 || fail "focusmonitor r did not focus HEADLESS-2"
[ "$(ctl -j activeworkspace | json 'd["id"]')" = 5 ] || fail "active workspace is not 5 after focusing HEADLESS-2"
grep -q '^focusedmon>>HEADLESS-2,5$' "$T/events" || fail "no focusedmon>>HEADLESS-2,5"
echo "ok: focusmonitor + focusedmon event"

# A new window opens on the focused monitor.
ctl dispatch exec weston-simple-shm >/dev/null
for _ in $(seq 50); do
    [ "$(clients_on 1)" = 1 ] && break
    sleep 0.1
done
[ "$(clients_on 1)" = 1 ] || fail "new window did not open on HEADLESS-2"
x=$(ctl -j clients | json "[c for c in d if c['monitor']==1][0]['at'][0]")
[ "$x" -ge 1000 ] || fail "window on HEADLESS-2 is at x=$x"
echo "ok: new window on the focused monitor, placed inside it"

# workspace 1 is shown on HEADLESS-1: switching to it moves the focus there.
ctl dispatch workspace 1 >/dev/null
wait_for "[m['name'] for m in d if m['focused']][0]" HEADLESS-1 || fail "workspace 1 did not focus HEADLESS-1"
[ "$(mon HEADLESS-2 '["activeWorkspace"]["id"]')" = 5 ] || fail "HEADLESS-2 lost workspace 5"
echo "ok: workspace on another monitor focuses that monitor"

# ---- movecurrentworkspacetomonitor ---------------------------------------------
ctl dispatch movecurrentworkspacetomonitor HEADLESS-2 >/dev/null
wait_for "[m['activeWorkspace']['id'] for m in d if m['name']=='HEADLESS-2'][0]" 1 \
    || fail "workspace 1 not moved to HEADLESS-2"
h1=$(mon HEADLESS-1 '["activeWorkspace"]["id"]')
[ "$h1" != 1 ] && [ "$h1" != 5 ] || fail "HEADLESS-1 shows $h1 after the move"
grep -q '^moveworkspacev2>>1,1,HEADLESS-2$' "$T/events" || fail "no moveworkspacev2 event"
w1x=$(ctl -j clients | json "[c for c in d if c['workspace']['id']==1][0]['at'][0]")
[ "$w1x" -ge 1000 ] || fail "window of workspace 1 not on HEADLESS-2 (x=$w1x)"
echo "ok: movecurrentworkspacetomonitor"

# ---- swapactiveworkspaces -----------------------------------------------------
ctl dispatch swapactiveworkspaces HEADLESS-1 HEADLESS-2 >/dev/null
wait_for "[m['activeWorkspace']['id'] for m in d if m['name']=='HEADLESS-1'][0]" 1 \
    || fail "swapactiveworkspaces did not bring workspace 1 back"
w1x=$(ctl -j clients | json "[c for c in d if c['workspace']['id']==1][0]['at'][0]")
[ "$w1x" -lt 1000 ] || fail "window of workspace 1 not back on HEADLESS-1 (x=$w1x)"
echo "ok: swapactiveworkspaces"

# ---- movewindow mon: ------------------------------------------------------------
ctl dispatch focusmonitor HEADLESS-1 >/dev/null
ctl dispatch focuswindow class:.* >/dev/null || true
before=$(clients_on 1)
ctl dispatch movewindow mon:HEADLESS-2 >/dev/null
for _ in $(seq 30); do
    [ "$(clients_on 1)" -gt "$before" ] && break
    sleep 0.1
done
[ "$(clients_on 1)" -gt "$before" ] || fail "movewindow mon:HEADLESS-2 did not move a window"
echo "ok: movewindow mon:"

# ---- hyprctl output create / remove ------------------------------------------
[ "$(ctl output create headless TEST)" = ok ] || fail "output create"
wait_for 'len(d)' 3 || fail "output create did not add a monitor"
[ "$(mon TEST '["width"]')" = 640 ] || fail "monitor rule for TEST not applied"
[ "$(mon TEST '["x"]')" = 2280 ] || fail "TEST not auto-placed after HEADLESS-2 (x=$(mon TEST '["x"]'))"
grep -q '^monitoraddedv2>>.*,TEST,' "$T/events" || fail "no monitoraddedv2 for TEST"
[ "$(ctl output remove TEST)" = ok ] || fail "output remove"
wait_for 'len(d)' 2 || fail "output remove did not remove the monitor"
grep -q '^monitorremoved>>TEST$' "$T/events" || fail "no monitorremoved for TEST"
[ "$(ctl output remove NOPE)" != ok ] || fail "removing a missing output succeeded"
echo "ok: output create/remove + events"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "multimon: OK"
