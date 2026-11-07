#!/bin/sh
# Dotfile-compat IPC test (headless): the hyprctl forms that popular dotfiles
# use. Flags after the command (`clients -j`), `--batch`, `submap`, `locked`,
# `getprop`, `eval`, `setcursor`, hyprpaper/hyprsunset failing cleanly, the
# monitors fields Quickshell reads, and the activespecialv2 / activelayout /
# submap events on .socket2.sock.
# Needs: weston (weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<'CONF'
bind = SUPER, A, submap, resize
submap = resize
bind = , escape, submap, reset
submap = reset
CONF

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

ctl() { timeout 5 "$HYPRCTL" "$@"; }

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
for _ in $(seq 50); do
    ctl -j clients >/dev/null 2>&1 && break
    sleep 0.2
done
HYPRLAND_INSTANCE_SIGNATURE=$(ls "$T/hypr" | head -n1)
export HYPRLAND_INSTANCE_SIGNATURE

EV="$T/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock"
python3 - "$EV" >"$T/events" <<'PY' &
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.sendall(b"j\n")  # Quickshell-style clients write to the event socket
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

# 1. Flags after the command and -q.
ctl clients -j | grep -q '^\[' || fail "clients -j"
ctl monitors -j | grep -q '"name"' || fail "monitors -j"
ctl monitors all -j | grep -q '"name"' || fail "monitors all -j"
ctl -j getoption general:border_size >"$T/o1"
ctl getoption general:border_size -j >"$T/o2"
cmp -s "$T/o1" "$T/o2" || fail "getoption -j position matters"
[ -z "$(ctl -q version)" ] || fail "-q prints"
ctl dispatch workspace 2 -q >/dev/null || fail "dispatch with trailing -q"
echo "ok: flags anywhere"

# 2. --batch with -j.
ctl --batch "keyword general:gaps_in 3 ; keyword general:gaps_out 4" | grep -c '^ok$' | grep -qx 2 \
    || fail "--batch keywords"
ctl --batch "activeworkspace ; monitors" -j >"$T/batch"
grep -q '^{' "$T/batch" || fail "--batch -j: $(cat "$T/batch")"
echo "ok: --batch"

# 3. Monitor fields Quickshell reads.
ctl monitors -j >"$T/mon.json"
python3 - "$T/mon.json" <<'PY' || fail "monitors fields: $(cat "$T/mon.json")"
import json, sys
(m,) = json.load(open(sys.argv[1]))
for k in ("id", "name", "description", "make", "model", "width", "height", "x", "y",
          "scale", "focused", "activeWorkspace", "specialWorkspace", "reserved",
          "availableModes", "dpmsStatus"):
    assert k in m, k
assert m["description"] and m["make"], m
assert len(m["reserved"]) == 4, m["reserved"]
print("monitor ok:", m["name"], m["description"])
PY

# 4. submap, locked, descriptions, eval, setcursor, getprop errors.
[ "$(ctl submap)" = default ] || fail "submap text"
[ "$(ctl -j submap)" = '"default"' ] || fail "submap json"
[ "$(ctl locked)" = false ] || fail "locked"
ctl -j locked | grep -q '"locked": false' || fail "locked json"
ctl descriptions | python3 -c 'import json,sys; json.load(sys.stdin)' || fail "descriptions json"
[ "$(ctl eval 'hl.config({general = {gaps_in = 7}})')" = ok ] || fail "eval config"
[ "$(ctl getoption general:gaps_in -j | python3 -c 'import json,sys; print(json.load(sys.stdin)["int"])')" = 7 ] \
    || fail "eval did not set gaps_in"
[ "$(ctl eval 'hl.dsp.focus({ workspace = 3 })' || true)" = ok ] || fail "eval dsp"
[ "$(ctl -j activeworkspace | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')" = 3 ] \
    || fail "eval dsp did not switch workspace"
[ "$(ctl setcursor Adwaita 24)" = ok ] || fail "setcursor"
[ "$(ctl setcursor Adwaita big)" = "size not int" ] || fail "setcursor error"
[ "$(ctl getprop)" = "not enough args" ] || fail "getprop args"
[ "$(ctl getprop class:nonexistant animation)" = "window not found" ] || fail "getprop window"
echo "ok: submap/locked/eval/setcursor"

# 5. hyprpaper / hyprsunset are other programs: clean failure, no hang.
if ctl hyprpaper wallpaper ",/tmp/x.png" 2>"$T/err"; then fail "hyprpaper should fail"; fi
grep -q "Couldn't connect" "$T/err" || fail "hyprpaper error: $(cat "$T/err")"
if ctl --quiet hyprsunset temperature 4000 2>/dev/null; then fail "hyprsunset should fail"; fi
echo "ok: hyprpaper/hyprsunset fail cleanly"

# 6. A window: getprop / setprop round trip.
ctl dispatch exec weston-simple-shm >/dev/null
for _ in $(seq 25); do
    n=$(ctl -j clients | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))' 2>/dev/null || echo 0)
    [ "$n" = 1 ] && break
    sleep 0.2
done
[ "$n" = 1 ] || fail "no client"
sleep 1
APP=$(ctl -j clients | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["class"])')
[ "$(ctl getprop "class:$APP" animation)" = "(unset)" ] || fail "getprop animation"
[ "$(ctl getprop "class:$APP" max_size)" = "inf inf" ] || fail "getprop max_size"
[ "$(ctl getprop "class:$APP" opacity -j)" = '{"opacity": 1}' ] || fail "getprop opacity json"
ctl dispatch setprop "class:$APP" opacity 0.3 >/dev/null
[ "$(ctl getprop "class:$APP" opacity)" = 0.3 ] || fail "getprop after setprop"
[ "$(ctl getprop active allows_input)" = false ] || fail "getprop active"
[ "$(ctl getprop "class:$APP" nonexistantprop)" = "prop not found" ] || fail "getprop prop"
echo "ok: getprop"

# 7. Events: activespecialv2 and submap.
ctl dispatch togglespecialworkspace magic >/dev/null
ctl dispatch togglespecialworkspace magic >/dev/null
ctl dispatch submap resize >/dev/null
ctl dispatch submap reset >/dev/null
ctl switchxkblayout all next >/dev/null
sleep 1
grep -q '^activespecialv2>>[-0-9]*,special:magic,HEADLESS-1$' "$T/events" || fail "activespecialv2 show: $(cat "$T/events")"
grep -q '^activespecialv2>>,,HEADLESS-1$' "$T/events" || fail "activespecialv2 hide"
grep -q '^submap>>resize$' "$T/events" || fail "submap event"
grep -q '^activelayout>>' "$T/events" || fail "activelayout event: $(cat "$T/events")"
echo "ok: events"

echo "PASS"
