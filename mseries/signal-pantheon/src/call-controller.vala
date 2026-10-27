namespace SignalNative {
    public class VoiceCall : Object {
        public int64 id;
        public string peer = "";
        public string state = "IDLE";
        public string input_device = "";
        public string output_device = "";
        public string reason = "";
        public bool outgoing = false;
        public bool video = false;
        public bool remote_video_known = false;
        public bool remote_video = false;
        public string video_socket = "";
        public string group_id = "";
        public Json.Array participants = new Json.Array ();
        public bool active { get { return state != "IDLE" && state != "ENDED"; } }

        public static VoiceCall? parse (Json.Node? node) {
            if (node == null || node.get_node_type () != Json.NodeType.OBJECT) { return null; }
            var o = node.get_object ();
            if (!o.has_member ("callId") || o.get_member ("callId").get_value_type () != typeof (int64)) { return null; }
            string state = JsonFields.text (o, "state");
            if (state != "RINGING_INCOMING" && state != "RINGING_OUTGOING" && state != "CONNECTING" &&
                state != "CONNECTED" && state != "RECONNECTING" && state != "ENDED") { return null; }
            var c = new VoiceCall ();
            c.id = JsonFields.integer (o, "callId");
            c.peer = JsonFields.address (o, "number", "uuid", "recipient");
            c.state = state;
            c.outgoing = JsonFields.boolean (o, "isOutgoing");
            c.video = JsonFields.boolean (o, "video");
            c.remote_video_known = o.has_member ("remoteVideo");
            c.remote_video = JsonFields.boolean (o, "remoteVideo");
            c.video_socket = JsonFields.text (o, "videoSocket");
            c.group_id = JsonFields.text (o, "groupId");
            c.participants = JsonFields.array (o, "participants");
            c.input_device = JsonFields.text (o, "inputDeviceName");
            c.output_device = JsonFields.text (o, "outputDeviceName");
            c.reason = JsonFields.text (o, "reason");
            return c;
        }
    }

    public class CallController : Object {
        public CallVideo video_media = new CallVideo ();
        private bool video_starting = false;
        private bool video_ready = false;
        private SignalBackend backend;
        private CallMedia media;
        private string account;
        private int64 subscription = -1;
        private bool requesting = false;
        private bool starting_outgoing = false;
        private bool requested_video = false;
        private bool requested_group = false;
        private string requested_peer = "";
        private bool initializing = false;
        private bool media_permitted = false;
        private GenericArray<Json.Node> early_events = new GenericArray<Json.Node> ();
        private HashTable<string, bool> ended = new HashTable<string, bool> (str_hash, str_equal);
        public VoiceCall? current { get; private set; }
        public bool available { get; private set; default = false; }
        public string error_message { get; private set; default = ""; }
        public signal void changed ();
        public bool active { get { return current != null && current.active; } }

        public CallController (SignalBackend backend, string account, CallMedia media) {
            this.backend = backend; this.account = account; this.media = media;
            backend.call_event.connect (receive);
            backend.disconnected.connect (() => { disconnected (); });
            video_media.failed.connect ((message) => { error_message = message; camera.begin (false); });
            media.failed.connect ((message) => { error_message = message; hangup.begin (); });
        }

        public async void initialize () {
            if (initializing || available) { return; }
            initializing = true;
            try {
                var response = yield backend.rpc ("subscribeCallEvents", backend.account_params (account));
                if (response == null || response.get_value_type () != typeof (int64)) {
                    throw new IOError.INVALID_DATA ("Signal returned no call subscription");
                }
                subscription = response.get_int ();
                available = true; error_message = "";
                var events = early_events; early_events = new GenericArray<Json.Node> ();
                foreach (var e in events) { receive (e); }
            } catch (Error error) { available = false; error_message = "Calls unavailable: " + error.message; }
            initializing = false;
            changed ();
        }

        private Json.Node id_params (int64 id) {
            var o = new Json.Object ();
            o.set_string_member ("account", account);
            o.set_int_member ("callId", id);
            return MessageCommands.node (o);
        }

        public async void start (string peer, bool video = false, bool group = false) {
            if (!available || active || requesting) { return; }
            requesting = true;
            starting_outgoing = true; requested_video = video; requested_group = group; requested_peer = peer;
            media_permitted = true;
            error_message = "";
            changed ();
            try {
                yield media.prepare ();
                var o = MessageCommands.destination (account, peer, group);
                o.set_boolean_member ("video", video);
                var response = yield backend.rpc ("startCall", MessageCommands.node (o));
                var call = VoiceCall.parse (response);
                if (call == null) { throw new IOError.INVALID_DATA ("Signal returned an invalid call"); }
                call.peer = peer; call.outgoing = true; call.video = video;
                if (group) { call.group_id = peer; }
                apply (call);
            } catch (Error error) { media_permitted = false; error_message = "Could not start call: " + error.message; media.stop (); }
            requesting = false; starting_outgoing = false;
            changed ();
        }

        public async void accept () {
            if (!active || current.state != "RINGING_INCOMING" || requesting) { return; }
            requesting = true;
            media_permitted = true;
            int64 id = current.id;
            try {
                yield media.prepare ();
                if (!active || current.id != id) { requesting = false; media_permitted = false; return; }
                var response = yield backend.rpc ("acceptCall", id_params (id));
                var call = VoiceCall.parse (response);
                if (call == null || call.id != id) { throw new IOError.INVALID_DATA ("Signal returned an invalid call acceptance"); }
                if (current != null && current.id == id && current.active) { apply (call); }
            } catch (Error error) { media_permitted = false; error_message = "Could not accept call: " + error.message; media.stop (); }
            requesting = false; changed ();
        }

        public async void reject () {
            if (!active || current.state != "RINGING_INCOMING") { return; }
            media_permitted = false;
            media.stop ();
            int64 id = current.id;
            try { yield backend.rpc ("rejectCall", id_params (id)); finish (id, "Declined"); }
            catch (Error error) { error_message = "Could not confirm rejection: " + error.message; }
            changed ();
        }

        public async void hangup () {
            media_permitted = false;
            media.stop (); video_media.stop (); video_ready = false;
            if (!active) { changed (); return; }
            int64 id = current.id;
            try { yield backend.rpc ("hangupCall", id_params (id)); finish (id, "Ended"); }
            catch (Error error) { error_message = "Could not confirm hangup: " + error.message; }
            changed ();
        }

        private void finish (int64 id, string reason) {
            ended.insert (id.to_string (), true);
            if (current != null && current.id == id) { media_permitted = false; }
            if (current != null && current.id == id) { current.state = "ENDED"; current.reason = reason; media.stop (); video_media.stop (); video_ready = false; }
        }

        private void receive (Json.Node event) {
            if (event.get_node_type () != Json.NodeType.OBJECT) { return; }
            var parameters = JsonFields.object (event.get_object (), "params");
            if (parameters == null) { return; }
            if (subscription < 0) { early_events.add (event.copy ()); return; }
            if (JsonFields.integer (parameters, "subscription", -2) != subscription) { return; }
            var call = VoiceCall.parse (parameters.get_member ("result"));
            if (call == null) { return; }
            // A connected event can arrive before the start RPC continuation resumes.
            if (starting_outgoing && call.state != "RINGING_INCOMING") {
                call.outgoing = true; call.video = call.video || requested_video;
                if (call.peer == "") { call.peer = requested_peer; }
                if (requested_group) { call.group_id = requested_peer; }
            }
            if (requesting && (current == null || !current.active) && call.state == "RINGING_INCOMING") { reject_extra.begin (call.id); return; }
            if (active && current.id != call.id) {
                if (call.state == "RINGING_INCOMING") { reject_extra.begin (call.id); }
                return;
            }
            apply (call);
        }

        private async void reject_extra (int64 id) {
            try { yield backend.rpc ("rejectCall", id_params (id)); } catch (Error error) {}
        }

        private void apply (VoiceCall call) {
            if (ended.contains (call.id.to_string ())) { return; }
            if (current != null && current.id == call.id) {
                if (call.peer == "") { call.peer = current.peer; }
                if (call.input_device == "") { call.input_device = current.input_device; }
                if (call.output_device == "") { call.output_device = current.output_device; }
                if (call.video_socket == "") { call.video_socket = current.video_socket; }
                if (call.group_id == "") { call.group_id = current.group_id; }
                call.video = call.video || current.video;
                call.outgoing = current.outgoing;
                if ((current.state == "CONNECTED" || current.state == "RECONNECTING") &&
                    (call.state == "CONNECTING" || call.state == "RINGING_INCOMING" || call.state == "RINGING_OUTGOING")) { return; }
                if (current.state == "CONNECTING" && call.state.has_prefix ("RINGING")) { return; }
            }
            if ((current == null || current.id != call.id) && call.state == "RINGING_INCOMING") { media_permitted = false; error_message = ""; }
            current = call;
            if (call.state == "ENDED") { finish (call.id, call.reason); }
            else if (call.state == "CONNECTED" && !media.running) {
                if (!media_permitted) { error_message = "Call connected without local acceptance"; hangup.begin (); return; }
                try { media.start (call.input_device, call.output_device); }
                catch (Error error) { error_message = "Call audio could not start: " + error.message; hangup.begin (); }
            }
            if (call.state == "CONNECTED" && (call.video || call.group_id != "") && !video_ready && !video_starting) { start_video.begin (call); }
            changed ();
        }

        private async void start_video (VoiceCall call) {
            video_starting = true;
            try {
                yield video_media.connect_to (call.video_socket);
                if (!active || current.id != call.id) { video_media.stop (); }
                else { video_ready = true; if (call.video && call.outgoing) { yield camera (true); } }
            } catch (Error error) { if (active && current.id == call.id) { error_message = "Video unavailable: " + error.message; } }
            video_starting = false;
            if (active && current.id != call.id && current.state == "CONNECTED" && (current.video || current.group_id != "") && !video_ready) { start_video.begin (current); }
            changed ();
        }
        public async void camera (bool enabled) {
            if (!active || !video_ready) { return; }
            try {
                video_media.set_camera (enabled);
                changed ();
                var parameters = id_params (current.id); parameters.get_object ().set_boolean_member ("enabled", enabled);
                yield backend.rpc ("setCallVideo", parameters);
                error_message = "";
            } catch (Error error) { try { video_media.set_camera (false); } catch (Error ignored) {} error_message = "Camera unavailable: " + error.message; }
            changed ();
        }

        public async void select_camera (CameraChoice choice) {
            if (!active) { return; }
            int64 id = current.id;
            bool resume = video_media.camera_enabled;
            if (resume) { yield camera (false); }
            video_media.choose_camera (choice);
            if (resume && active && current.id == id) { yield camera (true); }
            changed ();
        }

        public void mute_microphone (bool muted) { media.mute_microphone (muted); advertise_audio.begin (!muted); }
        private async void advertise_audio (bool enabled) {
            if (!active || current.state != "CONNECTED") { return; }
            int64 id = current.id;
            try {
                var parameters = id_params (id); parameters.get_object ().set_boolean_member ("enabled",enabled);
                yield backend.rpc ("setCallAudio",parameters);
            } catch (Error error) { if (active && current.id == id) { error_message = "Microphone state could not be synchronized"; changed (); } }
        }
        public void mute_speaker (bool muted) { media.mute_speaker (muted); }
        public void disconnected () {
            media.stop (); video_media.stop (); video_ready = false; available = false; subscription = -1; early_events = new GenericArray<Json.Node> ();
            if (active) { finish (current.id, "Disconnected"); }
            changed ();
        }
    }
}
