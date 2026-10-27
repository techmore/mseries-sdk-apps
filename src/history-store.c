#include "history-store.h"

#include <openssl/evp.h>
#include <openssl/hmac.h>
#include <openssl/rand.h>
#include <libsecret/secret.h>
#include <sqlite3.h>
#include <sys/stat.h>
#include <json-glib/json-glib.h>

#include <string.h>

#define HISTORY_ERROR (history_error_quark())
#define KEY_BYTES 32
#define NONCE_BYTES 12
#define TAG_BYTES 16

typedef enum {
    HISTORY_ERROR_SECRET,
    HISTORY_ERROR_CRYPTO,
    HISTORY_ERROR_DATABASE,
    HISTORY_ERROR_DATA
} HistoryError;

struct SignalHistoryStore {
    sqlite3 *db;
    guint8 key[KEY_BYTES];
    gboolean key_loaded;
};

static GQuark history_error_quark(void) {
    return g_quark_from_static_string("signal-pantheon-history-error");
}

static const SecretSchema history_schema = {
    "io.github.sdolbec.SignalPantheonClient.MessageHistory",
    SECRET_SCHEMA_NONE,
    {
        { "purpose", SECRET_SCHEMA_ATTRIBUTE_STRING },
        { NULL, 0 },
    },
};

static gboolean load_key(SignalHistoryStore *store, GError **error) {
    if (store->key_loaded) {
        return TRUE;
    }

    g_autoptr(GError) secret_error = NULL;
    g_autofree char *encoded = secret_password_lookup_sync(
        &history_schema, NULL, &secret_error,
        "purpose", "encrypted-message-history", NULL);
    if (secret_error != NULL) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_SECRET,
                    "Could not access the desktop keyring: %s", secret_error->message);
        return FALSE;
    }

    if (encoded == NULL) {
        guint8 generated[KEY_BYTES];
        if (RAND_bytes(generated, sizeof generated) != 1) {
            g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_CRYPTO,
                                "Could not generate a local history key");
            return FALSE;
        }
        g_autofree char *new_encoded = g_base64_encode(generated, sizeof generated);
        gboolean stored = secret_password_store_sync(
            &history_schema,
            SECRET_COLLECTION_DEFAULT,
            "Signal Pantheon message history key",
            new_encoded,
            NULL,
            &secret_error,
            "purpose", "encrypted-message-history", NULL);
        OPENSSL_cleanse(generated, sizeof generated);
        if (!stored || secret_error != NULL) {
            g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_SECRET,
                        "Could not store the history key in the desktop keyring: %s",
                        secret_error != NULL ? secret_error->message : "store failed");
            return FALSE;
        }
        encoded = g_steal_pointer(&new_encoded);
    }

    gsize decoded_length = 0;
    g_autofree guchar *decoded = g_base64_decode(encoded, &decoded_length);
    if (decoded_length != KEY_BYTES) {
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_SECRET,
                            "The saved local history key is invalid");
        return FALSE;
    }
    memcpy(store->key, decoded, KEY_BYTES);
    store->key_loaded = TRUE;
    return TRUE;
}

static gboolean hash_conversation(SignalHistoryStore *store,
                                  const char *conversation_id,
                                  guint8 digest[EVP_MAX_MD_SIZE],
                                  unsigned int *digest_length,
                                  GError **error) {
    if (!load_key(store, error)) {
        return FALSE;
    }
    if (HMAC(EVP_sha256(), store->key, KEY_BYTES,
             (const guint8 *) conversation_id, strlen(conversation_id),
             digest, digest_length) == NULL) {
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_CRYPTO,
                            "Could not protect the conversation identifier");
        return FALSE;
    }
    return TRUE;
}

static gboolean encrypt_message(SignalHistoryStore *store,
                               const char *message,
                               guint8 nonce[NONCE_BYTES],
                               GByteArray **ciphertext,
                               guint8 tag[TAG_BYTES],
                               GError **error) {
    if (!load_key(store, error) || RAND_bytes(nonce, NONCE_BYTES) != 1) {
        if (error != NULL && *error == NULL) {
            g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_CRYPTO,
                                "Could not create a message nonce");
        }
        return FALSE;
    }

    size_t input_length = strlen(message);
    GByteArray *output = g_byte_array_sized_new((guint) input_length + 16);
    g_byte_array_set_size(output, (guint) input_length + 16);
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    int written = 0;
    int final_written = 0;
    gboolean ok = ctx != NULL &&
        EVP_EncryptInit_ex(ctx, EVP_aes_256_gcm(), NULL, NULL, NULL) == 1 &&
        EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_SET_IVLEN, NONCE_BYTES, NULL) == 1 &&
        EVP_EncryptInit_ex(ctx, NULL, NULL, store->key, nonce) == 1 &&
        EVP_EncryptUpdate(ctx, output->data, &written,
                          (const guint8 *) message, (int) input_length) == 1 &&
        EVP_EncryptFinal_ex(ctx, output->data + written, &final_written) == 1 &&
        EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_GET_TAG, TAG_BYTES, tag) == 1;
    EVP_CIPHER_CTX_free(ctx);
    if (!ok) {
        g_byte_array_unref(output);
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_CRYPTO,
                            "Could not encrypt the message for local history");
        return FALSE;
    }
    g_byte_array_set_size(output, (guint) (written + final_written));
    *ciphertext = output;
    return TRUE;
}

static char *decrypt_message(SignalHistoryStore *store,
                             const guint8 *nonce,
                             const guint8 *ciphertext,
                             int ciphertext_length,
                             const guint8 *tag,
                             GError **error) {
    if (!load_key(store, error)) {
        return NULL;
    }
    g_autofree guint8 *plain = g_malloc((gsize) ciphertext_length + 1);
    EVP_CIPHER_CTX *ctx = EVP_CIPHER_CTX_new();
    int written = 0;
    int final_written = 0;
    gboolean ok = ctx != NULL &&
        EVP_DecryptInit_ex(ctx, EVP_aes_256_gcm(), NULL, NULL, NULL) == 1 &&
        EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_SET_IVLEN, NONCE_BYTES, NULL) == 1 &&
        EVP_DecryptInit_ex(ctx, NULL, NULL, store->key, nonce) == 1 &&
        EVP_DecryptUpdate(ctx, plain, &written, ciphertext, ciphertext_length) == 1 &&
        EVP_CIPHER_CTX_ctrl(ctx, EVP_CTRL_GCM_SET_TAG, TAG_BYTES, (void *) tag) == 1 &&
        EVP_DecryptFinal_ex(ctx, plain + written, &final_written) == 1;
    EVP_CIPHER_CTX_free(ctx);
    if (!ok) {
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_CRYPTO,
                            "A local history message could not be authenticated");
        return NULL;
    }
    plain[written + final_written] = '\0';
    return g_strdup((const char *) plain);
}

SignalHistoryStore *signal_history_store_open(const char *path, GError **error) {
    SignalHistoryStore *store = g_new0(SignalHistoryStore, 1);
    int result = sqlite3_open_v2(path, &store->db,
                                 SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX,
                                 NULL);
    if (result != SQLITE_OK) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not open local message history: %s",
                    store->db != NULL ? sqlite3_errmsg(store->db) : "sqlite error");
        if (store->db != NULL) sqlite3_close(store->db);
        g_free(store);
        return NULL;
    }
    chmod(path, 0600);
    sqlite3_exec(store->db, "PRAGMA journal_mode=WAL", NULL, NULL, NULL);
    sqlite3_exec(store->db, "PRAGMA synchronous=FULL", NULL, NULL, NULL);
    sqlite3_stmt *schema_statement = NULL;
    result = sqlite3_prepare_v2(
        store->db,
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='message_history'",
        -1, &schema_statement, NULL);
    if (result != SQLITE_OK) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not inspect local message history schema: %s",
                    sqlite3_errmsg(store->db));
        signal_history_store_close(store);
        return NULL;
    }

    gboolean table_exists = sqlite3_step(schema_statement) == SQLITE_ROW;
    const char *existing_schema = table_exists
        ? (const char *) sqlite3_column_text(schema_statement, 0) : NULL;
    gboolean has_overly_strict_unique_key = existing_schema != NULL &&
        strstr(existing_schema, "UNIQUE(conversation_hash,timestamp,outgoing)") != NULL;
    sqlite3_finalize(schema_statement);

    const char *schema =
        "CREATE TABLE IF NOT EXISTS message_history ("
        "id INTEGER PRIMARY KEY, conversation_hash BLOB NOT NULL, "
        "timestamp INTEGER NOT NULL, outgoing INTEGER NOT NULL, "
        "nonce BLOB NOT NULL, ciphertext BLOB NOT NULL, tag BLOB NOT NULL)";
    if (has_overly_strict_unique_key) {
        const char *migration =
            "BEGIN IMMEDIATE;"
            "ALTER TABLE message_history RENAME TO message_history_legacy;"
            "CREATE TABLE message_history ("
            "id INTEGER PRIMARY KEY, conversation_hash BLOB NOT NULL, "
            "timestamp INTEGER NOT NULL, outgoing INTEGER NOT NULL, "
            "nonce BLOB NOT NULL, ciphertext BLOB NOT NULL, tag BLOB NOT NULL);"
            "INSERT INTO message_history(id,conversation_hash,timestamp,outgoing,nonce,ciphertext,tag) "
            "SELECT id,conversation_hash,timestamp,outgoing,nonce,ciphertext,tag "
            "FROM message_history_legacy;"
            "DROP TABLE message_history_legacy;"
            "COMMIT;";
        result = sqlite3_exec(store->db, migration, NULL, NULL, NULL);
        if (result != SQLITE_OK) {
            sqlite3_exec(store->db, "ROLLBACK", NULL, NULL, NULL);
        }
    } else {
        result = sqlite3_exec(store->db, schema, NULL, NULL, NULL);
    }
    if (result != SQLITE_OK) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not initialize local message history: %s",
                    sqlite3_errmsg(store->db));
        signal_history_store_close(store);
        return NULL;
    }
    return store;
}

gboolean signal_history_store_save(SignalHistoryStore *store,
                                   const char *conversation_id,
                                   gint64 timestamp,
                                   gboolean outgoing,
                                   const char *message,
                                   GError **error) {
    guint8 conversation_hash[EVP_MAX_MD_SIZE];
    unsigned int hash_length = 0;
    guint8 nonce[NONCE_BYTES];
    guint8 tag[TAG_BYTES];
    GByteArray *ciphertext = NULL;
    if (!hash_conversation(store, conversation_id, conversation_hash, &hash_length, error) ||
        !encrypt_message(store, message, nonce, &ciphertext, tag, error)) {
        OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);
        OPENSSL_cleanse(nonce, sizeof nonce);
        OPENSSL_cleanse(tag, sizeof tag);
        if (ciphertext != NULL) g_byte_array_unref(ciphertext);
        return FALSE;
    }

    sqlite3_stmt *statement = NULL;
    const char *sql =
        "INSERT INTO message_history(conversation_hash,timestamp,outgoing,nonce,ciphertext,tag) "
        "VALUES(?,?,?,?,?,?)";
    int result = sqlite3_prepare_v2(store->db, sql, -1, &statement, NULL);
    if (result == SQLITE_OK) {
        sqlite3_bind_blob(statement, 1, conversation_hash, (int) hash_length, SQLITE_TRANSIENT);
        sqlite3_bind_int64(statement, 2, timestamp);
        sqlite3_bind_int(statement, 3, outgoing ? 1 : 0);
        sqlite3_bind_blob(statement, 4, nonce, NONCE_BYTES, SQLITE_TRANSIENT);
        sqlite3_bind_blob(statement, 5, ciphertext->data, (int) ciphertext->len, SQLITE_TRANSIENT);
        sqlite3_bind_blob(statement, 6, tag, TAG_BYTES, SQLITE_TRANSIENT);
        result = sqlite3_step(statement);
    }
    sqlite3_finalize(statement);
    g_byte_array_unref(ciphertext);
    OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);
    OPENSSL_cleanse(nonce, sizeof nonce);
    OPENSSL_cleanse(tag, sizeof tag);
    if (result != SQLITE_DONE) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not save local message history: %s", sqlite3_errmsg(store->db));
        return FALSE;
    }
    return TRUE;
}

gboolean signal_history_store_contains(SignalHistoryStore *store,
                                       const char *conversation_id,
                                       gint64 timestamp,
                                       gboolean outgoing,
                                       const char *message,
                                       GError **error) {
    guint8 conversation_hash[EVP_MAX_MD_SIZE];
    unsigned int hash_length = 0;
    if (!hash_conversation(store, conversation_id, conversation_hash, &hash_length, error)) {
        OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);
        return FALSE;
    }

    sqlite3_stmt *statement = NULL;
    const char *sql =
        "SELECT nonce,ciphertext,tag FROM message_history "
        "WHERE conversation_hash=? AND timestamp=? AND outgoing=? ORDER BY id";
    int result = sqlite3_prepare_v2(store->db, sql, -1, &statement, NULL);
    if (result != SQLITE_OK) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not check local message history: %s", sqlite3_errmsg(store->db));
        OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);
        return FALSE;
    }

    sqlite3_bind_blob(statement, 1, conversation_hash, (int) hash_length, SQLITE_TRANSIENT);
    sqlite3_bind_int64(statement, 2, timestamp);
    sqlite3_bind_int(statement, 3, outgoing ? 1 : 0);
    OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);

    gboolean found = FALSE;
    while ((result = sqlite3_step(statement)) == SQLITE_ROW) {
        const guint8 *nonce = sqlite3_column_blob(statement, 0);
        const guint8 *ciphertext = sqlite3_column_blob(statement, 1);
        int ciphertext_length = sqlite3_column_bytes(statement, 1);
        const guint8 *tag = sqlite3_column_blob(statement, 2);
        if (sqlite3_column_bytes(statement, 0) != NONCE_BYTES ||
            sqlite3_column_bytes(statement, 2) != TAG_BYTES) {
            g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATA,
                                "Local message history has an invalid record");
            sqlite3_finalize(statement);
            return FALSE;
        }
        g_autofree char *stored_message = decrypt_message(
            store, nonce, ciphertext, ciphertext_length, tag, error);
        if (stored_message == NULL) {
            sqlite3_finalize(statement);
            return FALSE;
        }
        if (g_strcmp0(stored_message, message) == 0) {
            found = TRUE;
            break;
        }
    }
    sqlite3_finalize(statement);
    if (result != SQLITE_DONE && result != SQLITE_ROW) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not check local message history: %s", sqlite3_errmsg(store->db));
        return FALSE;
    }
    return found;
}

char *signal_history_store_load_json(SignalHistoryStore *store,
                                     const char *conversation_id,
                                     GError **error) {
    guint8 conversation_hash[EVP_MAX_MD_SIZE];
    unsigned int hash_length = 0;
    if (!hash_conversation(store, conversation_id, conversation_hash, &hash_length, error)) {
        OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);
        return NULL;
    }

    sqlite3_stmt *statement = NULL;
    const char *sql =
        "SELECT timestamp,outgoing,nonce,ciphertext,tag FROM message_history "
        "WHERE conversation_hash=? ORDER BY timestamp,id";
    int result = sqlite3_prepare_v2(store->db, sql, -1, &statement, NULL);
    if (result != SQLITE_OK) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not load local message history: %s", sqlite3_errmsg(store->db));
        OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);
        return NULL;
    }
    sqlite3_bind_blob(statement, 1, conversation_hash, (int) hash_length, SQLITE_TRANSIENT);
    OPENSSL_cleanse(conversation_hash, sizeof conversation_hash);

    JsonBuilder *builder = json_builder_new();
    json_builder_begin_array(builder);
    while ((result = sqlite3_step(statement)) == SQLITE_ROW) {
        const guint8 *nonce = sqlite3_column_blob(statement, 2);
        const guint8 *ciphertext = sqlite3_column_blob(statement, 3);
        int ciphertext_length = sqlite3_column_bytes(statement, 3);
        const guint8 *tag = sqlite3_column_blob(statement, 4);
        if (sqlite3_column_bytes(statement, 2) != NONCE_BYTES ||
            sqlite3_column_bytes(statement, 4) != TAG_BYTES) {
            g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATA,
                                "Local message history has an invalid record");
            json_builder_end_array(builder);
            g_object_unref(builder);
            sqlite3_finalize(statement);
            return NULL;
        }
        g_autofree char *message = decrypt_message(store, nonce, ciphertext,
                                                    ciphertext_length, tag, error);
        if (message == NULL) {
            json_builder_end_array(builder);
            g_object_unref(builder);
            sqlite3_finalize(statement);
            return NULL;
        }
        json_builder_begin_object(builder);
        json_builder_set_member_name(builder, "timestamp");
        json_builder_add_int_value(builder, sqlite3_column_int64(statement, 0));
        json_builder_set_member_name(builder, "outgoing");
        json_builder_add_boolean_value(builder, sqlite3_column_int(statement, 1) != 0);
        json_builder_set_member_name(builder, "message");
        json_builder_add_string_value(builder, message);
        json_builder_end_object(builder);
    }
    sqlite3_finalize(statement);
    if (result != SQLITE_DONE) {
        g_set_error(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE,
                    "Could not read local message history: %s", sqlite3_errmsg(store->db));
        json_builder_end_array(builder);
        g_object_unref(builder);
        return NULL;
    }
    json_builder_end_array(builder);
    JsonGenerator *generator = json_generator_new();
    json_generator_set_root(generator, json_builder_get_root(builder));
    char *json = json_generator_to_data(generator, NULL);
    g_object_unref(generator);
    g_object_unref(builder);
    return json;
}

void signal_history_store_close(SignalHistoryStore *store) {
    if (store == NULL) return;
    if (store->db != NULL) sqlite3_close(store->db);
    OPENSSL_cleanse(store->key, sizeof store->key);
    g_free(store);
}

gboolean signal_history_store_snapshot(SignalHistoryStore *store,
                                       const char *conversation_id,
                                       const char *snapshot,
                                       GError **error) {
    guint8 digest[EVP_MAX_MD_SIZE];
    unsigned int length = 0;
    if (!hash_conversation(store, conversation_id, digest, &length, error)) return FALSE;
    if (sqlite3_exec(store->db, "PRAGMA secure_delete=ON; SAVEPOINT snapshot_update", NULL, NULL, NULL) != SQLITE_OK) {
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE, "Could not begin history update");
        OPENSSL_cleanse(digest, sizeof digest);
        return FALSE;
    }
    sqlite3_stmt *statement = NULL;
    int result = sqlite3_prepare_v2(store->db,
        "DELETE FROM message_history WHERE conversation_hash=?", -1, &statement, NULL);
    if (result == SQLITE_OK) {
        sqlite3_bind_blob(statement, 1, digest, length, SQLITE_TRANSIENT);
        result = sqlite3_step(statement);
    }
    sqlite3_finalize(statement);
    OPENSSL_cleanse(digest, sizeof digest);
    gboolean saved = FALSE;
    if (result != SQLITE_DONE) {
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE, "Could not replace history");
    } else {
        saved = signal_history_store_save(store, conversation_id, 0, FALSE, snapshot, error);
    }
    if (!saved || sqlite3_exec(store->db, "RELEASE snapshot_update", NULL, NULL, NULL) != SQLITE_OK) {
        if (saved) g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE, "Could not commit history update");
        sqlite3_exec(store->db, "ROLLBACK TO snapshot_update; RELEASE snapshot_update", NULL, NULL, NULL);
        return FALSE;
    }
    return TRUE;
}


gboolean signal_history_store_pair(SignalHistoryStore *store,
                                   const char *key, const char *value,
                                   const char *catalog_key, const char *catalog,
                                   GError **error) {
    if (sqlite3_exec(store->db, "BEGIN IMMEDIATE", NULL, NULL, NULL) != SQLITE_OK) {
        g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE, "Could not begin conversation update");
        return FALSE;
    }
    gboolean saved = signal_history_store_snapshot(store, key, value, error) &&
        signal_history_store_snapshot(store, catalog_key, catalog, error);
    if (!saved || sqlite3_exec(store->db, "COMMIT", NULL, NULL, NULL) != SQLITE_OK) {
        if (saved) g_set_error_literal(error, HISTORY_ERROR, HISTORY_ERROR_DATABASE, "Could not commit conversation update");
        sqlite3_exec(store->db, "ROLLBACK", NULL, NULL, NULL);
        return FALSE;
    }
    return TRUE;
}
