#!/bin/sh
# Lua config test (hyprland.lua, Hyprland >= 0.55 format): headless backend
# started with a .lua config. Checks hl.config options, hl.bind, a
# hl.window_rule, hl.on("hyprland.start"), `hyprctl dispatch` with a Lua
# expression, the live runtime (function binds with hl.dispatch, hl.get_*
# and runtime hl.config, hl.on("window.open"), hl.timer, `hyprctl eval`) and
# that a bad hl.* call is reported in configerrors.
# Needs: weston (weston-simple-shm), python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
cat > "$T/hyprland.lua" <<'LUA'
local mod = "SUPER"
local dir = os.getenv("XDG_RUNTIME_DIR")
hl.config({
    general = { gaps_in = 7, gaps_out = 11, border_size = 3 },
    input = { kb_layout = "tr", repeat_rate = 30 },
})
hl.bind(mod .. " + Q", hl.dsp.exec_cmd("true"))
hl.bind(mod .. " + SHIFT + 2", hl.dsp.window.move({ workspace = 2 }))
-- callback 0: runs inside the compositor when pressed (or dispatched).
hl.bind(mod .. " + Z", function()
    local ws = hl.get_active_workspace()
    hl.dispatch(hl.dsp.focus({ workspace = ws.id + 4 }))
    hl.config({ general = { gaps_in = 9 } })
end)
hl.window_rule({ name = "float-all", match = { class = ".*" }, float = true })
hl.on("hyprland.start", function()
    hl.exec_cmd("weston-simple-shm")
end)
hl.on("window.open", function(w)
    hl.exec_cmd("touch " .. dir .. "/opened-" .. tostring(w.floating))
end)
hl.timer(function() hl.exec_cmd("touch " .. dir .. "/timer") end, { timeout = 300, type = "oneshot" })
hl.timer(1000, function() end)
LUA

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

ctl() { timeout 5 "$HYPRCTL" "$@"; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
wait_file() {
    for _ in $(seq 50); do
        [ -e "$1" ] && return 0
        sleep 0.1
    done
    return 1
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/hyprland.lua" >"$T/log" 2>&1 &
COMP=$!

n=0
for _ in $(seq 50); do
    n=$(ctl -j clients 2>/dev/null | json 'len(d)' 2>/dev/null || echo 0)
    [ "$n" = 1 ] && break
    sleep 0.2
done
[ "$n" = 1 ] || fail "hl.on(\"hyprland.start\") did not open a window"
echo "ok: hl.on hyprland.start"

[ "$(ctl -j getoption general:gaps_in | json 'd["int"]')" = 7 ] || fail "general:gaps_in != 7"
[ "$(ctl -j getoption general:border_size | json 'd["int"]')" = 3 ] || fail "general:border_size != 3"
echo "ok: hl.config options"

ctl -j binds | json '[b["key"] for b in d]' | grep -q "'Q'\|'q'" || fail "SUPER+Q bind missing"
echo "ok: hl.bind"

[ "$(ctl -j clients | json 'd[0]["floating"]')" = True ] || fail "window_rule float not applied"
echo "ok: hl.window_rule"

wait_file "$T/opened-true" || fail "hl.on(\"window.open\") handler did not run with the window object"
echo "ok: hl.on window.open"

wait_file "$T/timer" || fail "hl.timer callback did not run"
echo "ok: hl.timer"

r=$(ctl dispatch 'hl.dsp.focus({ workspace = 3 })')
[ "$r" = ok ] || fail "Lua dispatch replied: $r"
[ "$(ctl -j activeworkspace | json 'd["id"]')" = 3 ] || fail "Lua dispatch did not switch to workspace 3"
echo "ok: hyprctl dispatch 'hl.dsp.focus({ workspace = 3 })'"

# The SUPER+Z function bind, dispatched as its hyprlang form.
r=$(ctl dispatch __lua 0)
[ "$r" = ok ] || fail "dispatch __lua 0 replied: $r"
[ "$(ctl -j activeworkspace | json 'd["id"]')" = 7 ] || fail "function bind did not switch 3 -> 7"
[ "$(ctl -j getoption general:gaps_in | json 'd["int"]')" = 9 ] || fail "hl.config in a callback did not apply"
echo "ok: function bind with hl.get_active_workspace, hl.dispatch, hl.config"

[ "$(ctl eval 'return hl.get_active_workspace().id')" = 7 ] || fail "hyprctl eval in the config state"
echo "ok: hyprctl eval"

ctl configerrors | grep -q "hl.timer: expected (function" || fail "bad hl.timer call not reported"
[ "$(ctl -j status | json "d[\"configProvider\"]")" = lua ] || fail "status configProvider != lua"
ctl systeminfo | grep -q "configProvider: lua" || fail "systeminfo configProvider"
echo "ok: configerrors reports bad hl.* calls"

timeout 5 "$HYPRCTL" dispatch 'hl.config({ cursor = { no_warps = true } })' | grep -qx ok || fail "dispatch hl.config"
[ "$(ctl -j getoption cursor:no_warps | json 'd["bool"]')" = True ] || fail "dispatch hl.config did not apply"
echo "ok: dispatch hl.config"

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
