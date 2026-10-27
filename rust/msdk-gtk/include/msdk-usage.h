/* SPDX-License-Identifier: GPL-3.0-only */
#pragma once
#include <gtk/gtk.h>
#include <stdbool.h>

/* GTK main thread only. The handle owns a reference to its widget. Unparent
 * that widget before freeing the handle. Returned widget is transfer-none. */
typedef struct MsdkUsageControl MsdkUsageControl;
MsdkUsageControl *msdk_usage_control_new(const char *absolute_executable);
GtkWidget *msdk_usage_control_widget(MsdkUsageControl *handle);
bool msdk_usage_control_open(MsdkUsageControl *handle);
void msdk_usage_control_close(MsdkUsageControl *handle);
void msdk_usage_control_free(MsdkUsageControl *handle);
