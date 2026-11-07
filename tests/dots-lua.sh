#!/bin/sh
# Popular hyprland.lua dotfiles (end-4, ML4W, HyDE) at pinned commits must
# load with zero config errors and run: --check-config, then a headless
# start (hyprland.start handlers, timers) with `hyprctl configerrors` empty.
# The dotfiles are cloned into a throwaway HOME (GPL, nothing vendored).
# Their Lua runs with the full stdlib, as on a real Hyprland: CI only.
# Needs: git, network access, python3.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
HYPRCTL=${HYPRCTL:-./target/debug/hyprctl}
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
HYPRCTL=$(cd "$(dirname "$HYPRCTL")" && pwd)/$(basename "$HYPRCTL")
T=$(mktemp -d)
COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    rm -rf "$T"
}
trap cleanup EXIT
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME DISPLAY WAYLAND_DISPLAY

fetch() { # repo sha dir
    git init -q "$3"
    git -C "$3" fetch -q --depth 1 "https://github.com/$1.git" "$2"
    git -C "$3" checkout -q FETCH_HEAD
}

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

fail=0
run() { # name home
    name=$1
    export HOME="$2"
    cfg="$HOME/.config/hypr/hyprland.lua"
    printf '== %s\n' "$name"
    if ! "$BIN" --check-config -c "$cfg"; then
        fail=1
        return
    fi
    run_dir="$T/run-$name"
    mkdir -p "$run_dir"
    chmod 700 "$run_dir"
    XDG_RUNTIME_DIR="$run_dir" HYPR_HEADLESS_SIZE=1920x1080 \
        "$BIN" --backend headless -c "$cfg" >"$run_dir/log" 2>&1 &
    COMP=$!
    errs=x
    for _ in $(seq 50); do
        errs=$(XDG_RUNTIME_DIR="$run_dir" timeout 5 "$HYPRCTL" -j configerrors 2>/dev/null \
            | json 'len([e for e in d if e])' 2>/dev/null) && break
        sleep 0.2
    done
    sleep 2 # start handlers and short timers
    errs=$(XDG_RUNTIME_DIR="$run_dir" timeout 5 "$HYPRCTL" -j configerrors | json 'len([e for e in d if e])') || errs=x
    if ! kill -0 "$COMP" 2>/dev/null; then
        echo "FAIL: $name: compositor died"
        tail -n 40 "$run_dir/log"
        fail=1
    elif [ "$errs" != 0 ]; then
        echo "FAIL: $name: $errs runtime config errors"
        XDG_RUNTIME_DIR="$run_dir" timeout 5 "$HYPRCTL" configerrors || true
        grep -i 'lua\|error' "$run_dir/log" | tail -n 20 || true
        fail=1
    else
        echo "ok: $name runs without errors"
    fi
    kill "$COMP" 2>/dev/null || true
    wait "$COMP" 2>/dev/null || true
    COMP=
}

fetch end-4/dots-hyprland 2f2a376fdb2ad738bf55f693dd0b474b64bb0ab8 "$T/end4"
mkdir -p "$T/home-end4" && cp -r "$T/end4/dots/." "$T/home-end4/"
run end-4 "$T/home-end4"

fetch mylinuxforwork/dotfiles 9ead14e78a9bff5f1ae7df3aec7a775730cf4431 "$T/ml4w"
mkdir -p "$T/home-ml4w" && cp -r "$T/ml4w/dotfiles/." "$T/home-ml4w/"
run ml4w "$T/home-ml4w"

fetch HyDE-Project/HyDE fa2fd57afc7203b1b5653d3271f3a57ea3873f97 "$T/hyde"
mkdir -p "$T/home-hyde" && cp -r "$T/hyde/Configs/." "$T/home-hyde/"
run hyde "$T/home-hyde"

[ "$fail" = 0 ] && echo "dots-lua: OK"
exit "$fail"
