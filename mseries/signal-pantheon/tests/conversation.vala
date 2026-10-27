using SignalNative;
private MessageEvent event (string kind, string author, int64 timestamp, string text = "") {
    var e = new MessageEvent ();
    e.kind = kind; e.author = author; e.timestamp = timestamp; e.text = text;
    return e;
}
int main (string[] args) {
    Test.init (ref args);
    Test.add_func ("/conversation/identity-and-dedup", () => {
        var c = new Conversation ("group", "Friends", true);
        assert (c.apply (event ("message", "alice", 100, "A")));
        assert (c.apply (event ("message", "bob", 100, "B")));
        assert (!c.apply (event ("message", "alice", 100, "A")));
        assert (c.messages.length == 2);
    });
    Test.add_func ("/conversation/ordered-edits-and-delete", () => {
        var c = new Conversation ("a", "A", false);
        c.apply (event ("message", "alice", 100, "Original"));
        var edit = event ("edit", "alice", 300, "Latest"); edit.target_timestamp = 100;
        c.apply (edit);
        edit = event ("edit", "alice", 200, "Stale"); edit.target_timestamp = 100;
        c.apply (edit);
        assert (c.find (100, "alice").text == "Latest");
        var remove = event ("delete", "mallory", 400); remove.target_timestamp = 100;
        c.apply (remove);
        assert (!c.find (100, "alice").deleted);
        remove = event ("delete", "alice", 400); remove.target_timestamp = 100;
        c.apply (remove);
        assert (c.find (100, "alice").deleted);
        assert (!c.serialize ().contains ("Original") && !c.serialize ().contains ("Latest"));
        c.apply (edit);
        assert (c.find (100, "alice").text == "");
    });
    Test.add_func ("/conversation/mutation-before-original", () => {
        var c = new Conversation ("a", "A", false);
        var remove = event ("delete", "alice", 400); remove.target_timestamp = 100;
        c.apply (remove);
        string saved = c.serialize ();
        var restored = new Conversation ("a", "A", false);
        try { restored.restore (saved); } catch (Error error) { assert_not_reached (); }
        restored.apply (event ("message", "alice", 100, "Must not survive"));
        assert (restored.find (100, "alice").deleted);
        assert (!restored.serialize ().contains ("Must not survive"));
    });
    Test.add_func ("/conversation/reaction-ordering", () => {
        var c = new Conversation ("a", "A", false);
        c.apply (event ("message", "alice", 100, "Hi"));
        var reaction = event ("reaction", "bob", 300); reaction.target_timestamp = 100;
        reaction.target_author = "alice"; reaction.emoji = "👍"; reaction.remove_reaction = true;
        c.apply (reaction);
        reaction = event ("reaction", "bob", 200); reaction.target_timestamp = 100;
        reaction.target_author = "alice"; reaction.emoji = "👍";
        c.apply (reaction);
        assert (c.find (100, "alice").reactions.get_size () == 0);
    });
    Test.add_func ("/conversation/receipt-does-not-regress", () => {
        var c = new Conversation ("a", "A", false);
        var sent = event ("message", "self", 100, "Hi"); sent.outgoing = true;
        c.apply (sent);
        var receipt = event ("receipt", "alice", 300); receipt.receipt_timestamps.add_int_element (100);
        receipt.read_receipt = true; c.apply (receipt);
        receipt.read_receipt = false; c.apply (receipt);
        assert (c.find (100, "self").status == "read");
    });
    Test.add_func ("/conversation/roundtrip-drafts-attachments", () => {
        var c = new Conversation ("a", "A", false);
        c.draft = "Draft 🔒"; c.attachment_draft = "/tmp/file.txt"; c.unread = 2;
        var e = event ("message", "alice", 100);
        var a = new Json.Object (); a.set_string_member ("id", "file-id"); a.set_string_member ("filename", "hello.txt");
        e.attachments.add_object_element (a); c.apply (e);
        var restored = new Conversation ("a", "", false);
        try { restored.restore (c.serialize ()); } catch (Error error) { assert_not_reached (); }
        assert (restored.draft == c.draft && restored.attachment_draft == c.attachment_draft && restored.unread == 2);
        assert (restored.messages[0].attachments.get_object_element (0).get_string_member ("id") == "file-id");
    });
    Test.add_func ("/commands/parameters-and-partial", () => {
        var c = new Conversation ("group-id", "G", true);
        var o = MessageCommands.send ("account", c, "Hi", "/tmp/test", 100).get_object ();
        assert (o.get_string_member ("groupId") == "group-id");
        assert (o.get_int_member ("editTimestamp") == 100);
        assert (o.get_array_member ("attachments").get_string_element (0) == "/tmp/test");
        try {
            var parser = new Json.Parser ();
            parser.load_from_data ("""{"timestamp":123,"results":[{"type":"SUCCESS"},{"type":"NETWORK_FAILURE"}]}""");
            bool partial;
            assert (MessageCommands.confirm (parser.get_root (), out partial) == 123 && partial);
            parser.load_from_data ("""{"timestamp":124,"results":[]}""");
            assert (MessageCommands.confirm (parser.get_root (), out partial, true) == 124 && !partial);
            bool empty_send_failed = false;
            try { MessageCommands.confirm (parser.get_root (), out partial); } catch (Error error) { empty_send_failed = true; }
            assert (empty_send_failed);
            parser.load_from_data ("{}");
            bool failed = false;
            try { MessageCommands.confirm (parser.get_root (), out partial); } catch (Error error) { failed = true; }
            assert (failed);
        } catch (Error error) { assert_not_reached (); }
    });
    Test.add_func ("/conversation/receipt-before-original", () => {
        var c = new Conversation ("a", "A", false);
        var receipt = event ("receipt", "alice", 300); receipt.receipt_timestamps.add_int_element (100);
        receipt.read_receipt = true; c.apply (receipt);
        var restored = new Conversation ("a", "A", false);
        try { restored.restore (c.serialize ()); } catch (Error error) { assert_not_reached (); }
        var sent = event ("message", "self", 100, "Hi"); sent.outgoing = true;
        restored.apply (sent);
        assert (restored.find (100, "self").status == "read");
    });
    Test.add_func ("/conversation/ephemeral-not-stored", () => {
        var c = new Conversation ("a", "A", false);
        var e = event ("message", "alice", 100, "Secret"); e.ephemeral = true;
        assert (!c.apply (e)); assert (!c.serialize ().contains ("Secret"));
    });
    return Test.run ();
}
