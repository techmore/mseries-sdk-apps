private MainLoop loop;
private int test_exit_code = 0;
private async void exercise () {
    var backend = new SignalNative.SignalBackend (1);
    try {
        yield backend.start ();
        var accounts = yield backend.rpc ("listAccounts");
        assert (accounts.get_array ().get_length () == 1);
        bool failed = false;
        try { yield backend.rpc ("fail"); }
        catch (Error error) { failed = error.message == "fixture failure"; }
        assert (failed);
        accounts = yield backend.rpc ("listAccounts");
        assert (accounts.get_array ().get_length () == 1);
        failed = false;
        try { yield backend.rpc ("drop"); }
        catch (Error error) { failed = true; }
        assert (failed);
        backend.stop ();
        yield backend.start ();
        bool concurrent_ok = false;
        backend.rpc.begin ("listAccounts", null, (source, response) => {
            try { concurrent_ok = backend.rpc.end (response).get_array ().get_length () == 1; }
            catch (Error error) { concurrent_ok = false; }
        });
        accounts = yield backend.rpc ("listAccounts");
        assert (concurrent_ok && accounts.get_array ().get_length () == 1);
        failed = false;
        try { yield backend.rpc ("hang"); }
        catch (Error error) { failed = error.message.contains ("timed out"); }
        assert (failed);
        backend.stop ();
        print ("PASS: RPC response, error recovery, EOF releases pending request, restart and timeout\n");
    } catch (Error error) {
        stderr.printf ("FAIL: %s\n", error.message);
        test_exit_code = 1;
    }
    loop.quit ();
}
int main () {
    loop = new MainLoop ();
    exercise.begin ();
    loop.run ();
    return test_exit_code;
}
