namespace SignalNative {
    public class AttachmentFiles : Object {
        private static async bool safe_owned_path (File file, File root) throws Error {
            if (!file.has_prefix (root)) { return false; }
            File? cursor = file;
            while (cursor != null && !cursor.equal (root)) {
                var info = yield cursor.query_info_async ("standard::is-symlink", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
                if (info.get_is_symlink ()) { return false; }
                cursor = cursor.get_parent ();
            }
            var root_info = yield root.query_info_async ("standard::is-symlink", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
            return !root_info.get_is_symlink ();
        }

        public static async void expire_owned (string incoming_directory, string outgoing_directory, Json.Object attachment) throws Error {
            string local = JsonFields.text (attachment, "localPath");
            if (local != "") {
                var file = File.new_for_path (local);
                if (yield safe_owned_path (file, File.new_for_path (outgoing_directory))) {
                    yield file.delete_async ();
                }
                return;
            }
            string id = JsonFields.text (attachment, "id");
            if (id == "" || id == "." || id == "..") { return; }
            var safe = new StringBuilder ();
            for (int i = 0; i < id.length; i++) {
                char ch = id[i];
                safe.append_c (ch.isalnum () || ch == '_' || ch == '.' || ch == '-' ? ch : '_');
            }
            var root = File.new_for_path (incoming_directory);
            var info = yield root.query_info_async ("standard::is-symlink", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
            if (info.get_is_symlink ()) { throw new IOError.INVALID_DATA ("Attachment directory is a symbolic link"); }
            var files = yield root.enumerate_children_async ("standard::name,standard::type,standard::is-symlink", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
            try {
                while (true) {
                    var batch = yield files.next_files_async (32);
                    if (batch == null) { break; }
                    foreach (var item in batch) {
                        string name = item.get_name ();
                        if ((name == safe.str || name.has_prefix (safe.str + ".")) &&
                            item.get_file_type () == FileType.REGULAR && !item.get_is_symlink ()) {
                            yield root.get_child (name).delete_async ();
                        }
                    }
                }
            } finally { try { files.close (null); } catch (Error ignored) {} }
        }

        public static async File resolve (string directory_path, Json.Object attachment) throws Error {
            string local = JsonFields.text (attachment, "localPath");
            if (local != "") {
                var file = File.new_for_path (local);
                var info = yield file.query_info_async ("standard::type,standard::is-symlink", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
                if (info.get_file_type () != FileType.REGULAR || info.get_is_symlink ()) {
                    throw new IOError.INVALID_DATA ("Attachment is not a regular local file");
                }
                return file;
            }
            string id = JsonFields.text (attachment, "id");
            if (id == "" || id == "." || id == "..") { throw new IOError.INVALID_DATA ("Attachment has no usable identifier"); }
            // Match signal-cli's AttachmentStore.sanitizeId; never interpret an ID as a path.
            var safe_builder = new StringBuilder ();
            for (int i = 0; i < id.length; i++) {
                char ch = id[i];
                safe_builder.append_c (ch.isalnum () || ch == '_' || ch == '.' || ch == '-' ? ch : '_');
            }
            string safe = safe_builder.str;
            var directory = File.new_for_path (directory_path);
            var files = yield directory.enumerate_children_async ("standard::name,standard::type,standard::is-symlink", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
            File? found = null;
            uint candidates = 0;
            string filename = JsonFields.text (attachment, "filename");
            int dot = filename.last_index_of_char ('.');
            string expected = dot >= 0 ? safe + filename.substring (dot) : safe;
            try {
                while (true) {
                    var batch = yield files.next_files_async (32);
                    if (batch == null) { break; }
                    foreach (var info in batch) {
                        if (info.get_file_type () != FileType.REGULAR || info.get_is_symlink ()) { continue; }
                        string name = info.get_name ();
                        if (name == safe || name == expected) {
                            return directory.get_child (name);
                        }
                        if (name.has_prefix (safe + ".") && !name.has_suffix (".preview") && !name.has_suffix (".tmp")) {
                            candidates++;
                            found = directory.get_child (name);
                        }
                    }
                }
            } finally { try { files.close (null); } catch (Error ignored) {} }
            if (found == null || candidates != 1) {
                throw new IOError.NOT_FOUND ("Attachment is unavailable or cannot be identified unambiguously");
            }
            return found;
        }
    }
}
