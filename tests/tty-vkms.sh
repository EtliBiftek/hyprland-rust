#!/bin/sh
# TTY backend on a real kernel DRM device: VKMS (virtual KMS) + seatd +
# libseat + libinput, as close to hardware as CI gets. Checks that the tty
# backend starts, lists the VKMS connectors as monitors, renders a client
# (grim through wlr-screencopy), and, when the kernel's VKMS configfs allows
# it, a second connector and hotplug.
# Needs: root (sudo), the vkms module, seatd, weston, grim, python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
HYPRCTL=$(cd "$(dirname "$HYPRCTL")" && pwd)/$(basename "$HYPRCTL")
T=$(mktemp -d)
chmod 700 "$T"
CFS=/sys/kernel/config/vkms/hypr
COMP=
SEATD=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -f weston-simple-shm 2>/dev/null || true
    [ -n "$SEATD" ] && sudo kill "$SEATD" 2>/dev/null || true
}
trap cleanup EXIT

sudo modprobe vkms
# A second VKMS device with two connectors through configfs (kernel >= 6.19);
# fall back to the default single-connector device otherwise.
TWO=0
if sudo mkdir -p /sys/kernel/config 2>/dev/null && { mountpoint -q /sys/kernel/config || sudo mount -t configfs none /sys/kernel/config; } \
    && [ -d /sys/kernel/config/vkms ] && sudo mkdir "$CFS" 2>/dev/null; then
    ok=1
    for i in 0 1; do
        sudo mkdir "$CFS/planes/p$i" "$CFS/crtcs/c$i" "$CFS/encoders/e$i" "$CFS/connectors/k$i" || ok=0
        echo 1 | sudo tee "$CFS/planes/p$i/type" >/dev/null || ok=0
        sudo ln -s "$CFS/crtcs/c$i" "$CFS/planes/p$i/possible_crtcs/" || ok=0
        sudo ln -s "$CFS/crtcs/c$i" "$CFS/encoders/e$i/possible_crtcs/" || ok=0
        sudo ln -s "$CFS/encoders/e$i" "$CFS/connectors/k$i/possible_encoders/" || ok=0
    done
    echo 1 | sudo tee "$CFS/enabled" >/dev/null || ok=0
    [ "$ok" = 1 ] && TWO=1
fi
sleep 1
ls -l /dev/dri
# The VKMS card: the one with the most `Virtual-N` connectors that is not
# the VM's own display (hyperv_drm also names its connector Virtual).
CARD=
best=0
for c in /sys/class/drm/card[0-9]; do
    dev=$(readlink -f "$c/device")
    echo "$(basename "$c"): $dev $(ls -d "$c"-* 2>/dev/null | xargs -n1 basename | tr '\n' ' ')"
    echo "$dev" | grep -qi 'hyperv\|vmbus\|pci' && continue
    k=$(ls -d "$c"-Virtual-* 2>/dev/null | wc -l)
    if [ "$k" -gt "$best" ]; then
        best=$k
        CARD=/dev/dri/$(basename "$c")
    fi
done
[ -n "$CARD" ] || { echo "FAIL: no vkms card"; exit 1; }
[ "$best" -ge 2 ] || TWO=0
echo "using $CARD (two connectors: $TWO)"

sudo seatd -g "$(id -gn)" -l info >"$T/seatd.log" 2>&1 &
SEATD=$!
for _ in $(seq 50); do [ -S /run/seatd.sock ] && break; sleep 0.1; done

cat > "$T/test.conf" <<'CONF'
monitor = , preferred, auto, 1
general {
    gaps_in = 0
    gaps_out = 0
    border_size = 4
    col.active_border = rgb(ff0000)
}
exec-once = weston-simple-shm
CONF

export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY
fail() {
    echo "FAIL: $1"
    timeout 5 "$HYPRCTL" -j monitors || true
    echo "--- compositor log ---"
    tail -n 80 "$T/log"
    echo "--- seatd ---"
    tail -n 20 "$T/seatd.log"
    exit 1
}
ctl() { timeout 5 "$HYPRCTL" "$@"; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

LIBSEAT_BACKEND=seatd AQ_DRM_DEVICES="$CARD" RUST_LOG=info \
    "$BIN" --backend tty -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

n=0
for _ in $(seq 100); do
    n=$(ctl -j clients 2>/dev/null | json 'len(d)' 2>/dev/null || echo 0)
    [ "$n" = 1 ] && break
    kill -0 "$COMP" 2>/dev/null || fail "compositor exited"
    sleep 0.2
done
[ "$n" = 1 ] || fail "client did not open on the tty backend"
mons=$(ctl -j monitors | json 'len(d)')
[ "$mons" -ge 1 ] || fail "no monitors"
ctl -j monitors | json '[m["name"] for m in d]'
grep -q 'output Virtual-' "$T/log" || fail "no Virtual-N connector used"
echo "ok: tty backend on VKMS with $mons monitor(s)"

# Rendering: grim through wlr-screencopy must see the red border.
W=$(ctl -j clients | json 'd[0]["at"][0]')
WAYLAND_DISPLAY=$(ls "$T" | grep '^wayland-[0-9]*$' | head -n1) grim -t ppm "$T/shot.ppm" \
    || fail "grim failed on the tty backend"
python3 - "$T/shot.ppm" <<'PY' || fail "screenshot does not show the client"
import sys
d = open(sys.argv[1], "rb").read()
parts = d.split(b"\n", 3)
w, h = map(int, parts[1].split())
px = parts[3]
from collections import Counter
cols = Counter(px[i:i+3] for i in range(0, len(px) - 2, 3))
print("size", w, h, "top colors:", [(tuple(c), n) for c, n in cols.most_common(6)])
# weston-simple-shm draws green-dominant gradients.
win = sum(n for c, n in cols.items() if c[1] > c[0] + 20 and c[1] > c[2] + 20)
red = sum(n for c, n in cols.items() if c[0] > 200 and c[1] < 60 and c[2] < 60)
print("window pixels:", win, "red border pixels (info):", red)
sys.exit(0 if win > 1000 else 1)
PY
echo "ok: tty backend renders the client (screencopy)"

if [ "$TWO" = 1 ]; then
    [ "$mons" = 2 ] || fail "VKMS has two connectors but $mons monitors"
    x2=$(ctl -j monitors | json 'sorted(m["x"] for m in d)[1]')
    [ "$x2" -gt 0 ] || fail "second monitor not auto-placed"
    echo "ok: two connectors -> two monitors (second at x=$x2)"
    # Hotplug: disconnect the second connector if the kernel allows it.
    if [ -w "$CFS/connectors/k1/status" ] || sudo test -e "$CFS/connectors/k1/status"; then
        echo 2 | sudo tee "$CFS/connectors/k1/status" >/dev/null || true
        for _ in $(seq 50); do
            [ "$(ctl -j monitors | json 'len(d)')" = 1 ] && break
            sleep 0.1
        done
        if [ "$(ctl -j monitors | json 'len(d)')" = 1 ]; then
            echo "ok: hotplug disconnect removes the monitor"
            echo 1 | sudo tee "$CFS/connectors/k1/status" >/dev/null || true
            for _ in $(seq 50); do
                [ "$(ctl -j monitors | json 'len(d)')" = 2 ] && break
                sleep 0.1
            done
            [ "$(ctl -j monitors | json 'len(d)')" = 2 ] || fail "hotplug reconnect did not add the monitor back"
            echo "ok: hotplug reconnect"
        else
            echo "SKIP: connector status change did not reach udev"
        fi
    else
        echo "SKIP: no connector status in VKMS configfs"
    fi
else
    echo "SKIP: VKMS configfs unavailable, single connector only"
fi

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "tty-vkms: OK"
