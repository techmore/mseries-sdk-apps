namespace SignalNative {
    // JSON is a service boundary: optional fields can be missing, null or unknown.
    public class JsonFields : Object {
        public static string text (Json.Object object, string name, string fallback = "") {
            Json.Node? node = object.get_member (name);
            if (node == null || node.get_node_type () != Json.NodeType.VALUE ||
                node.get_value_type () != typeof (string)) {
                return fallback;
            }
            return node.get_string () ?? fallback;
        }

        public static Json.Object? object (Json.Object parent, string name) {
            Json.Node? node = parent.get_member (name);
            return node != null && node.get_node_type () == Json.NodeType.OBJECT
                ? node.get_object () : null;
        }

        public static int64 integer (Json.Object object, string name, int64 fallback = 0) {
            Json.Node? node = object.get_member (name);
            return node != null && node.get_node_type () == Json.NodeType.VALUE &&
                node.get_value_type () == typeof (int64) ? node.get_int () : fallback;
        }

        public static bool boolean (Json.Object parent, string name) {
            Json.Node? node = parent.get_member (name);
            return node != null && node.get_node_type () == Json.NodeType.VALUE &&
                node.get_value_type () == typeof (bool) && node.get_boolean ();
        }

        public static Json.Array array (Json.Object parent, string name) {
            Json.Node? node = parent.get_member (name);
            return node != null && node.get_node_type () == Json.NodeType.ARRAY
                ? node.get_array () : new Json.Array ();
        }

        public static string encode (Json.Node node) {
            var generator = new Json.Generator ();
            generator.set_root (node);
            return generator.to_data (null);
        }

        public static string address (Json.Object object, string number, string uuid, string legacy) {
            string value = text (object, number);
            if (value == "") { value = text (object, uuid); }
            if (value == "") { value = text (object, legacy); }
            return value;
        }
    }

    public class MessageEvent : Object {
        public string conversation_id = "";
        public string text = "";
        public string name = "";
        public bool outgoing;
        public bool is_group;
        public bool ephemeral;
        public bool view_once = false;
        public int64 expiration_start_ms = 0;
        public Json.Array read_markers = new Json.Array ();
        public int64 expiration_seconds = 0;
        public int64 timestamp;
        public string author = "";
        public string kind = "message";
        public string target_author = "";
        public int64 target_timestamp = 0;
        public string emoji = "";
        public bool remove_reaction = false;
        public bool read_receipt = false;
        public Json.Array attachments = new Json.Array ();
        public Json.Array receipt_timestamps = new Json.Array ();

        public static MessageEvent? parse (Json.Node event, string account) {
            if (event.get_node_type () != Json.NodeType.OBJECT) { return null; }
            var parameters = JsonFields.object (event.get_object (), "params");
            if (parameters == null) { return null; }
            var result = JsonFields.object (parameters, "result");
            var container = result ?? parameters;
            string event_account = JsonFields.text (container, "account",
                JsonFields.text (parameters, "account"));
            if (event_account != "" && event_account != account) { return null; }
            var envelope = JsonFields.object (container, "envelope");
            if (envelope == null) { return null; }
            var message = new MessageEvent ();
            message.author = JsonFields.address (envelope, "sourceNumber", "sourceUuid", "source");
            var receipt = JsonFields.object (envelope, "receiptMessage");
            if (receipt != null) {
                message.kind = "receipt";
                message.receipt_timestamps = JsonFields.array (receipt, "timestamps");
                message.read_receipt = JsonFields.boolean (receipt, "isRead");
                if (!message.read_receipt && !JsonFields.boolean (receipt, "isDelivery")) { return null; }
                message.conversation_id = message.author;
                message.timestamp = JsonFields.integer (receipt, "when");
                return message;
            }
            var data = JsonFields.object (envelope, "dataMessage");
            var edit = JsonFields.object (envelope, "editMessage");
            bool outgoing = false;
            Json.Object? sent = null;
            if (data == null && edit == null) {
                var sync = JsonFields.object (envelope, "syncMessage");
                if (sync == null) { return null; }
                message.read_markers = JsonFields.array (sync, "readMessages");
                var viewed = JsonFields.array (sync, "viewedMessages");
                for (uint i = 0; i < viewed.get_length (); i++) { message.read_markers.add_element (viewed.get_element (i).copy ()); }
                if (message.read_markers.get_length () > 0) {
                    message.kind = "read_sync";
                    message.timestamp = JsonFields.integer (envelope, "timestamp", GLib.get_real_time () / 1000);
                    return message;
                }
                var opened = JsonFields.object (sync, "viewOnceOpenMessage");
                if (opened != null) {
                    message.kind = "view_once_open";
                    message.author = JsonFields.address (opened, "senderNumber", "senderUuid", "sender");
                    message.target_timestamp = JsonFields.integer (opened, "timestamp");
                    return message;
                }
                sent = JsonFields.object (sync, "sentMessage");
                if (sent == null) { return null; }
                data = sent;
                edit = JsonFields.object (sent, "editMessage");
                outgoing = true;
            }
            if (edit != null) {
                data = JsonFields.object (edit, "dataMessage");
                message.kind = "edit";
                message.target_timestamp = JsonFields.integer (edit, "targetSentTimestamp");
            }
            if (data == null) { return null; }
            message.text = JsonFields.text (data, "message");
            message.outgoing = outgoing;
            if (outgoing) { message.author = account; }
            message.expiration_seconds = JsonFields.integer (data, "expiresInSeconds");
            message.view_once = JsonFields.boolean (data, "viewOnce");
            message.ephemeral = false;
            if (sent != null) { message.expiration_start_ms = JsonFields.integer (sent, "expirationStartTimestamp"); }
            message.attachments = JsonFields.array (data, "attachments");
            var reaction = JsonFields.object (data, "reaction");
            var deletion = JsonFields.object (data, "remoteDelete");
            if (reaction != null) {
                message.kind = "reaction";
                message.target_timestamp = JsonFields.integer (reaction, "targetSentTimestamp");
                message.target_author = JsonFields.address (reaction, "targetAuthorNumber", "targetAuthorUuid", "targetAuthor");
                message.emoji = JsonFields.text (reaction, "emoji");
                message.remove_reaction = JsonFields.boolean (reaction, "isRemove");
            } else if (JsonFields.object (data, "groupCallUpdate") != null) {
                message.kind = "group_call";
            } else if (JsonFields.boolean (data, "isExpirationUpdate")) {
                message.kind = "timer";
            } else if (deletion != null) {
                message.kind = "delete";
                message.target_timestamp = JsonFields.integer (deletion, "targetTimestamp",
                    JsonFields.integer (deletion, "timestamp"));
            } else if (message.text == "" && message.attachments.get_length () == 0 && !message.view_once) { return null; }
            var group = JsonFields.object (data, "groupInfo");
            message.conversation_id = group != null ? JsonFields.text (group, "groupId") : "";
            message.is_group = message.conversation_id != "";
            if (!message.is_group) {
                message.conversation_id = outgoing
                    ? JsonFields.address (sent ?? data, "destinationNumber", "destinationUuid", "destination")
                    : JsonFields.address (envelope, "sourceNumber", "sourceUuid", "source");
            }
            if (message.conversation_id == "") { return null; }
            message.name = group != null ? JsonFields.text (group, "groupName")
                : JsonFields.text (envelope, "sourceName");
            if (message.name == "" || outgoing && !message.is_group) {
                message.name = message.conversation_id;
            }
            message.timestamp = JsonFields.integer (data, "timestamp",
                JsonFields.integer (envelope, "timestamp", GLib.get_real_time () / 1000));
            if (message.author == "") { return null; }
            if (message.view_once) { message.text = ""; }
            return message;
        }
    }
}
