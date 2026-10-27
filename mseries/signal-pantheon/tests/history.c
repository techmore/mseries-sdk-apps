/* Private implementation inclusion permits an in-memory test key without adding
 * any keyring bypass to the production library. */
#include "history-store.c"
#include <glib/gstdio.h>

int main(int argc, char **argv) {
    g_test_init(&argc, &argv, NULL);
    GError *error = NULL;
    char *directory = g_dir_make_tmp("signal-history-test-XXXXXX", &error);
    g_assert_no_error(error);
    char *path = g_build_filename(directory, "history.db", NULL);
    SignalHistoryStore *store = signal_history_store_open(path, &error);
    g_assert_no_error(error);
    g_assert_nonnull(store);
    memset(store->key, 42, KEY_BYTES);
    store->key_loaded = TRUE;
    const char *original = "private-body-5e8ff496";
    g_assert_true(signal_history_store_save(store, "private-contact", 100, FALSE, original, &error));
    g_assert_no_error(error);
    g_assert_true(signal_history_store_contains(store, "private-contact", 100, FALSE, original, &error));
    g_assert_no_error(error);
    g_assert_true(signal_history_store_save(store, "private-contact", 100, FALSE, "different-author", &error));
    char *json = signal_history_store_load_json(store, "private-contact", &error);
    g_assert_no_error(error);
    g_assert_nonnull(strstr(json, original));
    g_assert_nonnull(strstr(json, "different-author"));
    g_free(json);
    char *bytes = NULL; gsize count = 0;
    g_file_get_contents(path, &bytes, &count, &error);
    g_assert_no_error(error);
    for (gsize i = 0; i + strlen(original) <= count; i++) g_assert_cmpint(memcmp(bytes+i, original, strlen(original)), !=, 0);
    for (gsize i = 0; i + strlen("private-contact") <= count; i++) g_assert_cmpint(memcmp(bytes+i, "private-contact", strlen("private-contact")), !=, 0);
    g_free(bytes);
    g_assert_true(signal_history_store_snapshot(store, "private-contact", "snapshot-new", &error));
    g_assert_no_error(error);
    g_assert_false(signal_history_store_contains(store, "private-contact", 100, FALSE, original, &error));
    g_assert_no_error(error);
    // Force a failed replacement to verify that DELETE is rolled back too.
    g_assert_cmpint(sqlite3_exec(store->db,
        "CREATE TRIGGER reject_insert BEFORE INSERT ON message_history BEGIN SELECT RAISE(ABORT,'fixture'); END;",
        NULL, NULL, NULL), ==, SQLITE_OK);
    g_assert_false(signal_history_store_snapshot(store, "private-contact", "must-not-commit", &error));
    g_assert_nonnull(error); g_clear_error(&error);
    json = signal_history_store_load_json(store, "private-contact", &error);
    g_assert_no_error(error);
    g_assert_nonnull(strstr(json, "snapshot-new"));
    g_assert_null(strstr(json, "must-not-commit"));
    g_free(json);
    g_assert_cmpint(sqlite3_exec(store->db,
        "DROP TRIGGER reject_insert; CREATE TRIGGER reject_second AFTER INSERT ON message_history WHEN (SELECT count(*) FROM message_history)>1 BEGIN SELECT RAISE(ABORT,'second fixture'); END;", NULL, NULL, NULL), ==, SQLITE_OK);
    g_assert_false(signal_history_store_pair(store, "private-contact", "first-update", "catalog", "catalog-update", &error));
    g_assert_nonnull(error); g_clear_error(&error);
    json = signal_history_store_load_json(store, "private-contact", &error);
    g_assert_no_error(error);
    g_assert_nonnull(strstr(json, "snapshot-new"));
    g_assert_null(strstr(json, "first-update"));
    g_free(json);
    g_assert_cmpint(sqlite3_exec(store->db,
        "DROP TRIGGER reject_second; UPDATE message_history SET tag=zeroblob(16);", NULL, NULL, NULL), ==, SQLITE_OK);
    json = signal_history_store_load_json(store, "private-contact", &error);
    g_assert_null(json); g_assert_nonnull(error); g_clear_error(&error);
    signal_history_store_close(store);
    g_remove(path); g_rmdir(directory); g_free(path); g_free(directory);
    g_print("PASS: encrypted round trip, collision preservation, snapshot replacement, rollback and tamper rejection\n");
    return 0;
}
