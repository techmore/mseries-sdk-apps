namespace SignalNative {
    public class ClientStore : Object {
        private SignalBackend backend;
        private string account;
        public GenericArray<Conversation> conversations = new GenericArray<Conversation> ();
        private HashTable<string, Conversation> index = new HashTable<string, Conversation> (str_hash, str_equal);

        public ClientStore (SignalBackend backend, string account) {
            this.backend = backend;
            this.account = account;
        }

        private string key (string id) { return "pantheon-v1:" + account + ":" + id; }

        public void restore () throws Error {
            string? catalog = backend.load_snapshot (key ("catalog"));
            if (catalog == null) { return; }
            var parser = new Json.Parser ();
            parser.load_from_data (catalog);
            if (parser.get_root ().get_node_type () != Json.NodeType.ARRAY) {
                throw new IOError.INVALID_DATA ("Invalid encrypted conversation catalog");
            }
            var a = parser.get_root ().get_array ();
            for (uint i = 0; i < a.get_length (); i++) {
                if (a.get_element (i).get_value_type () != typeof (string)) { continue; }
                string id = a.get_string_element (i);
                string? saved = backend.load_snapshot (key ("conversation:" + id));
                if (saved == null) { continue; }
                var c = new Conversation (id, id, false);
                c.restore (saved);
                c.expire (GLib.get_real_time () / 1000, true);
                conversations.add (c);
                index.insert (id, c);
            }
        }

        public string canonical (string address) {
            if (address == "") { return ""; }
            foreach (var c in conversations) {
                if (!c.is_group && (c.id == address || JsonFields.text (c.metadata, "uuid") == address ||
                    JsonFields.text (c.metadata, "number") == address)) { return c.id; }
            }
            return address;
        }

        public void import_legacy (Conversation c) throws Error {
            var history = backend.load_message_history (c.id);
            if (history == null || history.get_node_type () != Json.NodeType.ARRAY) { return; }
            var a = history.get_array ();
            bool imported = false;
            for (uint i = 0; i < a.get_length (); i++) {
                if (a.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                var row = a.get_object_element (i);
                int64 timestamp = JsonFields.integer (row, "timestamp");
                if (timestamp <= 0) { continue; }
                var e = new MessageEvent ();
                e.timestamp = timestamp;
                e.text = JsonFields.text (row, "message");
                e.outgoing = JsonFields.boolean (row, "outgoing");
                e.author = e.outgoing ? account : c.id;
                c.apply (e);
                imported = true;
            }
            if (imported) {
                // Persist the replacement before clearing the old encrypted rows.
                save (c);
                backend.save_snapshot (c.id, "");
            }
        }

        public Conversation? lookup (string id) { return index.lookup (id); }

        public Conversation ensure (string id, string name, bool group) {
            var c = index.lookup (id);
            if (c != null) {
                if (name != "" && name != id && name != c.name) { c.name = name; }
                c.is_group = group;
                return c;
            }
            c = new Conversation (id, name, group);
            index.insert (id, c);
            conversations.add (c);
            return c;
        }

        public void save (Conversation c) throws Error {
            var a = new Json.Array ();
            foreach (var conversation in conversations) { a.add_string_element (conversation.id); }
            var n = new Json.Node (Json.NodeType.ARRAY);
            n.set_array (a);
            backend.save_conversation (key ("conversation:" + c.id), c.serialize (), key ("catalog"), JsonFields.encode (n));
        }
    }
}
