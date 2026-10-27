namespace SignalNative {
    [CCode (cname = "signal_history_store_open", cheader_filename = "history-store.h")]
    private extern void* signal_history_store_open (string path) throws Error;
    [CCode (cname = "signal_history_store_save", cheader_filename = "history-store.h")]
    private extern bool signal_history_store_save (
        void* store,
        string conversation_id,
        int64 timestamp,
        bool outgoing,
        string message
    ) throws Error;
    [CCode (cname = "signal_history_store_contains", cheader_filename = "history-store.h")]
    private extern bool signal_history_store_contains (
        void* store,
        string conversation_id,
        int64 timestamp,
        bool outgoing,
        string message
    ) throws Error;
    [CCode (cname = "signal_history_store_load_json", cheader_filename = "history-store.h")]
    private extern string? signal_history_store_load_json (
        void* store,
        string conversation_id
    ) throws Error;
    [CCode (cname = "signal_history_store_close", cheader_filename = "history-store.h")]
    private extern void signal_history_store_close (void* store);

    [CCode (cname = "signal_history_store_snapshot", cheader_filename = "history-store.h")]
    private extern bool signal_history_store_snapshot (void* store, string conversation_id, string snapshot) throws Error;

    [CCode (cname = "signal_history_store_pair", cheader_filename = "history-store.h")]
    private extern bool signal_history_store_pair (void* store, string key, string value, string catalog_key, string catalog) throws Error;

    private class RpcWaiter : Object {
        public Json.Node? response;
        public string? error_message;
        private bool done = false;

        public signal void completed ();

        public async void wait () {
            if (done) {
                return;
            }
            ulong handler = completed.connect (() => wait.callback ());
            yield;
            disconnect (handler);
        }

        public void finish () {
            if (done) {
                return;
            }
            done = true;
            completed ();
        }
    }

    public class SignalBackend : Object {
        private Subprocess? daemon;
        private SocketConnection? connection;
        private DataInputStream? input;
        private OutputStream? output;
        private string data_directory;
        private string socket_path;
        private void* history_store = null;
        private string? active_id;
        private RpcWaiter? active_waiter;
        private uint next_id = 1;
        private uint request_timeout_seconds;
        private uint generation = 0;

        public signal void call_event (Json.Node event);
        public signal void incoming_message (Json.Node event);
        public signal void disconnected ();
        private signal void slot_released ();

        private async void wait_for_slot () {
            ulong handler = slot_released.connect (() => wait_for_slot.callback ());
            yield;
            disconnect (handler);
        }

        public SignalBackend (uint request_timeout_seconds = 45) {
            this.request_timeout_seconds = request_timeout_seconds;
            data_directory = Path.build_filename (
                Environment.get_user_data_dir (), "signal-pantheon"
            );
            string runtime_directory = Environment.get_variable ("XDG_RUNTIME_DIR")
                ?? Environment.get_user_cache_dir ();
            string private_runtime = Path.build_filename (runtime_directory, "signal-pantheon");
            DirUtils.create_with_parents (private_runtime, 448);
            FileUtils.chmod (private_runtime, 448);
            socket_path = Path.build_filename (private_runtime, "signal-cli.sock");
        }

        public async void start () throws Error {
            if (connection != null) {
                return;
            }

            uint startup_generation = ++generation;
            DirUtils.create_with_parents (data_directory, 448);
            FileUtils.chmod (data_directory, 448);
            if (history_store == null) {
                string history_path = Path.build_filename (data_directory, "message-history.db");
                history_store = signal_history_store_open (history_path);
            }
            FileUtils.remove (socket_path);

            string executable = Environment.get_variable ("SIGNAL_CLI_PATH") ?? "signal-cli";
            string[] command = {
                executable,
                "--data-dir", data_directory,
                "daemon", "--socket=%s".printf (socket_path),
                null
            };
            daemon = new Subprocess.newv (
                command,
                SubprocessFlags.STDOUT_SILENCE | SubprocessFlags.STDERR_SILENCE
            );

            var client = new SocketClient ();
            var address = new UnixSocketAddress (socket_path);
            for (int attempt = 0; attempt < 188; attempt++) {
                if (generation != startup_generation) { throw new IOError.CANCELLED ("Signal startup canceled"); }
                try {
                    var connected = yield client.connect_async (address, null);
                    if (generation != startup_generation) {
                        connected.close (null);
                        throw new IOError.CANCELLED ("Signal startup canceled");
                    }
                    connection = connected;
                    input = new DataInputStream (connection.input_stream);
                    output = connection.output_stream;
                    read_messages.begin (input);
                    return;
                } catch (Error error) {
                    yield pause ();
                }
            }
            stop ();
            throw new IOError.TIMED_OUT (
                "Timed out waiting for the local Signal service"
            );
        }

        public async Json.Node? rpc (
            string method,
            Json.Node? params_node = null
        ) throws Error {
            while (active_waiter != null) {
                yield wait_for_slot ();
            }
            if (connection == null || output == null) {
                throw new IOError.NOT_CONNECTED ("Signal service is not running");
            }

            string id = "pantheon-%u".printf (next_id++);
            var builder = new Json.Builder ();
            builder.begin_object ();
            builder.set_member_name ("jsonrpc");
            builder.add_string_value ("2.0");
            builder.set_member_name ("method");
            builder.add_string_value (method);
            builder.set_member_name ("id");
            builder.add_string_value (id);
            if (params_node != null) {
                builder.set_member_name ("params");
                builder.add_value (params_node);
            }
            builder.end_object ();

            var generator = new Json.Generator ();
            generator.set_root (builder.get_root ());
            string payload = generator.to_data (null) + "\n";
            active_id = id;
            active_waiter = new RpcWaiter ();
            RpcWaiter waiter = active_waiter;
            size_t written;
            try {
                yield output.write_all_async (
                    (uint8[]) payload.data, Priority.DEFAULT, null, out written
                );
            } catch (Error error) {
                stop ();
                throw error;
            }
            uint deadline = 0;
            deadline = Timeout.add_seconds (method == "finishLink" ? 180 : (method == "hangupCall" || method == "rejectCall" || method == "acceptCall" ? 8 : request_timeout_seconds), () => {
                deadline = 0;
                if (active_waiter == waiter) {
                    waiter.error_message = "Signal request timed out; reconnect before retrying. A send may already have reached Signal.";
                    stop ();
                }
                return Source.REMOVE;
            });
            yield waiter.wait ();
            if (deadline != 0) {
                Source.remove (deadline);
            }

            if (active_waiter == waiter) {
                active_id = null;
                active_waiter = null;
                slot_released ();
            }
            if (waiter.error_message != null) {
                throw new IOError.FAILED (waiter.error_message);
            }
            return waiter.response;
        }

        public Json.Node link_params (string uri) {
            var builder = new Json.Builder ();
            builder.begin_object ();
            builder.set_member_name ("deviceLinkUri");
            builder.add_string_value (uri);
            builder.set_member_name ("deviceName");
            builder.add_string_value ("Pantheon");
            builder.end_object ();
            return builder.get_root ();
        }

        public Json.Node account_params (string account) {
            var builder = new Json.Builder ();
            builder.begin_object ();
            builder.set_member_name ("account");
            builder.add_string_value (account);
            builder.end_object ();
            return builder.get_root ();
        }

        public Json.Node send_params (
            string account,
            string recipient,
            string message_text,
            bool is_group = false
        ) {
            var builder = new Json.Builder ();
            builder.begin_object ();
            builder.set_member_name ("account");
            builder.add_string_value (account);
            if (is_group) {
                builder.set_member_name ("groupId");
                builder.add_string_value (recipient);
            } else {
                builder.set_member_name ("recipient");
                builder.begin_array ();
                builder.add_string_value (recipient);
                builder.end_array ();
            }
            builder.set_member_name ("message");
            builder.add_string_value (message_text);
            builder.end_object ();
            return builder.get_root ();
        }

        public void save_conversation (string key, string value, string catalog_key, string catalog) throws Error {
            if (history_store == null) { throw new IOError.NOT_CONNECTED ("History is not initialized"); }
            signal_history_store_pair (history_store, key, value, catalog_key, catalog);
        }

        public void save_snapshot (string key, string snapshot) throws Error {
            if (history_store == null) { throw new IOError.NOT_CONNECTED ("History is not initialized"); }
            signal_history_store_snapshot (history_store, key, snapshot);
        }

        public string? load_snapshot (string key) throws Error {
            var history = load_message_history (key);
            if (history == null || history.get_array ().get_length () == 0) { return null; }
            return history.get_array ().get_object_element (0).get_string_member ("message");
        }

        public string avatar_directory { owned get {
            return Path.build_filename (data_directory, "avatars");
        } }

        public string attachment_directory { owned get {
            return Path.build_filename (data_directory, "attachments");
        } }

        public void save_message (
            string conversation_id,
            int64 timestamp,
            bool outgoing,
            string message_text
        ) throws Error {
            if (history_store == null || !signal_history_store_save (
                history_store, conversation_id, timestamp, outgoing, message_text
            )) {
                throw new IOError.FAILED ("Could not save the encrypted message history");
            }
        }

        public Json.Node? load_message_history (string conversation_id) throws Error {
            if (history_store == null) {
                throw new IOError.NOT_CONNECTED ("Message history is not initialized");
            }
            string? json = signal_history_store_load_json (history_store, conversation_id);
            if (json == null) {
                return null;
            }
            var parser = new Json.Parser ();
            parser.load_from_data (json, -1);
            return parser.get_root ().copy ();
        }

        public bool message_exists (
            string conversation_id,
            int64 timestamp,
            bool outgoing,
            string message_text
        ) throws Error {
            if (history_store == null) {
                throw new IOError.NOT_CONNECTED ("Message history is not initialized");
            }
            return signal_history_store_contains (
                history_store, conversation_id, timestamp, outgoing, message_text
            );
        }

        public void stop () {
            generation++;
            bool was_connected = connection != null;
            RpcWaiter? waiter = active_waiter;
            active_id = null;
            active_waiter = null;
            if (waiter != null && waiter.response == null && waiter.error_message == null) {
                waiter.error_message = "Signal service stopped";
            }
            if (connection != null) {
                try {
                    connection.close (null);
                } catch (Error error) {
                }
            }
            connection = null;
            input = null;
            output = null;
            if (daemon != null) {
                var process = daemon;
                process.send_signal (15);
                reap_daemon.begin (process);
            }
            daemon = null;
            if (history_store != null) {
                signal_history_store_close (history_store);
                history_store = null;
            }
            if (was_connected) {
                disconnected ();
            }
            if (waiter != null) {
                waiter.finish ();
            }
            slot_released ();
        }

        private async void reap_daemon (Subprocess process) {
            uint deadline = Timeout.add_seconds (5, () => { process.force_exit (); return Source.REMOVE; });
            try { yield process.wait_async (); } catch (Error error) {}
            var source = MainContext.default ().find_source_by_id (deadline);
            if (source != null) { source.destroy (); }
        }

        private async void read_messages (DataInputStream stream) {
            while (input == stream) {
                try {
                    size_t length;
                    string? line = yield stream.read_line_async (
                        Priority.DEFAULT, null, out length
                    );
                    if (input != stream) {
                        return;
                    }
                    if (line == null) {
                        stop ();
                        return;
                    }
                    var parser = new Json.Parser ();
                    parser.load_from_data (line, (ssize_t) length);
                    Json.Node? root = parser.get_root ();
                    if (root == null || root.get_node_type () != Json.NodeType.OBJECT) {
                        continue;
                    }
                    Json.Object object = root.get_object ();
                    if (object.has_member ("id")) {
                        Json.Node id_node = object.get_member ("id");
                        string id = id_node.get_string ();
                        if (id == active_id && active_waiter != null) {
                            if (object.has_member ("error")) {
                                active_waiter.error_message = object.get_object_member ("error")
                                    .get_string_member ("message");
                            } else if (object.has_member ("result")) {
                                active_waiter.response = object.get_member ("result").copy ();
                            }
                            active_waiter.finish ();
                        }
                    } else if (
                        object.has_member ("method") &&
                        object.get_string_member ("method") == "receive"
                    ) {
                        incoming_message (root.copy ());
                    } else if (object.has_member ("method") && object.get_string_member ("method") == "callEvent") {
                        call_event (root.copy ());
                    }
                } catch (Error error) {
                    if (input == stream) {
                        stop ();
                    }
                    return;
                }
            }
        }

        private async void pause () {
            Timeout.add (160, pause.callback);
            yield;
        }
    }
}
