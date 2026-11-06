#!/bin/sh
# IPC test: runs the compositor (winit backend) inside Xvfb with one Wayland
# client and checks hyprctl against .socket.sock plus events on .socket2.sock.
# Needs: Xvfb, weston (weston-simple-shm), python3, libxkbcommon-x11-0, Mesa.
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
    N=$("$CTL" -j clients 2>/dev/null | grep -o '"address"' | wc -l || true)
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
end = time.time() + 15
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
[ "$R" = okok ] || fail "batch replied '$R'"
"$CTL" -j activeworkspace | grep -q '"id":3' || fail "batch did not reach workspace 3"
echo "ok: batch"
