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

# end-4 `m3ww` (EWW + hyprlang, OsmiumOS), archive branch at a pinned commit.
M3WW=2433298bddf6b1597b8e10dba67d26891664db11
git init -q "$T/m3"
git -C "$T/m3" fetch -q --depth 1 https://github.com/end-4/dots-hyprland.git "$M3WW"
git -C "$T/m3" checkout -q FETCH_HEAD
export HOME="$T/home-m3ww"
mkdir -p "$HOME/.config"
cp -r "$T/m3/m3ww/.config/hypr" "$HOME/.config/hypr"
check "$HOME/.config/hypr/hyprland.conf"

[ "$fail" = 0 ] && echo "dots-conf: OK"
exit "$fail"
