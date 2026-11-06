//! Window management: workspaces, tiling, focus and dispatchers.

use smithay::{
    desktop::Window,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
};

use crate::{
    config::Dispatcher,
    layout::{find_in_direction, Rect},
    state::Hypr,
};

fn to_rect(r: Rectangle<i32, Logical>) -> Rect {
    Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h)
}

fn from_rect(r: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((r.x, r.y).into(), (r.w, r.h).into())
}

fn wl_surface(w: &Window) -> Option<&WlSurface> {
    w.toplevel().map(|t| t.wl_surface())
}

impl Hypr {
    pub fn output_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.space.outputs().next()?;
        self.space.output_geometry(output)
    }

    /// The area tiled windows may use: the output minus the outer gaps.
    /// Window rects are later shrunk by `gaps_in`, so subtract that here to
    /// make the distance to the screen edge exactly `gaps_out`.
    pub fn tiling_area(&self) -> Rect {
        let g = &self.config.general;
        let out = self
            .output_geometry()
            .map(to_rect)
            .unwrap_or(Rect::new(0, 0, 1280, 800));
        out.shrink((g.gaps_out - g.gaps_in).max(0))
    }

    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<(usize, Window)> {
        self.workspaces.iter().enumerate().find_map(|(i, ws)| {
            ws.windows()
                .into_iter()
                .find(|w| wl_surface(w) == Some(surface))
                .map(|w| (i, w))
        })
    }

    pub fn focused_window(&self) -> Option<Window> {
        let ws = &self.workspaces[self.active_workspace];
        ws.last_focused.clone().filter(|w| ws.contains(w))
    }

    /// Called when a client creates a new toplevel window.
    pub fn add_window(&mut self, window: Window) {
        tracing::info!("window mapped on workspace {}", self.active_workspace + 1);
        let area = self.tiling_area();
        let target = self.focused_window();
        let (class, title) = crate::ipc::class_title(&window);
        let addr = crate::ipc::window_addr(&window);
        let ws_no = self.active_workspace + 1;
        self.ipc.emit(
            "openwindow",
            &format!("{},{ws_no},{class},{title}", addr.trim_start_matches("0x")),
        );
        let ws = &mut self.workspaces[self.active_workspace];
        ws.tiled.insert(window.clone(), target.as_ref(), area);
        self.relayout();
        self.focus_window(Some(window));
    }

    /// Called when a client destroys a toplevel window.
    pub fn remove_window(&mut self, surface: &WlSurface) {
        let Some((idx, window)) = self.window_for_surface(surface) else {
            return;
        };
        self.ipc.emit(
            "closewindow",
            crate::ipc::window_addr(&window).trim_start_matches("0x"),
        );
        self.workspaces[idx].remove(&window);
        self.space.unmap_elem(&window);
        if idx == self.active_workspace {
            self.relayout();
            let next = self.workspaces[idx].last_focused.clone();
            self.focus_window(next);
        }
    }

    /// Positions every window of the active workspace and sends configures.
    pub fn relayout(&mut self) {
        let area = self.tiling_area();
        let gaps_in = self.config.general.gaps_in;
        let output_geo = self.output_geometry();
        let ws = &mut self.workspaces[self.active_workspace];
        ws.tiled.preserve_split = self.config.dwindle.preserve_split;
        ws.tiled.split_ratio = self.config.dwindle.default_split_ratio;

        let mut placements: Vec<(Window, Rectangle<i32, Logical>, bool)> = ws
            .tiled
            .layout(area, gaps_in)
            .into_iter()
            .map(|(w, r)| (w, from_rect(r), false))
            .collect();

        for w in &ws.floating {
            let geo = self.space.element_geometry(w);
            let rect = match geo {
                Some(g) => g,
                None => {
                    // First time floating: center at 60% of the output.
                    let out = output_geo.unwrap_or(Rectangle::from_size((1280, 800).into()));
                    let size = (out.size.w * 3 / 5, out.size.h * 3 / 5);
                    let loc = (
                        out.loc.x + (out.size.w - size.0) / 2,
                        out.loc.y + (out.size.h - size.1) / 2,
                    );
                    Rectangle::new(loc.into(), size.into())
                }
            };
            placements.push((w.clone(), rect, false));
        }

        if let (Some(fs), Some(out)) = (ws.fullscreen.clone(), output_geo) {
            for p in placements.iter_mut() {
                if p.0 == fs {
                    *p = (fs.clone(), out, true);
                }
            }
        }

        let fullscreen = ws.fullscreen.clone();
        let floating = ws.floating.clone();
        for (window, rect, is_fullscreen) in placements {
            if let Some(toplevel) = window.toplevel() {
                let is_floating = floating.contains(&window);
                toplevel.with_pending_state(|state| {
                    // Floating windows pick their own size after the first configure.
                    if !is_floating || self.space.element_geometry(&window).is_none() {
                        state.size = Some(rect.size);
                    }
                    if is_fullscreen {
                        state.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        state.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                    for s in [
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        if is_floating {
                            state.states.unset(s);
                        } else {
                            state.states.set(s);
                        }
                    }
                });
                if toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
            // `Space` positions the window's geometry (it applies the client-side
            // decoration offset, e.g. shadows, itself).
            self.space.map_element(window.clone(), rect.loc, false);
        }

        // Keep floating windows above tiled ones, fullscreen above everything.
        for w in floating.iter().chain(fullscreen.iter()) {
            self.space.raise_element(w, false);
        }
    }

    /// Gives keyboard focus to `window` (or clears focus) and raises it.
    pub fn focus_window(&mut self, window: Option<Window>) {
        let serial = SERIAL_COUNTER.next_serial();
        let keyboard = self.seat.get_keyboard().unwrap();
        let ws = &mut self.workspaces[self.active_workspace];

        if let Some(w) = &window {
            ws.last_focused = Some(w.clone());
            if ws.is_floating(w) {
                // Move to top of the floating stack.
                ws.floating.retain(|f| f != w);
                ws.floating.push(w.clone());
                self.space.raise_element(w, true);
            }
        }
        for w in self.space.elements() {
            w.set_activated(Some(w) == window.as_ref());
            if let Some(t) = w.toplevel() {
                if t.is_initial_configure_sent() {
                    t.send_pending_configure();
                }
            }
        }
        keyboard.set_focus(self, window.as_ref().and_then(wl_surface).cloned(), serial);
        self.ipc.emit_active(window.as_ref());
    }

    pub fn switch_workspace(&mut self, number: usize) {
        let idx = number - 1;
        if idx == self.active_workspace {
            return;
        }
        for w in self.workspaces[self.active_workspace].windows() {
            self.space.unmap_elem(&w);
        }
        self.active_workspace = idx;
        self.ipc.emit("workspace", &number.to_string());
        self.ipc.emit("workspacev2", &format!("{number},{number}"));
        self.relayout();
        let focus = self.workspaces[idx].last_focused.clone();
        self.focus_window(focus);
    }

    pub fn move_to_workspace(&mut self, number: usize) {
        let idx = number - 1;
        let Some(window) = self.focused_window() else {
            return;
        };
        if idx == self.active_workspace {
            return;
        }
        let area = self.tiling_area();
        let was_floating = self.workspaces[self.active_workspace].is_floating(&window);
        self.workspaces[self.active_workspace].remove(&window);
        self.space.unmap_elem(&window);

        let addr = crate::ipc::window_addr(&window);
        let addr = addr.trim_start_matches("0x");
        self.ipc.emit("movewindow", &format!("{addr},{number}"));
        self.ipc
            .emit("movewindowv2", &format!("{addr},{number},{number}"));
        let target = &mut self.workspaces[idx];
        if was_floating {
            target.floating.push(window.clone());
        } else {
            let anchor = target.last_focused.clone();
            target.tiled.insert(window.clone(), anchor.as_ref(), area);
        }
        target.last_focused = Some(window);

        self.relayout();
        let next = self.workspaces[self.active_workspace].last_focused.clone();
        self.focus_window(next);
    }

    pub fn toggle_floating(&mut self) {
        let Some(window) = self.focused_window() else {
            return;
        };
        let area = self.tiling_area();
        let ws = &mut self.workspaces[self.active_workspace];
        if ws.is_floating(&window) {
            ws.floating.retain(|w| w != &window);
            ws.tiled.insert(window.clone(), None, area);
        } else {
            ws.tiled.remove(&window);
            ws.floating.push(window.clone());
            // Forget the tiled position so relayout centers it.
            self.space.unmap_elem(&window);
        }
        self.relayout();
        let floating = self.workspaces[self.active_workspace].is_floating(&window);
        let addr = crate::ipc::window_addr(&window);
        self.ipc.emit(
            "changefloatingmode",
            &format!("{},{}", addr.trim_start_matches("0x"), floating as u8),
        );
        self.focus_window(Some(window));
    }

    pub fn toggle_fullscreen(&mut self) {
        let Some(window) = self.focused_window() else {
            return;
        };
        let ws = &mut self.workspaces[self.active_workspace];
        ws.fullscreen = if ws.fullscreen.as_ref() == Some(&window) {
            None
        } else {
            Some(window)
        };
        self.relayout();
        let fs = self.workspaces[self.active_workspace].fullscreen.is_some();
        self.ipc.emit("fullscreen", &(fs as u8).to_string());
    }

    fn move_focus(&mut self, dir: crate::config::Direction) {
        let Some(from) = self.focused_window() else {
            return;
        };
        let rects = self.workspaces[self.active_workspace]
            .tiled
            .layout(self.tiling_area(), 0);
        if let Some(to) = find_in_direction(&rects, &from, dir) {
            self.focus_window(Some(to));
        }
    }

    fn cycle_next(&mut self) {
        let windows = self.workspaces[self.active_workspace].windows();
        if windows.is_empty() {
            return;
        }
        let next = match self
            .focused_window()
            .and_then(|f| windows.iter().position(|w| *w == f))
        {
            Some(i) => windows[(i + 1) % windows.len()].clone(),
            None => windows[0].clone(),
        };
        self.focus_window(Some(next));
    }

    pub fn dispatch(&mut self, action: &Dispatcher) {
        tracing::debug!(?action, "dispatch");
        match action {
            Dispatcher::Exec(cmd) => spawn(cmd),
            Dispatcher::KillActive => {
                if let Some(t) = self.focused_window().as_ref().and_then(|w| w.toplevel()) {
                    t.send_close();
                }
            }
            Dispatcher::Exit => self.loop_signal.stop(),
            Dispatcher::Workspace(n) => self.switch_workspace(*n),
            Dispatcher::MoveToWorkspace(n) => self.move_to_workspace(*n),
            Dispatcher::ToggleFloating => self.toggle_floating(),
            Dispatcher::Fullscreen => self.toggle_fullscreen(),
            Dispatcher::MoveFocus(dir) => self.move_focus(*dir),
            Dispatcher::CycleNext => self.cycle_next(),
            Dispatcher::ToggleSplit => {
                if let Some(w) = self.focused_window() {
                    let area = self.tiling_area();
                    self.workspaces[self.active_workspace]
                        .tiled
                        .toggle_split(&w, area);
                    self.relayout();
                }
            }
        }
    }

    /// Border rectangles (outside each window) with their colors, for rendering.
    pub fn borders(&self) -> Vec<(Rectangle<i32, Logical>, [f32; 4])> {
        let g = &self.config.general;
        let b = g.border_size;
        if b <= 0 {
            return Vec::new();
        }
        let ws = &self.workspaces[self.active_workspace];
        let focused = self.focused_window();
        let mut out = Vec::new();
        for w in ws.windows() {
            if ws.fullscreen.as_ref() == Some(&w) {
                continue;
            }
            let Some(loc) = self.space.element_location(&w) else {
                continue;
            };
            let r = Rectangle::new(loc, w.geometry().size);
            let color = if focused.as_ref() == Some(&w) {
                g.active_border_color
            } else {
                g.inactive_border_color
            };
            let (x, y, ww, hh) = (r.loc.x, r.loc.y, r.size.w, r.size.h);
            for rect in [
                Rectangle::new((x - b, y - b).into(), (ww + 2 * b, b).into()), // top
                Rectangle::new((x - b, y + hh).into(), (ww + 2 * b, b).into()), // bottom
                Rectangle::new((x - b, y).into(), (b, hh).into()),             // left
                Rectangle::new((x + ww, y).into(), (b, hh).into()),            // right
            ] {
                out.push((rect, color));
            }
        }
        out
    }

    /// Window under the pointer on the active workspace.
    pub fn window_under(&self, pos: Point<f64, Logical>) -> Option<Window> {
        self.space.element_under(pos).map(|(w, _)| w.clone())
    }
}

pub fn spawn(cmd: &str) {
    tracing::info!("exec: {cmd}");
    // Double fork: the inner `sh` backgrounds the command and exits at once, so
    // we reap it here and the program is re-parented to init (no zombies).
    match std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("eval \"$1\" &")
        .arg("sh")
        .arg(cmd)
        .spawn()
    {
        Ok(mut child) => {
            if let Err(e) = child.wait() {
                tracing::warn!("failed to reap `{cmd}`: {e}");
            }
        }
        Err(e) => tracing::warn!("failed to spawn `{cmd}`: {e}"),
    }
}

#[cfg(test)]
mod tests {
    /// Zombie children of this process, read from /proc.
    fn zombies() -> usize {
        let me = std::process::id().to_string();
        std::fs::read_dir("/proc")
            .unwrap()
            .filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("stat")).ok())
            .filter(|stat| {
                // Fields after the `(comm)`: state, ppid, ...
                let rest = stat.rsplit_once(") ").map_or("", |(_, r)| r);
                let mut fields = rest.split(' ');
                fields.next() == Some("Z") && fields.next() == Some(me.as_str())
            })
            .count()
    }

    #[test]
    fn spawn_leaves_no_zombies() {
        let marker = std::env::temp_dir().join(format!("hypr-spawn-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        for _ in 0..3 {
            super::spawn("true");
        }
        super::spawn(&format!("touch '{}'", marker.display()));
        for _ in 0..50 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists(), "spawned command did not run");
        let _ = std::fs::remove_file(&marker);
        assert_eq!(zombies(), 0);
    }
}
