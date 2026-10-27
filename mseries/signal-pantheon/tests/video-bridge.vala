private MainLoop video_loop;
private int video_result = 1;
private async void exercise_video (string path) {
    var video = new SignalNative.CallVideo ();
    try {
        video.failed.connect ((message) => { stderr.printf ("%s\n",message); video_loop.quit (); });
        video.frame.connect ((track,picture) => { print ("frame\n"); });
        yield video.connect_to (path);
        video.set_camera (true);
        print ("ready\n");
        var input = new DataInputStream (new UnixInputStream (0,false));
        while (true) {
            string? command = yield input.read_line_async ();
            if (command == null || command == "stop") { break; }
            if (command == "camera-off") { video.set_camera (false); print ("ok\n"); }
            else if (command == "camera-on") { video.set_camera (true); print ("ok\n"); }
        }
        video.stop (); video_result = 0;
    } catch (Error error) { stderr.printf ("%s\n",error.message); video.stop (); }
    video_loop.quit ();
}
int main (string[] args) {
    if (args.length != 2 || Environment.get_variable ("SIGNAL_PRIVATE_AUDIO_TEST") != "1") { return 2; }
    video_loop = new MainLoop (); exercise_video.begin (args[1]); video_loop.run (); return video_result;
}
