using SignalNative;
private MainLoop loop;
private int test_exit_code = 1;
private async void exercise () {
    string? directory = null;
    try {
        directory = DirUtils.make_tmp ("signal-attachments-XXXXXX");
        FileUtils.set_contents (Path.build_filename (directory, "remote_id.txt"), "attachment bytes");
        var attachment = new Json.Object ();
        attachment.set_string_member ("id", "remote/id");
        attachment.set_string_member ("filename", "../../hello.txt");
        File file = yield AttachmentFiles.resolve (directory, attachment);
        assert (file.get_basename () == "remote_id.txt");
        attachment.set_string_member ("id", "..");
        bool rejected = false;
        try { yield AttachmentFiles.resolve (directory, attachment); } catch (Error error) { rejected = true; }
        assert (rejected);
        FileUtils.symlink ("/etc/passwd", Path.build_filename (directory, "symlink.txt"));
        attachment.set_string_member ("id", "symlink"); attachment.set_string_member ("filename", "symlink.txt");
        rejected = false;
        try { yield AttachmentFiles.resolve (directory, attachment); } catch (Error error) { rejected = true; }
        assert (rejected);
        FileUtils.set_contents (Path.build_filename (directory, "ambiguous.txt"), "one");
        FileUtils.set_contents (Path.build_filename (directory, "ambiguous.pdf"), "two");
        attachment.set_string_member ("id", "ambiguous"); attachment.set_string_member ("filename", "unknown.bin");
        rejected = false;
        try { yield AttachmentFiles.resolve (directory, attachment); } catch (Error error) { rejected = true; }
        assert (rejected);
        FileUtils.set_contents (Path.build_filename (directory, "remote_id.txt.preview"), "preview bytes");
        FileUtils.set_contents (Path.build_filename (directory, "remote_id_other.txt"), "unrelated bytes");
        attachment.set_string_member ("id", "remote/id");
        yield AttachmentFiles.expire_owned (directory, directory, attachment);
        assert (!FileUtils.test (Path.build_filename (directory, "remote_id.txt"), FileTest.EXISTS));
        assert (!FileUtils.test (Path.build_filename (directory, "remote_id.txt.preview"), FileTest.EXISTS));
        assert (FileUtils.test (Path.build_filename (directory, "remote_id_other.txt"), FileTest.EXISTS));
        string owned = Path.build_filename (directory, "owned");
        string external = Path.build_filename (directory, "external");
        DirUtils.create (owned, 448); DirUtils.create (external, 448);
        string original = Path.build_filename (external, "original.txt");
        FileUtils.set_contents (original, "user original");
        attachment = new Json.Object (); attachment.set_string_member ("localPath", original);
        yield AttachmentFiles.expire_owned (directory, owned, attachment);
        assert (FileUtils.test (original, FileTest.EXISTS));
        FileUtils.symlink (external, Path.build_filename (owned, "escape"));
        attachment.set_string_member ("localPath", Path.build_filename (owned, "escape", "original.txt"));
        yield AttachmentFiles.expire_owned (directory, owned, attachment);
        assert (FileUtils.test (original, FileTest.EXISTS));
        FileUtils.remove (Path.build_filename (owned, "escape"));
        FileUtils.remove (original); DirUtils.remove (external); DirUtils.remove (owned);
        print ("PASS: attachment ID sanitization, filename traversal isolation, symlink and ambiguity rejection\n");
        test_exit_code = 0;
    } catch (Error error) { stderr.printf ("FAIL: %s\n", error.message); }
    if (directory != null) {
        foreach (string name in new string[] { "remote_id.txt", "remote_id.txt.preview", "remote_id_other.txt", "symlink.txt", "ambiguous.txt", "ambiguous.pdf" }) {
            FileUtils.remove (Path.build_filename (directory, name));
        }
        DirUtils.remove (directory);
    }
    loop.quit ();
}
int main () { loop = new MainLoop (); exercise.begin (); loop.run (); return test_exit_code; }
