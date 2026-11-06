#!/bin/sh
# Keybinding test: runs the compositor (winit backend) inside Xvfb, presses
# keys with xdotool (XTEST) and checks that the binds fire.
# Needs: Xvfb, xdotool, libxkbcommon-x11-0, Mesa (llvmpipe).
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
input {
    repeat_delay = 200
    repeat_rate = 20
    follow_mouse = 1
}
bind = SUPER, Return, exec, touch $T/exec-fired
bind = SUPER SHIFT, E, exit,
bind = SUPER, 2, workspace, 2
bind = SUPER, N, workspace, +1
bind = SUPER, W, workspace, name:web
bind = SUPER, P, workspace, previous
bindr = SUPER, R, exec, touch $T/release-fired
binde = SUPER, K, exec, echo x >> $T/repeat
bindd = SUPER, D, Test description, exec, true
bind = SUPER, S, submap, test
submap = test
bind = , A, exec, touch $T/submap-fired
bind = , escape, submap, reset
submap = reset
CONF

Xvfb :99 -screen 0 1280x800x24 >/dev/null 2>&1 &
XVFB=$!
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    kill "$XVFB" 2>/dev/null || true
}
trap cleanup EXIT
export DISPLAY=:99 XDG_RUNTIME_DIR="$T" WAYLAND_DISPLAY=
sleep 1

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    cat "$T/log"
    exit 1
}

RUST_LOG=debug "$BIN" --backend winit -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!

WIN=
for _ in $(seq 50); do
    WIN=$(xdotool search --pid "$COMP" 2>/dev/null | head -n1 || true)
    [ -n "$WIN" ] && break
    sleep 0.2
done
[ -n "$WIN" ] || fail "compositor window did not appear"
xdotool windowfocus --sync "$WIN"
xdotool mousemove --window "$WIN" 100 100
sleep 1

xdotool key super+Return
for _ in $(seq 25); do
    [ -e "$T/exec-fired" ] && break
    sleep 0.2
done
[ -e "$T/exec-fired" ] || fail "SUPER+Return did not run exec"
echo "ok: SUPER+Return -> exec"

# --- workspace selectors, bind flags and submaps, checked through hyprctl ---
CTL=${CTL:-./target/debug/hyprctl}
unset HYPRLAND_INSTANCE_SIGNATURE
for _ in $(seq 25); do
    "$CTL" -j activeworkspace >/dev/null 2>&1 && break
    sleep 0.2
done

ws_is() { # $1 = expected JSON fragment of activeworkspace
    for _ in $(seq 25); do
        "$CTL" -j activeworkspace 2>/dev/null | grep -q "$1" && return 0
        sleep 0.2
    done
    return 1
}

"$CTL" -j activeworkspace | grep -q '"id":1' || fail "initial workspace is not 1"
xdotool key super+2
ws_is '"id":2' || fail "SUPER+2 did not select workspace 2"
echo "ok: workspace 2 (id selector)"
xdotool key super+n
ws_is '"id":3' || fail "workspace +1 did not select workspace 3"
echo "ok: workspace +1 (relative selector)"
xdotool key super+w
ws_is '"name":"web"' || fail "workspace name:web did not select the named workspace"
echo "ok: workspace name:web"
xdotool key super+p
ws_is '"id":3' || fail "workspace previous did not return to 3"
echo "ok: workspace previous"

xdotool key super+r
for _ in $(seq 25); do
    [ -e "$T/release-fired" ] && break
    sleep 0.2
done
[ -e "$T/release-fired" ] || fail "bindr did not fire on release"
echo "ok: bindr fires on release"

xdotool keydown super keydown k
sleep 1.5
xdotool keyup k keyup super
sleep 0.3
N1=$(wc -l < "$T/repeat" 2>/dev/null || echo 0)
[ "$N1" -ge 3 ] || fail "binde fired $N1 times, expected a repeat (>= 3)"
sleep 0.6
N2=$(wc -l < "$T/repeat")
[ "$N1" = "$N2" ] || fail "binde kept repeating after release ($N1 -> $N2)"
echo "ok: binde repeats while held ($N1 times) and stops on release"

"$CTL" -j binds | grep -q '"description":"Test description"' || fail "bindd description missing from binds"

xdotool key a
sleep 0.5
[ ! -e "$T/submap-fired" ] || fail "submap bind fired outside its submap"
xdotool key super+s
sleep 0.5
xdotool key a
for _ in $(seq 25); do
    [ -e "$T/submap-fired" ] && break
    sleep 0.2
done
[ -e "$T/submap-fired" ] || fail "submap bind did not fire inside the submap"
rm -f "$T/submap-fired"
xdotool key Escape
sleep 0.5
xdotool key a
sleep 0.5
[ ! -e "$T/submap-fired" ] || fail "submap bind still active after submap reset"
echo "ok: submap enter, bind inside, reset"

xdotool key super+shift+e
for _ in $(seq 25); do
    kill -0 "$COMP" 2>/dev/null || break
    sleep 0.2
done
kill -0 "$COMP" 2>/dev/null && fail "SUPER+SHIFT+E did not exit"
wait "$COMP" || fail "compositor exited with an error"
COMP=
echo "ok: SUPER+SHIFT+E -> exit"
