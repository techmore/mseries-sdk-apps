#pragma once

#include <glib.h>

typedef struct SignalHistoryStore SignalHistoryStore;

SignalHistoryStore *signal_history_store_open(const char *path, GError **error);
gboolean signal_history_store_save(SignalHistoryStore *store,
                                   const char *conversation_id,
                                   gint64 timestamp,
                                   gboolean outgoing,
                                   const char *message,
                                   GError **error);
gboolean signal_history_store_contains(SignalHistoryStore *store,
                                       const char *conversation_id,
                                       gint64 timestamp,
                                       gboolean outgoing,
                                       const char *message,
                                       GError **error);
char *signal_history_store_load_json(SignalHistoryStore *store,
                                     const char *conversation_id,
                                     GError **error);
void signal_history_store_close(SignalHistoryStore *store);
/* Atomically replace a conversation with one encrypted JSON snapshot. */
gboolean signal_history_store_snapshot(SignalHistoryStore *store,
                                       const char *conversation_id,
                                       const char *snapshot,
                                       GError **error);

gboolean signal_history_store_pair(SignalHistoryStore *store,
                                   const char *key, const char *value,
                                   const char *catalog_key, const char *catalog,
                                   GError **error);
