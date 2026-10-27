namespace SignalNative {
    public class ExpirationClock : Object {
        private int64 wall_anchor = GLib.get_real_time () / 1000;
        private int64 mono_anchor = GLib.get_monotonic_time () / 1000;
        public virtual int64 now_ms () {
            int64 mono = GLib.get_monotonic_time () / 1000;
            int64 wall = GLib.get_real_time () / 1000;
            int64 predicted = wall_anchor + mono - mono_anchor;
            if (wall > predicted) {
                // Retain forward clock jumps so a later rollback cannot extend timers.
                wall_anchor = wall; mono_anchor = mono;
                return wall;
            }
            return predicted;
        }
    }

    public class ExpirationService : Object {
        private ClientStore store;
        private string attachments_directory;
        private string outgoing_directory;
        private ExpirationClock clock;
        private uint timer = 0;
        private bool ticking = false;
        private int64 last_checkpoint = 0;
        private HashTable<Conversation, bool> dirty = new HashTable<Conversation, bool> (direct_hash, direct_equal);
        private signal void tick_finished ();
        public signal void pulse (int64 now);
        public signal void changed (Conversation conversation);
        public signal void failed (string message);

        public ExpirationService (ClientStore store, string attachments_directory,
                                  string outgoing_directory, ExpirationClock? clock = null) {
            this.store = store;
            this.attachments_directory = attachments_directory;
            this.outgoing_directory = outgoing_directory;
            this.clock = clock ?? new ExpirationClock ();
        }

        public int64 now_ms () { return clock.now_ms (); }
        public void start () {
            if (timer != 0) { return; }
            tick.begin (true);
            timer = Timeout.add (500, () => { tick.begin (); return Source.CONTINUE; });
        }
        public void stop () { if (timer != 0) { Source.remove (timer); timer = 0; } }

        private bool referenced (Json.Object attachment) {
            string id = JsonFields.text (attachment, "id");
            string path = JsonFields.text (attachment, "localPath");
            foreach (var c in store.conversations) {
                foreach (var m in c.messages) {
                    if (m.deleted) { continue; }
                    for (uint i = 0; i < m.attachments.get_length (); i++) {
                        if (m.attachments.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                        var a = m.attachments.get_object_element (i);
                        if ((id != "" && JsonFields.text (a, "id") == id) ||
                            (path != "" && JsonFields.text (a, "localPath") == path)) { return true; }
                    }
                }
            }
            return false;
        }

        public async void tick (bool after_restart = false) {
            if (ticking) {
                ulong waiter = tick_finished.connect (() => { tick.callback (); });
                yield;
                disconnect (waiter);
                return;
            }
            ticking = true;
            pulse (clock.now_ms ());
            foreach (var c in store.conversations) {
                bool removed = c.expire (clock.now_ms (), after_restart);
                if (removed) { dirty.insert (c, true); changed (c); }
                bool checkpoint = clock.now_ms () - last_checkpoint >= 60000;
                if (!after_restart && !checkpoint && !dirty.contains (c) && !c.expiry_dirty && c.cleanup.get_length () == 0) { continue; }
                try {
                    // Persist expiry before touching owned attachment files. Failure is retried.
                    store.save (c);
                    dirty.remove (c); c.expiry_dirty = false;
                    for (uint i = c.cleanup.get_length (); i > 0; i--) {
                        var n = c.cleanup.get_element (i - 1);
                        if (n.get_node_type () != Json.NodeType.OBJECT) { c.cleanup.remove_element (i - 1); continue; }
                        var a = n.get_object ();
                        if (referenced (a)) { continue; }
                        try {
                            yield AttachmentFiles.expire_owned (attachments_directory, outgoing_directory, a);
                            c.cleanup.remove_element (i - 1);
                        } catch (IOError.NOT_FOUND error) {
                            c.cleanup.remove_element (i - 1);
                        } catch (Error error) { failed ("Expired attachment cleanup will retry: " + error.message); }
                    }
                    store.save (c);
                } catch (Error error) { failed ("Expired history could not be saved: " + error.message); }
            }
            if (clock.now_ms () - last_checkpoint >= 60000) { last_checkpoint = clock.now_ms (); }
            ticking = false;
            tick_finished ();
        }
    }
}
