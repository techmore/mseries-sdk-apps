namespace SignalNative {
    public class MessageCommands : Object {
        public static Json.Object destination (string account, string recipient, bool group) {
            var o = new Json.Object ();
            o.set_string_member ("account", account);
            if (group) { o.set_string_member ("groupId", recipient); }
            else {
                var a = new Json.Array ();
                a.add_string_element (recipient);
                o.set_array_member ("recipient", a);
            }
            return o;
        }

        public static Json.Node node (Json.Object o) {
            var n = new Json.Node (Json.NodeType.OBJECT);
            n.set_object (o);
            return n;
        }

        public static Json.Node send (string account, Conversation conversation, string text,
                                      string attachment = "", int64 edit_timestamp = 0) {
            var o = destination (account, conversation.id, conversation.is_group);
            o.set_string_member ("message", text);
            if (attachment != "") {
                var a = new Json.Array ();
                a.add_string_element (attachment);
                o.set_array_member ("attachments", a);
            }
            if (edit_timestamp > 0) { o.set_int_member ("editTimestamp", edit_timestamp); }
            return node (o);
        }

        public static Json.Node reaction (string account, Conversation conversation,
                                           MessageRecord message, string emoji, bool remove) {
            var o = destination (account, conversation.id, conversation.is_group);
            o.set_string_member ("emoji", emoji);
            o.set_string_member ("targetAuthor", message.author);
            o.set_int_member ("targetTimestamp", message.timestamp);
            o.set_boolean_member ("remove", remove);
            return node (o);
        }

        public static Json.Node delete_message (string account, Conversation conversation, MessageRecord message) {
            var o = destination (account, conversation.id, conversation.is_group);
            o.set_int_member ("targetTimestamp", message.timestamp);
            return node (o);
        }

        public static Json.Node receipt (string account, string author, int64 timestamp) {
            var o = new Json.Object ();
            o.set_string_member ("account", account);
            o.set_string_member ("recipient", author);
            o.set_string_member ("type", "read");
            var a = new Json.Array ();
            a.add_int_element (timestamp);
            o.set_array_member ("targetTimestamp", a);
            return node (o);
        }

        public static int64 confirm (Json.Node? response, out bool partial, bool allow_empty_receipt = false) throws Error {
            partial = false;
            if (response == null || response.get_node_type () != Json.NodeType.OBJECT) {
                throw new IOError.INVALID_DATA ("Signal returned no delivery result; check before retrying");
            }
            var o = response.get_object ();
            int64 timestamp = JsonFields.integer (o, "timestamp");
            var results = JsonFields.array (o, "results");
            uint success = 0;
            for (uint i = 0; i < results.get_length (); i++) {
                if (results.get_element (i).get_node_type () == Json.NodeType.OBJECT &&
                    JsonFields.text (results.get_object_element (i), "type") == "SUCCESS") { success++; }
            }
            if (timestamp <= 0 || (success == 0 && !(allow_empty_receipt && o.has_member ("results") && results.get_length () == 0))) {
                throw new IOError.FAILED ("Signal did not confirm success. Check the conversation before retrying.");
            }
            partial = success < results.get_length ();
            return timestamp;
        }
    }
}
