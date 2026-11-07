// Tiny Wayland test client for tests/render-fx.sh: one xdg toplevel filled with
// a solid color or an 8px black/white checkerboard.
// usage: fx-client APP_ID solid|checker AARRGGBB WIDTH HEIGHT
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "xdg-shell-client-protocol.h"

static struct wl_compositor *comp;
static struct wl_shm *shm;
static struct xdg_wm_base *wm;
static struct wl_surface *surf;
static int W = 400, H = 300, checker = 0;
static uint32_t argb = 0xff00ff00;

static void reg_global(void *d, struct wl_registry *r, uint32_t name, const char *iface, uint32_t ver) {
    (void)d; (void)ver;
    if (!strcmp(iface, "wl_compositor"))
        comp = wl_registry_bind(r, name, &wl_compositor_interface, 4);
    else if (!strcmp(iface, "wl_shm"))
        shm = wl_registry_bind(r, name, &wl_shm_interface, 1);
    else if (!strcmp(iface, "xdg_wm_base"))
        wm = wl_registry_bind(r, name, &xdg_wm_base_interface, 1);
}
static void reg_remove(void *d, struct wl_registry *r, uint32_t n) { (void)d; (void)r; (void)n; }
static const struct wl_registry_listener rl = {reg_global, reg_remove};

static void wm_ping(void *d, struct xdg_wm_base *b, uint32_t s) { (void)d; xdg_wm_base_pong(b, s); }
static const struct xdg_wm_base_listener wml = {wm_ping};

static void draw(void) {
    int stride = W * 4, size = stride * H;
    int fd = memfd_create("fx", 0);
    if (fd < 0 || ftruncate(fd, size) < 0) exit(1);
    uint32_t *p = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    uint32_t a = argb >> 24, r = (argb >> 16) & 255, g = (argb >> 8) & 255, b = argb & 255;
    uint32_t solid = (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255);
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++)
            p[y * W + x] = checker ? (((x / 8 + y / 8) & 1) ? 0xffffffffu : 0xff000000u) : solid;
    struct wl_shm_pool *pool = wl_shm_create_pool(shm, fd, size);
    struct wl_buffer *buf = wl_shm_pool_create_buffer(pool, 0, W, H, stride, WL_SHM_FORMAT_ARGB8888);
    wl_shm_pool_destroy(pool);
    close(fd);
    munmap(p, size);
    wl_surface_attach(surf, buf, 0, 0);
    wl_surface_damage(surf, 0, 0, W, H);
    wl_surface_commit(surf);
}

static void xs_configure(void *d, struct xdg_surface *x, uint32_t serial) {
    (void)d;
    xdg_surface_ack_configure(x, serial);
    draw();
}
static const struct xdg_surface_listener xsl = {xs_configure};
static void tl_configure(void *d, struct xdg_toplevel *t, int32_t w, int32_t h, struct wl_array *s) {
    (void)d; (void)t; (void)w; (void)h; (void)s;
}
static void tl_close(void *d, struct xdg_toplevel *t) { (void)d; (void)t; exit(0); }
static const struct xdg_toplevel_listener tll = {tl_configure, tl_close, NULL, NULL};

int main(int argc, char **argv) {
    if (argc < 6) return 2;
    checker = !strcmp(argv[2], "checker");
    argb = (uint32_t)strtoul(argv[3], NULL, 16);
    W = atoi(argv[4]);
    H = atoi(argv[5]);
    struct wl_display *dpy = wl_display_connect(NULL);
    if (!dpy) return 1;
    struct wl_registry *reg = wl_display_get_registry(dpy);
    wl_registry_add_listener(reg, &rl, NULL);
    wl_display_roundtrip(dpy);
    if (!comp || !shm || !wm) return 1;
    xdg_wm_base_add_listener(wm, &wml, NULL);
    surf = wl_compositor_create_surface(comp);
    struct xdg_surface *xs = xdg_wm_base_get_xdg_surface(wm, surf);
    xdg_surface_add_listener(xs, &xsl, NULL);
    struct xdg_toplevel *tl = xdg_surface_get_toplevel(xs);
    xdg_toplevel_add_listener(tl, &tll, NULL);
    xdg_toplevel_set_app_id(tl, argv[1]);
    wl_surface_commit(surf);
    while (wl_display_dispatch(dpy) != -1) {}
    return 0;
}
