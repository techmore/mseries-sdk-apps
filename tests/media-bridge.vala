private MainLoop loop;
private SignalNative.CallMedia bridge;
private int test_exit_code = 1;
private async void run_bridge (string input, string output) {
    try {
        yield bridge.prepare ();
        bridge.start (input, output);
        stdout.printf ("ready\n"); stdout.flush ();
        var stream = new DataInputStream (new UnixInputStream (0, false));
        while (true) {
            size_t length;
            string? line = yield stream.read_line_async (Priority.DEFAULT, null, out length);
            if (line == null || line == "stop") { break; }
            if (line == "mute-mic") { bridge.mute_microphone (true); }
            if (line == "unmute-mic") { bridge.mute_microphone (false); }
            if (line == "mute-speaker") { bridge.mute_speaker (true); }
            if (line == "unmute-speaker") { bridge.mute_speaker (false); }
            stdout.printf ("ok\n"); stdout.flush ();
        }
        test_exit_code = 0;
    } catch (Error error) { stderr.printf ("Audio bridge: %s\n", error.message); }
    bridge.stop (); loop.quit ();
}
int main (string[] args) {
    if (args.length != 3 || Environment.get_variable ("SIGNAL_PRIVATE_AUDIO_TEST") != "1" ||
        !(Environment.get_variable ("PULSE_SERVER") ?? "").contains ("signal-call-media-")) { return 2; }
    loop = new MainLoop ();
    bridge = new SignalNative.CallMedia ();
    bridge.failed.connect ((message) => { stderr.printf ("%s\n", message); bridge.stop (); loop.quit (); });
    run_bridge.begin (args[1], args[2]); loop.run (); return test_exit_code;
}
