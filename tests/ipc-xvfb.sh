#!/bin/sh
# IPC test: runs the compositor (winit backend) inside Xvfb with one Wayland
# client and checks hyprctl against .socket.sock plus events on .socket2.sock.
# Needs: Xvfb, weston (weston-simple-shm, weston-terminal), python3,
# libxkbcommon-x11-0, Mesa.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
exec-once = weston-simple-shm
CONF

Xvfb :98 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
    pkill -f weston-simple-shm 2>/dev/null || true
    pkill -f weston-terminal 2>/dev/null || true
}
trap cleanup EXIT
# No HYPRLAND_INSTANCE_SIGNATURE here: hyprctl must find the instance itself.
unset HYPRLAND_INSTANCE_SIGNATURE
export DISPLAY=:98 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    cat "$T/log"
    exit 1
}

RUST_LOG=debug "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

# Wait for the sockets and for the client window to be mapped.
N=0
for _ in $(seq 75); do
    N=$("$CTL" -j clients 2>/dev/null | grep -o '"stableId"' | wc -l || true)
    [ "$N" = 1 ] && break
    sleep 0.2
done
[ "$N" = 1 ] || fail "hyprctl -j clients did not return 1 client (got $N)"
"$CTL" -j clients | grep -q '^\[{' || fail "clients is not a JSON array"
echo "ok: hyprctl -j clients -> 1 element"

"$CTL" version | grep -q Hyprland || fail "hyprctl version"

# Event reader on .socket2.sock, started before the dispatch.
EV=$(ls -d "$T"/hypr/*/ | head -n1).socket2.sock
python3 - "$EV" >"$T/events" <<'PY' &
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.connect(sys.argv[1])
s.settimeout(1)
end = time.time() + 90
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

R=$("$CTL" dispatch workspace 2)
[ "$R" = ok ] || fail "dispatch workspace 2 replied '$R'"
"$CTL" -j activeworkspace | grep -q '"id":2' || fail "activeworkspace is not 2"
echo "ok: dispatch workspace 2 -> activeworkspace id 2"

for _ in $(seq 25); do
    grep -q '^workspace>>2$' "$T/events" && break
    sleep 0.2
done
grep -q '^workspace>>2$' "$T/events" || { cat "$T/events"; fail "no workspace>>2 event"; }
echo "ok: event workspace>>2"

R=$("$CTL" --batch "dispatch workspace 1; dispatch workspace 3")
[ "$R" = "$(printf 'ok


ok')" ] || fail "batch replied '$R'"
"$CTL" -j activeworkspace | grep -q '"id":3' || fail "batch did not reach workspace 3"
echo "ok: batch"

# ---- config: getoption, keyword, reload, autoreload ------------------------
"$CTL" dispatch workspace 1 >/dev/null
"$CTL" -j getoption general:gaps_in | grep -q '"int": 5' || fail "getoption -j gaps_in"
"$CTL" getoption general:gaps_in | head -n1 | grep -qx 'int: 5' || fail "getoption gaps_in text"
"$CTL" -j getoption general:gaps_out | grep -q '"set": false' || fail "gaps_out should not be set"
echo "ok: getoption"

AT=$("$CTL" -j clients | grep -o '"at":\[[-0-9,]*\]' | head -n1)
R=$("$CTL" keyword general:gaps_out 8)
[ "$R" = ok ] || fail "keyword replied '$R'"
AT2=$("$CTL" -j clients | grep -o '"at":\[[-0-9,]*\]' | head -n1)
[ "$AT2" = '"at":[8,8]' ] || fail "keyword gaps_out 8: at was $AT, now $AT2"
"$CTL" -j getoption general:gaps_out | grep -q '"int": 8, "set": true' || fail "getoption after keyword"
R=$("$CTL" keyword nope:nope 1)
[ "$R" != ok ] || fail "keyword accepted an unknown option"
echo "ok: keyword changes layout ($AT -> $AT2)"

wait_at() { # wait_at X Y: client at [X,Y] within ~6 s
    for _ in $(seq 30); do
        "$CTL" -j clients | grep -q "\"at\":\[$1,$2\]" && return 0
        sleep 0.2
    done
    return 1
}

printf 'exec-once = weston-simple-shm
general {
 gaps_out = 7
}
' > "$T/test.conf"
R=$("$CTL" reload)
[ "$R" = ok ] || fail "reload replied '$R'"
wait_at 7 7 || fail "reload did not apply gaps_out = 7"
echo "ok: reload"

printf 'exec-once = weston-simple-shm
general:gaps_in = abc
' > "$T/test.conf"
"$CTL" reload >/dev/null
"$CTL" configerrors | grep -q 'invalid integer' || fail "configerrors lacks the parse error"
"$CTL" -j configerrors | grep -q 'invalid integer' || fail "configerrors -j lacks the parse error"
echo "ok: configerrors"

printf 'exec-once = weston-simple-shm
general {
 gaps_out = 11
}
' > "$T/test.conf"
wait_at 11 11 || fail "autoreload did not apply gaps_out = 11"
echo "ok: autoreload"

printf 'exec-once = weston-simple-shm
misc {
 disable_autoreload = true
}
general {
 gaps_out = 13
}
' > "$T/test.conf"
wait_at 13 13 || fail "autoreload did not apply gaps_out = 13"
printf 'exec-once = weston-simple-shm
misc {
 disable_autoreload = true
}
general {
 gaps_out = 17
}
' > "$T/test.conf"
sleep 3
"$CTL" -j clients | grep -q '"at":\[13,13\]' || fail "disable_autoreload was ignored"
"$CTL" reload >/dev/null
wait_at 17 17 || fail "manual reload after disable_autoreload"
echo "ok: disable_autoreload"

for _ in $(seq 25); do
    grep -q '^configreloaded>>$' "$T/events" && break
    sleep 0.2
done
grep -q '^configreloaded>>$' "$T/events" || { cat "$T/events"; fail "no configreloaded event"; }
grep -q '^openwindow>>[0-9a-f]*,[0-9]*,[^,][^,]*,[^,][^,]*$' "$T/events" || { cat "$T/events"; fail "openwindow lacks class/title"; }
grep -q '^focusedmon>>' "$T/events" || fail "no focusedmon event"
echo "ok: events configreloaded, openwindow, focusedmon"

# ---- extra requests ----------------------------------------------------------
"$CTL" -j devices | grep -q '"keyboards"' || fail "devices"
"$CTL" cursorpos | grep -Eq '^-?[0-9]+, -?[0-9]+$' || fail "cursorpos"
[ "$("$CTL" notify 1 1000 0 hello)" = ok ] || fail "notify"
[ "$("$CTL" switchxkblayout main next)" = ok ] || fail "switchxkblayout"
[ -n "$("$CTL" splash)" ] || fail "splash"
"$CTL" -j monitors | grep -q '"hardwareDetails"' || fail "monitors shape"
"$CTL" -r -j activeworkspace | grep -q '"type":"normal"' || fail "activeworkspace shape"
"$CTL" instances | grep -q "pid: $COMP" || fail "instances"
echo "ok: devices, cursorpos, notify, instances, ..."

# ---- windowtitle event (weston-terminal sets its title with an OSC escape) -----
cat > "$T/shell.sh" <<'SH'
#!/bin/sh
sleep 3
printf '\033]0;hypr-title-test\007'
sleep 30
SH
chmod +x "$T/shell.sh"
"$CTL" dispatch exec "weston-terminal --shell=$T/shell.sh" >/dev/null
for _ in $(seq 100); do
    grep -q '^windowtitlev2>>[0-9a-f]*,hypr-title-test$' "$T/events" && break
    sleep 0.2
done
grep -q '^windowtitlev2>>[0-9a-f]*,hypr-title-test$' "$T/events" || { cat "$T/events"; fail "no windowtitlev2 event"; }
grep -q '^windowtitle>>[0-9a-f]*$' "$T/events" || fail "no windowtitle event"
"$CTL" -j clients | grep -q '"title":"hypr-title-test"' || fail "clients lacks the new title"
echo "ok: windowtitle events"
