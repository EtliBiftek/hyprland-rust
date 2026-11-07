//! Applies window rules (`crate::rules`) to windows: static effects once when
//! the window opens, dynamic ones whenever a property changes.

use std::cell::RefCell;

use smithay::{backend::renderer::utils::with_renderer_surface_state, desktop::Window};

use crate::{
    ipc::{class_title, window_addr},
    rules::{self, BorderColor, DynProps, Vars, WindowProps},
    selectors::{resolve_ws, Target, WinSel, WsSel},
    state::Hypr,
};

/// Per-window rule state, kept in the window's user data.
#[derive(Default)]
struct RuleState {
    /// Static rules were applied (the window "opened").
    applied: bool,
    initial_class: String,
    initial_title: String,
    /// Class and title seen at the last evaluation.
    last: (String, String),
    /// Tags set by `tagwindow` (without `*`).
    static_tags: Vec<String>,
    dynp: DynProps,
    /// `suppress_event` is stored; clients' requests are not filtered yet.
    #[allow(dead_code)]
    suppress_event: Vec<String>,
}

fn state(w: &Window) -> &RefCell<RuleState> {
    w.user_data().get_or_insert(RefCell::<RuleState>::default)
}

impl Hypr {
    fn rule_props(&self, w: &Window) -> WindowProps {
        let (class, title) = class_title(w);
        let s = state(w).borrow();
        let ws = self.ws_of(w).map(|i| &self.workspaces[i]);
        let (initial_class, initial_title) = if s.applied {
            (s.initial_class.clone(), s.initial_title.clone())
        } else {
            (class.clone(), title.clone())
        };
        WindowProps {
            initial_class,
            initial_title,
            class,
            title,
            xwayland: w.is_x11(),
            floating: ws.is_some_and(|ws| ws.is_floating(w)),
            fullscreen: ws.is_some_and(|ws| ws.fullscreen.as_ref() == Some(w)),
            pinned: self.wm.pinned.contains(w),
            focus: self.wm.focused.as_ref() == Some(w),
            workspace_id: ws.map_or(0, |ws| ws.id),
            workspace_name: ws.map(|ws| ws.name.clone()).unwrap_or_default(),
            tags: rules::all_tags(&s.static_tags, &s.dynp.tags),
            fullscreen_state: match ws {
                Some(ws) if ws.fullscreen.as_ref() == Some(w) => 2,
                Some(ws) if ws.maximized.as_ref() == Some(w) => 1,
                _ => 0,
            },
        }
    }

    /// Tags as shown by `hyprctl clients`.
    pub fn window_tags(&self, w: &Window) -> Vec<String> {
        let s = state(w).borrow();
        rules::all_tags(&s.static_tags, &s.dynp.tags)
    }

    /// Class and title at the time the window opened.
    pub fn window_initial(&self, w: &Window) -> (String, String) {
        let s = state(w).borrow();
        if s.applied {
            (s.initial_class.clone(), s.initial_title.clone())
        } else {
            class_title(w)
        }
    }

    /// Border size and colors forced by rules for this window.
    pub fn rule_border(&self, w: &Window) -> (Option<i32>, BorderColor) {
        let s = state(w).borrow();
        (s.dynp.border_size, s.dynp.border_color.unwrap_or_default())
    }

    /// Re-evaluates the dynamic effects of one window.
    pub fn refresh_window_rules(&mut self, w: &Window) {
        if !state(w).borrow().applied {
            return;
        }
        let mut p = self.rule_props(w);
        p.tags.retain(|t| !t.ends_with('*'));
        let dynp = rules::dynamic_props(&self.config.rules.window, &p);
        state(w).borrow_mut().dynp = dynp;
    }

    pub fn refresh_all_rules(&mut self) {
        let wins: Vec<Window> = self.workspaces.iter().flat_map(|ws| ws.windows()).collect();
        for w in wins {
            self.refresh_window_rules(&w);
        }
    }

    /// Focus changed: only rules with `match:focus` care.
    pub fn refresh_rules_on_focus(&mut self) {
        if self.config.rules.uses_focus() {
            self.refresh_all_rules();
        }
    }

    /// Called after every commit of a toplevel, before `openwindow` is sent.
    /// Applies the static rules once class/title are known (before the initial
    /// configure, or at the first buffer), then tracks class/title changes.
    pub fn rules_on_commit(&mut self, w: &Window) {
        let (class, title) = class_title(w);
        let (applied, changed) = {
            let s = state(w).borrow();
            (s.applied, s.last != (class.clone(), title.clone()))
        };
        if !applied {
            let has_buffer = crate::xwayland::wl_surface_of(w).is_some_and(|s| {
                with_renderer_surface_state(&s, |s| s.buffer().is_some()).unwrap_or(false)
            });
            // X11 windows have no initial configure: class is known at map.
            let before_configure = w.toplevel().is_none_or(|t| !t.is_initial_configure_sent());
            if (before_configure && !class.is_empty()) || has_buffer {
                self.apply_static_rules(w);
            }
        } else if changed {
            state(w).borrow_mut().last = (class, title);
            self.refresh_window_rules(w);
        }
    }

    /// xdg_toplevel title change.
    pub fn rules_on_title(&mut self, w: &Window) {
        let last = class_title(w);
        state(w).borrow_mut().last = last;
        self.refresh_window_rules(w);
    }

    fn apply_static_rules(&mut self, w: &Window) {
        let (class, title) = class_title(w);
        {
            let mut s = state(w).borrow_mut();
            s.applied = true;
            s.initial_class = class.clone();
            s.initial_title = title.clone();
            s.last = (class, title);
        }
        if self.config.rules.window.is_empty() {
            return;
        }
        // Dynamic effects first: they may add tags that static rules match.
        self.refresh_window_rules(w);
        let p = self.rule_props(w);
        let plan = rules::static_plan(&self.config.rules.window, &p);
        state(w).borrow_mut().suppress_event = plan.suppress_event.clone();
        self.run_plan(w, &plan);
    }

    fn run_plan(&mut self, w: &Window, plan: &rules::StaticPlan) {
        let mut refocus = plan.no_initial_focus;
        if let Some(f) = plan.float {
            self.set_floating(w, Some(f));
        }
        let Some(i) = self.ws_of(w) else { return };
        let floating = self.workspaces[i].is_floating(w);
        if plan.fullscreen {
            self.workspaces[i].fullscreen = Some(w.clone());
            self.ipc.emit("fullscreen", "1");
        } else if plan.maximize {
            self.workspaces[i].maximized = Some(w.clone());
        }
        if plan.pin && floating && !self.wm.pinned.contains(w) {
            self.wm.pinned.push(w.clone());
            let addr = window_addr(w);
            self.ipc
                .emit("pin", &format!("{},1", addr.trim_start_matches("0x")));
        }
        self.relayout();
        if floating && (plan.size.is_some() || plan.mv.is_some() || plan.center) {
            self.place_floating(w, plan);
        }
        if let Some((sel, silent)) = &plan.workspace {
            // shortcut: a floating window moved to a workspace that is not
            // shown loses its rule position and gets the default placement.
            refocus |= self.rule_move_to_workspace(w, sel, *silent);
        }
        if refocus {
            // Give focus back to what had it before the new window opened.
            let prev = self.wm.prev_focus.clone().filter(|p| {
                p != w
                    && self
                        .visible_ws()
                        .into_iter()
                        .any(|i| self.workspaces[i].contains(p))
            });
            self.focus_window(prev);
        }
    }

    /// `size`, `move` and `center` for a floating window on a visible workspace.
    fn place_floating(&mut self, w: &Window, plan: &rules::StaticPlan) {
        let Some(out) = self.output_geometry() else {
            return;
        };
        let g = w.geometry().size;
        let mut cur = if g.w > 0 && g.h > 0 {
            (g.w, g.h)
        } else {
            // Same default as `layout_ws`: 60% of the output.
            (out.size.w * 3 / 5, out.size.h * 3 / 5)
        };
        let cursor = self.seat.get_pointer().map_or((0.0, 0.0), |p| {
            let l = p.current_location();
            (l.x, l.y)
        });
        let mut vars = Vars {
            monitor_w: out.size.w as f64,
            monitor_h: out.size.h as f64,
            window_w: cur.0 as f64,
            window_h: cur.1 as f64,
            cursor_x: cursor.0 - out.loc.x as f64,
            cursor_y: cursor.1 - out.loc.y as f64,
            ..Default::default()
        };
        let eval =
            |e: &str, v: &Vars, base: f64| rules::eval_expr(e, v, base).map(|n| n.round() as i32);
        if let Some((sx, sy)) = &plan.size {
            match (
                eval(sx, &vars, vars.monitor_w),
                eval(sy, &vars, vars.monitor_h),
            ) {
                (Ok(x), Ok(y)) => {
                    cur = (x.max(20), y.max(20));
                    vars.window_w = cur.0 as f64;
                    vars.window_h = cur.1 as f64;
                    let at = self.space.element_location(w).unwrap_or((0, 0).into());
                    crate::xwayland::request_size(w, at, cur.into());
                }
                (a, b) => tracing::warn!("size rule: {:?} {:?}", a.err(), b.err()),
            }
        }
        // New floating windows are centered unless a rule says otherwise.
        let mut loc = ((out.size.w - cur.0) / 2, (out.size.h - cur.1) / 2);
        if let (Some((mx, my)), false) = (&plan.mv, plan.center) {
            if let (Ok(x), Ok(y)) = (
                eval(mx, &vars, vars.monitor_w),
                eval(my, &vars, vars.monitor_h),
            ) {
                loc = (x, y);
            }
        }
        let loc = (out.loc.x + loc.0, out.loc.y + loc.1);
        self.space.map_element(w.clone(), loc, false);
    }

    /// Moves a just-opened window to the workspace a rule names. Returns true
    /// when the window ended up on a workspace that is not shown.
    fn rule_move_to_workspace(&mut self, w: &Window, sel: &str, silent: bool) -> bool {
        let Ok(sel) = sel.parse::<WsSel>() else {
            return false;
        };
        let infos = self.ws_infos();
        let Some(target) = resolve_ws(&sel, self.active_id(), self.wm.previous_workspace, &infos)
        else {
            return false;
        };
        let (Some(dst), Some(src)) = (self.ensure_ws(&target), self.ws_of(w)) else {
            return false;
        };
        if dst == src {
            return false;
        }
        let area = self.tiling_area();
        let was_floating = self.workspaces[src].is_floating(w);
        self.workspaces[src].remove(w);
        self.wm.pinned.retain(|p| p != w);
        self.space.unmap_elem(w);
        let ws = &mut self.workspaces[dst];
        if was_floating {
            ws.floating.push(w.clone());
        } else {
            let anchor = ws.last_focused.clone();
            ws.tiled.insert(w.clone(), anchor.as_ref(), area);
        }
        ws.last_focused = Some(w.clone());
        self.relayout();
        let special = matches!(target, Target::Special(_));
        if !silent && !special {
            self.switch_to_index(dst);
            false
        } else {
            self.gc_workspaces();
            !self.visible_ws().contains(&dst)
        }
    }

    /// `tagwindow`: `+tag` adds, `-tag` removes, `tag` toggles a static tag.
    pub fn tag_window(&mut self, tag: &str, sel: Option<&WinSel>) {
        let w = match sel {
            Some(s) => self.find_window(s).map(|(_, w)| w),
            None => self.focused_window(),
        };
        let Some(w) = w else { return };
        let (op, name) = match tag.chars().next() {
            Some(c @ ('+' | '-')) => (Some(c), &tag[1..]),
            _ => (None, tag),
        };
        {
            let mut s = state(&w).borrow_mut();
            let at = s.static_tags.iter().position(|t| t == name);
            match (op, at) {
                (Some('+') | None, None) => s.static_tags.push(name.to_string()),
                (Some('-') | None, Some(i)) => {
                    s.static_tags.remove(i);
                }
                _ => {}
            }
        }
        self.refresh_window_rules(&w);
    }
}
