// Pantheon protocol XML: LGPL-2.1-or-later, Corentin Noël (2023).
// GDK owns the display and wl_surface. Only proxies created here are destroyed here.
#include <gtk/gtk.h>
#include <gdk/wayland/gdkwayland.h>
#include <wayland-client.h>
#include <string.h>
#include "pantheon.h"
struct binding { struct io_elementary_pantheon_shell_v1 *shell; };
static void global(void *data, struct wl_registry *registry, uint32_t name, const char *interface, uint32_t version) {
    struct binding *b = data;
    if (!strcmp(interface, "io_elementary_pantheon_shell_v1") && !b->shell)
        b->shell = wl_registry_bind(registry, name, &io_elementary_pantheon_shell_v1_interface, MIN(version, 2));
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) { (void)data; (void)registry; (void)name; }
static const struct wl_registry_listener listener = { global, removed };
static void unrealized(GtkWidget *widget, gpointer data) {
    (void)data;
    GdkSurface *surface = gtk_native_get_surface(GTK_NATIVE(widget));
    if (surface) g_object_set_data(G_OBJECT(surface), "msdk-panel", NULL);
}
static void cleanup(gpointer value) {
    io_elementary_pantheon_panel_v1_destroy(value);
}
int msdk_panel_attach(GtkWindow *window, unsigned anchor, unsigned hide, int size) {
    GdkSurface *surface = gtk_native_get_surface(GTK_NATIVE(window));
    if (!surface || !GDK_IS_WAYLAND_SURFACE(surface)) return 0;
    // One panel proxy per realization; never assign the role twice to a surface.
    if (g_object_get_data(G_OBJECT(surface), "msdk-panel")) return 1;
    struct wl_display *display = gdk_wayland_display_get_wl_display(gdk_surface_get_display(surface));
    struct wl_event_queue *queue = wl_display_create_queue(display);
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_proxy_set_queue((struct wl_proxy *)registry, queue);
    struct binding b = {0};
    wl_registry_add_listener(registry, &listener, &b);
    int result = wl_display_roundtrip_queue(display, queue);
    if (result < 0 || !b.shell) {
        if (b.shell) wl_proxy_destroy((struct wl_proxy *)b.shell);
        wl_registry_destroy(registry); wl_event_queue_destroy(queue); return 0;
    }
    struct io_elementary_pantheon_panel_v1 *panel = io_elementary_pantheon_shell_v1_get_panel(b.shell, gdk_wayland_surface_get_wl_surface(surface));
    wl_proxy_set_queue((struct wl_proxy *)panel, NULL);
    io_elementary_pantheon_panel_v1_set_anchor(panel, anchor);
    io_elementary_pantheon_panel_v1_set_size(panel, anchor < 2 ? -1 : size, anchor < 2 ? size : -1);
    io_elementary_pantheon_panel_v1_set_hide_mode(panel, hide);
    g_object_set_data_full(G_OBJECT(surface), "msdk-panel", panel, cleanup);
    wl_proxy_destroy((struct wl_proxy *)b.shell);
    wl_registry_destroy(registry); wl_event_queue_destroy(queue);
    g_signal_connect(window, "unrealize", G_CALLBACK(unrealized), NULL);
    wl_display_flush(display);
    return 1;
}
