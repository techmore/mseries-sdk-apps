#!/usr/bin/python3
"""GTK 4 / Granite 7 Messages client for the existing BlueFerry daemon."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
import gettext
import os
from pathlib import Path
import subprocess
import time

os.environ.setdefault('GSK_RENDERER', 'cairo')
import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
gi.require_version('Granite', '7.0')
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import Gdk, GdkPixbuf, Gio, GLib, Granite, Gtk, Pango
from blueferry.client import BackendClient
from blueferry.bus import get_system_bus
from blueferry.recipients import validate_recipient
from messages_model import ConversationState, merge_inbox
from message_pins import PinStore, profile_key, read_snapshot, suggested_matches, matched_state, ordered_keys, effective_choices

_ = gettext.gettext
APP_ID = 'io.elementary.mseries.Messages'
STYLE = '''
.ms-sidebar { background: @theme_base_color; }
.ms-status { padding: 8px 14px; border-top: 1px solid alpha(@theme_fg_color, .12); }
.ms-row { padding: 12px; }
.ms-avatar { border-radius: 99px; background: alpha(@theme_selected_bg_color,.18); padding: 12px; }
.ms-contact-name { font-size: 1.15em; font-weight: 600; }
.ms-photo { border-radius: 99px; min-width: 42px; min-height: 42px; }
.ms-bubble { border-radius: 16px; padding: 12px 16px; background: alpha(@theme_fg_color,.08); }
.ms-outgoing { background: alpha(@theme_selected_bg_color,.25); color: @theme_fg_color; border: 1px solid alpha(@theme_selected_bg_color,.35); }
.ms-composer { padding: 14px 20px; border-top: 1px solid alpha(@theme_fg_color,.12); }
.ms-heading { padding: 12px 20px; border-bottom: 1px solid alpha(@theme_fg_color,.12); }
.ms-time { font-size: .85em; opacity: .72; }
.ms-notice { padding: 8px 20px; background: alpha(@theme_selected_bg_color,.10); }
.ms-section { padding: 12px 12px 4px; font-weight: 600; opacity: .75; }
.ms-pin-note { padding: 4px 14px 10px; }
.ms-pin-row { border-bottom: 1px solid alpha(@theme_fg_color,.08); padding: 12px 0; }
.ms-favorite { padding: 4px; }
.ms-favorite:checked { background: alpha(@theme_selected_bg_color,.18); }
.ms-favorite-avatar { border-radius: 99px; background: alpha(@theme_selected_bg_color,.18); min-width: 42px; min-height: 42px; }
'''


def label(text='', css=None, **kwargs):
    widget = Gtk.Label(label=text, **kwargs)
    if css:
        widget.add_css_class(css)
    return widget


def clear(box):
    while box.get_first_child():
        box.remove(box.get_first_child())


class Messages(Gtk.Application):
    def __init__(self, fixture=False, dark=False, narrow=False):
        super().__init__(application_id=APP_ID + ('.Preview' if fixture else ''),
                         flags=Gio.ApplicationFlags.NON_UNIQUE if fixture else Gio.ApplicationFlags.DEFAULT_FLAGS)
        self.fixture, self.dark, self.narrow = fixture, dark, narrow
        self.pool = ThreadPoolExecutor(max_workers=1)
        self.model = ConversationState()
        self.threads = {}
        self.rows = {}
        self.inbox = []
        self.last_map = False
        self.status = {}
        self.snapshot_pending = False
        self.closed = False
        self.rendered = None
        self.poll = 0
        self.search_generation = 0
        self.last_refresh = 0
        self.inbox_requested = True
        self.inbox_request_epoch = 0
        self.sleep_watch = None
        self.send_feedback = {}
        self.photo_cache = {}
        self.photo_revision = None
        self.photo_pending = False
        self.avatars = {}
        self.favorite_avatars = {}
        self.favorite_buttons = {}
        self.pin_state = dict(version=1, snapshot=None, choices=[])
        self.pin_profile = None
        self.pin_store = None
        self.pin_pending = False

    def do_activate(self):
        if self.get_active_window():
            self.get_active_window().present()
            return
        if hasattr(Granite, 'init'):
            Granite.init()
        settings = Granite.Settings.get_default()
        def scheme(*_args):
            Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme = self.dark if self.fixture else settings.get_prefers_color_scheme() == Granite.SettingsColorScheme.DARK
        settings.connect('notify::prefers-color-scheme', scheme)
        scheme()
        provider = Gtk.CssProvider()
        provider.load_from_string(STYLE)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.window = Gtk.ApplicationWindow(application=self, title=_('Messages'), default_width=1080, default_height=720)
        self.window.set_icon_name(APP_ID)
        self.window.set_size_request(420, 420)
        self.window.connect('close-request', self.close)
        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        header = Gtk.HeaderBar()
        self.back = Gtk.Button(icon_name='go-previous-symbolic', tooltip_text=_('Conversations'), visible=False)
        self.back.connect('clicked', self.go_back)
        self.back.update_property([Gtk.AccessibleProperty.LABEL], [_('Conversations')])
        header.pack_start(self.back)
        header.set_title_widget(label(_('Messages'), 'title'))
        new = Gtk.Button(icon_name='document-edit-symbolic', tooltip_text=_('New Message'))
        new.connect('clicked', self.new_message)
        new.update_property([Gtk.AccessibleProperty.LABEL], [_('New Message')])
        header.pack_start(new)
        refresh = Gtk.Button(icon_name='view-refresh-symbolic', tooltip_text=_('Refresh iPhone Inbox'))
        refresh.connect('clicked', lambda *_args: self.refresh(True))
        refresh.update_property([Gtk.AccessibleProperty.LABEL], [_('Refresh iPhone Inbox')])
        header.pack_end(refresh)
        menu = Gio.Menu()
        for text, action, method in [(_('Import iPhone Pins…'), 'import-pins', self.import_pins), (_('Review Imported Pins…'), 'review-pins', self.review_pins), (_('Connect iPhone…'), 'setup', self.setup), (_('Advanced BlueFerry…'), 'advanced', self.advanced)]:
            a = Gio.SimpleAction.new(action, None)
            a.connect('activate', method)
            self.add_action(a)
            menu.append(text, 'app.' + action)
        header.pack_end(Gtk.MenuButton(icon_name='open-menu-symbolic', menu_model=menu))
        outer.append(header)
        self.split = Gtk.Paned(orientation=Gtk.Orientation.HORIZONTAL, vexpand=True)
        self.split.set_position(300)
        self.split.set_shrink_start_child(False)
        self.split.set_shrink_end_child(False)
        self.sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, width_request=260)
        self.sidebar.add_css_class('ms-sidebar')
        self.search = Gtk.SearchEntry(placeholder_text=_('Search conversations'), margin_top=10, margin_bottom=10, margin_start=12, margin_end=12)
        self.search.connect('search-changed', lambda *_args: self.filter_rows())
        self.sidebar.append(self.search)
        self.pin_note = label('', 'ms-pin-note', xalign=0, wrap=True, visible=False)
        self.pin_note.add_css_class('dim-label')
        self.sidebar.append(self.pin_note)
        self.list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        self.list.add_css_class('navigation-sidebar')
        self.list.connect('row-selected', self.selected)
        contacts = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.favorites = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, visible=False)
        self.favorites.append(label(_('Favorites'), 'ms-section', xalign=0))
        self.favorite_grid = Gtk.Grid(column_homogeneous=True, row_spacing=8, column_spacing=4, margin_start=10, margin_end=10, margin_bottom=12)
        self.favorites.append(self.favorite_grid)
        contacts.append(self.favorites)
        self.favorite_separator = Gtk.Separator(orientation=Gtk.Orientation.HORIZONTAL, margin_start=12, margin_end=12, margin_bottom=8, visible=False)
        contacts.append(self.favorite_separator)
        contacts.append(self.list)
        list_scroll = Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=contacts)
        self.sidebar.append(list_scroll)
        self.connection = label(_('Connecting to iPhone…'), xalign=0, wrap=True)
        self.connection.add_css_class('ms-status')
        self.sidebar.append(self.connection)
        self.split.set_start_child(self.sidebar)
        self.detail = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, width_request=280)
        self.title = label(_('Your conversations'), 'title-2', xalign=0, ellipsize=Pango.EllipsizeMode.END)
        self.subtitle = label(_('Messages sent through your iPhone'), 'dim-label', xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE)
        heading = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        heading.add_css_class('ms-heading')
        heading.append(self.title)
        heading.append(self.subtitle)
        self.detail.append(heading)
        self.notice = label('', xalign=0, wrap=True, visible=False)
        self.notice.add_css_class('ms-notice')
        self.detail.append(self.notice)
        self.stack = Gtk.Stack(vexpand=True)
        welcome = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, valign=Gtk.Align.CENTER, halign=Gtk.Align.CENTER, margin_start=24, margin_end=24)
        icon = Gtk.Image.new_from_icon_name('mail-message-new-symbolic')
        icon.set_pixel_size(64)
        welcome.append(icon)
        welcome.append(label(_('A little closer, wherever you work.'), 'title-2', wrap=True, justify=Gtk.Justification.CENTER))
        welcome.append(label(_('Choose a conversation or start a new message.\nYour iPhone sends and receives the texts.'), 'dim-label', wrap=True, justify=Gtk.Justification.CENTER))
        start = Gtk.Button(label=_('New Message'), halign=Gtk.Align.CENTER)
        start.add_css_class('suggested-action')
        start.connect('clicked', self.new_message)
        welcome.append(start)
        self.stack.add_named(welcome, 'welcome')
        self.messages = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14, margin_start=24, margin_end=24, margin_top=22, margin_bottom=22)
        self.message_scroll = Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=self.messages)
        self.stack.add_named(self.message_scroll, 'messages')
        self.detail.append(self.stack)
        compose = Gtk.Box(spacing=12)
        compose.add_css_class('ms-composer')
        self.editor = Gtk.TextView(wrap_mode=Gtk.WrapMode.WORD_CHAR, accepts_tab=False, top_margin=10, bottom_margin=10, left_margin=12, right_margin=12)
        self.editor.update_property([Gtk.AccessibleProperty.LABEL], [_('Message')])
        self.editor.get_buffer().connect('changed', self.draft_changed)
        editor_scroll = Gtk.ScrolledWindow(hexpand=True, min_content_height=64, max_content_height=140, propagate_natural_height=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=self.editor)
        editor_scroll.add_css_class('frame')
        compose.append(editor_scroll)
        self.send = Gtk.Button(icon_name='mail-send-symbolic', tooltip_text=_('Send Message (Ctrl+Enter)'), valign=Gtk.Align.END)
        self.send.add_css_class('suggested-action')
        self.send.connect('clicked', self.send_message)
        self.send.update_property([Gtk.AccessibleProperty.LABEL], [_('Send Message')])
        compose.append(self.send)
        self.detail.append(compose)
        self.send_status = label(_('Choose a conversation to write a message.'), 'dim-label', xalign=0, wrap=True, margin_start=20, margin_end=20, margin_bottom=10)
        self.detail.append(self.send_status)
        self.split.set_end_child(self.detail)
        outer.append(self.split)
        self.window.set_child(outer)
        self.window.connect('notify::default-width', lambda *_a: self.layout(self.window.get_default_size()[0] < 760))
        keys = Gtk.EventControllerKey()
        keys.connect('key-pressed', self.key_pressed)
        self.editor.add_controller(keys)
        self.window.add_tick_callback(self.responsive)
        self.window.present()
        if self.narrow:
            self.window.set_default_size(480, 680)
        if self.fixture:
            self.preview()
        else:
            try:
                self.sleep_watch = get_system_bus().add_signal_receiver(
                    self.sleep_changed, signal_name='PrepareForSleep',
                    dbus_interface='org.freedesktop.login1.Manager',
                    bus_name='org.freedesktop.login1', path='/org/freedesktop/login1')
            except Exception:
                # Periodic wall-time/reconnect checks still work without logind.
                pass
            self.refresh(True)
            self.poll = GLib.timeout_add_seconds(5, self.refresh)

    def task(self, work, completed):
        future = self.pool.submit(work)
        def done(result):
            try:
                value, error = result.result(), None
            except Exception as exc:
                value, error = None, str(exc)
            GLib.idle_add(self.deliver, completed, value, error)
        future.add_done_callback(done)

    def deliver(self, completed, value, error):
        if not self.closed:
            completed(value, error)
        return False

    def refresh(self, inbox=False):
        if inbox:
            self.inbox_request_epoch += 1
        self.inbox_requested = self.inbox_requested or inbox
        if self.closed or self.fixture or self.snapshot_pending:
            return True
        self.snapshot_pending = True
        prior = self.last_map
        requested = self.inbox_requested
        epoch = self.inbox_request_epoch
        def load():
            client = BackendClient()
            status = client.status().to_dict()
            recent, warning = None, ''
            # Wall time includes laptop suspend; the next awake poll catches up.
            if status.get('map') and (requested or not prior or time.time() - self.last_refresh > 300):
                try:
                    recent = client.recent('telecom/msg/INBOX', 200)
                except Exception:
                    warning = _('Could not refresh the iPhone inbox. Showing messages already received.')
            return status, [t.to_dict() for t in client.threads()], recent, warning, epoch
        self.task(load, self.loaded)
        return True

    def loaded(self, result, error):
        self.snapshot_pending = False
        if error:
            self.status = dict(map=False)
            self.last_map = False
            self.connection.set_label(_('iPhone unavailable · conversations kept'))
            self.notice.set_label(_('Messages are offline. Keep your iPhone nearby with Bluetooth on. Use Connect iPhone if it needs setup.'))
            self.notice.set_visible(True)
            self.update_composer()
            return
        status, threads, recent, warning, epoch = result
        self.status, self.last_map = status, bool(status.get('map'))
        revision = status.get('contact_photo_revision')
        if revision != self.photo_revision:
            self.photo_cache.clear()
            self.photo_revision = revision
            self.row_signature = None
        if recent is not None:
            self.inbox = recent
            self.last_refresh = time.time()
            self.inbox_requested = epoch != self.inbox_request_epoch
        self.connection.set_label(_('iPhone connected · Messages ready') if self.last_map else _('iPhone reconnecting…'))
        self.notice.set_label(warning or ('' if self.last_map else _('Waiting for message access. Existing conversations remain available.')))
        self.notice.set_visible(bool(self.notice.get_label()))
        if self.last_refresh:
            stamp = datetime.now().astimezone().strftime('%X') if recent is not None else self.checked_text
            self.checked_text = stamp
            self.connection.set_tooltip_text(_('Recent iPhone inbox checked at %s. Up to 200 messages exposed by the phone; full iCloud history is not available.') % stamp)
        combined = merge_inbox(threads, self.inbox)
        # A phone-only/new conversation acquires a canonical backend identity
        # after the first live event. Move the draft without changing its route.
        for key, thread in self.threads.items():
            if (thread.get('new') or thread.get('inbox_only')) and key not in {t['key'] for t in combined}:
                match = next((t for t in combined if not t.get('is_group') and t.get('recipients') == thread.get('recipients')), None)
                if match and key not in self.model.pending:
                    if key in self.model.drafts:
                        self.model.drafts[match['key']] = self.model.drafts.pop(key)
                    if key in self.send_feedback:
                        self.send_feedback[match['key']] = self.send_feedback.pop(key)
                    if self.model.selected == key:
                        self.model.selected = match['key']
                elif thread.get('new') or key in self.model.pending:
                    combined.append(thread)
        self.threads = {t['key']: t for t in combined}
        self.load_pins()
        self.update_pin_note()
        self.populate()
        self.show_conversation()
        self.load_photos()

    def photo_widget(self, data):
        try:
            loader = GdkPixbuf.PixbufLoader.new()
            loader.set_size(42, 42)
            loader.write(data)
            loader.close()
            texture = Gdk.Texture.new_for_pixbuf(loader.get_pixbuf())
        except GLib.Error:
            return None
        picture = Gtk.Picture.new_for_paintable(texture)
        picture.set_content_fit(Gtk.ContentFit.COVER)
        picture.set_size_request(42, 42)
        picture.set_can_shrink(True)
        frame = Gtk.Box(overflow=Gtk.Overflow.HIDDEN, valign=Gtk.Align.CENTER)
        frame.add_css_class('ms-photo')
        frame.append(picture)
        return frame

    def load_photos(self):
        if self.fixture or self.photo_pending or not self.status.get('contact_photos'):
            return
        # BlueFerry validates encoded size and image dimensions before returning
        # photos. Keep decoded textures in this process only, never in a new file.
        visible = [self.threads[key] for key in ordered_keys(self.threads, self.pin_state)[:40]]
        wanted = {t['recipients'][0] for t in visible
                  if not t.get('is_group') and t.get('recipients') and t['recipients'][0] not in self.photo_cache}
        if not wanted:
            return
        self.photo_pending = True
        revision = self.photo_revision
        def load():
            client = BackendClient()
            return {address: client.contact_photo(address) for address in wanted}
        def loaded(photos, error):
            self.photo_pending = False
            if error or revision != self.photo_revision:
                return
            self.photo_cache.update(photos)
            for key, (content, old) in [*self.avatars.items(), *self.favorite_avatars.items()]:
                thread = self.threads.get(key, {})
                address = (thread.get('recipients') or [''])[0]
                data = photos.get(address)
                if data and (photo := self.photo_widget(data)):
                    content.remove(old)
                    content.insert_child_after(photo, None)
                    registry = self.favorite_avatars if content.has_css_class('ms-favorite-content') else self.avatars
                    registry[key] = (content, photo)
            self.load_photos()
        self.task(load, loaded)

    def populate(self):
        order = ordered_keys(self.threads, self.pin_state)
        pinned = set(effective_choices(self.threads, self.pin_state))
        # Avoid destroying focus/selection when a status poll changes nothing.
        signatures = [(key, self.threads[key]['name'], key in pinned, self.threads[key].get('unread_count', 0), self.threads[key]['messages'][-1].get('body', '') if self.threads[key]['messages'] else '') for key in order]
        if signatures != getattr(self, 'row_signature', None):
            self.row_signature = signatures
            self.list.unselect_all()
            clear(self.list)
            self.rows = {}
            self.avatars = {}
            self.favorite_avatars = {}
            self.favorite_buttons = {}
            clear(self.favorite_grid)
            self.grid_filter_signature = None
            for key in order:
                thread = self.threads[key]
                row = Gtk.ListBoxRow()
                row.key = key
                is_pinned = row.is_pinned = key in pinned
                content = Gtk.Box(spacing=12)
                content.add_css_class('ms-row')
                avatar = label((thread['name'] or '?')[0].upper(), 'ms-avatar')
                address = (thread.get('recipients') or [''])[0]
                if not thread.get('is_group') and (data := self.photo_cache.get(address)):
                    avatar = self.photo_widget(data) or avatar
                content.append(avatar)
                self.avatars[key] = (content, avatar)
                text = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, hexpand=True)
                title = label(thread['name'], xalign=0, ellipsize=Pango.EllipsizeMode.END)
                title.add_css_class('ms-contact-name')
                if thread.get('unread_count'):
                    title.add_css_class('heading')
                text.append(title)
                last = thread['messages'][-1].get('body', '') if thread['messages'] else _('New conversation')
                text.append(label(last.replace('\n', ' '), 'dim-label', xalign=0, ellipsize=Pango.EllipsizeMode.END, max_width_chars=24))
                content.append(text)
                if is_pinned:
                    pin_icon = Gtk.Image.new_from_icon_name('starred-symbolic')
                    pin_icon.set_tooltip_text(_('Imported from Apple Messages · this computer keeps a snapshot'))
                    content.append(pin_icon)
                row.set_child(content)
                self.rows[key] = row
                self.list.append(row)
                if is_pinned:
                    button = Gtk.ToggleButton(tooltip_text=thread['name'])
                    button.add_css_class('flat');button.add_css_class('ms-favorite')
                    button.update_property([Gtk.AccessibleProperty.LABEL], [thread['name']])
                    tile = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5, halign=Gtk.Align.CENTER)
                    tile.add_css_class('ms-favorite-content')
                    portrait = label((thread['name'] or '?')[0].upper(), 'ms-favorite-avatar')
                    if not thread.get('is_group') and (data := self.photo_cache.get(address)):
                        portrait = self.photo_widget(data) or portrait
                    tile.append(portrait)
                    tile.append(label(thread['name'], xalign=.5, ellipsize=Pango.EllipsizeMode.END, max_width_chars=7))
                    button.set_child(tile)
                    def choose(_button, identity=key):
                        self.list.unselect_all()
                        self.selected(None, self.rows[identity])
                        self.sync_favorite_selection()
                    button.connect('clicked', choose)
                    self.favorite_buttons[key] = button
                    self.favorite_avatars[key] = (tile, portrait)
            if self.model.selected in self.rows:
                self.list.select_row(self.rows[self.model.selected])
        self.filter_rows()
        self.sync_favorite_selection()

    def sync_favorite_selection(self):
        for key, button in self.favorite_buttons.items():
            button.set_active(key == self.model.selected)

    def pin_notice(self, text):
        self.pin_note.set_label(text)
        self.pin_note.set_visible(bool(text))

    def load_pins(self):
        if self.fixture or self.pin_pending:
            return
        try:
            profile = profile_key()
        except ValueError:
            return
        if profile == self.pin_profile:
            return
        self.pin_profile = profile
        self.pin_state = dict(version=1, snapshot=None, choices=[])
        self.pin_pending = True
        def load():
            store = PinStore(profile)
            return store, store.load()
        def loaded(result, error):
            self.pin_pending = False
            if profile != self.pin_profile:
                return
            if error:
                self.pin_store = None
                self.pin_notice(_('Could not open saved pins. Import iPhone Pins to try again.'))
                return
            self.pin_store, self.pin_state = result
            self.update_pin_note()
            self.populate()
            self.load_photos()
        self.task(load, loaded)

    def update_pin_note(self):
        snapshot = self.pin_state.get('snapshot')
        if not snapshot:
            self.pin_notice('')
            return
        stamp = datetime.fromisoformat(snapshot['captured_at']).astimezone().strftime('%x %X')
        missing = sum(key not in self.threads for key in effective_choices(self.threads, self.pin_state))
        self.pin_notice((_('Pins from Mac · %s') % stamp) + (_(' · %d need matching') % missing if missing else ''))
        self.pin_note.set_tooltip_text(_('This is the Mac’s exported pin order. Changes on your iPhone require a new export. Review Imported Pins to match missing conversations.'))
        if snapshot.get('source_modified_at'):
            modified = datetime.fromisoformat(snapshot['source_modified_at']).astimezone().strftime('%x %X')
            self.pin_note.set_tooltip_text(self.pin_note.get_tooltip_text() + '\n' + (_('Apple pin settings last changed: %s') % modified))

    def import_pins(self, *_args):
        chooser = Gtk.FileDialog(title=_('Import iPhone Pins'))
        file_filter = Gtk.FileFilter(name=_('Apple Messages pin export'))
        file_filter.add_pattern('*.json')
        chooser.set_default_filter(file_filter)
        def selected(dialog, result):
            try:
                path = dialog.open_finish(result).get_path()
            except GLib.Error:
                return
            if not path:
                self.pin_notice(_('Choose a local pin export file.'))
                return
            def loaded(snapshot, error):
                if error:
                    self.pin_notice(_('Could not read that pin export. Use Export-Message-Pins on your Mac.'))
                else:
                    self.show_pin_review(snapshot)
            self.task(lambda: read_snapshot(path), loaded)
        chooser.open(self.window, None, selected)

    def review_pins(self, *_args):
        if self.pin_state.get('snapshot'):
            self.show_pin_review(self.pin_state['snapshot'], effective_choices(self.threads, self.pin_state))
        else:
            self.import_pins()

    def show_pin_review(self, snapshot, previous=None):
        dialog = Gtk.Window(title=_('Import iPhone Pins'), transient_for=self.window, destroy_with_parent=True, modal=True, default_width=480, default_height=540)
        header = Gtk.HeaderBar()
        header.set_title_widget(label(_('Import iPhone Pins'), 'title'))
        dialog.set_titlebar(header)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14, margin_top=18, margin_bottom=18, margin_start=20, margin_end=20)
        box.append(label(_('Your people, in the same order.'), 'title-2', xalign=0, wrap=True))
        box.append(label(_('Match the pins exported from Apple Messages. Groups need your selection. This imports a snapshot; it does not change your iPhone.'), 'dim-label', xalign=0, wrap=True, max_width_chars=48))
        rows = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        box.append(Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=rows))
        keys = sorted(self.threads, key=lambda key: self.threads[key]['name'].casefold())
        names = [_('Not matched yet')] + [self.threads[key]['name'] for key in keys]
        choices = previous if previous is not None else suggested_matches(snapshot, self.threads, self.pin_state)
        dropdowns = []
        for index, pin in enumerate(snapshot['pins']):
            row = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
            row.add_css_class('ms-pin-row')
            row.append(label(_('%d. %s') % (index+1, pin['apple_identifier']), xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE, max_width_chars=40, tooltip_text=pin['apple_identifier']))
            dropdown = Gtk.DropDown(model=Gtk.StringList.new(names), enable_search=True)
            factory = Gtk.SignalListItemFactory()
            factory.connect('setup', lambda _f, item: item.set_child(label(xalign=0, ellipsize=Pango.EllipsizeMode.END, max_width_chars=34)))
            factory.connect('bind', lambda _f, item: item.get_child().set_label(item.get_item().get_string()))
            dropdown.set_factory(factory)
            dropdown.set_selected(keys.index(choices[index])+1 if choices[index] in keys else 0)
            dropdown.update_property([Gtk.AccessibleProperty.LABEL], [_('Conversation for pin %d') % (index+1)])
            row.append(dropdown)
            details = label('', 'dim-label', xalign=0, wrap=True, max_width_chars=42)
            def describe_match(control, *_args, destination=details):
                key = keys[control.get_selected()-1] if control.get_selected() else None
                addresses = ', '.join(self.threads.get(key, {}).get('recipients', []))
                destination.set_label(addresses or _('Choose a conversation, or leave this pin for later.'))
                control.set_tooltip_text(self.threads.get(key, {}).get('name', '') + ('\n' + addresses if addresses else ''))
            dropdown.connect('notify::selected', describe_match);describe_match(dropdown)
            row.append(details);rows.append(row);dropdowns.append(dropdown)
        if not snapshot['pins']:
            rows.append(label(_('Apple Messages exported an empty pin list. Importing it clears this computer’s imported pins.'), xalign=0, wrap=True, max_width_chars=45))
        error_label = label('', 'error', xalign=0, wrap=True, visible=False)
        box.append(error_label)
        actions = Gtk.Box(spacing=8, halign=Gtk.Align.END)
        cancel = Gtk.Button(label=_('Cancel'));cancel.connect('clicked', lambda *_a: dialog.close());actions.append(cancel)
        apply = Gtk.Button(label=_('Import Pins'));apply.add_css_class('suggested-action');actions.append(apply)
        box.append(actions);dialog.set_child(box)
        dialog.set_default_widget(apply)
        keys_controller = Gtk.EventControllerKey()
        def key_pressed(_c, key, _code, _mods):
            if key == Gdk.KEY_Escape:
                dialog.close();return True
            return False
        keys_controller.connect('key-pressed', key_pressed);dialog.add_controller(keys_controller)
        state = dict(alive=True)
        dialog.connect('close-request', lambda *_a: state.update(alive=False) or False)
        def save(*_args):
            try:
                selected = [keys[d.get_selected()-1] if d.get_selected() else None for d in dropdowns]
                new_state = matched_state(snapshot, selected, self.threads)
                profile = 'fixture' if self.fixture else profile_key()
            except ValueError as error:
                error_label.set_label(str(error));error_label.set_visible(True);return
            apply.set_sensitive(False)
            def persist():
                if self.fixture:
                    return new_state
                if profile_key() != profile:
                    raise RuntimeError('The connected phone changed')
                return PinStore(profile).save(new_state)
            def saved(result, error):
                if error:
                    if state['alive']:
                        error_label.set_label(_('Could not save pins in the desktop keyring. Your previous pins are kept.'))
                        error_label.set_visible(True);apply.set_sensitive(True)
                    return
                if self.fixture or self.pin_profile == profile:
                    self.pin_state = result
                    self.update_pin_note();self.populate()
                    self.load_photos()
                if state['alive']:
                    dialog.close()
            self.task(persist, saved)
        apply.connect('clicked', save)
        dialog.present()
        # Stable handles for private interaction probes, never real account data.
        dialog.pin_controls = dropdowns, apply, error_label
        return dialog

    def filter_rows(self):
        query = self.search.get_text().casefold()
        favorites = []
        ordinary = False
        for key, row in self.rows.items():
            thread = self.threads[key]
            matches = not query or query in thread['name'].casefold() or any(query in m.get('body', '').casefold() for m in thread['messages'])
            row.set_visible(matches and not row.is_pinned)
            ordinary = ordinary or row.get_visible()
            if row.is_pinned and matches: favorites.append(key)
        if tuple(favorites) != getattr(self, 'grid_filter_signature', None):
            self.grid_filter_signature = tuple(favorites)
            clear(self.favorite_grid)
            for index, key in enumerate(favorites):
                self.favorite_grid.attach(self.favorite_buttons[key], index % 4, index // 4, 1, 1)
            if 0 < len(favorites) < 4:
                self.favorite_grid.attach(Gtk.Box(), 3, 0, 1, 1)
        self.favorites.set_visible(bool(favorites))
        self.favorite_separator.set_visible(bool(favorites) and ordinary)

    def selected(self, _list, row):
        if row is None or row.key == self.model.selected:
            return
        self.model.selected = row.key
        self.sync_favorite_selection()
        self.rendered = None
        self.editor.get_buffer().set_text(self.model.drafts.get(row.key, ''))
        self.show_conversation()
        thread = self.threads[row.key]
        if not self.fixture and not thread.get('inbox_only') and not thread.get('new'):
            self.task(lambda: BackendClient().mark_thread_read(thread['key']), lambda _v, _e: None)

    def show_conversation(self):
        thread = self.threads.get(self.model.selected)
        if thread is None:
            self.stack.set_visible_child_name('welcome')
            self.update_composer()
            return
        self.stack.set_visible_child_name('messages')
        self.title.set_label(thread['name'])
        self.subtitle.set_label(', '.join(thread.get('recipients', [])))
        signature = (thread['key'], thread['messages'])
        if signature != self.rendered:
            adjustment = self.message_scroll.get_vadjustment()
            near_bottom = adjustment.get_value() + adjustment.get_page_size() >= adjustment.get_upper() - 80
            switching = self.rendered is None or self.rendered[0] != thread['key']
            self.rendered = (thread['key'], [dict(m) for m in thread['messages']])
            clear(self.messages)
            if not thread['messages']:
                self.messages.append(label(_('Say hello.'), 'dim-label', vexpand=True, valign=Gtk.Align.CENTER))
            for message in thread['messages']:
                outgoing = message.get('outgoing', False)
                line = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4, halign=Gtk.Align.END if outgoing else Gtk.Align.START)
                bubble = label(message.get('body', ''), 'ms-bubble', wrap=True, selectable=True, xalign=0, max_width_chars=48, natural_wrap_mode=Gtk.NaturalWrapMode.NONE)
                bubble.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
                if outgoing:
                    bubble.add_css_class('ms-outgoing')
                line.append(bubble)
                stamp = message.get('display_timestamp') or message.get('timestamp', '')
                line.append(label(stamp, 'ms-time', xalign=1 if outgoing else 0))
                self.messages.append(line)
            if near_bottom or switching:
                GLib.idle_add(self.scroll_bottom)
        self.update_composer()

    def scroll_bottom(self):
        if not self.closed:
            a = self.message_scroll.get_vadjustment()
            a.set_value(max(a.get_lower(), a.get_upper() - a.get_page_size()))
        return False

    def draft_changed(self, buffer):
        if self.model.selected:
            self.model.drafts[self.model.selected] = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)
            self.send_feedback.pop(self.model.selected, None)
        self.update_composer()

    def update_composer(self):
        key = self.model.selected
        thread = self.threads.get(key)
        pending = key in self.model.pending
        self.editor.set_sensitive(bool(thread))
        self.send.set_sensitive(bool(thread and thread.get('reply_ready') and self.status.get('map') and self.model.drafts.get(key, '').strip() and not pending))
        if pending:
            self.send_status.set_label(_('Sending…'))
        elif thread and not thread.get('reply_ready'):
            self.send_status.set_label(_('Check the group participants in Advanced BlueFerry before replying.'))
        elif thread:
            self.send_status.set_label(self.send_feedback.get(key) or (_('Ctrl+Enter to send · Enter for a new line') if self.status.get('map') else _('Offline · your draft is kept while this window is open')))

    def key_pressed(self, _controller, key, _code, modifiers):
        if key in (Gdk.KEY_Return, Gdk.KEY_KP_Enter) and modifiers & Gdk.ModifierType.CONTROL_MASK:
            if self.send.get_sensitive():
                self.send_message()
            return True
        return False

    def send_message(self, *_args):
        thread = self.threads.get(self.model.selected)
        if not self.send.get_sensitive() or not thread:
            return
        try:
            attempt = self.model.prepare(thread)
        except ValueError as error:
            self.send_status.set_label(str(error))
            return
        self.update_composer()
        def send():
            if self.fixture:
                raise RuntimeError('Preview cannot send messages')
            client = BackendClient()
            if attempt.backend_thread:
                return client.send_to_thread(attempt.key, attempt.body, expected_group_token=attempt.token)
            return client.send(attempt.recipient, attempt.body)
        def sent(_transfer, error):
            self.model.finish(attempt, not error)
            if self.model.selected == attempt.key:
                self.editor.get_buffer().set_text(self.model.drafts.get(attempt.key, ''))
            self.send_feedback[attempt.key] = _('Could not send. Your draft is kept; choose Send to retry.') if error else _('Sent through your iPhone')
            self.update_composer()
            self.refresh()
        self.task(send, sent)

    def new_message(self, *_args):
        dialog = Gtk.Window(title=_('New Message'), transient_for=self.window, modal=True, default_width=460, default_height=420)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, margin_start=18, margin_end=18, margin_top=18, margin_bottom=18)
        box.append(label(_('To'), 'title-2', xalign=0))
        entry = Gtk.SearchEntry(placeholder_text=_('Name, phone number, or email'))
        box.append(entry)
        results = Gtk.ListBox(selection_mode=Gtk.SelectionMode.NONE)
        box.append(Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=results))
        hint = label(_('Choose a contact or enter a complete address.'), 'dim-label', wrap=True, xalign=0)
        box.append(hint)
        use = Gtk.Button(label=_('Start Conversation'), sensitive=False)
        use.add_css_class('suggested-action')
        box.append(use)
        dialog.set_child(box)
        state = dict(generation=0, alive=True)
        dialog.connect('close-request', lambda *_a: state.update(alive=False) or False)
        def choose(name, address):
            try:
                address = validate_recipient(address)
            except ValueError:
                return
            key = 'new:' + address
            existing = next((t for t in self.threads.values() if not t.get('is_group') and t.get('recipients') == [address]), None)
            if existing:
                key = existing['key']
            else:
                self.threads[key] = dict(key=key, name=name or address, recipients=[address], reply_ready=True, is_group=False, new=True, messages=[], last_ts=datetime.now().astimezone().isoformat())
            self.populate()
            self.list.select_row(self.rows[key])
            state['alive'] = False
            dialog.destroy()
            self.editor.grab_focus()
        def changed(*_a):
            state['generation'] += 1
            generation = state['generation']
            query = entry.get_text().strip()
            try:
                validate_recipient(query)
                use.set_sensitive(True)
            except ValueError:
                use.set_sensitive(False)
            clear(results)
            if len(query) < 2:
                return
            def found(matches, error):
                if not state['alive'] or generation != state['generation']:
                    return
                clear(results)
                hint.set_label(_('Could not search contacts.') if error else _('Choose the address you want to message.'))
                for name, address in (matches or [])[:30]:
                    button = Gtk.Button()
                    content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=3, margin_top=6, margin_bottom=6)
                    content.append(label(name, xalign=0, ellipsize=Pango.EllipsizeMode.END))
                    content.append(label(address, 'dim-label', xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE))
                    button.set_child(content)
                    button.add_css_class('flat')
                    button.connect('clicked', lambda _b, n=name, a=address: choose(n, a))
                    results.append(button)
            self.task(lambda: [] if self.fixture else BackendClient().find_contacts(query), found)
        entry.connect('search-changed', changed)
        use.connect('clicked', lambda *_a: choose('', entry.get_text()))
        dialog.present()

    def responsive(self, *_args):
        self.layout(self.window.get_width() < 760)
        return not self.closed

    def layout(self, narrow):
        selected = bool(self.model.selected)
        self.sidebar.set_visible(not narrow or not selected)
        self.detail.set_visible(not narrow or selected)
        self.back.set_visible(narrow and selected)

    def go_back(self, *_args):
        self.model.selected = ''
        self.list.unselect_all()
        self.stack.set_visible_child_name('welcome')

    def setup(self, *_args):
        subprocess.Popen([str(Path.home()/'.local/bin/mseries-phone-setup')])

    def advanced(self, *_args):
        subprocess.Popen(['blueferry-gtk'])

    def close(self, *_args):
        self.closed = True
        if self.poll:
            GLib.source_remove(self.poll)
        if self.sleep_watch:
            self.sleep_watch.remove()
        self.pool.shutdown(wait=False, cancel_futures=True)
        return False

    def sleep_changed(self, sleeping):
        if not sleeping and not self.closed:
            self.refresh(True)

    def preview(self):
        self.status = dict(map=True)
        self.connection.set_label(_('iPhone connected · Messages ready'))
        self.threads = {'demo': dict(key='demo', name='Alex Morgan', recipients=['+15550100123'], reply_ready=True, messages=[dict(handle='demo1', body='Made it home! How is the new desktop?', outgoing=False, timestamp='Today, 11:42'), dict(handle='demo2', body='Even better now. Messages are right here with everything else.', outgoing=True, timestamp='Today, 11:43'), dict(handle='demo3', body='That looks so much easier. See you for coffee tomorrow ☕', outgoing=False, timestamp='Today, 11:44')]), 'demo2': dict(key='demo2', name='A very long contact name that should fit comfortably', recipients=['+15550100124'], reply_ready=True, messages=[dict(body='This is a preview conversation.', outgoing=False, timestamp='Yesterday')])}
        self.populate()
        self.list.select_row(self.rows['demo'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--preview', action='store_true')
    parser.add_argument('--dark', action='store_true')
    parser.add_argument('--narrow', action='store_true')
    args = parser.parse_args()
    Messages(args.preview, args.dark, args.narrow).run([])
