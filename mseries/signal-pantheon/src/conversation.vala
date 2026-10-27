namespace SignalNative {
    public class MessageRecord : Object {
        public string text = "";
        public string author = "";
        public int64 timestamp;
        public int64 modified = 0;
        public bool outgoing;
        public bool deleted = false;
        public bool view_once = false;
        public bool view_opened = false;
        public bool local_read = false;
        public int64 read_at_ms = 0;
        public int64 expiration_seconds = 0;
        public int64 expiration_start_ms = 0;
        public int64 expires_at_ms { get {
            if (view_once && !view_opened) {
                int64 unopened_limit = timestamp + 30L * 24 * 60 * 60 * 1000;
                if (expiration_seconds <= 0 || expiration_start_ms <= 0) { return unopened_limit; }
                return int64.min (unopened_limit, expiration_start_ms + expiration_seconds * 1000);
            }
            if (expiration_seconds <= 0 || expiration_start_ms <= 0) { return 0; }
            return expiration_start_ms + expiration_seconds * 1000;
        } }

        public void start_expiration (int64 read_at, int64 now) {
            int64 start = int64.min (read_at, now);
            if (start <= 0) { return; }
            if (read_at_ms == 0 || start < read_at_ms) { read_at_ms = start; }
            if (expiration_seconds > 0 && (expiration_start_ms == 0 || start < expiration_start_ms)) {
                expiration_start_ms = start;
            }
        }
        public string status = "sent";
        public Json.Array attachments = new Json.Array ();
        public Json.Object reactions = new Json.Object ();
        public Json.Object reaction_versions = new Json.Object ();

        public Json.Object to_json () {
            var o = new Json.Object ();
            o.set_string_member ("text", text);
            o.set_string_member ("author", author);
            o.set_int_member ("timestamp", timestamp);
            o.set_int_member ("modified", modified);
            o.set_boolean_member ("outgoing", outgoing);
            o.set_boolean_member ("deleted", deleted);
            o.set_boolean_member ("viewOnce", view_once);
            o.set_boolean_member ("viewOpened", view_opened);
            o.set_boolean_member ("localRead", local_read);
            o.set_int_member ("readAt", read_at_ms);
            o.set_int_member ("expirationSeconds", expiration_seconds);
            o.set_int_member ("expirationStart", expiration_start_ms);
            o.set_string_member ("status", status);
            o.set_array_member ("attachments", attachments.ref ());
            o.set_object_member ("reactions", reactions.ref ());
            o.set_object_member ("reactionVersions", reaction_versions.ref ());
            return o;
        }

        public static MessageRecord from_json (Json.Object o) {
            var m = new MessageRecord ();
            m.text = JsonFields.text (o, "text");
            m.author = JsonFields.text (o, "author");
            m.timestamp = JsonFields.integer (o, "timestamp");
            m.modified = JsonFields.integer (o, "modified");
            m.outgoing = JsonFields.boolean (o, "outgoing");
            m.deleted = JsonFields.boolean (o, "deleted");
            m.view_once = JsonFields.boolean (o, "viewOnce");
            m.view_opened = JsonFields.boolean (o, "viewOpened");
            m.local_read = JsonFields.boolean (o, "localRead");
            m.read_at_ms = JsonFields.integer (o, "readAt");
            m.expiration_seconds = int64.max (0, int64.min (int.MAX, JsonFields.integer (o, "expirationSeconds")));
            m.expiration_start_ms = JsonFields.integer (o, "expirationStart");
            m.status = JsonFields.text (o, "status", "sent");
            m.attachments = JsonFields.array (o, "attachments");
            m.reactions = JsonFields.object (o, "reactions") ?? new Json.Object ();
            m.reaction_versions = JsonFields.object (o, "reactionVersions") ?? new Json.Object ();
            return m;
        }
    }

    public class Conversation : Object {
        public string id;
        public string name;
        public bool is_group;
        public string draft = "";
        public string attachment_draft = "";
        public bool view_once_draft = false;
        public uint unread = 0;
        public Json.Object metadata = new Json.Object ();
        public GenericArray<MessageRecord> messages = new GenericArray<MessageRecord> ();
        public Json.Object expired = new Json.Object ();
        public Json.Array cleanup = new Json.Array ();
        public Json.Array read_sync_queue = new Json.Array ();
        public int64 clock_floor_ms = 0;
        public bool expiry_dirty = false;
        private GenericArray<MessageEvent> pending = new GenericArray<MessageEvent> ();

        public Conversation (string id, string name, bool is_group) {
            this.id = id;
            this.name = name;
            this.is_group = is_group;
        }

        public MessageRecord? find (int64 timestamp, string author = "") {
            foreach (var m in messages) {
                if (m.timestamp == timestamp && (author == "" || m.author == author)) { return m; }
            }
            return null;
        }

        private string identity (string author, int64 timestamp) { return author + "\x1f" + timestamp.to_string (); }

        public bool mark_read (int64 now, bool send_remote_receipt) {
            bool changed = false;
            foreach (var m in messages) {
                if (m.outgoing || m.deleted || m.read_at_ms > 0) { continue; }
                m.start_expiration (now, now);
                var marker = new Json.Object ();
                marker.set_string_member ("author", m.author);
                marker.set_int_member ("timestamp", m.timestamp);
                marker.set_int_member ("readAt", now);
                marker.set_boolean_member ("sendRemote", send_remote_receipt);
                marker.set_boolean_member ("syncSent", false);
                marker.set_boolean_member ("remoteSent", false);
                read_sync_queue.add_object_element (marker);
                changed = true;
            }
            unread = 0;
            return changed;
        }

        public bool consume_view_once (MessageRecord m, bool sync = true) {
            if (!m.view_once || m.view_opened) { return false; }
            m.view_opened = true;
            foreach (var n in m.attachments.get_elements ()) { cleanup.add_element (n.copy ()); }
            m.attachments = new Json.Array ();
            m.text = "";
            if (sync && !m.outgoing) {
                var marker = new Json.Object ();
                marker.set_string_member ("author", m.author);
                marker.set_int_member ("timestamp", m.timestamp);
                marker.set_int_member ("readAt", GLib.get_real_time () / 1000);
                marker.set_boolean_member ("viewOnceOpen", true);
                read_sync_queue.add_object_element (marker);
            }
            expiry_dirty = true;
            return true;
        }

        public bool expire (int64 now, bool after_restart = false) {
            bool rollback = after_restart && clock_floor_ms > now;
            clock_floor_ms = int64.max (clock_floor_ms, now);
            bool changed = false;
            for (uint i = messages.length; i > 0; i--) {
                var m = messages[i - 1];
                if (m.expires_at_ms == 0 || (!rollback && m.expires_at_ms > now)) { continue; }
                expired.set_int_member (identity (m.author, m.timestamp), m.expires_at_ms);
                for (uint j = 0; j < m.attachments.get_length (); j++) { cleanup.add_element (m.attachments.get_element (j).copy ()); }
                messages.remove_index (i - 1);
                for (uint j = pending.length; j > 0; j--) {
                    if (pending[j - 1].target_timestamp == m.timestamp &&
                        (pending[j - 1].kind == "reaction" ? pending[j - 1].target_author : pending[j - 1].author) == m.author) {
                        pending.remove_index (j - 1);
                    }
                }
                changed = true;
            }
            if (changed) { expiry_dirty = true; }
            return changed;
        }

        public bool apply (MessageEvent e, int64 now = 0) {
            if (now == 0) { now = GLib.get_real_time () / 1000; }
            if (e.ephemeral) { return false; }
            if (e.kind == "view_once_open") {
                var target = find (e.target_timestamp, e.author);
                if (target == null) { if (pending.length < 2048) { pending.add (e); } return true; }
                return consume_view_once (target, false);
            }
            if (expired.has_member (identity (e.kind == "reaction" ? e.target_author : e.author, e.kind == "message" ? e.timestamp : e.target_timestamp))) { return false; }
            if (e.kind == "read_sync") {
                var target = find (e.target_timestamp, e.author);
                if (target == null) { if (pending.length < 2048) { pending.add (e); } return true; }
                if (target.outgoing) { return false; }
                target.start_expiration (e.timestamp, now);
                return true;
            }
            if (e.kind == "receipt") {
                bool changed = false;
                for (uint i = 0; i < e.receipt_timestamps.get_length (); i++) {
                    var n = e.receipt_timestamps.get_element (i);
                    if (n.get_node_type () != Json.NodeType.VALUE || n.get_value_type () != typeof (int64)) { continue; }
                    bool known = false;
                    foreach (var m in messages) {
                        if (!m.outgoing || m.timestamp != n.get_int ()) { continue; }
                        known = true;
                        if (m.status != "read" && m.status != (e.read_receipt ? "read" : "delivered")) {
                            m.status = e.read_receipt ? "read" : "delivered";
                            changed = true;
                        }
                    }
                    if (!known && pending.length < 2048) {
                        bool duplicate = false;
                        foreach (var waiting in pending) {
                            if (waiting.kind == "receipt" && waiting.receipt_timestamps.get_length () == 1 &&
                                waiting.receipt_timestamps.get_int_element (0) == n.get_int () &&
                                waiting.read_receipt == e.read_receipt) { duplicate = true; break; }
                        }
                        if (!duplicate) {
                            var waiting = new MessageEvent ();
                            waiting.kind = "receipt";
                            waiting.author = e.author;
                            waiting.read_receipt = e.read_receipt;
                            waiting.receipt_timestamps.add_int_element (n.get_int ());
                            pending.add (waiting);
                            changed = true;
                        }
                    }
                }
                return changed;
            }
            if (e.kind != "message") {
                string target_author = e.kind == "reaction" ? e.target_author : e.author;
                if (target_author == "" || e.target_timestamp <= 0) { return false; }
                var m = find (e.target_timestamp, target_author);
                if (m == null) {
                    // Arrival order may differ from message timestamp order.
                    if (pending.length < 2048) { pending.add (e); }
                    return true;
                }
                if (m.deleted) { return false; }
                if (e.kind == "delete") {
                    m.deleted = true;
                    m.text = "";
                    m.attachments = new Json.Array ();
                    m.reactions = new Json.Object ();
                } else if (e.kind == "edit" && e.timestamp > m.modified) {
                    m.text = e.text;
                    m.attachments = e.attachments;
                    m.modified = e.timestamp;
                } else if (e.kind == "reaction" && e.author != "" &&
                    e.timestamp > JsonFields.integer (m.reaction_versions, e.author)) {
                    m.reaction_versions.set_int_member (e.author, e.timestamp);
                    if (e.remove_reaction) { m.reactions.remove_member (e.author); }
                    else { m.reactions.set_string_member (e.author, e.emoji); }
                }
                return true;
            }
            var existing = find (e.timestamp, e.author);
            if (existing != null) {
                if (existing.view_once && existing.view_opened) {
                    foreach (var n in e.attachments.get_elements ()) { cleanup.add_element (n.copy ()); }
                    expiry_dirty = true;
                    return true;
                }
                if (e.expiration_seconds > 0) {
                    if (existing.expiration_seconds == 0) { existing.expiration_seconds = int64.min (int.MAX, e.expiration_seconds); }
                    if (e.expiration_start_ms > 0) { existing.start_expiration (e.expiration_start_ms, now); }
                    return true;
                }
                return false;
            }
            var message = new MessageRecord ();
            message.view_once = e.view_once;
            message.text = e.view_once ? "" : e.text;
            message.author = e.author;
            message.timestamp = e.timestamp;
            message.outgoing = e.outgoing;
            message.attachments = e.attachments;
            message.expiration_seconds = int64.max (0, int64.min (int.MAX, e.expiration_seconds));
            if (e.expiration_start_ms > 0) { message.start_expiration (e.expiration_start_ms, now); }
            else if (e.outgoing && message.expiration_seconds > 0) { message.start_expiration (now, now); }
            if (message.view_once && message.outgoing) { consume_view_once (message, false); }
            messages.add (message);
            messages.sort ((a, b) => a.timestamp < b.timestamp ? -1 : (a.timestamp > b.timestamp ? 1 : 0));
            for (uint i = pending.length; i > 0; i--) {
                var mutation = pending[i - 1];
                if (mutation.kind == "receipt" && message.outgoing) {
                    bool matches = false;
                    for (uint j = 0; j < mutation.receipt_timestamps.get_length (); j++) {
                        var n = mutation.receipt_timestamps.get_element (j);
                        if (n.get_node_type () == Json.NodeType.VALUE && n.get_value_type () == typeof (int64) &&
                            n.get_int () == message.timestamp) { matches = true; }
                    }
                    if (matches) { pending.remove_index (i - 1); apply (mutation); }
                    continue;
                }
                if (mutation.target_timestamp == message.timestamp &&
                    (mutation.kind == "reaction" ? mutation.target_author : mutation.author) == message.author) {
                    pending.remove_index (i - 1);
                    apply (mutation, now);
                }
            }
            return true;
        }

        public string serialize () {
            var root = new Json.Object ();
            root.set_int_member ("version", 1);
            root.set_string_member ("id", id);
            root.set_string_member ("name", name);
            root.set_boolean_member ("group", is_group);
            root.set_string_member ("draft", draft);
            root.set_string_member ("attachmentDraft", attachment_draft);
            root.set_int_member ("unread", unread);
            root.set_object_member ("expired", expired.ref ());
            root.set_array_member ("cleanup", cleanup.ref ());
            root.set_boolean_member ("viewOnceDraft", view_once_draft);
            root.set_array_member ("readSyncQueue", read_sync_queue.ref ());
            root.set_int_member ("clockFloor", clock_floor_ms);
            root.set_object_member ("metadata", metadata.ref ());
            var records = new Json.Array ();
            foreach (var m in messages) { records.add_object_element (m.to_json ()); }
            root.set_array_member ("messages", records);
            var events = new Json.Array ();
            foreach (var e in pending) {
                // Keep only the target and replacement, never the original content.
                var o = new Json.Object ();
                o.set_string_member ("kind", e.kind);
                o.set_string_member ("author", e.author);
                o.set_string_member ("targetAuthor", e.target_author);
                o.set_int_member ("target", e.target_timestamp);
                o.set_int_member ("timestamp", e.timestamp);
                o.set_string_member ("text", e.text ?? "");
                o.set_boolean_member ("readReceipt", e.read_receipt);
                o.set_array_member ("receiptTimestamps", e.receipt_timestamps.ref ());
                o.set_string_member ("emoji", e.emoji);
                o.set_boolean_member ("remove", e.remove_reaction);
                o.set_array_member ("attachments", e.attachments.ref ());
                events.add_object_element (o);
            }
            root.set_array_member ("pending", events);
            var node = new Json.Node (Json.NodeType.OBJECT);
            node.set_object (root);
            return JsonFields.encode (node);
        }

        public void restore (string json) throws Error {
            var parser = new Json.Parser ();
            parser.load_from_data (json);
            if (parser.get_root ().get_node_type () != Json.NodeType.OBJECT) {
                throw new IOError.INVALID_DATA ("Invalid conversation history");
            }
            var o = parser.get_root ().get_object ();
            if (JsonFields.integer (o, "version") != 1 || JsonFields.text (o, "id") != id) {
                throw new IOError.INVALID_DATA ("Unsupported conversation history");
            }
            name = JsonFields.text (o, "name", name);
            is_group = JsonFields.boolean (o, "group");
            draft = JsonFields.text (o, "draft");
            attachment_draft = JsonFields.text (o, "attachmentDraft");
            unread = (uint) JsonFields.integer (o, "unread");
            expired = JsonFields.object (o, "expired") ?? new Json.Object ();
            cleanup = JsonFields.array (o, "cleanup");
            view_once_draft = JsonFields.boolean (o, "viewOnceDraft");
            read_sync_queue = JsonFields.array (o, "readSyncQueue");
            clock_floor_ms = JsonFields.integer (o, "clockFloor");
            metadata = JsonFields.object (o, "metadata") ?? new Json.Object ();
            messages = new GenericArray<MessageRecord> ();
            var a = JsonFields.array (o, "messages");
            for (uint i = 0; i < a.get_length (); i++) {
                var n = a.get_element (i);
                if (n.get_node_type () == Json.NodeType.OBJECT) { messages.add (MessageRecord.from_json (n.get_object ())); }
            }
            pending = new GenericArray<MessageEvent> ();
            a = JsonFields.array (o, "pending");
            for (uint i = 0; i < a.get_length (); i++) {
                if (a.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                var p = a.get_object_element (i);
                var e = new MessageEvent ();
                e.kind = JsonFields.text (p, "kind");
                e.author = JsonFields.text (p, "author");
                e.target_author = JsonFields.text (p, "targetAuthor");
                e.target_timestamp = JsonFields.integer (p, "target");
                e.timestamp = JsonFields.integer (p, "timestamp");
                e.text = JsonFields.text (p, "text");
                e.read_receipt = JsonFields.boolean (p, "readReceipt");
                e.receipt_timestamps = JsonFields.array (p, "receiptTimestamps");
                e.emoji = JsonFields.text (p, "emoji");
                e.remove_reaction = JsonFields.boolean (p, "remove");
                e.attachments = JsonFields.array (p, "attachments");
                pending.add (e);
            }
        }
    }
}
