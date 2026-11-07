#!/bin/sh
# Session environment: XDG_CURRENT_DESKTOP / XDG_SESSION_TYPE /
# XDG_SESSION_DESKTOP and `env = KEY,VALUE` reach exec-once commands.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/test.conf" <<CONF
env = HYPR_TEST_VAR,hello world
exec-once = sh -c 'echo "\$XDG_CURRENT_DESKTOP \$XDG_SESSION_TYPE \$XDG_SESSION_DESKTOP \$HYPR_TEST_VAR" > $T/env.txt'
CONF
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY XDG_CURRENT_DESKTOP
"$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
trap 'kill $COMP 2>/dev/null || true' EXIT
for _ in $(seq 50); do [ -s "$T/env.txt" ] && break; sleep 0.1; done
got=$(cat "$T/env.txt" 2>/dev/null || true)
[ "$got" = "Hyprland wayland Hyprland hello world" ] || { echo "FAIL: got '$got'"; tail -20 "$T/log"; exit 1; }
echo "session-env: OK"
