#!/bin/sh
# Window rules test: headless backend, rules added with `hyprctl keyword`
# (canonical and legacy syntax), windows opened with `hyprctl dispatch exec`.
# Checks float + size + center, `workspace N silent`, class/title regex
# matchers (also negative:) and dynamic tags through `hyprctl -j clients`.
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
    ps -o pid,stat,pcpu,wchan:20,args -p "$COMP" 2>&1 || true
    sudo gdb -p "$COMP" -batch -ex "thread apply all bt 30" 2>&1 | grep -E "^(#|Thread)" | cut -c1-200 || true
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

# Gives the client time to resize after the configure.
settle() { sleep 1.5; }

kw() {
    r=$(timeout 5 "$HYPRCTL" keyword "$@") || fail "hyprctl keyword $* failed"
    [ "$r" = ok ] || fail "keyword $*: $r"
}

HYPR_HEADLESS_SIZE=1000x800 RUST_LOG=debug "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
for _ in $(seq 50); do
    clients >/dev/null 2>&1 && break
    sleep 0.2
done

# 1. Probe: learn the class and title of weston-simple-shm.
timeout 5 "$HYPRCTL" dispatch exec weston-simple-shm >/dev/null || fail "dispatch exec"
wait_clients 1
settle
clients >"$T/probe.json"
eval "$(python3 - "$T/probe.json" <<'PY'
import json, re, sys
c = json.load(open(sys.argv[1]))[0]
esc = lambda s: re.sub(r'([.^$*+?()\[\]{}|\\])', r'\\\1', s)
print(f"CLASS='{esc(c['class'])}'; TITLE='{esc(c['title'])}'")
PY
)"
echo "probe: class=[$CLASS] title=[$TITLE]"
[ -n "$CLASS$TITLE" ] || fail "weston-simple-shm has neither app_id nor title"
pkill -f weston-simple-shm || true
wait_clients 0

# 2. float + size + center, legacy tag rule, title matcher, negative matcher.
kw windowrule "match:class $CLASS, float on, size 400 300, center on"
kw windowrulev2 "tag +probe, class:^($CLASS)\$"
kw windowrule "match:title $TITLE, tag +bytitle"
kw windowrule "match:class negative:$CLASS, tag +neg, workspace 7"
kw windowrule "match:class ^zzz\$, workspace 6"
timeout 5 "$HYPRCTL" dispatch exec weston-simple-shm >/dev/null
wait_clients 1
settle
clients >"$T/c1.json"
python3 - "$T/c1.json" <<'PY' || fail "rule 1 not applied: $(cat "$T/c1.json")"
import json, sys
(w,) = json.load(open(sys.argv[1]))
assert w["floating"], "not floating"
sx, sy = w["size"]
assert abs(sx - 400) <= 2 and abs(sy - 300) <= 2, f"size {w['size']}"
x, y = w["at"]
assert abs(x - (1000 - sx) // 2) <= 2 and abs(y - (800 - sy) // 2) <= 2, f"at {w['at']}"
assert "probe*" in w["tags"], w["tags"]
assert "bytitle*" in w["tags"], w["tags"]
assert "neg*" not in w["tags"], w["tags"]
assert w["workspace"]["id"] == 1, w["workspace"]
print("float/size/center/tags ok:", w["at"], w["size"], w["tags"])
PY
pkill -f weston-simple-shm || true
wait_clients 0

# 3. workspace 3 silent: the window opens on workspace 3, focus stays.
kw windowrule "match:class $CLASS, workspace 3 silent"
timeout 5 "$HYPRCTL" dispatch exec weston-simple-shm >/dev/null
wait_clients 1
settle
clients >"$T/c2.json"
python3 - "$T/c2.json" <<'PY' || fail "workspace rule not applied: $(cat "$T/c2.json")"
import json, sys
(w,) = json.load(open(sys.argv[1]))
assert w["workspace"]["id"] == 3, w["workspace"]
assert not w["visible"], "window should not be visible (silent)"
print("workspace 3 silent ok")
PY

# 4. tagwindow dispatcher: static tag shows up without `*`.
timeout 5 "$HYPRCTL" dispatch tagwindow "+manual" >/dev/null || fail "tagwindow"
timeout 5 "$HYPRCTL" dispatch tagwindow "+manual2 class:$CLASS" >/dev/null || fail "tagwindow with selector"
clients >"$T/c3.json"
python3 - "$T/c3.json" <<'PY' || fail "tagwindow tags missing: $(cat "$T/c3.json")"
import json, sys
(w,) = json.load(open(sys.argv[1]))
assert "manual2" in w["tags"], w["tags"]
print("tagwindow ok:", w["tags"])
PY

kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK: window rules (float/size/center, workspace silent, regex matchers, tags)"
