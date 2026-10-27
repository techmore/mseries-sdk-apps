using SignalNative;
private MessageEvent message (string author, int64 timestamp, int64 ttl, bool outgoing = false) {
    var e = new MessageEvent ();
    e.author = author; e.timestamp = timestamp; e.text = "Timed secret";
    e.expiration_seconds = ttl; e.outgoing = outgoing;
    return e;
}
int main (string[] args) {
    Test.init (ref args);
    Test.add_func ("/expiration/incoming-starts-on-read", () => {
        var c = new Conversation ("a", "A", false);
        c.apply (message ("alice", 1000, 60), 10000);
        assert (c.messages[0].expires_at_ms == 0);
        c.expire (100000);
        assert (c.messages.length == 1);
        c.mark_read (100000, false);
        assert (c.messages[0].expires_at_ms == 160000 && c.read_sync_queue.get_length () == 1);
        assert (!JsonFields.boolean (c.read_sync_queue.get_object_element (0), "sendRemote"));
        assert (!c.expire (159999));
        assert (c.expire (160000) && c.messages.length == 0);
        assert (!c.serialize ().contains ("Timed secret"));
    });
    Test.add_func ("/expiration/outgoing-start-and-replay", () => {
        var c = new Conversation ("a", "A", false);
        var e = message ("self", 1000, 10, true); e.expiration_start_ms = 2000;
        c.apply (e, 2000);
        assert (c.messages[0].expires_at_ms == 12000);
        c.expire (12000);
        assert (!c.apply (e, 13000) && c.messages.length == 0);
    });
    Test.add_func ("/expiration/restart-purges-before-use", () => {
        var c = new Conversation ("a", "A", false);
        c.apply (message ("self", 1000, 10, true), 2000);
        var restored = new Conversation ("a", "A", false);
        try { restored.restore (c.serialize ()); } catch (Error e) { assert_not_reached (); }
        assert (restored.expire (12000, true) && restored.messages.length == 0);
    });
    Test.add_func ("/expiration/read-sync-before-original", () => {
        var c = new Conversation ("a", "A", false);
        var sync = new MessageEvent (); sync.kind = "read_sync"; sync.author = "alice";
        sync.target_timestamp = 1000; sync.timestamp = 5000;
        c.apply (sync, 10000);
        c.apply (message ("alice", 1000, 10), 10000);
        assert (c.messages[0].expires_at_ms == 15000);
        sync.timestamp = 12000; c.apply (sync, 12000);
        assert (c.messages[0].expires_at_ms == 15000);
    });
    Test.add_func ("/expiration/edit-and-default-change-do-not-extend", () => {
        var c = new Conversation ("a", "A", false);
        c.apply (message ("self", 1000, 10, true), 2000);
        c.metadata.set_int_member ("messageExpirationTime", 1000);
        var edit = message ("self", 8000, 1000, true); edit.kind = "edit"; edit.target_timestamp = 1000;
        c.apply (edit, 8000);
        assert (c.messages[0].expires_at_ms == 12000);
        var duplicate = message ("self", 1000, 1000, true);
        duplicate.expiration_start_ms = 8000;
        c.apply (duplicate, 8000);
        assert (c.messages[0].expires_at_ms == 12000);
    });
    Test.add_func ("/expiration/clock-rollback-is-conservative", () => {
        var c = new Conversation ("a", "A", false);
        c.apply (message ("self", 1000, 60, true), 5000);
        c.expire (10000);
        assert (c.expire (3000, true) && c.messages.length == 0);
    });
    Test.add_func ("/expiration/attachment-cleanup-recorded", () => {
        var c = new Conversation ("a", "A", false);
        var e = message ("self", 1000, 1, true);
        var a = new Json.Object (); a.set_string_member ("id", "attachment-id");
        e.attachments.add_object_element (a); c.apply (e, 2000);
        c.expire (3000);
        assert (c.cleanup.get_length () == 1 && c.messages.length == 0);
    });
    Test.add_func ("/expiration/parser-linked-sync-start", () => {
        try {
            var parser = new Json.Parser ();
            parser.load_from_data ("""{"params":{"envelope":{"sourceUuid":"self","syncMessage":{"sentMessage":{"destinationNumber":"alice","expirationStartTimestamp":3000,"timestamp":1000,"message":"Hi","expiresInSeconds":10}}}}}""");
            var e = MessageEvent.parse (parser.get_root (), "self");
            assert (e != null && e.expiration_start_ms == 3000 && e.expiration_seconds == 10);
        } catch (Error e) { assert_not_reached (); }
    });
    return Test.run ();
}
