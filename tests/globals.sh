#!/bin/sh
# Global registry test: wayland-info against the headless backend must list every
# protocol added in protocols2 with the right version, wlsunset must run against
# zwlr_gamma_control, and wtype + wev must exercise virtual-keyboard.
# Needs: wayland-utils (wayland-info), wlsunset, wtype, wev.
set -eu
BIN=${BIN:-./target/debug/hyprland-rust}
CTL=${CTL:-./target/debug/hyprctl}
T=$(mktemp -d)
chmod 700 "$T"
echo "# empty" > "$T/test.conf"

COMP=
cleanup() {
    [ -n "$COMP" ] && kill "$COMP" 2>/dev/null || true
    pkill -x wev 2>/dev/null || true
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$T"
unset DISPLAY WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE

fail() {
    echo "FAIL: $1"
    echo "--- compositor log ---"
    tail -n 60 "$T/log"
    exit 1
}

HYPR_HEADLESS_SIZE=800x600 "$BIN" --backend headless -c "$T/test.conf" >"$T/log" 2>&1 &
COMP=$!
WL=
for _ in $(seq 50); do
    WL=$(ls "$T" 2>/dev/null | grep -E '^wayland-[0-9]+$' | head -n 1 || true)
    [ -n "$WL" ] && break
    sleep 0.2
done
[ -n "$WL" ] || fail "no wayland socket"
export WAYLAND_DISPLAY="$WL"

wayland-info > "$T/info" 2>&1 || fail "wayland-info failed"
check() { # name version|any
    if [ "$2" = any ]; then
        grep -Eq "interface: '$1',[[:space:]]+version:[[:space:]]+[0-9]+" "$T/info" || { cat "$T/info"; fail "global $1 missing"; }
    else
        grep -Eq "interface: '$1',[[:space:]]+version:[[:space:]]+$2," "$T/info" || { cat "$T/info"; fail "global $1 v$2 missing"; }
    fi
}
check hyprland_focus_grab_manager_v1 1
check hyprland_toplevel_export_manager_v1 2
check hyprland_ctm_control_manager_v1 2
check hyprland_surface_manager_v1 2
check hyprland_lock_notifier_v1 1
check hyprland_toplevel_mapping_manager_v1 1
check hyprland_global_shortcuts_manager_v1 1
check zwlr_gamma_control_manager_v1 1
check ext_output_image_capture_source_manager_v1 1
check ext_foreign_toplevel_image_capture_source_manager_v1 1
check ext_image_copy_capture_manager_v1 1
check zwp_linux_dmabuf_v1 3
check xdg_wm_dialog_v1 1
check xdg_toplevel_icon_manager_v1 1
check wp_alpha_modifier_v1 1
check wp_tearing_control_manager_v1 1
check xdg_system_bell_v1 1
check wp_fifo_manager_v1 any
check wp_commit_timing_manager_v1 any
check wp_security_context_manager_v1 any
check zwp_tablet_manager_v2 any
check zwp_text_input_manager_v3 any
check zwp_input_method_manager_v2 any
check zwp_virtual_keyboard_manager_v1 any
check zwlr_virtual_pointer_manager_v1 any
check ext_workspace_manager_v1 any
echo "all globals present"

# wlsunset must stay alive (killed by timeout = 124) with the gamma global.
rc=0
timeout 4 wlsunset -S 06:00 -s 18:00 -t 4000 -T 6500 >"$T/wlsunset.log" 2>&1 || rc=$?
[ "$rc" = 124 ] || { cat "$T/wlsunset.log"; fail "wlsunset exited early (rc=$rc)"; }

# wtype types into wev, which gets keyboard focus when its window maps.
wev >"$T/wev.out" 2>&1 &
N=0
for _ in $(seq 50); do
    N=$("$CTL" -j clients 2>/dev/null | grep -o '"stableId"' | wc -l || true)
    [ "$N" = 1 ] && break
    sleep 0.2
done
[ "$N" = 1 ] || fail "wev window did not map"
sleep 1
wtype hello || fail "wtype failed"
sleep 1
grep -q "sym: h " "$T/wev.out" || { cat "$T/wev.out"; fail "wev did not receive the typed keys"; }
kill -0 "$COMP" 2>/dev/null || fail "compositor died"
echo "OK"
