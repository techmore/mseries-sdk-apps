private MainLoop loop;
private SignalNative.MainWindow window;
private int test_exit_code = 1;
private async void exercise () {
    try {
        yield window.fixture_exercise (Environment.get_variable ("SIGNAL_TEST_SCREENSHOT") ?? "");
        test_exit_code = 0;
    } catch (Error error) { stderr.printf ("FAIL: %s\n", error.message); }
    window.close ();
    loop.quit ();
}
int main () {
    Gtk.init ();
    var app = new Gtk.Application ("io.github.sdolbec.SignalPantheonFixture", ApplicationFlags.NON_UNIQUE);
    try { app.register (); } catch (Error error) { return 1; }
    SignalNative.Appearance.install ();
    window = new SignalNative.MainWindow (app);
    if (Environment.get_variable ("SIGNAL_TEST_DARK") == "1") {
        Gtk.Settings.get_default ().gtk_application_prefer_dark_theme = true;
    }
    if (Environment.get_variable ("SIGNAL_TEST_NARROW") == "1") { window.set_default_size (760, 520); }
    window.present ();
    loop = new MainLoop ();
    Timeout.add (100, () => {
        if (!window.fixture_ready ()) { return Source.CONTINUE; }
        exercise.begin ();
        return Source.REMOVE;
    });
    Timeout.add_seconds (25, () => { stderr.printf ("FAIL: UI fixture timed out\n"); loop.quit (); return Source.REMOVE; });
    loop.run ();
    return test_exit_code;
}
namespace SignalNative {
    public class FixtureCallMedia : CallMedia {
        public override async void prepare () throws Error {}
        public override void start (string input, string output) throws Error { running = true; }
        public override void stop () { running = false; }
    }
}
