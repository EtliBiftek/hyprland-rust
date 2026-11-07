//! Server bindings for the hyprland-protocols XMLs in `protocols/` (vendored from
//! github.com/hyprwm/hyprland-protocols v0.8.0, BSD-3, see
//! `protocols/LICENSE-hyprland-protocols`). Not in any crate, so they are generated here.

macro_rules! hypr_proto {
    ($name:ident, $xml:literal, [$($dep:path),*]) => {
        pub mod $name {
            pub mod server {
                #![allow(dead_code, non_camel_case_types, unused_unsafe, unused_imports)]
                #![allow(non_upper_case_globals, non_snake_case, missing_docs, clippy::all)]
                use wayland_server;
                use wayland_server::protocol::*;
                $(use $dep::{server::*};)*

                pub mod __interfaces {
                    use wayland_server::protocol::__interfaces::*;
                    $(use $dep::{server::__interfaces::*};)*
                    wayland_scanner::generate_interfaces!($xml);
                }
                use self::__interfaces::*;

                wayland_scanner::generate_server_code!($xml);
            }
        }
    };
}

hypr_proto!(
    global_shortcuts,
    "protocols/hyprland-global-shortcuts-v1.xml",
    []
);
hypr_proto!(focus_grab, "protocols/hyprland-focus-grab-v1.xml", []);
hypr_proto!(ctm_control, "protocols/hyprland-ctm-control-v1.xml", []);
hypr_proto!(surface, "protocols/hyprland-surface-v1.xml", []);
hypr_proto!(lock_notify, "protocols/hyprland-lock-notify-v1.xml", []);
hypr_proto!(
    toplevel_export,
    "protocols/hyprland-toplevel-export-v1.xml",
    [smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1]
);
hypr_proto!(
    toplevel_mapping,
    "protocols/hyprland-toplevel-mapping-v1.xml",
    [
        smithay::reexports::wayland_protocols::ext::foreign_toplevel_list::v1,
        smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1
    ]
);
