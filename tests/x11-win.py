#!/usr/bin/env python3
"""Opens one X11 window for tests/xwayland.sh and runs until it is closed.

usage: x11-win.py MODE CLASS
MODE: normal | dialog (_NET_WM_WINDOW_TYPE_DIALOG) | fixed (min size == max
size) | or (override-redirect, like a menu) | fullscreen (asks for
_NET_WM_STATE_FULLSCREEN after mapping)
Exits on WM_DELETE_WINDOW. Needs python3-xlib.
"""
import sys
import time

from Xlib import X, Xatom, Xutil, display
from Xlib.protocol import event

mode, cls = sys.argv[1], sys.argv[2]
d = display.Display()
s = d.screen()
win = s.root.create_window(
    100, 100, 300, 200, 0, s.root_depth, X.InputOutput, X.CopyFromParent,
    background_pixel=s.white_pixel,
    override_redirect=1 if mode == "or" else 0,
    event_mask=X.StructureNotifyMask | X.ExposureMask,
)
atom = d.intern_atom
win.set_wm_name(cls)
win.set_wm_class(cls, cls)
if mode == "dialog":
    win.change_property(atom("_NET_WM_WINDOW_TYPE"), Xatom.ATOM, 32,
                        [atom("_NET_WM_WINDOW_TYPE_DIALOG")])
if mode == "fixed":
    win.set_wm_normal_hints(flags=Xutil.PMinSize | Xutil.PMaxSize,
                            min_width=300, min_height=200,
                            max_width=300, max_height=200)
delete = atom("WM_DELETE_WINDOW")
win.change_property(atom("WM_PROTOCOLS"), Xatom.ATOM, 32, [delete])
gc = win.create_gc(foreground=s.black_pixel)
win.map()
d.sync()

if mode == "fullscreen":
    time.sleep(1)
    ev = event.ClientMessage(
        window=win, client_type=atom("_NET_WM_STATE"),
        data=(32, [1, atom("_NET_WM_STATE_FULLSCREEN"), 0, 1, 0]))
    s.root.send_event(
        ev, event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
    d.sync()

while True:
    e = d.next_event()
    if e.type == X.Expose:
        g = win.get_geometry()
        win.fill_rectangle(gc, 0, 0, max(g.width // 2, 1), max(g.height // 2, 1))
        d.flush()
    elif e.type == X.ClientMessage and e.client_type == atom("WM_PROTOCOLS"):
        if e.data[1][0] == delete:
            sys.exit(0)
