namespace SignalNative {
    public class CallMedia : Object {
        protected Gst.Pipeline? capture;
        protected Gst.Pipeline? playback;
        private Gst.Element? microphone_volume;
        private Gst.Element? speaker_volume;
        private bool microphone_muted = false;
        private bool speaker_muted = false;
        private string microphone = "";
        private string speaker = "";
        public bool running { get; protected set; default = false; }
        public signal void failed (string message);

        public virtual async void prepare () throws Error {
            unowned string[]? gst_args = null;
            Gst.init (ref gst_args);
            if (Gst.ElementFactory.find ("pulsesrc") == null || Gst.ElementFactory.find ("pulsesink") == null) {
                throw new IOError.NOT_SUPPORTED ("GStreamer PulseAudio plugins are unavailable");
            }
            microphone = yield default_device ("get-default-source");
            speaker = yield default_device ("get-default-sink");
            if (microphone.contains ("signal_input_") || microphone.has_suffix (".monitor") ||
                speaker.contains ("signal_output_") || speaker.contains ("sink_for_signal_input_")) {
                throw new IOError.NOT_SUPPORTED ("Choose a hardware microphone and speaker as system defaults before enabling calls");
            }
        }

        private async string default_device (string command) throws Error {
            var process = new Subprocess.newv ({"pactl", command}, SubprocessFlags.STDOUT_PIPE | SubprocessFlags.STDERR_PIPE);
            string? output;
            string? error;
            yield process.communicate_utf8_async (null, null, out output, out error);
            if (!process.get_successful () || output == null || output.strip () == "") {
                throw new IOError.NOT_CONNECTED ("Could not identify the system audio device");
            }
            return output.strip ();
        }

        private Gst.Pipeline pipeline (string source_device, string sink_device, out Gst.Element volume) throws Error {
            var p = new Gst.Pipeline (null);
            var source = Gst.ElementFactory.make ("pulsesrc", null);
            var convert = Gst.ElementFactory.make ("audioconvert", null);
            var resample = Gst.ElementFactory.make ("audioresample", null);
            volume = Gst.ElementFactory.make ("volume", null);
            var sink = Gst.ElementFactory.make ("pulsesink", null);
            if (source == null || convert == null || resample == null || volume == null || sink == null) {
                throw new IOError.NOT_SUPPORTED ("Audio pipeline elements are unavailable");
            }
            source.set ("device", source_device);
            sink.set ("device", sink_device);
            p.add_many (source, convert, resample, volume, sink);
            if (!source.link (convert) || !convert.link (resample) || !resample.link (volume) || !volume.link (sink)) {
                throw new IOError.FAILED ("Could not link the call audio pipeline");
            }
            p.get_bus ().add_watch (Priority.DEFAULT, (bus, message) => {
                if (message.type == Gst.MessageType.ERROR) {
                    Error error;
                    string debug;
                    message.parse_error (out error, out debug);
                    stop ();
                    failed ("Call audio failed: " + error.message);
                    return Source.REMOVE;
                }
                return Source.CONTINUE;
            });
            return p;
        }

        public virtual void start (string input_device, string output_device) throws Error {
            stop ();
            if (microphone == "" || speaker == "" || input_device == "" || output_device == "") {
                throw new IOError.NOT_INITIALIZED ("Call audio devices are not ready");
            }
            try {
                Gst.Element mic;
                Gst.Element ear;
                capture = pipeline (microphone, "sink_for_" + input_device, out mic);
                microphone_volume = mic;
                playback = pipeline (output_device + ".monitor", speaker, out ear);
                speaker_volume = ear;
                mute_microphone (microphone_muted);
                mute_speaker (speaker_muted);
                if (capture.set_state (Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE ||
                    playback.set_state (Gst.State.PLAYING) == Gst.StateChangeReturn.FAILURE) {
                    throw new IOError.FAILED ("Could not start call audio");
                }
                running = true;
            } catch (Error error) { stop (); throw error; }
        }

        public virtual void mute_microphone (bool muted) { microphone_muted = muted; if (microphone_volume != null) { microphone_volume.set ("mute", muted); } }
        public virtual void mute_speaker (bool muted) { speaker_muted = muted; if (speaker_volume != null) { speaker_volume.set ("mute", muted); } }
        public virtual void stop () {
            if (capture != null) { capture.get_bus ().remove_watch (); capture.set_state (Gst.State.NULL); }
            if (playback != null) { playback.get_bus ().remove_watch (); playback.set_state (Gst.State.NULL); }
            capture = null; playback = null;
            microphone_volume = null; speaker_volume = null;
            running = false;
        }
    }
}
