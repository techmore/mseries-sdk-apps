using SignalNative;
private MessageEvent media (bool outgoing = false) {
    var e = new MessageEvent (); e.view_once = true; e.author = "alice"; e.timestamp = 1000;
    e.outgoing = outgoing; e.text = "No caption retained";
    var a = new Json.Object (); a.set_string_member ("id", "owned-photo"); a.set_string_member ("contentType", "image/png");
    e.attachments.add_object_element (a); return e;
}
int main (string[] args) {
    Test.init (ref args);
    Test.add_func ("/view-once/read-does-not-consume", () => {
        var c = new Conversation ("alice", "Alice", false); c.apply (media (), 2000); c.mark_read (3000, false);
        assert (c.messages[0].view_once && !c.messages[0].view_opened && c.messages[0].attachments.get_length () == 1);
        assert (!c.serialize ().contains ("No caption retained"));
    });
    Test.add_func ("/view-once/consume-persists-and-cannot-reopen", () => {
        var c = new Conversation ("alice", "Alice", false); c.apply (media (), 2000);
        assert (c.consume_view_once (c.messages[0])); assert (!c.consume_view_once (c.messages[0]));
        var restored = new Conversation ("alice", "Alice", false);
        try { restored.restore (c.serialize ()); } catch (Error e) { assert_not_reached (); }
        assert (restored.messages[0].view_opened && restored.messages[0].attachments.get_length () == 0);
        assert (restored.cleanup.get_length () == 1 && restored.read_sync_queue.get_length () == 1);
        assert (JsonFields.boolean (restored.read_sync_queue.get_object_element (0), "viewOnceOpen"));
    });
    Test.add_func ("/view-once/linked-open-before-original", () => {
        var c = new Conversation ("alice", "Alice", false);
        var opened = new MessageEvent (); opened.kind = "view_once_open"; opened.author = "alice"; opened.target_timestamp = 1000;
        c.apply (opened, 2000); c.apply (media (), 3000);
        assert (c.messages[0].view_opened && c.messages[0].attachments.get_length () == 0 && c.read_sync_queue.get_length () == 0);
        c.apply (media (), 4000); assert (c.messages[0].view_opened && c.cleanup.get_length () == 2);
    });
    Test.add_func ("/view-once/outgoing-is-immediately-consumed", () => {
        var c = new Conversation ("alice", "Alice", false); c.apply (media (true), 2000);
        assert (c.messages[0].view_opened && c.messages[0].attachments.get_length () == 0 && c.cleanup.get_length () == 1);
    });
    Test.add_func ("/view-once/unopened-thirty-day-limit", () => {
        var c = new Conversation ("alice", "Alice", false); c.apply (media (), 2000);
        assert (!c.expire (1000 + 30L * 24 * 60 * 60 * 1000 - 1));
        assert (c.expire (1000 + 30L * 24 * 60 * 60 * 1000));
        assert (!c.apply (media (), 1000 + 31L * 24 * 60 * 60 * 1000));
    });
    Test.add_func ("/view-once/parser-retains-media-and-open-sync", () => {
        try {
            var p = new Json.Parser (); p.load_from_data ("""{"params":{"envelope":{"sourceNumber":"alice","dataMessage":{"timestamp":1000,"viewOnce":true,"message":"caption","attachments":[{"id":"photo","contentType":"image/png"}]}}}}""");
            var e = MessageEvent.parse (p.get_root (), "self");
            assert (e.view_once && !e.ephemeral && e.attachments.get_length () == 1 && e.text == "");
            p.load_from_data ("""{"params":{"envelope":{"sourceNumber":"alice","dataMessage":{"timestamp":1001,"viewOnce":true}}}}""");
            e = MessageEvent.parse (p.get_root (), "self"); assert (e != null && e.view_once && e.attachments.get_length () == 0);
            p.load_from_data ("""{"params":{"envelope":{"sourceNumber":"self","syncMessage":{"viewOnceOpenMessage":{"senderNumber":"alice","timestamp":1000}}}}}""");
            e = MessageEvent.parse (p.get_root (), "self"); assert (e.kind == "view_once_open" && e.author == "alice" && e.target_timestamp == 1000);
        } catch (Error e) { assert_not_reached (); }
    });
    return Test.run ();
}
