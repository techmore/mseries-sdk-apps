[CCode (cheader_filename = "qr-image.h")]
private extern bool signal_native_qr_write_png (string text, string path);

namespace SignalNative {
    private class MainWindow : Gtk.ApplicationWindow {
        private SignalBackend backend;
        private Gtk.Label connection_status;
        private Gtk.ListBox conversation_list;
        private Gtk.Label conversation_title;
        private Gtk.Label conversation_status;
        private Gtk.Box message_history;
        private MessageComposer message_entry;
        private Gtk.Button send_button;
        private Gtk.Button link_button;
        private string selected_recipient = "";
        private string active_account = "";
        private bool selected_group = false;
        private bool send_in_progress = false;
        private string conversation_query = "";
        private ClientStore? store;
        private ExpirationService? expiration;
        private CallController? calls;
        private bool expiration_backend = false;
        private bool video_backend = false;
        private bool group_backend = false;
        private bool view_once_backend = false;
        private bool flushing_reads = false;
        private bool closing = false;
        private uint maintenance_timer = 0;
        private Gtk.Button voice_button;
        private HashTable<MessageRecord, Gtk.Label> expiry_labels = new HashTable<MessageRecord, Gtk.Label> (direct_hash, direct_equal);
        private Gtk.Label call_status;
        private Gtk.Box call_bar;
        private Gtk.Button call_accept;
        private Gtk.Button call_reject;
        private Gtk.Button call_hangup;
        private Gtk.ToggleButton call_mute;
        private Gtk.ToggleButton call_camera;
        private Gtk.Label call_roster;
        private Gtk.Button call_show_video;
        private Gtk.Button video_button;
        private Gtk.Window? video_window;
        private Gtk.Window? view_once_window;
        private MessageRecord? viewing_once;
        private Gtk.MediaStream? view_once_stream;
        private Gtk.FlowBox video_tiles;
        private Gtk.Picture? self_preview;
        private Gtk.Label? self_preview_label;
        private Gtk.DropDown? camera_selector;
        private HashTable<string,Gtk.Picture> video_pictures = new HashTable<string,Gtk.Picture> (str_hash, str_equal);
        private Gtk.ToggleButton call_speaker;
        private Conversation? current;
        private Gtk.SearchEntry history_search;
        private Gtk.Button attach_button;
        private Gtk.Button new_conversation_button;
        private Gtk.Button clear_attachment_button;
        private Gtk.Label attachment_label;
        private Gtk.CheckButton view_once_toggle;
        private Gtk.ScrolledWindow history_scroll;
        private Gtk.CheckButton notifications;
        private Gtk.CheckButton read_receipts;
        private uint draft_timer = 0;
        private bool updating_composer = false;
        private bool service_ready = false;
        private bool catalog_ready = false;
        private GenericArray<Json.Node> pending_events = new GenericArray<Json.Node> ();
        private int64 editing_timestamp = 0;
        private string self_aci = "";
        private string preference_path;
        private KeyFile preferences = new KeyFile ();
        private uint history_limit = 100;
        private bool link_in_progress = false;
        private bool linking_cancelled = false;
        private Gtk.Window? pairing_window;
        private string? pairing_image_path;

        public MainWindow (Gtk.Application application) {
            Object (
                application: application,
                title: "Signal",
                default_width: 1080,
                default_height: 720,
                width_request: 760,
                height_request: 520
            );

            preference_path = Path.build_filename (Environment.get_user_config_dir (), "signal-pantheon", "preferences.ini");
            try { preferences.load_from_file (preference_path, KeyFileFlags.NONE); } catch (Error error) {}
            backend = new SignalBackend ();
            backend.incoming_message.connect (handle_incoming_message);
            backend.disconnected.connect (() => {
                service_ready = false;
                connection_status.set_text ("Signal service disconnected. Choose Reconnect to retry.");
                link_button.label = "Reconnect";
                message_entry.set_sensitive (false);
                send_button.set_sensitive (false);
                link_button.set_sensitive (true);
            });
            var header = new Gtk.HeaderBar ();
            var heading = new Granite.HeaderLabel ("Signal") {
                secondary_text = "Pantheon native client"
            };
            header.set_title_widget (heading);
            link_button = new Gtk.Button.with_label ("Link Signal") {
                tooltip_text = "Link this client from Signal on your phone"
            };
            link_button.set_sensitive (false);
            link_button.add_css_class ("suggested-action");
            link_button.clicked.connect (() => start_link.begin ());
            header.pack_end (link_button);
            var settings_button = new Gtk.MenuButton () {
                icon_name = "open-menu-symbolic", tooltip_text = "Messaging preferences"
            };
            var options = new Gtk.Box (Gtk.Orientation.VERTICAL, 12) {
                margin_start = 16, margin_end = 16, margin_top = 16, margin_bottom = 16
            };
            notifications = new Gtk.CheckButton.with_label ("Show desktop notifications");
            read_receipts = new Gtk.CheckButton.with_label ("Send read receipts");
            try { notifications.active = preferences.get_boolean ("Messaging", "notifications"); } catch (Error error) {}
            try { read_receipts.active = preferences.get_boolean ("Messaging", "read-receipts"); } catch (Error error) {}
            notifications.toggled.connect (save_preferences);
            read_receipts.toggled.connect (save_preferences);
            options.append (notifications);
            options.append (read_receipts);
            var note = new Gtk.Label ("Notifications hide message contents. Read receipts are sent only while this window is active.") {
                wrap = true, max_width_chars = 32, xalign = 0.0f
            };
            note.add_css_class ("dim-label");
            options.append (note);
            var popover = new Gtk.Popover ();
            popover.set_child (options);
            settings_button.set_popover (popover);
            header.pack_end (settings_button);
            set_titlebar (header);

            var root = new Gtk.Box (Gtk.Orientation.VERTICAL, 0);
            set_child (root);

            connection_status = new Gtk.Label (
                "Signal is not linked · this client uses a separate profile"
            ) {
                wrap = true,
                xalign = 0.0f,
                margin_start = 16,
                margin_end = 16,
                margin_top = 8,
                margin_bottom = 8
            };
            connection_status.add_css_class ("dim-label");
            root.append (connection_status);
            call_bar = new Gtk.Box (Gtk.Orientation.VERTICAL, 8) {
                visible = false, margin_start = 16, margin_end = 16, margin_top = 8, margin_bottom = 8
            };
            call_status = new Gtk.Label ("") { wrap = true, xalign = 0.0f, hexpand = true, max_width_chars = 38 };
            call_accept = new Gtk.Button.with_label ("Answer");
            call_reject = new Gtk.Button.with_label ("Decline");
            call_hangup = new Gtk.Button.with_label ("End call");
            call_mute = new Gtk.ToggleButton.with_label ("Mute mic");
            call_speaker = new Gtk.ToggleButton.with_label ("Mute speaker");
            call_accept.clicked.connect (() => { if (calls != null) { calls.accept.begin (); } });
            call_reject.clicked.connect (() => { if (calls != null) { calls.reject.begin (); } });
            call_hangup.clicked.connect (() => { if (calls != null) { calls.hangup.begin (); } });
            call_mute.toggled.connect (() => { call_mute.label = call_mute.active ? "Unmute mic" : "Mute mic"; if (calls != null) { calls.mute_microphone (call_mute.active); } });
            call_speaker.toggled.connect (() => { call_speaker.label = call_speaker.active ? "Unmute speaker" : "Mute speaker"; if (calls != null) { calls.mute_speaker (call_speaker.active); } });
            call_bar.append (call_status);
            var call_controls = new Gtk.FlowBox () { selection_mode = Gtk.SelectionMode.NONE, column_spacing = 6, row_spacing = 6, max_children_per_line = 6, homogeneous = false };
            call_camera = new Gtk.ToggleButton.with_label ("Camera");
            call_camera.clicked.connect (() => { if (calls != null) { calls.camera.begin (call_camera.active); } });
            call_controls.insert (call_accept,-1); call_controls.insert (call_reject,-1); call_controls.insert (call_hangup,-1);
            call_controls.insert (call_mute,-1); call_controls.insert (call_speaker,-1); call_controls.insert (call_camera,-1);
            call_show_video = new Gtk.Button.with_label ("Show video");
            call_show_video.clicked.connect (() => { if (calls != null && calls.active) { ensure_video_window (); video_window.present (); } });
            call_controls.insert (call_show_video,-1);
            Gtk.Widget[] controls = { call_accept,call_reject,call_hangup,call_mute,call_speaker,call_camera,call_show_video };
            foreach (var control in controls) { control.bind_property ("visible",control.get_parent (),"visible",BindingFlags.SYNC_CREATE); }
            call_bar.append (call_controls);
            call_roster = new Gtk.Label ("") { wrap = true, xalign = 0.0f, selectable = true, max_width_chars = 70, wrap_mode = Pango.WrapMode.WORD_CHAR };
            call_bar.append (call_roster);
            root.append (call_bar);

            var pane = new Gtk.Paned (Gtk.Orientation.HORIZONTAL) {
                wide_handle = true,
                position = 340,
                vexpand = true,
                hexpand = true
            };
            root.append (pane);
            pane.set_start_child (build_sidebar ());
            pane.set_end_child (build_conversation ());
            pane.set_resize_start_child (false);
            pane.set_shrink_start_child (false);
            var keyboard = new Gtk.EventControllerKey ();
            keyboard.key_pressed.connect ((key, code, state) => {
                if (key == Gdk.Key.Escape && editing_timestamp > 0 && current != null) {
                    editing_timestamp = 0;
                    updating_composer = true;
                    message_entry.text = current.draft;
                    updating_composer = false;
                    conversation_status.label = "Edit canceled";
                    update_composer ();
                    return true;
                }
                return false;
            });
            ((Gtk.Widget) this).add_controller (keyboard);
            var show = new SimpleAction ("show-conversation", VariantType.STRING);
            show.activate.connect ((parameter) => {
                if (parameter == null) { return; }
                present ();
                for (Gtk.Widget? row = conversation_list.get_first_child (); row != null; row = row.get_next_sibling ()) {
                    if (row.get_data<string> ("recipient") == parameter.get_string ()) {
                        conversation_list.select_row ((Gtk.ListBoxRow) row);
                        break;
                    }
                }
            });
            application.add_action (show);
            maintenance_timer = Timeout.add_seconds (10, () => { flush_all_reads.begin (); return Source.CONTINUE; });
            initialize_backend.begin ();

            notify["is-active"].connect (() => {
                if (is_active && current != null) { mark_read.begin (current); }
            });
            close_request.connect (() => {
                if (view_once_window != null) { view_once_window.close (); }
                if (closing) { return true; }
                if (calls != null && calls.active) {
                    closing = true;
                    close_with_call.begin ();
                    return true;
                }
                save_draft ();
                if (draft_timer != 0) { Source.remove (draft_timer); draft_timer = 0; }
                if (expiration != null) { expiration.stop (); }
                if (maintenance_timer != 0) { Source.remove (maintenance_timer); maintenance_timer = 0; }
                backend.stop ();
                if (pairing_image_path != null) {
                    FileUtils.remove (pairing_image_path);
                }
                return false;
            });
        }

        private Gtk.Widget build_sidebar () {
            var sidebar = new Gtk.Box (Gtk.Orientation.VERTICAL, 12) {
                margin_start = 16,
                margin_end = 12,
                margin_top = 12,
                margin_bottom = 16
            };
            sidebar.append (new Granite.HeaderLabel ("Conversations"));
            new_conversation_button = new Gtk.Button.with_label ("New conversation…") { sensitive = false };
            new_conversation_button.clicked.connect (new_conversation_dialog);
            sidebar.append (new_conversation_button);

            var search = new Gtk.SearchEntry () {
                placeholder_text = "Search conversations"
            };
            search.search_changed.connect (() => {
                conversation_query = search.text.casefold ();
                conversation_list.invalidate_filter ();
            });
            sidebar.append (search);

            conversation_list = new Gtk.ListBox () {
                selection_mode = Gtk.SelectionMode.SINGLE,
                vexpand = true
            };
            conversation_list.add_css_class ("navigation-sidebar");
            conversation_list.set_filter_func ((row) => {
                string? name = row.get_data<string> ("contact-name");
                string? recipient = row.get_data<string> ("recipient");
                return conversation_query == "" || (name != null &&
                    (name.casefold ().contains (conversation_query) ||
                     (recipient != null && recipient.casefold ().contains (conversation_query))));
            });
            conversation_list.row_selected.connect ((row) => {
                if (row == null) { return; }
                if (store == null) { return; }
                save_draft ();
                selected_recipient = row.get_data<string> ("recipient") ?? "";
                current = store.lookup (selected_recipient);
                if (current == null) { return; }
                selected_group = current.is_group;
                editing_timestamp = 0;
                conversation_title.set_text (current.name);
                conversation_status.set_text (selected_group ? "Signal group" : "Signal contact");
                updating_composer = true;
                message_entry.text = current.draft;
                updating_composer = false;
                update_composer ();
                history_limit = 100;
                render_history ();
                mark_read.begin (current);
            });

            var empty = new Gtk.Label (
                "No matching conversations. Link or refresh Signal to load contacts and groups."
            ) {
                wrap = true,
                xalign = 0.0f,
                margin_start = 12,
                margin_end = 12,
                margin_top = 12
            };
            empty.add_css_class ("dim-label");
            conversation_list.set_placeholder (empty);

            var list_scroll = new Gtk.ScrolledWindow () {
                child = conversation_list,
                vexpand = true,
                hscrollbar_policy = Gtk.PolicyType.NEVER
            };
            sidebar.append (list_scroll);
            return sidebar;
        }

        private Gtk.Widget build_conversation () {
            var conversation = new Gtk.Box (Gtk.Orientation.VERTICAL, 0) {
                hexpand = true,
                vexpand = true
            };

            var top = new Gtk.Box (Gtk.Orientation.VERTICAL, 3) {
                margin_start = 20,
                margin_end = 20,
                margin_top = 14,
                margin_bottom = 14
            };
            conversation_title = new Gtk.Label ("No conversation selected") {
                ellipsize = Pango.EllipsizeMode.END,
                xalign = 0.0f
            };
            conversation_title.add_css_class ("title-3");
            conversation_status = new Gtk.Label ("Link Signal to begin") {
                wrap = true, wrap_mode = Pango.WrapMode.WORD_CHAR, max_width_chars = 58,
                xalign = 0.0f
            };
            conversation_status.add_css_class ("dim-label");
            top.append (conversation_title);
            top.append (conversation_status);
            var info_button = new Gtk.Button.with_label ("Conversation details") { halign = Gtk.Align.START };
            info_button.clicked.connect (() => show_details ());
            top.append (info_button);
            voice_button = new Gtk.Button.with_label ("Voice call") { halign = Gtk.Align.START, sensitive = false };
            voice_button.clicked.connect (() => { if (calls != null && current != null) { calls.start.begin (current.id, false, current.is_group); } });
            top.append (voice_button);
            video_button = new Gtk.Button.with_label ("Video call") { halign = Gtk.Align.START, sensitive = false };
            video_button.clicked.connect (() => { if (calls != null && current != null) { calls.start.begin (current.id, true, current.is_group); } });
            top.append (video_button);
            conversation.append (top);
            history_search = new Gtk.SearchEntry () {
                placeholder_text = "Search messages in this conversation",
                margin_start = 16, margin_end = 16, margin_top = 8, margin_bottom = 8
            };
            history_search.search_changed.connect (() => render_history ());
            conversation.append (history_search);
            conversation.append (new Gtk.Separator (Gtk.Orientation.HORIZONTAL));

            message_history = new Gtk.Box (Gtk.Orientation.VERTICAL, 12) {
                valign = Gtk.Align.END,
                hexpand = true,
                margin_start = 24,
                margin_end = 24,
                margin_top = 24,
                margin_bottom = 20
            };
            history_scroll = new Gtk.ScrolledWindow () {
                child = message_history,
                vexpand = true,
                hexpand = true,
                hscrollbar_policy = Gtk.PolicyType.NEVER
            };
            conversation.append (history_scroll);

            var composer = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8) {
                margin_start = 16,
                margin_end = 16,
                margin_top = 12,
                margin_bottom = 16
            };
            message_entry = new MessageComposer () {
                placeholder_text = "Write a message",
                hexpand = true,
                sensitive = false
            };
            message_entry.submitted.connect (() => send_message.begin ());
            message_entry.changed.connect (() => {
                if (updating_composer || current == null) { return; }
                if (editing_timestamp == 0) { current.draft = message_entry.text; }
                if (draft_timer != 0) { Source.remove (draft_timer); }
                draft_timer = Timeout.add (600, () => {
                    draft_timer = 0;
                    save_draft ();
                    return Source.REMOVE;
                });
            });
            send_button = new Gtk.Button.with_label ("Send") {
                sensitive = false
            };
            send_button.clicked.connect (() => send_message.begin ());
            attach_button = new Gtk.Button.from_icon_name ("mail-attachment-symbolic") {
                tooltip_text = "Attach a file", sensitive = false
            };
            attach_button.clicked.connect (() => choose_attachment.begin ());
            composer.append (attach_button);
            var composer_scroll = new Gtk.ScrolledWindow () {
                child = message_entry, min_content_height = 44, max_content_height = 140,
                propagate_natural_height = true, hscrollbar_policy = Gtk.PolicyType.NEVER, hexpand = true
            };
            composer_scroll.add_css_class ("frame");
            composer.append (composer_scroll);
            composer.append (send_button);
            conversation.append (new Gtk.Separator (Gtk.Orientation.HORIZONTAL));
            var attachment_row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8) {
                margin_start = 16, margin_end = 16, margin_bottom = 8
            };
            attachment_label = new Gtk.Label ("") { xalign = 0.0f, ellipsize = Pango.EllipsizeMode.MIDDLE, hexpand = true };
            clear_attachment_button = new Gtk.Button.with_label ("Remove attachment") { visible = false };
            clear_attachment_button.clicked.connect (() => {
                if (current != null) { current.attachment_draft = ""; persist (current); update_composer (); }
            });
            attachment_row.append (attachment_label);
            view_once_toggle = new Gtk.CheckButton.with_label ("View once");
            view_once_toggle.toggled.connect (() => {
                if (!updating_composer && current != null) { current.view_once_draft = view_once_toggle.active; persist (current); }
            });
            attachment_row.append (view_once_toggle);
            attachment_row.append (clear_attachment_button);
            conversation.append (attachment_row);
            conversation.append (composer);
            return conversation;
        }

        private async void start_link () {
            if (link_in_progress) {
                return;
            }
            link_in_progress = true;
            linking_cancelled = false;
            link_button.set_sensitive (false);
            try {
                connection_status.set_text ("Starting the isolated Signal service…");
                yield backend.start ();
                Json.Node? accounts = yield backend.rpc ("listAccounts");
                if (set_active_account (accounts)) {
                    connection_status.set_text ("Signal is linked");
                    yield load_contacts ();
                    link_in_progress = false;
                    link_button.set_sensitive (true);
                    return;
                }

                Json.Node? result = yield backend.rpc ("startLink");
                if (result == null || result.get_node_type () != Json.NodeType.OBJECT) {
                    throw new IOError.INVALID_DATA ("Signal did not return a device link");
                }
                string link_uri = result.get_object ().get_string_member ("deviceLinkUri");
                if (!show_pairing (link_uri)) {
                    backend.stop ();
                    link_in_progress = false;
                    link_button.set_sensitive (true);
                    return;
                }
                finish_link.begin (link_uri);
            } catch (Error error) {
                connection_status.set_text ("Could not start Signal: %s".printf (error.message));
                link_in_progress = false;
                link_button.set_sensitive (true);
            }
        }

        private async void initialize_backend () {
            try {
                connection_status.set_text ("Connecting to the local Signal service…");
                yield backend.start ();
                Json.Node? accounts = yield backend.rpc ("listAccounts");
                if (set_active_account (accounts)) {
                    connection_status.set_text ("Signal is linked");
                    yield load_contacts ();
                } else {
                    connection_status.set_text (
                        "Ready to link Signal · profile stored separately from Signal Beta"
                    );
                }
                link_button.set_sensitive (true);
            } catch (Error error) {
                connection_status.set_text (
                    "Signal service could not start: %s".printf (error.message)
                );
                link_button.set_sensitive (true);
            }
        }

        private bool show_pairing (string link_uri) {
            var cache_directory = Path.build_filename (
                Environment.get_user_cache_dir (), "signal-pantheon"
            );
            DirUtils.create_with_parents (cache_directory, 448);
            pairing_image_path = Path.build_filename (cache_directory, "pairing.png");
            if (!signal_native_qr_write_png (link_uri, pairing_image_path)) {
                connection_status.set_text ("Could not create the Signal pairing code");
                return false;
            }

            pairing_window = new Gtk.Window () {
                title = "Link Signal",
                transient_for = this,
                modal = true,
                default_width = 380,
                default_height = 460
            };
            var header = new Gtk.HeaderBar ();
            var close_button = new Gtk.Button.with_label ("Close");
            close_button.clicked.connect (cancel_link);
            header.pack_end (close_button);
            pairing_window.set_titlebar (header);
            var content = new Gtk.Box (Gtk.Orientation.VERTICAL, 8) {
                margin_bottom = 12
            };
            pairing_window.set_child (content);
            var instructions = new Gtk.Label (
                "On your phone, open Signal → Settings → Linked devices → Link new device, then scan this code."
            ) {
                wrap = true,
                margin_start = 20,
                margin_end = 20,
                margin_top = 18,
                margin_bottom = 10
            };
            content.append (instructions);
            var qr = new Gtk.Image.from_file (pairing_image_path) {
                pixel_size = 300,
                halign = Gtk.Align.CENTER,
                margin_bottom = 12
            };
            content.append (qr);
            var link = new Gtk.Entry () {
                text = link_uri,
                editable = false,
                visibility = false,
                margin_start = 20,
                margin_end = 20,
                margin_bottom = 16
            };
            content.append (link);
            pairing_window.close_request.connect (() => {
                cancel_link ();
                return true;
            });
            pairing_window.present ();
            connection_status.set_text ("Waiting for phone approval…");
            return true;
        }

        private void cancel_link () {
            if (!link_in_progress) {
                if (pairing_window != null) {
                    pairing_window.hide ();
                }
                return;
            }

            linking_cancelled = true;
            link_in_progress = false;
            backend.stop ();
            if (pairing_window != null) {
                pairing_window.hide ();
            }
            link_button.set_sensitive (true);
            connection_status.set_text ("Linking canceled. Choose Link Signal to retry.");
        }

        private async void finish_link (string link_uri) {
            try {
                yield backend.rpc (
                    "finishLink", backend.link_params (link_uri)
                );
                Json.Node? accounts = yield backend.rpc ("listAccounts");
                if (!set_active_account (accounts)) {
                    throw new IOError.INVALID_DATA (
                        "Signal linked the device but returned no usable account"
                    );
                }
                link_in_progress = false;
                link_button.set_sensitive (true);
                connection_status.set_text ("Signal is linked");
                if (pairing_window != null) {
                    pairing_window.hide ();
                }
                yield load_contacts ();
            } catch (Error error) {
                if (!linking_cancelled) {
                    link_in_progress = false;
                    link_button.set_sensitive (true);
                    connection_status.set_text (
                        "Linking did not finish: %s. Try Link Signal again.".printf (error.message)
                    );
                }
            }
        }

        private async void load_contacts () {
            catalog_ready = false;
            try {
                if (active_account == "") {
                    throw new IOError.NOT_CONNECTED ("No linked Signal account is selected");
                }
                service_ready = true;
                try {
                    var capability = yield backend.rpc ("pantheonCapabilities", backend.account_params (active_account));
                    expiration_backend = capability != null && capability.get_node_type () == Json.NodeType.OBJECT &&
                        JsonFields.integer (capability.get_object (), "expiration") == 1 &&
                        JsonFields.integer (capability.get_object (), "readSync") == 1;
                    video_backend = expiration_backend && JsonFields.integer (capability.get_object (), "videoCalls") == 1;
                    group_backend = expiration_backend && JsonFields.integer (capability.get_object (), "groupCalls") == 1;
                    view_once_backend = expiration_backend && JsonFields.integer (capability.get_object (), "viewOnce") == 1;
                } catch (Error error) { expiration_backend = false; video_backend = false; group_backend = false; view_once_backend = false; }
                if (store == null) {
                    store = new ClientStore (backend, active_account);
                    try { store.restore (); } catch (Error error) {
                        store = null;
                        throw error;
                    }
                }
                Json.Node? result = yield backend.rpc (
                    "listContacts", backend.account_params (active_account)
                );
                // Preserve existing rows and selection while refreshing. 
                if (result != null && result.get_node_type () == Json.NodeType.ARRAY) {
                    Json.Array contacts = result.get_array ();
                    for (uint i = 0; i < contacts.get_length (); i++) {
                        Json.Node? item = contacts.get_element (i);
                        if (item == null || item.get_node_type () != Json.NodeType.OBJECT) {
                            continue;
                        }
                        Json.Object contact = item.get_object ();
                        string name = JsonFields.text (contact, "name", "Signal contact");
                        string recipient = JsonFields.address (contact, "number", "uuid", "username");
                        if (name == "") { name = recipient; }
                        if (recipient != "") {
                            add_conversation_row (name, recipient, false);
                            store.ensure (recipient, name, false).metadata = contact;
                        }
                    }
                }
                Json.Node? groups_result = yield backend.rpc (
                    "listGroups", backend.account_params (active_account)
                );
                if (groups_result != null && groups_result.get_node_type () == Json.NodeType.ARRAY) {
                    Json.Array groups = groups_result.get_array ();
                    for (uint i = 0; i < groups.get_length (); i++) {
                        Json.Node? item = groups.get_element (i);
                        if (item == null || item.get_node_type () != Json.NodeType.OBJECT) {
                            continue;
                        }
                        Json.Object group = item.get_object ();
                        string name = JsonFields.text (group, "name", "Signal group");
                        string id = JsonFields.text (group, "id");
                        if (id != "") {
                            add_conversation_row (name, id, true);
                            store.ensure (id, name, true).metadata = group;
                        }
                    }
                }
                foreach (var c in store.conversations) {
                    store.import_legacy (c);
                    add_conversation_row (c.name, c.id, c.is_group);
                    persist (c);
                }
                if (expiration == null) {
                    expiration = new ExpirationService (store, backend.attachment_directory,
                        Path.build_filename (Environment.get_user_data_dir (), "signal-pantheon", "outgoing"));
                    expiration.changed.connect ((c) => { if (current == c) { render_history (); } });
                    expiration.pulse.connect (refresh_expiry_labels);
                    expiration.failed.connect ((message) => connection_status.label = message);
                    expiration.start ();
                }
                if (calls == null) {
#if UI_TEST
                    calls = new CallController (backend, active_account, new FixtureCallMedia ());
#else
                    calls = new CallController (backend, active_account, new CallMedia ());
#endif
                    calls.changed.connect (update_call_panel);
                    calls.video_media.frame.connect (show_video_frame);
                    calls.video_media.preview_frame.connect (show_self_preview);
                }
                if (!calls.available) { calls.initialize.begin (); }
                catalog_ready = true;
                drain_events ();
                update_composer ();
            } catch (Error error) {
                catalog_ready = store != null;
                if (catalog_ready) { drain_events (); }
                connection_status.set_text (
                    "Signal is linked, but contacts could not be loaded: %s".printf (error.message)
                );
            }
        }

        private void add_conversation_row (string name, string recipient, bool is_group) {
            if (store != null) { store.ensure (recipient, name, is_group); }
            if (current != null && current.id == recipient) { conversation_title.label = current.name; }
            for (Gtk.Widget? child = conversation_list.get_first_child (); child != null; child = child.get_next_sibling ()) {
                if (child.get_data<string> ("recipient") == recipient) {
                    child.set_data<string> ("contact-name", name);
                    child.set_data<bool> ("is-group", is_group);
                    var label = child.get_data<Gtk.Label> ("name-label");
                    if (label != null) { label.label = name; }
                    conversation_list.invalidate_filter ();
                    return;
                }
            }
            var row = new Gtk.ListBoxRow ();
            row.set_data<string> ("recipient", recipient);
            row.set_data<string> ("contact-name", name);
            row.set_data<bool> ("is-group", is_group);
            var layout = new Gtk.Box (Gtk.Orientation.VERTICAL, 4) {
                margin_start = 12,
                margin_end = 12,
                margin_top = 10,
                margin_bottom = 10
            };
            var label = new Gtk.Label (name) {
                xalign = 0.0f,
                ellipsize = Pango.EllipsizeMode.END
            };
            label.add_css_class ("heading");
            layout.append (label);
            row.set_data<Gtk.Label> ("name-label", label);
            row.set_child (layout);
            conversation_list.append (row);
        }

        private void remove_all_rows () {
            Gtk.Widget? child = conversation_list.get_first_child ();
            while (child != null) {
                Gtk.Widget? next = child.get_next_sibling ();
                conversation_list.remove (child);
                child = next;
            }
        }

        private bool set_active_account (Json.Node? accounts) throws Error {
            if (accounts == null || accounts.get_node_type () != Json.NodeType.ARRAY) { return false; }
            var list = accounts.get_array ();
            if (list.get_length () != 1) {
                if (list.get_length () > 1) { throw new IOError.NOT_SUPPORTED ("This profile contains multiple accounts. Use a separate profile for each account."); }
                return false;
            }
            var item = list.get_element (0);
            if (item.get_node_type () != Json.NodeType.OBJECT) { return false; }
            var account = item.get_object ();
            string id = JsonFields.text (account, "number");
            if (id == "") { id = JsonFields.text (account, "aci"); }
            if (id == "") { return false; }
            if (active_account != "" && active_account != id) { store = null; current = null; remove_all_rows (); }
            active_account = id;
            self_aci = JsonFields.text (account, "aci");
            link_button.label = "Refresh";
            link_button.tooltip_text = "Refresh linked Signal contacts and groups";
            return true;
        }

        private void new_conversation_dialog () {
            if (store == null) { return; }
            Gtk.Box content;
            var dialog = action_window ("New conversation", out content);
            content.append (new Gtk.Label ("Enter a Signal phone number with country code or a recipient UUID.") { wrap = true, max_width_chars = 38, xalign = 0.0f });
            var recipient = new Gtk.Entry () { placeholder_text = "+15551234567", activates_default = true };
            content.append (recipient);
            var name = new Gtk.Entry () { placeholder_text = "Name (optional)" };
            content.append (name);
            var error_label = new Gtk.Label ("") { wrap = true, xalign = 0.0f };
            content.append (error_label);
            var open = new Gtk.Button.with_label ("Open conversation");
            open.add_css_class ("suggested-action");
            open.clicked.connect (() => {
                string id = recipient.text.strip ();
                try {
                    var pattern = new Regex ("^(\\+[1-9][0-9]{3,14}|[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})$");
                    if (!pattern.match (id)) { error_label.label = "Enter a full phone number or a UUID."; return; }
                } catch (Error error) { return; }
                var c = store.ensure (id, name.text.strip () == "" ? id : name.text.strip (), false);
                persist (c);
                add_conversation_row (c.name, c.id, false);
                for (Gtk.Widget? row = conversation_list.get_first_child (); row != null; row = row.get_next_sibling ()) {
                    if (row.get_data<string> ("recipient") == c.id) { conversation_list.select_row ((Gtk.ListBoxRow) row); break; }
                }
                dialog.destroy ();
            });
            content.append (open);
            dialog.set_default_widget (open);
            dialog.present ();
        }

        private void show_details () {
            if (current == null) { return; }
            Gtk.Box content;
            var dialog = action_window ("Conversation details", out content);
            content.append (new Gtk.Label (current.name) { wrap = true, selectable = true, xalign = 0.0f });
            content.append (new Gtk.Label (current.id) { wrap = true, selectable = true, xalign = 0.0f });
            string description = JsonFields.text (current.metadata, "description");
            if (description != "") { content.append (new Gtk.Label (description) { wrap = true, xalign = 0.0f }); }
            int64 expiration = JsonFields.integer (current.metadata, "messageExpirationTime");
            content.append (new Gtk.Label (expiration > 0 ? ("Disappearing messages: %" + int64.FORMAT + " seconds").printf (expiration) : "Disappearing messages: off") {
                xalign = 0.0f
            });
            if (current.is_group) {
                var members = JsonFields.array (current.metadata, "members");
                var list = new Gtk.Box (Gtk.Orientation.VERTICAL, 6);
                for (uint i = 0; i < members.get_length (); i++) {
                    if (members.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                    var member = members.get_object_element (i);
                    string address = JsonFields.address (member, "number", "uuid", "number");
                    list.append (new Gtk.Label (address + (JsonFields.boolean (member, "isAdmin") ? " · Admin" : "")) {
                        wrap = true, selectable = true, xalign = 0.0f
                    });
                }
                content.append (new Gtk.ScrolledWindow () { child = list, min_content_height = 120, max_content_height = 260, propagate_natural_height = true });
            }
            var timer_entry = new Gtk.SpinButton.with_range (0, 2419200, 1) { value = expiration, sensitive = expiration_backend };
            content.append (new Gtk.Label ("Disappearing messages · seconds (0 turns the timer off)") { wrap = true, xalign = 0.0f });
            content.append (timer_entry);
            var set_timer = new Gtk.Button.with_label ("Set timer") { sensitive = expiration_backend };
            var timer_conversation = current;
            set_timer.clicked.connect (() => {
                set_timer.sensitive = false;
                set_expiration.begin (timer_conversation, (int) timer_entry.value, set_timer);
            });
            content.append (set_timer);
            var close = new Gtk.Button.with_label ("Close");
            close.clicked.connect (() => dialog.destroy ());
            content.append (close);
            dialog.present ();
        }

        private async void set_expiration (Conversation c, int seconds, Gtk.Button button) {
            try {
                var o = new Json.Object ();
                o.set_string_member ("account", active_account);
                o.set_string_member (c.is_group ? "groupId" : "recipient", c.id);
                o.set_int_member ("expiration", seconds);
                yield backend.rpc (c.is_group ? "updateGroup" : "updateContact", MessageCommands.node (o));
                c.metadata.set_int_member ("messageExpirationTime", seconds);
                c.metadata.set_int_member ("timerUpdatedAt", GLib.get_real_time () / 1000);
                persist (c);
                conversation_status.label = seconds == 0 ? "Disappearing messages turned off" : "Disappearing-message timer updated";
                update_composer ();
            } catch (Error error) { conversation_status.label = "Timer update failed: " + error.message; }
            button.sensitive = true;
        }

        private string display_peer (string address) {
            if (store == null) { return address; }
            string canonical = store.canonical (address);
            foreach (var c in store.conversations) { if (c.id == canonical) { return c.name; } }
            return address;
        }

        private void update_call_panel () {
            if (calls == null) { return; }
            var call = calls.current;
            call_bar.visible = call != null || calls.error_message != "";
            bool incoming = call != null && call.state == "RINGING_INCOMING";
            bool connected = call != null && (call.state == "CONNECTED" || call.state == "RECONNECTING");
            call_accept.visible = incoming;
            call_accept.label = call != null && call.group_id != "" ? "Join call" : "Answer";
            call_accept.tooltip_text = call != null && call.video ? "Answer with camera off" : "Answer voice call";
            call_reject.visible = incoming;
            call_hangup.visible = calls.active && !incoming;
            call_mute.visible = connected;
            call_speaker.visible = connected;
            call_camera.visible = connected && call != null && (call.video || call.group_id != "");
            call_camera.active = calls.video_media.camera_enabled;
            call_camera.label = call_camera.active ? "Turn camera off" : "Turn camera on";
            call_mute.label = call_mute.active ? "Unmute mic" : "Mute mic";
            call_speaker.label = call_speaker.active ? "Unmute speaker" : "Mute speaker";
            call_show_video.visible = connected && call != null && (call.video || call.group_id != "");
            call_roster.visible = connected && call != null && call.group_id != "";
            if (call != null && call.group_id != "") {
                var names = new StringBuilder ();
                for (uint i=0; i<call.participants.get_length (); i++) {
                    var participant = call.participants.get_object_element (i);
                    if (i>0) { names.append ("\n"); }
                    names.append (display_peer (JsonFields.text (participant,"uuid","Participant")) +
                        (JsonFields.boolean (participant,"audioMuted") ? " · Microphone muted" : "") +
                        (JsonFields.boolean (participant,"videoMuted") ? " · Camera off" : ""));
                }
                call_roster.label = names.str;
                foreach (string key in video_pictures.get_keys ()) {
                    bool present = false;
                    for (uint i=0; i<call.participants.get_length (); i++) {
                        var participant = call.participants.get_object_element (i);
                        if (JsonFields.integer (participant,"demuxId").to_string () == key && !JsonFields.boolean (participant,"videoMuted")) { present = true; break; }
                    }
                    if (!present) {
                        var picture = video_pictures.lookup (key);
                        if (picture != null && picture.get_parent () != null) { video_tiles.remove (picture.get_parent ()); }
                        video_pictures.remove (key);
                    }
                }
            }
            if (call != null && call.group_id == "" && call.remote_video_known && !call.remote_video) {
                foreach (var tile in video_pictures.get_values ()) { tile.paintable = null; }
            }
            if (call == null) { call_status.label = calls.error_message; }
            else {
                string state = call.state == "RINGING_INCOMING" ? (call.group_id != "" ? "Incoming group call" : call.video ? "Incoming video call" : "Incoming voice call") :
                    call.state == "RINGING_OUTGOING" ? "Calling" : call.state == "CONNECTED" ? "Call connected" :
                    call.state == "RECONNECTING" ? "Reconnecting call" : call.state == "CONNECTING" ? "Connecting call" : "Call ended";
                call_status.label = state + (call.peer != "" ? " · " + display_peer (call.group_id != "" ? call.group_id : call.peer) : "") +
                    (calls.error_message != "" ? " · " + calls.error_message : "") +
                    (connected && call.group_id != "" ? " · " + call.participants.get_length ().to_string () + (call.participants.get_length () == 1 ? " other device" : " other devices") : "");
            }
            if (incoming && !is_active && notifications.active) {
                var notification = new Notification (call.group_id != "" ? "Incoming Signal group call" : call.video ? "Incoming Signal video call" : "Incoming Signal voice call");
                notification.set_body ("Open Signal Pantheon to answer or decline");
                notification.set_priority (NotificationPriority.HIGH);
                notification.set_default_action_and_target_value ("app.show-conversation", new Variant.string (call.group_id != "" ? call.group_id : call.peer));
                application.send_notification ("voice-call", notification);
            }
            if (!calls.active && video_window != null) { video_window.destroy (); video_window = null; self_preview = null; self_preview_label = null; camera_selector = null; video_pictures.remove_all (); }
            if (!incoming || !notifications.active) { application.withdraw_notification ("voice-call"); }
            if (!calls.active) { application.withdraw_notification ("voice-call"); call_mute.active = false; call_speaker.active = false; }
            update_composer ();
        }

        private void ensure_video_window () {
            if (video_window != null) { return; }
            var window = new Gtk.Window () { title = "Signal call video", transient_for = this, default_width = 680, default_height = 520 };
            video_window = window;
#if UI_TEST
            if (Environment.get_variable ("SIGNAL_TEST_NARROW") == "1") { window.set_default_size (440, 480); }
#endif
            var layout = new Gtk.Box (Gtk.Orientation.VERTICAL, 8) { margin_start = 12, margin_end = 12, margin_top = 12, margin_bottom = 12 };
            var controls = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            var label = new Gtk.Label ("Camera") { xalign = 0 };
            var selector = new Gtk.DropDown (new Gtk.StringList ({ "Loading cameras…" }), null) { sensitive = false, hexpand = true, tooltip_text = "Choose the camera used for this call" };
            camera_selector = selector;
            label.mnemonic_widget = selector;
            controls.append (label); controls.append (selector); layout.append (controls);
            self_preview = new Gtk.Picture () { can_shrink = true, content_fit = Gtk.ContentFit.CONTAIN, visible = false, tooltip_text = "Your camera preview" };
            self_preview_label = new Gtk.Label ("Your camera is off") { xalign = 0 };
            var preview_box = new Gtk.Overlay () { width_request = 160, height_request = 120, halign = Gtk.Align.START, valign = Gtk.Align.START };
            preview_box.set_child (new Gtk.Box (Gtk.Orientation.HORIZONTAL, 0));
            preview_box.add_overlay (self_preview);
            self_preview.bind_property ("visible", preview_box, "visible", BindingFlags.SYNC_CREATE);
            layout.append (self_preview_label); layout.append (preview_box);
            video_tiles = new Gtk.FlowBox () { selection_mode = Gtk.SelectionMode.NONE, max_children_per_line = 3, column_spacing = 8, row_spacing = 8 };
            layout.append (new Gtk.ScrolledWindow () { child = video_tiles, vexpand = true });
            window.set_child (layout);
            window.close_request.connect (() => { window.hide (); return true; });
            window.present ();
            call_show_video.visible = true;
            populate_cameras.begin (window, selector);
        }

        private async void populate_cameras (Gtk.Window window, Gtk.DropDown selector) {
            var choices = yield calls.video_media.list_cameras ();
            if (video_window != window || calls == null || !calls.active) { return; }
            var names = new Gtk.StringList (null);
            foreach (var choice in choices) { names.append (choice.name); }
            selector.model = names; selector.selected = 0; selector.sensitive = true;
            selector.notify["selected"].connect (() => {
                if (selector.selected >= choices.length || !selector.sensitive) { return; }
                selector.sensitive = false;
                calls.select_camera.begin (choices[selector.selected], (obj, res) => {
                    calls.select_camera.end (res);
                    if (video_window == window && calls.active) { selector.sensitive = true; }
                });
            });
        }

        private void show_self_preview (Gdk.Paintable? picture) {
            if (picture != null && calls != null && calls.active) { ensure_video_window (); }
            if (self_preview == null) { return; }
            self_preview.paintable = picture;
            self_preview.visible = picture != null;
            self_preview_label.label = picture != null ? "You · Camera on" : "Your camera is off";
        }

        private void show_video_frame (uint track, Gdk.Paintable picture) {
            if (calls == null || !calls.active) { return; }
            if (calls.current.group_id == "" && calls.current.remote_video_known && !calls.current.remote_video) { return; }
            ensure_video_window ();
            if (calls.current.group_id != "") {
                bool allowed = false;
                for (uint i=0; i<calls.current.participants.get_length (); i++) {
                    var participant = calls.current.participants.get_object_element (i);
                    if (JsonFields.integer (participant,"demuxId") == track && !JsonFields.boolean (participant,"videoMuted")) { allowed = true; break; }
                }
                if (!allowed) { return; }
            }
            string key = track.to_string ();
            var tile = video_pictures.lookup (key);
            if (tile == null) {
                if (video_pictures.size () >= 16) { return; }
                tile = new Gtk.Picture () { width_request = 240, height_request = 180, can_shrink = true, content_fit = Gtk.ContentFit.CONTAIN };
                tile.tooltip_text = "Participant video " + key;
                video_pictures.insert (key, tile); video_tiles.insert (tile, -1);
            }
            tile.paintable = picture;
        }

        private async void close_with_call () {
            save_draft ();
            if (calls != null) { yield calls.hangup (); }
            if (expiration != null) { expiration.stop (); }
            if (maintenance_timer != 0) { Source.remove (maintenance_timer); maintenance_timer = 0; }
            backend.stop ();
            destroy ();
        }

#if UI_TEST
        public bool fixture_ready () { return service_ready && store != null && store.conversations.length >= 2; }
        public async void fixture_exercise (string screenshot) throws Error {
            assert (store != null);
            var c = store.lookup ("+15550000001");
            assert (c != null && c.messages.length == 1);
            for (Gtk.Widget? row = conversation_list.get_first_child (); row != null; row = row.get_next_sibling ()) {
                if (row.get_data<string> ("recipient") == c.id) { conversation_list.select_row ((Gtk.ListBoxRow) row); break; }
            }
            assert (current == c && message_entry.sensitive);
            message_entry.text = "Keyboard";
            message_entry.grab_focus ();
            var keyboard = new Subprocess.newv ({"xdotool", "search", "--onlyvisible", "--name", "^Signal$", "windowfocus", "--sync", "key", "shift+Return"}, SubprocessFlags.STDOUT_SILENCE);
            yield keyboard.wait_check_async ();
            yield fixture_pause ();
            assert (message_entry.text == "Keyboard\n" && c.messages.length == 1);
            message_entry.text = "fixture-fail";
            yield send_message ();
            assert (c.messages.length == 1 && message_entry.text == "fixture-fail");
            message_entry.text = "Outgoing fixture text";
            yield send_message ();
            assert (c.messages.length == 2 && c.draft == "");
            var sent = c.messages[1];
            editing_timestamp = sent.timestamp;
            message_entry.text = "Edited fixture text";
            yield send_message ();
            assert (c.messages.length == 2 && sent.text == "Edited fixture text" && sent.modified > 0);
            message_entry.text = "Encrypted draft survives restart";
            save_draft ();
            string attachment_path = Path.build_filename (Environment.get_user_cache_dir (), "fixture-attachment.txt");
            DirUtils.create_with_parents (Path.get_dirname (attachment_path), 448);
            FileUtils.set_contents (attachment_path, "Fixture attachment bytes");
            c.attachment_draft = attachment_path;
            message_entry.text = "";
            yield send_message ();
            assert (c.messages.length == 3 && c.messages[2].attachments.get_length () == 1);
            var staged_attachment = c.messages[2].attachments.get_object_element (0);
            var staged_file = yield AttachmentFiles.resolve (backend.attachment_directory, staged_attachment);
            assert (staged_file.get_path () != attachment_path);
            string staged_contents;
            FileUtils.get_contents (staged_file.get_path (), out staged_contents);
            assert (staged_contents == "Fixture attachment bytes");
            message_entry.text = "Encrypted draft survives restart";
            save_draft ();
            var saved = new ClientStore (backend, active_account);
            saved.restore ();
            var restored = saved.lookup (c.id);
            assert (restored != null && restored.draft == c.draft && restored.messages.length == 3);
            var dialog = new Gtk.Window ();
            yield mutate_message (c, sent, "reaction", "👍", false, dialog);
            assert (JsonFields.text (sent.reactions, active_account) == "👍");
            history_search.text = "does-not-match";
            render_history ();
            assert (((Gtk.Label) message_history.get_first_child ()).label == "No matching messages");
            history_search.text = "";
            render_history ();
            yield fixture_capture (screenshot);
            dialog = new Gtk.Window ();
            yield mutate_message (c, sent, "delete", "", false, dialog);
            assert (sent.deleted && sent.text == "" && sent.attachments.get_length () == 0);
            // The older prototype format migrates without retaining a second live history copy.
            backend.save_message ("legacy-recipient", 12345, false, "Legacy fixture text");
            var legacy = store.ensure ("legacy-recipient", "Legacy", false);
            store.import_legacy (legacy);
            assert (legacy.messages.length == 1 && legacy.messages[0].text == "Legacy fixture text");
            assert (!backend.message_exists ("legacy-recipient", 12345, false, "Legacy fixture text"));
            notifications.active = true;
            read_receipts.active = true;
            assert (FileUtils.test (preference_path, FileTest.EXISTS));
            var notification_event = new Json.Parser ();
            notification_event.load_from_data ("""{"method":"receive","params":{"account":"+15550000000","envelope":{"sourceNumber":"+15550000001","dataMessage":{"timestamp":10001,"message":"Private notification fixture body","groupInfo":{"groupId":"fixture-group","groupName":"Friends Fixture"}}}}}""");
            handle_incoming_message (notification_event.get_root ());
            yield fixture_pause ();
            string notification_log;
            FileUtils.get_contents (Environment.get_variable ("SIGNAL_TEST_NOTIFICATION_LOG"), out notification_log);
            assert (notification_log.contains ("You have a new message") && !notification_log.contains ("Private notification fixture body"));
            yield set_expiration (c, 1, new Gtk.Button ());
            assert (JsonFields.integer (c.metadata, "messageExpirationTime") == 1);
            string expiring_path = Path.build_filename (Environment.get_user_cache_dir (), "expiring-original.txt");
            FileUtils.set_contents (expiring_path, "Expiring attachment bytes");
            c.attachment_draft = expiring_path;
            message_entry.text = "Disappearing fixture text";
            yield send_message ();
            var expiring_record = c.messages[c.messages.length - 1];
            assert (expiring_record.expiration_seconds == 1 && expiring_record.expires_at_ms > 0);
            string expiring_owned = JsonFields.text (expiring_record.attachments.get_object_element (0), "localPath");
            assert (FileUtils.test (expiring_owned, FileTest.EXISTS));
            yield fixture_capture (Environment.get_variable ("SIGNAL_TEST_EXPIRATION_SCREENSHOT") ?? "");
            yield fixture_pause (1200);
            yield expiration.tick ();
            assert (c.find (expiring_record.timestamp, active_account) == null);
            assert (!FileUtils.test (expiring_owned, FileTest.EXISTS) && FileUtils.test (expiring_path, FileTest.EXISTS));
            assert (!c.serialize ().contains ("Disappearing fixture text"));
            yield set_expiration (c, 0, new Gtk.Button ());
            assert (calls != null && calls.available);
            yield calls.start (c.id);
            yield fixture_pause ();
            assert (calls.current != null && calls.current.id == int64.MIN + 1 && calls.current.state == "CONNECTED");
            call_mute.active = true;
            call_speaker.active = true;
            yield fixture_capture (Environment.get_variable ("SIGNAL_TEST_CALL_SCREENSHOT") ?? "");
            yield calls.hangup ();
            assert (!calls.active && calls.current.state == "ENDED");
            var incoming_call = new Json.Parser ();
            incoming_call.load_from_data ("""{"params":{"subscription":7,"result":{"callId":42,"number":"+15550000001","state":"RINGING_INCOMING","inputDeviceName":"signal_input_42","outputDeviceName":"signal_output_42"}}}""");
            backend.call_event (incoming_call.get_root ());
            assert (calls.current.id == 42 && call_accept.visible);
            yield calls.reject ();
            assert (!calls.active);
            incoming_call.load_from_data ("""{"params":{"subscription":7,"result":{"callId":43,"number":"+15550000001","state":"RINGING_INCOMING","inputDeviceName":"signal_input_43","outputDeviceName":"signal_output_43"}}}""");
            backend.call_event (incoming_call.get_root ());
            yield calls.accept ();
            yield fixture_pause ();
            assert (calls.current.id == 43 && calls.current.state == "CONNECTED");
            yield calls.hangup ();
            yield calls.start (c.id, true);
            for (int attempt=0; attempt<50 && (!calls.video_media.camera_enabled || video_window == null); attempt++) { yield fixture_pause (100); }
            if (video_window == null || !calls.video_media.camera_enabled) { stderr.printf ("Fixture video diagnostic: state=%s camera=%s error=%s\n",calls.current.state,calls.video_media.camera_enabled.to_string (),calls.error_message); }
            assert (calls.current.video && calls.video_media.camera_enabled && video_window != null);
            for (int attempt=0; attempt<40 && (self_preview == null || self_preview.paintable == null || camera_selector == null || !camera_selector.sensitive); attempt++) { yield fixture_pause (50); }
            assert (self_preview != null && self_preview.paintable != null && self_preview.visible);
            assert (camera_selector != null && camera_selector.sensitive);
            camera_selector.selected = 2;
            for (int attempt=0; attempt<40 && (!camera_selector.sensitive || self_preview.paintable == null); attempt++) { yield fixture_pause (50); }
            assert (camera_selector.sensitive && calls.video_media.camera_enabled && self_preview.paintable != null);
            camera_selector.selected = 3;
            for (int attempt=0; attempt<40 && !camera_selector.sensitive; attempt++) { yield fixture_pause (50); }
            assert (!calls.video_media.camera_enabled && self_preview.paintable == null && calls.error_message != "");
            camera_selector.selected = 2;
            yield fixture_pause ();
            yield calls.camera (true);
            for (int attempt=0; attempt<40 && self_preview.paintable == null; attempt++) { yield fixture_pause (50); }
            assert (calls.video_media.camera_enabled && self_preview.paintable != null && calls.error_message == "");
            yield fixture_capture (Environment.get_variable ("SIGNAL_TEST_VIDEO_SCREENSHOT") ?? "", video_window);
            yield calls.camera (false);
            assert (!calls.video_media.camera_enabled && self_preview.paintable == null && !self_preview.visible);
            yield fixture_pause (150);
            assert (self_preview.paintable == null);
            camera_selector.selected = 1;
            yield fixture_pause ();
            assert (!calls.video_media.camera_enabled && self_preview.paintable == null);
            string fixture_video_socket = calls.current.video_socket;
            yield calls.hangup ();
            assert (video_window == null && self_preview == null);
            var video_incoming = new Json.Object ();
            video_incoming.set_int_member ("callId",144); video_incoming.set_string_member ("state","RINGING_INCOMING");
            video_incoming.set_string_member ("number",c.id); video_incoming.set_boolean_member ("video",true);
            video_incoming.set_string_member ("videoSocket",fixture_video_socket);
            video_incoming.set_string_member ("inputDeviceName","signal_input_1"); video_incoming.set_string_member ("outputDeviceName","signal_output_1");
            var video_parameters = new Json.Object (); video_parameters.set_int_member ("subscription",7); video_parameters.set_object_member ("result",video_incoming);
            var video_event = new Json.Object (); video_event.set_object_member ("params",video_parameters);
            backend.call_event (MessageCommands.node (video_event));
            assert (!calls.video_media.camera_enabled);
            yield calls.accept (); yield fixture_pause ();
            assert (calls.current.video && !calls.current.outgoing && !calls.video_media.camera_enabled);
            call_show_video.clicked ();
            for (int attempt=0; attempt<40 && (camera_selector == null || !camera_selector.sensitive); attempt++) { yield fixture_pause (50); }
            assert (camera_selector != null && camera_selector.sensitive);
            camera_selector.selected = 2;
            yield fixture_pause ();
            assert (!calls.video_media.camera_enabled && self_preview.paintable == null);
            yield calls.camera (true);
            for (int attempt=0; attempt<40 && self_preview.paintable == null; attempt++) { yield fixture_pause (50); }
            assert (self_preview.paintable != null);

            yield calls.hangup ();
            yield calls.start ("fixture-group", true, true);
            for (int attempt=0; attempt<50 && (!calls.video_media.camera_enabled || video_window == null); attempt++) { yield fixture_pause (100); }
            if (video_window == null || !calls.video_media.camera_enabled) { stderr.printf ("Fixture group diagnostic: group=%s state=%s outgoing=%s camera=%s error=%s\n",calls.current.group_id,calls.current.state,calls.current.outgoing.to_string (),calls.video_media.camera_enabled.to_string (),calls.error_message); }
            assert (calls.current.group_id == "fixture-group" && calls.current.participants.get_length () == 1 && video_window != null);
            yield fixture_capture (Environment.get_variable ("SIGNAL_TEST_GROUP_SCREENSHOT") ?? "");
            yield calls.hangup ();
            uint8[] pixels = new uint8[320*240*4];
            for (int i=0; i<pixels.length; i+=4) { pixels[i]=40; pixels[i+1]=160; pixels[i+2]=200; pixels[i+3]=255; }
            var texture = new Gdk.MemoryTexture (320,240,Gdk.MemoryFormat.R8G8B8A8,new Bytes.take ((owned) pixels),320*4);
            string media_path = Path.build_filename (Environment.get_user_data_dir (), "signal-pantheon", "outgoing", "view-once.png");
            assert (texture.save_to_png (media_path));
            var once = new MessageEvent (); once.author = c.id; once.conversation_id = c.id; once.timestamp = GLib.get_real_time ()/1000;
            once.view_once = true;
            var photo = new Json.Object (); photo.set_string_member ("localPath",media_path); photo.set_string_member ("contentType","image/png");
            once.attachments.add_object_element (photo); c.apply (once); store.save (c);
            var record = c.find (once.timestamp,once.author);
            yield open_view_once (c,record);
            assert (record.view_opened && record.attachments.get_length () == 0 && !FileUtils.test (media_path,FileTest.EXISTS));
            var windows = Gtk.Window.get_toplevels ();
            for (uint i=0; i<windows.get_n_items (); i++) {
                var viewer = (Gtk.Window) windows.get_item (i);
                if (viewer.title == "View-once media") { yield fixture_capture (Environment.get_variable ("SIGNAL_TEST_VIEW_ONCE_SCREENSHOT") ?? "",viewer); viewer.close (); break; }
            }
            yield open_view_once (c,record);
            assert (record.view_opened);
            string clip = Path.build_filename (Path.get_dirname (Environment.get_variable ("SIGNAL_CLI_PATH")),"fixtures","view-once.webm");
            string owned_clip = Path.build_filename (Environment.get_user_data_dir (),"signal-pantheon","outgoing","view-once.webm");
            yield File.new_for_path (clip).copy_async (File.new_for_path (owned_clip),FileCopyFlags.NONE,Priority.DEFAULT,null,null);
            var once_video = new MessageEvent (); once_video.author=c.id;once_video.conversation_id=c.id;once_video.timestamp=once.timestamp+1;once_video.view_once=true;
            var film = new Json.Object ();film.set_string_member ("localPath",owned_clip);film.set_string_member ("contentType","video/webm");once_video.attachments.add_object_element (film);
            c.apply (once_video);store.save(c);
            yield open_view_once (c,c.find (once_video.timestamp,c.id));
            for (int attempt=0; attempt<50 && view_once_stream != null && !view_once_stream.is_prepared () && view_once_stream.get_error () == null; attempt++) { yield fixture_pause (100); }
            assert (view_once_stream != null && view_once_stream.is_prepared () && view_once_stream.get_error () == null);
            assert (!FileUtils.test (owned_clip,FileTest.EXISTS));
            if (view_once_window != null) { view_once_window.close (); }
            var ring = new Json.Object (); ring.set_int_member ("callId",171);ring.set_string_member ("state","RINGING_INCOMING");ring.set_string_member ("number",c.id);ring.set_string_member ("groupId","fixture-group");ring.set_boolean_member ("video",true);ring.set_string_member ("videoSocket",fixture_video_socket);
            var parameters = new Json.Object ();parameters.set_int_member ("subscription",7);parameters.set_object_member ("result",ring);
            var ring_event = new Json.Object ();ring_event.set_object_member ("params",parameters);
            backend.call_event (MessageCommands.node (ring_event));
            assert (calls.active && call_accept.label == "Join call" && !calls.video_media.camera_enabled);
            yield fixture_capture (Environment.get_variable ("SIGNAL_TEST_RING_SCREENSHOT") ?? "");
            ring.set_string_member ("state","ENDED");ring.set_string_member ("reason","cancelled_by_ringer");backend.call_event (MessageCommands.node (ring_event));assert (!calls.active);
            ring.set_string_member ("state","RINGING_INCOMING");backend.call_event (MessageCommands.node (ring_event));assert (!calls.active);
            ring.set_int_member ("callId",172);backend.call_event (MessageCommands.node (ring_event));yield calls.accept ();yield fixture_pause ();
            assert (calls.current.group_id == "fixture-group" && calls.current.state == "CONNECTED" && !calls.video_media.camera_enabled);
            yield calls.hangup ();
            ring.set_int_member ("callId",173);backend.call_event (MessageCommands.node (ring_event));yield calls.reject ();assert (!calls.active);
            ring.set_int_member ("callId",174);backend.call_event (MessageCommands.node (ring_event));ring.set_string_member ("state","ENDED");ring.set_string_member ("reason","accepted_on_another_device");backend.call_event (MessageCommands.node (ring_event));assert (!calls.active);
            print ("PASS: native messaging, media, view once, group ringing, camera selection/failure/recovery and preview lifecycle\n");
        }
        private async void fixture_capture (string screenshot, Gtk.Window? target = null) throws Error {
            if (screenshot != "") {
                yield fixture_pause ();
                var widget = target ?? this;
                var paintable = new Gtk.WidgetPaintable (widget);
                var snapshot = new Gtk.Snapshot ();
                paintable.snapshot (snapshot, widget.get_width (), widget.get_height ());
                var node = snapshot.to_node ();
                assert (node != null);
                var renderer = Gsk.Renderer.for_surface (widget.get_surface ());
                var texture = renderer.render_texture (node, null);
                assert (texture.save_to_png (screenshot));
                renderer.unrealize ();
            }
        }
        private async void fixture_pause (uint duration = 250) {
            Timeout.add (duration, fixture_pause.callback);
            yield;
        }
#endif

        private void save_preferences () {
            preferences.set_boolean ("Messaging", "notifications", notifications.active);
            preferences.set_boolean ("Messaging", "read-receipts", read_receipts.active);
            try {
                DirUtils.create_with_parents (Path.get_dirname (preference_path), 448);
                FileUtils.set_contents (preference_path, preferences.to_data ());
            } catch (Error error) { connection_status.label = "Could not save preferences: " + error.message; }
        }

        private void persist (Conversation c) {
            if (store == null) { return; }
            try { store.save (c); }
            catch (Error error) { connection_status.label = "Encrypted history could not be saved: " + error.message; }
        }

        private void save_draft () {
            if (current == null || updating_composer) { return; }
            if (editing_timestamp == 0) { current.draft = message_entry.text; }
            persist (current);
        }

        private void update_composer () {
            bool allowed = current != null && !JsonFields.boolean (current.metadata, "isBlocked") &&
                !JsonFields.boolean (current.metadata, "unregistered") &&
                !JsonFields.boolean (current.metadata, "isTerminated") &&
                (expiration_backend || JsonFields.integer (current.metadata, "messageExpirationTime") == 0);
            if (current != null && !allowed) {
                conversation_status.label = "Messaging is unavailable here: the conversation is blocked, inactive, or uses disappearing messages.";
            }
            if (current != null && current.is_group) {
                if (current.metadata.has_member ("isMember") && !JsonFields.boolean (current.metadata, "isMember")) { allowed = false; }
                if (JsonFields.text (current.metadata, "permissionSendMessage") == "ONLY_ADMINS") {
                    bool admin = false;
                    var members = JsonFields.array (current.metadata, "members");
                    for (uint i = 0; i < members.get_length (); i++) {
                        if (members.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                        var member = members.get_object_element (i);
                        if ((JsonFields.text (member, "number") == active_account ||
                            self_aci != "" && JsonFields.text (member, "uuid") == self_aci) && JsonFields.boolean (member, "isAdmin")) { admin = true; }
                    }
                    allowed = allowed && admin;
                }
                if (!allowed) { conversation_status.label = "Messaging is unavailable with this group's current membership, permissions or timer."; }
            }
            bool enabled = allowed && service_ready && !send_in_progress;
            new_conversation_button.sensitive = service_ready && store != null;
            voice_button.sensitive = current != null && service_ready && calls != null && calls.available && !calls.active;
            if (current != null && current.is_group) { voice_button.sensitive = voice_button.sensitive && group_backend; }
            video_button.sensitive = voice_button.sensitive && video_backend;
            voice_button.label = current != null && current.is_group ? "Join group voice call" : "Voice call";
            video_button.label = current != null && current.is_group ? "Join group video call" : "Video call";
            voice_button.tooltip_text = calls != null && calls.error_message != "" ? calls.error_message : "Start or join a call";
            message_entry.sensitive = enabled;
            send_button.sensitive = enabled;
            attach_button.sensitive = enabled && editing_timestamp == 0;
            string path = current != null ? current.attachment_draft : "";
            attachment_label.label = path == "" ? "" : "Attachment: " + Path.get_basename (path);
            attachment_label.tooltip_text = path;
            updating_composer = true;
            view_once_toggle.active = current != null && current.view_once_draft;
            updating_composer = false;
            view_once_toggle.visible = path != "";
            view_once_toggle.sensitive = enabled && view_once_backend;
            clear_attachment_button.visible = path != "";
            clear_attachment_button.sensitive = enabled;
            send_button.label = editing_timestamp == 0 ? "Send" : "Save Edit";
        }

        private async void choose_attachment () {
            if (current == null) { return; }
            Conversation c = current;
            try {
                var dialog = new Gtk.FileDialog () { title = "Attach a file" };
                File file = yield dialog.open (this, null);
                string? path = file.get_path ();
                var info = yield file.query_info_async ("standard::type,standard::size", FileQueryInfoFlags.NONE);
                if (path == null || info.get_file_type () != FileType.REGULAR || info.get_size () > 100 * 1024 * 1024) {
                    throw new IOError.INVALID_ARGUMENT ("Choose a local file up to 100 MiB");
                }
                c.attachment_draft = path;
                persist (c);
                if (current == c) { update_composer (); }
            } catch (Gtk.DialogError.DISMISSED error) {
            } catch (Error error) { conversation_status.label = "Could not attach file: " + error.message; }
        }

        private async void send_message () {
            if (send_in_progress || current == null || !service_ready || !send_button.sensitive) { return; }
            Conversation c = current;
            string text = message_entry.text.strip ();
            string attachment = editing_timestamp == 0 ? c.attachment_draft : "";
            bool view_once = editing_timestamp == 0 && c.view_once_draft;
            int64 edit = editing_timestamp;
            if (text == "" && attachment == "") { return; }
            send_in_progress = true;
            update_composer ();
            try {
                if (attachment != "") {
                    var file = File.new_for_path (attachment);
                    var info = yield file.query_info_async ("standard::type,standard::size", FileQueryInfoFlags.NONE);
                    if (info.get_file_type () != FileType.REGULAR || info.get_size () > 100 * 1024 * 1024) {
                        throw new IOError.INVALID_ARGUMENT ("The attachment is unavailable or exceeds 100 MiB");
                    }
                }
                if (view_once) {
                    if (!view_once_backend) { throw new IOError.NOT_SUPPORTED ("View-once media requires the bundled backend"); }
                    if (attachment == "" || text != "") { throw new IOError.INVALID_ARGUMENT ("View-once messages require one photo or video without text"); }
                    var info = yield File.new_for_path (attachment).query_info_async ("standard::content-type", FileQueryInfoFlags.NONE);
                    string mime = ContentType.get_mime_type (info.get_content_type ());
                    if (!mime.has_prefix ("image/") && !mime.has_prefix ("video/")) { throw new IOError.INVALID_ARGUMENT ("Choose a photo or video for view once"); }
                }
                string original_attachment = attachment;
                if (attachment != "") {
                    string folder = Path.build_filename (Environment.get_user_data_dir (), "signal-pantheon", "outgoing", Uuid.string_random ());
                    DirUtils.create_with_parents (folder, 448);
                    string staged = Path.build_filename (folder, Path.get_basename (attachment));
                    yield File.new_for_path (attachment).copy_async (File.new_for_path (staged), FileCopyFlags.NONE, Priority.DEFAULT, null, null);
                    FileUtils.chmod (staged, 384);
                    attachment = staged;
                }
                var parameters = MessageCommands.send (active_account, c, text, attachment, edit);
                if (view_once) { parameters.get_object ().set_boolean_member ("viewOnce", true); }
                var result = yield backend.rpc ("send", parameters);
                bool partial;
                int64 timestamp = MessageCommands.confirm (result, out partial);
                var e = new MessageEvent ();
                e.view_once = view_once;
                e.kind = edit > 0 ? "edit" : "message";
                e.conversation_id = c.id;
                e.text = text;
                e.outgoing = true;
                e.author = active_account;
                e.timestamp = timestamp;
                var result_object = result.get_object ();
                var expirations = JsonFields.object (result_object, "expirationByRecipient");
                e.expiration_seconds = expirations != null ? JsonFields.integer (expirations, c.id,
                    JsonFields.integer (c.metadata, "messageExpirationTime")) : JsonFields.integer (c.metadata, "messageExpirationTime");
                e.expiration_start_ms = JsonFields.integer (result_object, "expirationStartTimestamp",
                    expiration != null ? expiration.now_ms () : GLib.get_real_time () / 1000);
                e.target_timestamp = edit;
                if (attachment != "") {
                    var a = new Json.Object ();
                    a.set_string_member ("filename", Path.get_basename (original_attachment));
                    a.set_string_member ("localPath", attachment);
                    e.attachments.add_object_element (a);
                }
                c.apply (e);
                var sent = c.find (edit > 0 ? edit : timestamp, active_account);
                if (sent != null && partial) { sent.status = "partial"; }
                if (edit == 0) { c.draft = ""; c.attachment_draft = ""; c.view_once_draft = false; }
                persist (c);
                if (current == c) {
                    updating_composer = true;
                    message_entry.text = edit > 0 ? c.draft : "";
                    updating_composer = false;
                    editing_timestamp = 0;
                    conversation_status.label = partial ? "Some recipients did not receive the message. Check before retrying." : "Accepted by Signal";
                    render_history ();
                }
            } catch (Error error) {
                if (view_once && attachment != "" && attachment != c.attachment_draft) {
                    var owned_attachment = new Json.Object (); owned_attachment.set_string_member ("localPath",attachment);
                    try { yield AttachmentFiles.expire_owned (backend.attachment_directory, Path.build_filename (Environment.get_user_data_dir (),"signal-pantheon","outgoing"),owned_attachment); } catch (Error ignored) {}
                }
                if (current == c) { conversation_status.label = "Send could not be confirmed: " + error.message; }
            }
            send_in_progress = false;
            update_composer ();
        }

        private void drain_events () {
            var queued = pending_events;
            pending_events = new GenericArray<Json.Node> ();
            foreach (var event in queued) { handle_incoming_message (event); }
        }

        private void handle_incoming_message (Json.Node event) {
            if (store == null || active_account == "" || !catalog_ready) {
                pending_events.add (event.copy ());
                return;
            }
            var e = MessageEvent.parse (event, active_account);
            if (e == null) { return; }
            if (e.outgoing) { e.author = active_account; }
            else { e.author = store.canonical (e.author); }
            if (!e.is_group) { e.conversation_id = store.canonical (e.conversation_id); }
            if (e.target_author == self_aci && self_aci != "") { e.target_author = active_account; }
            else { e.target_author = store.canonical (e.target_author); }
            if (e.kind == "group_call") {
                var group = store.ensure (e.conversation_id,e.name,true);
                group.metadata.set_int_member ("groupCallActivityAt",e.timestamp); persist (group);
                if (current == group) { conversation_status.label = "Group call activity. Use Join group voice call or Join group video call."; }
                return;
            }
            if (e.kind == "view_once_open") {
                if (viewing_once != null && viewing_once.author == e.author && viewing_once.timestamp == e.target_timestamp && view_once_window != null) { view_once_window.close (); }
                foreach (var c in store.conversations) { if (c.apply (e)) { persist (c); } }
                if (current != null) { render_history (); }
                expiration.tick.begin ();
                return;
            }
            if (e.kind == "timer") {
                var timed = store.ensure (e.conversation_id, e.name, e.is_group);
                if (e.kind == "timer" && e.timestamp >= JsonFields.integer (timed.metadata, "timerUpdatedAt")) {
                    timed.metadata.set_int_member ("messageExpirationTime", e.expiration_seconds);
                    timed.metadata.set_int_member ("timerUpdatedAt", e.timestamp);
                }
                if (e.ephemeral) {
                    for (uint i = timed.messages.length; i > 0; i--) {
                        if (timed.messages[i - 1].timestamp == e.timestamp && timed.messages[i - 1].author == e.author) {
                            timed.messages.remove_index (i - 1);
                        }
                    }
                    connection_status.label = "A view-once message arrived. Read it on your phone; it was not added to local history.";
                }
                persist (timed);
                if (current == timed) { update_composer (); render_history (); }
                return;
            }
            if (e.kind == "read_sync") {
                foreach (var c in store.conversations) {
                    bool changed = false;
                    for (uint i = 0; i < e.read_markers.get_length (); i++) {
                        if (e.read_markers.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                        var marker = e.read_markers.get_object_element (i);
                        var read = new MessageEvent ();
                        read.kind = "read_sync";
                        read.author = store.canonical (JsonFields.address (marker, "senderNumber", "senderUuid", "sender"));
                        read.target_timestamp = JsonFields.integer (marker, "timestamp");
                        read.timestamp = e.timestamp;
                        if (read.author != "" && read.target_timestamp > 0) { changed = c.apply (read) || changed; }
                    }
                    if (changed) { persist (c); if (current == c) { render_history (); } }
                }
                expiration.tick.begin ();
                return;
            }
            if (e.kind == "receipt") {
                // Direct receipts have no group context. Do not infer all-member group delivery.
                var c = store.lookup (e.conversation_id);
                if (c != null && !c.is_group && c.apply (e)) {
                    persist (c);
                    if (current == c) { render_history (); }
                }
                return;
            }
            var c = store.ensure (e.conversation_id, e.name, e.is_group);
            if (!c.apply (e)) { return; }
            if (!e.outgoing && e.kind == "message" && (!is_active || current != c)) { c.unread++; }
            c.expire (expiration.now_ms ());
            persist (c);
            add_conversation_row (c.name, c.id, c.is_group);
            update_unread (c);
            if (current == c) { render_history (); if (is_active) { mark_read.begin (c); } }
            if (e.kind == "message" && !e.outgoing && notifications.active && (!is_active || current != c)) {
                var notification = new Notification ("Signal Pantheon");
                notification.set_body ("You have a new message");
                notification.set_default_action_and_target_value ("app.show-conversation", new Variant.string (c.id));
                application.send_notification ("message", notification);
            }
        }

        private void update_unread (Conversation c) {
            for (Gtk.Widget? row = conversation_list.get_first_child (); row != null; row = row.get_next_sibling ()) {
                if (row.get_data<string> ("recipient") == c.id) {
                    var label = row.get_data<Gtk.Label> ("name-label");
                    if (label != null) { label.label = c.unread > 0 ? "%s (%u)".printf (c.name, c.unread) : c.name; }
                    return;
                }
            }
        }

        private async void mark_read (Conversation c) {
            if (!is_active || current != c) { return; }
            int64 now = expiration != null ? expiration.now_ms () : GLib.get_real_time () / 1000;
            bool newly_read = c.mark_read (now, read_receipts.active);
            if (newly_read) { render_history (); }
            update_unread (c);
            application.withdraw_notification ("message");
            try { store.save (c); } catch (Error error) {
                connection_status.label = "Read state could not be saved: " + error.message;
                return;
            }
            if (expiration != null) { expiration.tick.begin (); }
            yield flush_reads (c);
        }

        private async void flush_all_reads () {
            if (store == null || flushing_reads || closing) { return; }
            foreach (var c in store.conversations) { yield flush_reads (c); }
        }

        private async void flush_reads (Conversation c) {
            if (flushing_reads || !service_ready || !expiration_backend) { return; }
            flushing_reads = true;
            for (uint i = c.read_sync_queue.get_length (); i > 0; i--) {
                var marker = c.read_sync_queue.get_object_element (i - 1);
                try {
                    bool partial;
                    var parameters = MessageCommands.receipt (active_account, JsonFields.text (marker, "author"), JsonFields.integer (marker, "timestamp"));
                    parameters.get_object ().set_boolean_member ("syncOnly", true);
                    if (JsonFields.boolean (marker, "viewOnceOpen")) { parameters.get_object ().set_boolean_member ("viewOnceOpen", true); }
                    parameters.get_object ().set_int_member ("readAt", JsonFields.integer (marker, "readAt"));
                    if (!JsonFields.boolean (marker, "syncSent")) {
                        var result = yield backend.rpc ("sendReceipt", parameters.copy ());
                        MessageCommands.confirm (result, out partial);
                        if (partial) { throw new IOError.FAILED ("Read sync was partial"); }
                        marker.set_boolean_member ("syncSent", true);
                        store.save (c);
                    }
                    if (JsonFields.boolean (marker, "sendRemote") && !JsonFields.boolean (marker, "remoteSent")) {
                        parameters.get_object ().set_boolean_member ("syncOnly", false);
                        if (JsonFields.boolean (marker, "viewedReceipt")) { parameters.get_object ().set_string_member ("type", "viewed"); }
                        var result = yield backend.rpc ("sendReceipt", parameters);
                        MessageCommands.confirm (result, out partial, true);
                        if (partial) { throw new IOError.FAILED ("Read receipt was partial"); }
                        marker.set_boolean_member ("remoteSent", true);
                    }
                    c.read_sync_queue.remove_element (i - 1);
                    store.save (c);
                } catch (Error error) {
                    conversation_status.label = "Read sync will retry: " + error.message;
                    break;
                }
            }
            flushing_reads = false;
        }

        private void render_history () {
            clear_history ();
            if (current == null) { return; }
            Conversation c = current;
            c.expire (expiration != null ? expiration.now_ms () : GLib.get_real_time () / 1000);
            string query = history_search.text.casefold ();
            uint start = c.messages.length > history_limit && query == "" ? c.messages.length - history_limit : 0;
            if (start > 0) {
                var older = new Gtk.Button.with_label ("Load older messages");
                older.clicked.connect (() => { history_limit += 100; render_history (); });
                message_history.append (older);
            }
            uint shown = 0;
            for (uint i = start; i < c.messages.length; i++) {
                var message = c.messages[i];
                if (query != "" && !message.text.casefold ().contains (query) &&
                    !attachment_names (message).casefold ().contains (query)) { continue; }
                append_record (c, message);
                shown++;
            }
            refresh_expiry_labels (expiration != null ? expiration.now_ms () : GLib.get_real_time () / 1000);
            if (shown == 0) {
                message_history.append (new Gtk.Label (query == "" ? "No messages yet" : "No matching messages"));
            }
            if (query == "") {
                Idle.add (() => {
                    var adjustment = history_scroll.vadjustment;
                    adjustment.value = adjustment.upper - adjustment.page_size;
                    return Source.REMOVE;
                });
            }
        }

        private async void open_view_once (Conversation c, MessageRecord message) {
            if (message.view_opened || !message.view_once || message.attachments.get_length () != 1) { return; }
            var attachment = message.attachments.get_object_element (0).ref ();
            try {
                var file = yield AttachmentFiles.resolve (backend.attachment_directory, attachment);
                var info = yield file.query_info_async ("standard::size", FileQueryInfoFlags.NOFOLLOW_SYMLINKS);
                if (info.get_size () > 100 * 1024 * 1024) { throw new IOError.INVALID_DATA ("View-once media exceeds 100 MiB"); }
                var input = yield file.read_async ();
                // Commit consumption before opening the bytes: a crash cannot allow a second view.
                c.consume_view_once (message);
                if (c.read_sync_queue.get_length () > 0) {
                    var marker = c.read_sync_queue.get_object_element (c.read_sync_queue.get_length () - 1);
                    marker.set_boolean_member ("sendRemote", read_receipts.active);
                    marker.set_boolean_member ("viewedReceipt", true);
                }
                store.save (c);
                if (current == c) { render_history (); }
                var contents = new ByteArray ();
                while (true) {
                    var chunk = yield input.read_bytes_async (65536);
                    if (chunk.get_size () == 0) { break; }
                    if (contents.len + chunk.get_size () > 100 * 1024 * 1024) { throw new IOError.INVALID_DATA ("View-once media exceeds 100 MiB"); }
                    contents.append (chunk.get_data ());
                }
                yield input.close_async ();
                yield expiration.tick ();
                flush_reads.begin (c);
                var bytes = ByteArray.free_to_bytes ((owned) contents);
                var dialog = new Gtk.Window () { title = "View-once media", transient_for = this, modal = true, default_width = 640, default_height = 480 };
                if (view_once_window != null) { view_once_window.close (); }
                viewing_once = message; view_once_window = dialog;
                dialog.close_request.connect (() => {
                    dialog.set_child (null);
                    if (view_once_window == dialog) { view_once_window = null; viewing_once = null; view_once_stream = null; }
                    return false;
                });
                string mime = JsonFields.text (attachment, "contentType");
                if (mime.has_prefix ("video/")) {
                    var stream = Gtk.MediaFile.for_input_stream (new MemoryInputStream.from_bytes (bytes));
                    view_once_stream = stream;
                    stream.notify["error"].connect (() => { if (stream.get_error () != null) { conversation_status.label = "View-once video could not be played: " + stream.get_error ().message; } });
                    var video = new Gtk.Video.for_media_stream (stream) { autoplay = true };
                    dialog.set_child (video);
                    dialog.close_request.connect (() => { stream.pause (); video.media_stream = null; return false; });
                } else { dialog.set_child (new Gtk.Picture.for_paintable (Gdk.Texture.from_bytes (bytes)) { can_shrink = true, content_fit = Gtk.ContentFit.CONTAIN }); }
                var escape = new Gtk.EventControllerKey ();
                escape.key_pressed.connect ((key, code, state) => { if (key == Gdk.Key.Escape) { dialog.close (); return true; } return false; });
                ((Gtk.Widget) dialog).add_controller (escape);
                dialog.present ();
            } catch (Error error) { conversation_status.label = "Could not open view-once media: " + error.message; }
        }

        private string attachment_names (MessageRecord message) {
            var names = new StringBuilder ();
            for (uint i = 0; i < message.attachments.get_length (); i++) {
                if (message.attachments.get_element (i).get_node_type () == Json.NodeType.OBJECT) {
                    names.append (JsonFields.text (message.attachments.get_object_element (i), "filename", "Attachment"));
                }
            }
            return names.str;
        }

        private void append_record (Conversation c, MessageRecord message) {
            var box = new Gtk.Box (Gtk.Orientation.VERTICAL, 6) {
                halign = message.outgoing ? Gtk.Align.END : Gtk.Align.START
            };
            var bubble = new Gtk.Label (message.deleted ? "Message deleted" : message.text) {
                wrap = true, selectable = true, xalign = 0.0f, max_width_chars = 58,
                wrap_mode = Pango.WrapMode.WORD_CHAR
            };
            bubble.add_css_class ("message-bubble");
            bubble.add_css_class (message.outgoing ? "outgoing" : "incoming");
            if (message.text != "" || message.deleted) { box.append (bubble); }
            if (message.view_once && !message.deleted) {
                var view = new Gtk.Button.with_label (message.view_opened ? (message.outgoing ? "View-once media sent" : "View-once media opened") : message.attachments.get_length () != 1 ? "View-once media unavailable" : "Open view-once media");
                view.sensitive = !message.view_opened && !message.outgoing && message.attachments.get_length () == 1;
                view.clicked.connect (() => open_view_once.begin (c, message));
                box.append (view);
            }
            if (!message.deleted && !message.view_once) {
                for (uint i = 0; i < message.attachments.get_length (); i++) {
                    if (message.attachments.get_element (i).get_node_type () != Json.NodeType.OBJECT) { continue; }
                    var attachment = message.attachments.get_object_element (i);
                    var button = new Gtk.Button.with_label ("Save " + JsonFields.text (attachment, "filename", "attachment"));
                    button.clicked.connect (() => save_attachment.begin (attachment));
                    box.append (button);
                }
            }
            var details = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 6);
            var time = new DateTime.from_unix_local (message.timestamp / 1000);
            string status = message.outgoing ? (message.status == "read" ? "Read" : message.status == "delivered" ? "Delivered" : message.status == "partial" ? "Partially sent" : "Sent") : message.author;
            var caption = new Gtk.Label (time.format ("%x %H:%M") + " · " + status + (message.modified > 0 ? " · Edited" : "")) {
                wrap = true, xalign = 0.0f, max_width_chars = 52
            };
            caption.set_data<string> ("base-caption", caption.label);
            if (message.expiration_seconds > 0) { expiry_labels.insert (message, caption); }
            caption.add_css_class ("dim-label");
            details.append (caption);
            if (!message.deleted && !message.view_once) {
                var actions = new Gtk.MenuButton () { icon_name = "view-more-symbolic", tooltip_text = "Message actions" };
                var popover = new Gtk.Popover ();
                var buttons = new Gtk.Box (Gtk.Orientation.VERTICAL, 6) {
                    margin_start = 8, margin_end = 8, margin_top = 8, margin_bottom = 8
                };
                var react = new Gtk.Button.with_label ("React…");
                react.clicked.connect (() => { popover.popdown (); reaction_dialog (c, message); });
                buttons.append (react);
                if (message.outgoing) {
                    var edit = new Gtk.Button.with_label ("Edit…");
                    edit.clicked.connect (() => {
                        popover.popdown ();
                        if (current != c || send_in_progress) { return; }
                        save_draft ();
                        editing_timestamp = message.timestamp;
                        updating_composer = true;
                        message_entry.text = message.text;
                        updating_composer = false;
                        conversation_status.label = "Editing a sent message. Press Escape to cancel.";
                        update_composer ();
                        message_entry.grab_focus ();
                    });
                    edit.sensitive = message.attachments.get_length () == 0;
                    buttons.append (edit);
                    var remove = new Gtk.Button.with_label ("Delete for everyone…");
                    remove.clicked.connect (() => { popover.popdown (); delete_dialog (c, message); });
                    buttons.append (remove);
                }
                popover.set_child (buttons);
                actions.set_popover (popover);
                details.append (actions);
            }
            box.append (details);
            if (message.reactions.get_size () > 0) {
                var reaction_text = new StringBuilder ();
                foreach (string author in message.reactions.get_members ()) {
                    reaction_text.append (JsonFields.text (message.reactions, author) + " ");
                }
                var label = new Gtk.Label (reaction_text.str) { tooltip_text = "Reactions", xalign = 0.0f, wrap = true };
                box.append (label);
            }
            message_history.append (box);
        }

        private Gtk.Window action_window (string title, out Gtk.Box content) {
            var window = new Gtk.Window () { title = title, transient_for = this, modal = true, default_width = 380 };
            window.set_titlebar (new Gtk.HeaderBar ());
            content = new Gtk.Box (Gtk.Orientation.VERTICAL, 12) {
                margin_start = 20, margin_end = 20, margin_top = 20, margin_bottom = 20
            };
            window.set_child (content);
            return window;
        }

        private void reaction_dialog (Conversation c, MessageRecord message) {
            Gtk.Box content;
            var window = action_window ("React to message", out content);
            var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 6);
            foreach (string emoji in new string[] { "👍", "❤️", "😂", "😮", "😢", "🙏" }) {
                string chosen = emoji;
                var button = new Gtk.Button.with_label (chosen);
                button.clicked.connect (() => {
                    window.sensitive = false;
                    bool remove = JsonFields.text (message.reactions, active_account) == chosen;
                    mutate_message.begin (c, message, "reaction", chosen, remove, window);
                });
                row.append (button);
            }
            content.append (row);
            content.append (new Gtk.Label ("Choose your current reaction again to remove it.") { wrap = true });
            window.present ();
        }

        private void delete_dialog (Conversation c, MessageRecord message) {
            Gtk.Box content;
            var window = action_window ("Delete for everyone?", out content);
            content.append (new Gtk.Label ("Signal may reject deletion of an older message. Local history changes only after Signal confirms the request.") { wrap = true, max_width_chars = 38 });
            var cancel = new Gtk.Button.with_label ("Cancel");
            cancel.clicked.connect (() => window.destroy ());
            content.append (cancel);
            var button = new Gtk.Button.with_label ("Delete for everyone");
            button.add_css_class ("destructive-action");
            button.clicked.connect (() => {
                window.sensitive = false;
                mutate_message.begin (c, message, "delete", "", false, window);
            });
            content.append (button);
            window.present ();
        }

        private async void mutate_message (Conversation c, MessageRecord message, string kind,
                                           string emoji, bool remove, Gtk.Window window) {
            try {
                if (!service_ready) { throw new IOError.NOT_CONNECTED ("Reconnect Signal first"); }
                Json.Node parameters = kind == "delete"
                    ? MessageCommands.delete_message (active_account, c, message)
                    : MessageCommands.reaction (active_account, c, message, emoji, remove);
                var result = yield backend.rpc (kind == "delete" ? "remoteDelete" : "sendReaction", parameters);
                bool partial;
                int64 timestamp = MessageCommands.confirm (result, out partial);
                if (partial) { throw new IOError.FAILED ("Only some recipients accepted the request; local content is unchanged"); }
                var e = new MessageEvent ();
                e.kind = kind;
                e.author = active_account;
                e.target_author = message.author;
                e.target_timestamp = message.timestamp;
                e.timestamp = timestamp;
                e.emoji = emoji;
                e.remove_reaction = remove;
                c.apply (e);
                persist (c);
                if (current == c) { render_history (); }
                window.destroy ();
            } catch (Error error) {
                window.sensitive = true;
                conversation_status.label = "Action could not be confirmed: " + error.message;
            }
        }

        private async void save_attachment (Json.Object attachment) {
            try {
                File source = yield AttachmentFiles.resolve (backend.attachment_directory, attachment);
                var dialog = new Gtk.FileDialog () { title = "Save attachment", initial_name = Path.get_basename (JsonFields.text (attachment, "filename", "attachment")) };
                var destination = yield dialog.save (this, null);
                yield source.copy_async (destination, FileCopyFlags.OVERWRITE, Priority.DEFAULT, null, null);
                conversation_status.label = "Attachment saved";
            } catch (Gtk.DialogError.DISMISSED error) {
            } catch (Error error) { conversation_status.label = "Could not save attachment: " + error.message; }
        }

        private void refresh_expiry_labels (int64 now) {
            expiry_labels.foreach ((message, label) => {
                string suffix = message.expires_at_ms == 0 ? " · Disappears after reading" :
                    " · Disappears in " + int64.max (0, (message.expires_at_ms - now + 999) / 1000).to_string () + " s";
                label.label = label.get_data<string> ("base-caption") + suffix;
            });
        }

        private void clear_history () {
            expiry_labels.remove_all ();
            Gtk.Widget? child = message_history.get_first_child ();
            while (child != null) {
                Gtk.Widget? next = child.get_next_sibling ();
                message_history.remove (child);
                child = next;
            }
        }
    }
}
