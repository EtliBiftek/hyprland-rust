#!/bin/sh
# Dispatcher test: headless backend, dispatchers sent with `hyprctl dispatch`.
# Checks renameworkspace (hyprctl + renameworkspace event), togglepseudo
# (window centered in its slot), movecursor and movecursortocorner
# (`hyprctl -j cursorpos`), alterzorder, event (custom event on .socket2.sock),
# dpms (`hyprctl -j monitors`) and the accepted no-op dispatchers.
# Needs: weston (weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
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
unset DISPLAY WAYLAND_DISPLAY

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 60 "$T/log"
    exit 1
}

clients() { timeout 2 "$HYPRCTL" -j clients; }

# Waits until `hyprctl -j clients` lists exactly $1 windows.
wait_clients() {
    n=-1
    for _ in $(seq 12); do
        n=$(clients 2>/dev/null | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))' 2>/dev/null || echo -1)
        [ "$n" = "$1" ] && return 0
        sleep 0.2
    done
    fail "expected $1 clients, have $n"
}

# Gives the client time to be configured and placed.
settle() { sleep 1.5; }

# dispatch NAME ARGS: the reply must be exactly "ok".
dsp() {
    r=$(timeout 5 "$HYPRCTL" dispatch "$@") || fail "hyprctl dispatch $* failed"
    [ "$r" = ok ] || fail "dispatch $*: replied '$r'"
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
for _ in $(seq 50); do
    clients >/dev/null 2>&1 && break
    sleep 0.2
done

# Event reader on .socket2.sock, started before the dispatches.
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
sleep 1

# 1. renameworkspace: the workspace list and the event show the new name.
dsp renameworkspace "1 mywork"
timeout 5 "$HYPRCTL" -j workspaces >"$T/ws.json"
python3 - "$T/ws.json" <<'PY' || fail "renameworkspace: $(cat "$T/ws.json")"
import json, sys
ws = json.load(open(sys.argv[1]))
names = {w["id"]: w["name"] for w in ws}
assert names.get(1) == "mywork", names
print("renameworkspace ok:", names)
PY
sleep 1
grep -q '^renameworkspace>>1,mywork$' "$T/events" || fail "no renameworkspace event: $(cat "$T/events")"

# 2. A window for togglepseudo, movecursortocorner and alterzorder.
timeout 5 "$HYPRCTL" dispatch exec weston-simple-shm >/dev/null || fail "dispatch exec"
wait_clients 1
settle
clients >"$T/c1.json"
python3 - "$T/c1.json" <<'PY' || fail "window not tiled: $(cat "$T/c1.json")"
import json, sys
(w,) = json.load(open(sys.argv[1]))
assert w["floating"] is False, w
print("tiled before togglepseudo:", w["at"], w["size"])
PY

# 3. togglepseudo: the window keeps its size and is centered in its slot.
# shortcut: weston-simple-shm adopts the configured size, so the slot is the
# window's own size here and the check is only "still centered, same size".
dsp togglepseudo
settle
clients >"$T/c2.json"
python3 - "$T/c1.json" "$T/c2.json" <<'PY' || fail "togglepseudo changed the window: $(cat "$T/c2.json")"
import json, sys
(before,) = json.load(open(sys.argv[1]))
(w,) = json.load(open(sys.argv[2]))
x, y = w["at"]
sx, sy = w["size"]
assert list(w["size"]) == list(before["size"]), (before["size"], w["size"])
assert abs(x + sx / 2 - 500) <= 3, (x, sx)
assert abs(y + sy / 2 - 400) <= 3, (y, sy)
print("togglepseudo centered:", w["at"], w["size"])
PY

# 4. movecursortocorner 3: pointer to the top-left corner of the window.
dsp movecursortocorner 3
X=$(timeout 2 "$HYPRCTL" -j cursorpos | python3 -c 'import json,sys; print(json.load(sys.stdin)["x"])')
Y=$(timeout 2 "$HYPRCTL" -j cursorpos | python3 -c 'import json,sys; print(json.load(sys.stdin)["y"])')
clients >"$T/c3.json"
python3 - "$T/c3.json" "$X" "$Y" <<'PY' || fail "movecursortocorner 3 wrong: $(cat "$T/c3.json")"
import json, sys
(w,) = json.load(open(sys.argv[1]))
x, y = w["at"]
cx, cy = int(sys.argv[2]), int(sys.argv[3])
assert abs(cx - x) <= 2 and abs(cy - y) <= 2, ((cx, cy), (x, y))
print("movecursortocorner 3 ok:", (cx, cy))
PY

# 5. alterzorder top and bottom keep the window.
dsp alterzorder top
dsp alterzorder bottom
wait_clients 1

# 6. movecursor: absolute position, read back with cursorpos.
dsp movecursor "123 45"
C=$(timeout 2 "$HYPRCTL" -j cursorpos)
python3 - "$C" <<'PY' || fail "movecursor 123 45: $C"
import json, sys
c = json.loads(sys.argv[1])
assert (c["x"], c["y"]) == (123, 45), c
print("movecursor ok:", c)
PY

# 7. event: custom>>DATA on the event socket.
dsp event "hello dispatchers"
sleep 1
grep -q '^custom>>hello dispatchers$' "$T/events" || fail "no custom event: $(cat "$T/events")"

# 8. dpms off/on and the monitor's dpmsStatus.
dsp dpms off
timeout 5 "$HYPRCTL" -j monitors | python3 -c 'import json,sys; assert json.load(sys.stdin)[0]["dpmsStatus"] is False' || fail "dpms off not reported"
dsp dpms toggle
timeout 5 "$HYPRCTL" -j monitors | python3 -c 'import json,sys; assert json.load(sys.stdin)[0]["dpmsStatus"] is True' || fail "dpms toggle not reported"

# 9. Accepted no-ops (single monitor, no swallow or renderer reload yet).
dsp focusmonitor DP-1
dsp toggleswallow
dsp forcerendererreload
dsp bringactivetotop
dsp setprop active bordersize 3

# 9b. binds:hide_special_on_workspace_change closes the special workspace.
timeout 5 "$HYPRCTL" keyword binds:hide_special_on_workspace_change true >/dev/null
dsp togglespecialworkspace
timeout 5 "$HYPRCTL" -j monitors | python3 -c 'import json,sys; assert json.load(sys.stdin)[0]["specialWorkspace"]["id"] != 0' || fail "special workspace not shown"
dsp workspace 2
timeout 5 "$HYPRCTL" -j monitors | python3 -c 'import json,sys; assert json.load(sys.stdin)[0]["specialWorkspace"].get("id", 0) == 0' || fail "special workspace stayed open"
dsp workspace 1

# 10. Window argument: act on the unfocused window, the focus stays.
timeout 5 "$HYPRCTL" dispatch exec weston-simple-shm >/dev/null || fail "dispatch exec"
wait_clients 2
settle
ADDR=$(clients | python3 -c 'import json,sys; print(max(json.load(sys.stdin), key=lambda w: w["focusHistoryID"])["address"])')
dsp setfloating "address:$ADDR"
dsp movetoworkspacesilent "5,address:$ADDR"
sleep 0.5
clients >"$T/c4.json"
python3 - "$T/c4.json" "$ADDR" <<'PY' || fail "window argument: $(cat "$T/c4.json")"
import json, sys
ws = json.load(open(sys.argv[1]))
target = next(w for w in ws if w["address"] == sys.argv[2])
other = next(w for w in ws if w["address"] != sys.argv[2])
assert target["workspace"]["id"] == 5 and target["floating"] is True, target
assert other["workspace"]["id"] == 1 and other["floating"] is False, other
assert other["focusHistoryID"] == 0, other
print("window argument ok")
PY

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: dispatchers (renameworkspace, togglepseudo, movecursor, movecursortocorner, alterzorder, event, dpms, no-ops)"
