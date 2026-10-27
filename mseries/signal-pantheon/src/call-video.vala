namespace SignalNative {
    public class CameraChoice : Object {
        public string name;
        public Gst.Device? device;
        public int pattern;
        public CameraChoice (string name, Gst.Device? device = null, int pattern = 0) {
            this.name = name; this.device = device; this.pattern = pattern;
        }
    }

    // Private RGBA stream; only explicit camera activation creates a capture pipeline.
    public class CallVideo : Object {
        private SocketConnection? connection;
        private Cancellable cancel = new Cancellable ();
        private Gst.Pipeline? camera;
        private Mutex write_lock;
        private uint generation = 0;
        private int frame_queued = 0;
        private int preview_queued = 0;
        private uint capture_generation = 0;
        private CameraChoice selected_camera = new CameraChoice ("Default camera");
        public signal void preview_frame (Gdk.Paintable? picture);

        public async GenericArray<CameraChoice> list_cameras () {
            unowned string[]? args = null; Gst.init (ref args);
            var choices = new GenericArray<CameraChoice> ();
            choices.add (new CameraChoice ("Default camera"));
            new Thread<void*> ("signal-camera-list", () => {
#if UI_TEST
                choices.add (new CameraChoice ("Fixture camera", null, 0));
                choices.add (new CameraChoice ("Second fixture camera", null, 18));
                choices.add (new CameraChoice ("Disconnected fixture camera", null, -1));
#else
                var monitor = new Gst.DeviceMonitor ();
                monitor.add_filter ("Video/Source", Gst.Caps.from_string ("video/x-raw"));
                if (monitor.start ()) {
                    foreach (var device in monitor.get_devices ()) { choices.add (new CameraChoice (device.get_display_name (), device)); }
                    monitor.stop ();
                }
#endif
                Idle.add (() => { list_cameras.callback (); return Source.REMOVE; });
                return null;
            });
            yield;
            return choices;
        }
        public void choose_camera (CameraChoice choice) { selected_camera = choice; }
        public bool camera_enabled { get; private set; default = false; }
        public signal void frame (uint track, Gdk.Paintable picture);
        public signal void failed (string message);

        public async void connect_to (string path) throws Error {
            stop (false);
            unowned string[]? gst_args = null; Gst.init (ref gst_args);
            if (path == "") { throw new IOError.NOT_SUPPORTED ("Backend did not provide video transport"); }
            cancel = new Cancellable ();
            var client = new SocketClient () { timeout = 2 };
            connection = yield client.connect_async (new UnixSocketAddress (path), cancel);
            connection.socket.timeout = 0;
            var stream = connection.input_stream;
            var token = cancel;
            uint expected = generation;
            new Thread<void*> ("signal-video-receive", () => {
                try {
                    while (!token.is_cancelled ()) {
                        uint8[] header = new uint8[16]; size_t got;
                        if (!stream.read_all (header, out got, token) || got != 16) { break; }
                        uint w = word (header, 4), h = word (header, 8), track = word (header, 12);
                        if (word (header, 0) != 0x53565031 || w == 0 || h == 0 || w > 1280 || h > 720) { throw new IOError.INVALID_DATA ("Invalid video frame"); }
                        uint8[] pixels = new uint8[w * h * 4];
                        if (!stream.read_all (pixels, out got, token) || got != pixels.length) { break; }
                        if (!AtomicInt.compare_and_exchange (ref frame_queued, 0, 1)) { continue; }
                        var bytes = new Bytes.take ((owned) pixels);
                        Idle.add (() => {
                            if (generation == expected && !token.is_cancelled ()) {
                                frame (track, new Gdk.MemoryTexture ((int) w, (int) h, Gdk.MemoryFormat.R8G8B8A8, bytes, w * 4));
                            }
                            AtomicInt.set (ref frame_queued, 0);
                            return Source.REMOVE;
                        });
                    }
                } catch (Error error) {
                    if (!token.is_cancelled ()) { Idle.add (() => { if (generation == expected) { failed ("Video connection failed"); } return Source.REMOVE; }); }
                }
                return null;
            });
        }
        private static uint word (uint8[] data, int offset) {
            return ((uint) data[offset] << 24) | ((uint) data[offset+1] << 16) | ((uint) data[offset+2] << 8) | data[offset+3];
        }
        public void set_camera (bool enabled) throws Error {
            capture_generation++;
            preview_frame (null);
            if (camera != null) { camera.get_bus ().remove_watch (); camera.set_state (Gst.State.NULL); camera = null; }
            camera_enabled = false;
            if (!enabled) { return; }
            if (connection == null) { throw new IOError.NOT_CONNECTED ("Video transport is not connected"); }
            Gst.Element? source;
#if UI_TEST
            source = selected_camera.pattern < 0 ? null : Gst.ElementFactory.make ("videotestsrc", "camera-source");
            if (source != null) { source.set ("is-live", true, "pattern", selected_camera.pattern); }
#else
            source = selected_camera.device != null ? selected_camera.device.create_element ("camera-source") : Gst.ElementFactory.make ("autovideosrc", "camera-source");
#endif
            if (source == null) { throw new IOError.NOT_FOUND ("Selected camera is unavailable. Choose another camera."); }
            camera = (Gst.Pipeline) Gst.parse_launch ("videoconvert name=convert ! videoscale ! videorate ! video/x-raw,format=RGBA,width=320,height=240,framerate=15/1 ! appsink name=frames emit-signals=true max-buffers=1 drop=true");
            camera.add (source);
            if (!source.link (camera.get_by_name ("convert"))) { camera.set_state (Gst.State.NULL); camera = null; throw new IOError.FAILED ("Selected camera could not be connected"); }
            uint expected_capture = capture_generation;
            var sink = (Gst.App.Sink) camera.get_by_name ("frames");
            var socket = connection.socket;
            var token = cancel;
            sink.new_sample.connect (() => {
                var sample = sink.pull_sample ();
                var buffer = sample.get_buffer ();
                Gst.MapInfo info;
                if (!buffer.map (out info, Gst.MapFlags.READ)) { return Gst.FlowReturn.ERROR; }
                uint8[] header = {0x53,0x56,0x50,0x31,0,0,1,64,0,0,0,240,0,0,0,0};
                try {
                    write_lock.lock ();
                    write_frame (socket, header, token);
                    write_frame (socket, info.data, token);
                } catch (Error error) { buffer.unmap (info); write_lock.unlock (); return Gst.FlowReturn.ERROR; }
                write_lock.unlock ();
                if (AtomicInt.compare_and_exchange (ref preview_queued, 0, 1)) {
                    var bytes = new Bytes (info.data);
                    Idle.add (() => {
                        if (expected_capture == capture_generation && camera_enabled && !token.is_cancelled ()) {
                            preview_frame (new Gdk.MemoryTexture (320, 240, Gdk.MemoryFormat.R8G8B8A8, bytes, 320 * 4));
                        }
                        AtomicInt.set (ref preview_queued, 0);
                        return Source.REMOVE;
                    });
                }
                buffer.unmap (info);
                return Gst.FlowReturn.OK;
            });
            camera.get_bus ().add_watch (Priority.DEFAULT, (bus, message) => {
                if (message.type == Gst.MessageType.ERROR) { failed ("Camera unavailable. Check camera permissions or another app using it."); return Source.REMOVE; }
                return Source.CONTINUE;
            });
            if (camera.set_state (Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE) { throw new IOError.FAILED ("Camera could not start"); }
            camera_enabled = true;
        }
        private void write_frame (Socket socket, uint8[] data, Cancellable token) throws Error {
            int sent = 0;
            int64 deadline = GLib.get_monotonic_time () + 2000000;
            while (sent < data.length) {
                int64 left = deadline - GLib.get_monotonic_time ();
                if (left <= 0) { throw new IOError.TIMED_OUT ("Video transport stalled"); }
                socket.condition_timed_wait (IOCondition.OUT, left, token);
                try {
                    ssize_t count = socket.send_with_blocking (data[sent:data.length], false, token);
                    if (count <= 0) { throw new IOError.CLOSED ("Video transport closed"); }
                    sent += (int) count;
                } catch (IOError.WOULD_BLOCK error) {}
            }
        }
        public void stop (bool reset_camera = true) {
            generation++;
            capture_generation++;
            preview_frame (null);
            cancel.cancel ();
            if (connection != null) { try { connection.close (); } catch (Error ignored) {} connection = null; }
            if (camera != null) { camera.get_bus ().remove_watch (); camera.set_state (Gst.State.NULL); camera = null; }
            camera_enabled = false;
            if (reset_camera) { selected_camera = new CameraChoice ("Default camera"); }
        }
    }
}
