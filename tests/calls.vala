using SignalNative;
private class TestMedia : CallMedia {
    public uint starts = 0;
    public bool mic_muted = false;
    public override async void prepare () throws Error {}
    public override void start (string input, string output) throws Error { starts++; running = true; }
    public override void stop () { running = false; }
    public override void mute_microphone (bool value) { mic_muted = value; }
}
private MainLoop loop;
private int test_exit_code = 1;
private async void call_test_pause () { Timeout.add (80, call_test_pause.callback); yield; }
private void notify (SignalBackend backend, int64 id, string state, int subscription = 7) {
    var o = new Json.Object (); o.set_int_member ("callId", id); o.set_string_member ("state", state);
    o.set_string_member ("number", "peer"); o.set_string_member ("inputDeviceName", "signal_input_1");
    o.set_string_member ("outputDeviceName", "signal_output_1");
    var p = new Json.Object (); p.set_int_member ("subscription", subscription); p.set_object_member ("result", o);
    var root = new Json.Object (); root.set_object_member ("params", p);
    backend.call_event (MessageCommands.node (root));
}
private async void exercise () {
    var backend = new SignalBackend (2);
    var media = new TestMedia ();
    var calls = new CallController (backend, "self", media);
    try {
        yield backend.start ();
        yield calls.initialize ();
        assert (calls.available);
        yield calls.start ("peer"); yield call_test_pause ();
        assert (calls.current.id == int64.MIN + 1 && calls.current.state == "CONNECTED");
        assert (media.running && media.starts == 1);
        calls.mute_microphone (true); assert (media.mic_muted);
        notify (backend, calls.current.id, "ENDED", 8);
        assert (calls.active);
        yield calls.hangup ();
        assert (!calls.active && !media.running);
        notify (backend, int64.MIN + 1, "CONNECTED"); assert (!calls.active);
        notify (backend, 42, "RINGING_INCOMING"); assert (!media.running);
        // An incoming call cannot open the microphone until locally accepted.
        notify (backend, 42, "CONNECTED"); yield call_test_pause ();
        assert (!calls.active && media.starts == 1);
        notify (backend, 43, "RINGING_INCOMING");
        yield calls.accept (); yield call_test_pause ();
        assert (calls.current.id == 43 && media.running && media.starts == 2);
        notify (backend, 44, "RINGING_INCOMING"); yield call_test_pause ();
        assert (calls.current.id == 43 && calls.active);
        notify (backend, 43, "RECONNECTING"); notify (backend, 43, "CONNECTED");
        assert (media.starts == 2);
        media.failed ("Fixture audio failure"); yield call_test_pause ();
        assert (!media.running && !calls.active);
        notify (backend, 45, "RINGING_INCOMING"); yield calls.reject ();
        assert (!calls.active);
        backend.stop ();
        assert (!calls.available && !media.running);
        print ("PASS: voice lifecycle, signed 64-bit IDs, subscriptions, acceptance gating, mute, reconnect, busy and failure cleanup\n");
        test_exit_code = 0;
    } catch (Error error) { stderr.printf ("FAIL: %s\n", error.message); backend.stop (); }
    loop.quit ();
}
int main () { loop = new MainLoop (); exercise.begin (); loop.run (); return test_exit_code; }
