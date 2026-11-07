#!/bin/sh
# Popular dotfiles must load with zero config errors (--check-config).
# Checks Hyprland's v0.54.3 example config (vendored in examples/, BSD-3) and
# JaKooLit/Hyprland-Dots at a pinned commit (cloned, nothing vendored; GPL).
# Needs: git and network access. Config files are only parsed, never run.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
BIN=$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")
SHA=c093e836764d62346390ba307c9163ca11d0229b
ROOT=$(pwd)
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
export HOME="$T/home"
unset XDG_CONFIG_HOME
mkdir -p "$HOME/.config"

fail=0
check() {
    printf '== %s\n' "$1"
    if ! "$BIN" --check-config -c "$1"; then
        fail=1
    fi
}

check "$ROOT/examples/hyprland-0.54.3-example.conf"

git init -q "$T/jk"
git -C "$T/jk" fetch -q --depth 1 https://github.com/JaKooLit/Hyprland-Dots.git "$SHA"
git -C "$T/jk" checkout -q FETCH_HEAD
cp -r "$T/jk/config/hypr" "$HOME/.config/hypr"
check "$HOME/.config/hypr/hyprland.conf"
# Rule syntaxes that the main file does not source at the same time.
for f in WindowRules-config-v3.conf WindowRules-pre-53.conf; do
    check "$HOME/.config/hypr/configs/$f"
done

[ "$fail" = 0 ] && echo "dots-conf: OK"
exit "$fail"
