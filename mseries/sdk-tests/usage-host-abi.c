/* Isolated C host: test the SDK control boundary without a desktop panel. */
#include "msdk-usage.h"
#include <glib.h>

int main(int argc, char **argv) {
    g_assert_cmpint(argc, ==, 3);
    gtk_init();
    g_assert_null(msdk_usage_control_new(NULL));
    g_assert_null(msdk_usage_control_new("relative-program"));
    MsdkUsageControl *control = msdk_usage_control_new(argv[1]);
    g_assert_nonnull(control);
    GtkWidget *content = msdk_usage_control_widget(control);
    g_assert_nonnull(content);
    GtkWidget *window = gtk_window_new();
    gtk_window_set_child(GTK_WINDOW(window), content);
    gtk_window_present(GTK_WINDOW(window));
    while (g_main_context_iteration(NULL, FALSE)) {}
    g_assert_false(g_file_test(argv[2], G_FILE_TEST_EXISTS));
    g_assert_true(msdk_usage_control_open(control));
    gint64 deadline = g_get_monotonic_time() + 5 * G_TIME_SPAN_SECOND;
    while (!g_file_test(argv[2], G_FILE_TEST_EXISTS) && g_get_monotonic_time() < deadline) {
        while (g_main_context_iteration(NULL, FALSE)) {}
        g_usleep(10000);
    }
    g_assert_true(g_file_test(argv[2], G_FILE_TEST_EXISTS));
    msdk_usage_control_close(control);
    gtk_window_set_child(GTK_WINDOW(window), NULL);
    msdk_usage_control_free(control);
    gtk_window_destroy(GTK_WINDOW(window));
    return 0;
}
