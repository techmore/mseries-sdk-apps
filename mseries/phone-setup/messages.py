#!/usr/bin/python3
"""GTK 4 / Granite 7 Messages client for the existing BlueFerry daemon.

This module is the controller: it owns state, talks to BlueFerry on a worker
thread and decides what to show. Look and feel live in messages_style.py and
messages_widgets.py; pure presentation logic lives in messages_model.py.
"""
import argparse
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timedelta
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
from gi.repository import Gdk, Gio, GLib, Granite, Gtk, Pango
from blueferry.client import BackendClient
from blueferry.bus import get_system_bus
from blueferry.recipients import validate_recipient
from messages_model import (ConversationState, Outbox, blocks_group_confirmation, contact_index, group_roster, layout_messages,
                            merge_inbox, needs_group_confirmation, quote_snippet, reply_body, reply_block_reason, resolve_contact_names, send_failure_reason)
from message_media import load_preview, youtube_id
import messages_settings
from messages_style import build_style, missing_accent_names
from messages_widgets import (AVATAR_PX, avatar, build_composer, build_notice, build_status, clear,
                              conversation_row, empty_state, favorite_tile, icon_for, identity_of,
                              label, photo_frame, render_messages)
from message_pins import (PinStore, profile_key, read_snapshot, suggested_matches, matched_state,
                          ordered_keys, effective_choices, effective_pins, unresolved_imports,
                          toggle_pin_state, default_state)

_ = gettext.gettext
ngettext = gettext.ngettext
APP_ID = 'io.elementary.mseries.Messages'
# Below this width the window shows one pane at a time, like a phone.
NARROW = 760
GROUP_INFO_NOTE = _('Your iPhone is not sharing notifications with this computer, so group texts appear under the person who sent them and replies here go to that person only. On the iPhone, open Settings › Bluetooth, tap the info button beside this computer and turn on Share System Notifications.')
CONNECTION = {
    'connecting': ('wait', _('Connecting to iPhone…')),
    'ready': ('ok', _('iPhone connected · Messages ready')),
    'reconnecting': ('wait', _('iPhone reconnecting…')),
    'offline': ('off', _('iPhone unavailable · conversations kept')),
}


class Messages(Gtk.Application):
    def __init__(self, fixture=False, dark=False, narrow=False):
        super().__init__(application_id=APP_ID + ('.Preview' if fixture else ''),
                         flags=Gio.ApplicationFlags.NON_UNIQUE if fixture else Gio.ApplicationFlags.DEFAULT_FLAGS)
        self.fixture, self.dark, self.narrow = fixture, dark, narrow
        self.pool = ThreadPoolExecutor(max_workers=1)
        # thread key -> quote line the next message will carry (a Reply in progress)
        self.replying = {}
        # Link previews fetch over the network, so they never queue behind phone calls.
        self.media_pool = ThreadPoolExecutor(max_workers=2)
        self.media_cache = OrderedDict()
        self.media_pending = {}
        self.media_timers = set()
        self.contact_names = {}
        self.contact_revision = None
        self.contacts_pending = False
        self.model = ConversationState()
        self.outbox = Outbox()
        self.threads = {}
        self.rows = {}
        self.inbox = []
        self.last_map = False
        self.status = {}
        self.snapshot_pending = False
        self.closed = False
        self.rendered = None
        self.rendered_count = 0
        self.force_scroll = False
        self.poll = 0
        self.search_generation = 0
        self.last_refresh = 0
        self.checked_text = ''
        self.inbox_requested = True
        self.inbox_request_epoch = 0
        self.sleep_watch = None
        self.send_feedback = {}
        self.read_marked = {}
        self.connection_state = ''
        self.retrying = False
        self.css_errors = []
        self.photo_cache = {}
        self.photo_revision = None
        self.photo_pending = False
        self.avatars = {}
        self.favorite_avatars = {}
        self.favorite_buttons = {}
        self.heading_signature = None
        self.pin_state = default_state()
        self.pin_profile = None
        self.pin_store = None
        self.pin_pending = False
        self.group_dialog = None
        # Fixtures never read or write the owner's real preferences.
        self.settings = dict(messages_settings.DEFAULTS) if fixture else messages_settings.load()
        self.enter_armed = False

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
        # Kept so tests can prove the stylesheet parses cleanly on this toolkit.
        provider.connect('parsing-error', lambda _p, section, error: self.css_errors.append((section.to_string(), error.message)))
        # Probe the theme through a throwaway widget before our own CSS is added.
        self.stylesheet = build_style(missing_accent_names(Gtk.Label().get_style_context()))
        provider.load_from_string(self.stylesheet)
        Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(), provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        self.window = Gtk.ApplicationWindow(application=self, title=_('Messages'), default_width=1080, default_height=720)
        self.window.set_icon_name(APP_ID)
        self.window.set_size_request(420, 420)
        self.window.connect('close-request', self.close)
        self.install_actions()
        self.window.set_child(self.build_window())
        self.window.connect('notify::default-width', lambda *_a: self.layout(self.window.get_default_size()[0] < NARROW))
        keys = Gtk.EventControllerKey()
        keys.connect('key-pressed', self.key_pressed)
        self.editor.add_controller(keys)
        # Enter-to-send. A plain Return is handled by the text view itself before any
        # controller of ours could stop it, so note the press (capture phase) and act
        # only when a literal newline is then inserted. If an input method used the key
        # to confirm a composition, other text arrives instead and nothing is sent.
        arm = Gtk.EventControllerKey()
        arm.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        arm.connect('key-pressed', self.arm_enter)
        self.editor.add_controller(arm)
        self.editor.get_buffer().connect('insert-text', self.guard_newline)
        self.window.present()
        self.watch_size()
        if self.narrow:
            self.window.set_default_size(480, 680)
        self.set_connection('connecting')
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
        self.update_composer()

    def install_actions(self):
        """Application actions with accelerators; also listed in the menu and tooltips."""
        shortcuts = [('new-message', ['<Primary>n'], self.new_message),
                     ('search', ['<Primary>f'], self.focus_search),
                     ('refresh', ['<Primary>r', 'F5'], lambda *_a: self.refresh(True)),
                     ('back', ['<Alt>Left'], self.go_back)]
        for name, accels, handler in shortcuts:
            action = Gio.SimpleAction.new(name, None)
            action.connect('activate', handler)
            self.add_action(action)
            self.set_accels_for_action('app.' + name, accels)
        toggle = Gio.SimpleAction.new_stateful('enter-sends', None, GLib.Variant.new_boolean(self.settings['enter_sends']))
        toggle.connect('change-state', self.set_enter_sends)
        toggle.connect('activate', lambda action, _p: action.change_state(GLib.Variant.new_boolean(not action.get_state().get_boolean())))
        self.add_action(toggle)

    def build_window(self):
        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        outer.append(self.build_header())
        self.split = Gtk.Paned(orientation=Gtk.Orientation.HORIZONTAL, vexpand=True)
        self.split.set_position(320)
        self.split.set_shrink_start_child(False)
        self.split.set_shrink_end_child(False)
        self.split.set_start_child(self.build_sidebar())
        self.split.set_end_child(self.build_detail())
        outer.append(self.split)
        return outer

    def build_header(self):
        header = Gtk.HeaderBar()
        self.back = Gtk.Button(icon_name='go-previous-symbolic', tooltip_text=_('Conversations'), visible=False)
        self.back.connect('clicked', self.go_back)
        self.back.update_property([Gtk.AccessibleProperty.LABEL], [_('Conversations')])
        header.pack_start(self.back)
        header.set_title_widget(label(_('Messages'), 'title'))
        new = Gtk.Button(icon_name='document-edit-symbolic', tooltip_text=_('New Message (Ctrl+N)'))
        new.connect('clicked', self.new_message)
        new.update_property([Gtk.AccessibleProperty.LABEL], [_('New Message')])
        header.pack_start(new)
        refresh = Gtk.Button(icon_name='view-refresh-symbolic', tooltip_text=_('Refresh iPhone Inbox (Ctrl+R)'))
        refresh.connect('clicked', lambda *_args: self.refresh(True))
        refresh.update_property([Gtk.AccessibleProperty.LABEL], [_('Refresh iPhone Inbox')])
        header.pack_end(refresh)
        menu = Gio.Menu()
        for text, action, method in [(_('Import iPhone Pins…'), 'import-pins', self.import_pins), (_('Review Imported Pins…'), 'review-pins', self.review_pins), (_('Connect iPhone…'), 'setup', self.setup), (_('Advanced BlueFerry…'), 'advanced', self.advanced), (_('Keyboard Shortcuts'), 'shortcuts', self.show_shortcuts)]:
            a = Gio.SimpleAction.new(action, None)
            a.connect('activate', method)
            self.add_action(a)
            menu.append(text, 'app.' + action)
        menu.append(_('Press Enter to Send'), 'app.enter-sends')
        header.pack_end(Gtk.MenuButton(icon_name='open-menu-symbolic', menu_model=menu, tooltip_text=_('Menu')))
        return header

    def build_sidebar(self):
        self.sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, width_request=260)
        self.sidebar.add_css_class('ms-sidebar')
        self.search = Gtk.SearchEntry(placeholder_text=_('Search conversations'))
        self.search.add_css_class('ms-search')
        self.search.connect('search-changed', lambda *_args: self.filter_rows())
        self.sidebar.append(self.search)
        self.pin_note = label('', 'ms-pin-note', xalign=0, wrap=True, visible=False)
        self.pin_note.add_css_class('dim-label')
        self.sidebar.append(self.pin_note)
        self.list = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE)
        self.list.add_css_class('navigation-sidebar')
        self.list.add_css_class('ms-list')
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
        self.empty_list = empty_state('mail-message-new-symbolic', _('No Conversations'), _('Start one with the pencil button above.'))
        self.empty_list.set_visible(False)
        contacts.append(self.empty_list)
        self.sidebar.append(Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=contacts))
        self.status_bar = build_status()
        self.connection = self.status_bar.text
        self.sidebar.append(self.status_bar.box)
        return self.sidebar

    def build_detail(self):
        self.detail = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, width_request=280)
        self.heading = Gtk.Box(spacing=12, visible=False)
        self.heading.add_css_class('ms-heading')
        self.heading_slot = Gtk.Box(valign=Gtk.Align.CENTER, hexpand=False)
        self.heading.append(self.heading_slot)
        titles = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=1, valign=Gtk.Align.CENTER, hexpand=True)
        self.title = label(_('Your conversations'), 'title-3', xalign=0, ellipsize=Pango.EllipsizeMode.END)
        self.subtitle = label(_('Messages sent through your iPhone'), 'dim-label', xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE)
        titles.append(self.title)
        titles.append(self.subtitle)
        self.heading.append(titles)
        self.detail.append(self.heading)
        self.notice = build_notice()
        self.notice.retry.connect('clicked', self.retry_connection)
        self.detail.append(self.notice.box)
        self.stack = Gtk.Stack(vexpand=True, transition_type=Gtk.StackTransitionType.CROSSFADE, transition_duration=150)
        welcome = Granite.Placeholder(title=_('A little closer, wherever you work.'),
                                      description=_('Choose a conversation or start a new message.\nYour iPhone sends and receives the texts.'),
                                      icon=Gio.ThemedIcon.new('mail-message-new-symbolic'))
        welcome.append_button(Gio.ThemedIcon.new('document-edit-symbolic'), _('New Message'),
                              _('Write to anyone in your contacts')).connect('clicked', self.new_message)
        # Granite's placeholder wants ~420px; let it scroll instead of forcing a tall window.
        self.stack.add_named(Gtk.ScrolledWindow(hscrollbar_policy=Gtk.PolicyType.NEVER, child=welcome), 'welcome')
        self.messages = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, margin_start=20, margin_end=20, margin_top=8, margin_bottom=16)
        self.message_scroll = Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=self.messages)
        self.message_scroll.get_vadjustment().connect('value-changed', self.scrolled)
        self.jump = Gtk.Button(label=_('New Messages'), halign=Gtk.Align.CENTER, valign=Gtk.Align.END, visible=False)
        for css in ('suggested-action', 'pill', 'ms-jump'):
            self.jump.add_css_class(css)
        self.jump.connect('clicked', lambda *_a: self.scroll_bottom(True))
        overlay = Gtk.Overlay(child=self.message_scroll)
        overlay.add_overlay(self.jump)
        self.stack.add_named(overlay, 'messages')
        self.detail.append(self.stack)
        self.composer = build_composer()
        self.editor, self.send, self.send_status = self.composer.editor, self.composer.send, self.composer.status
        self.editor.get_buffer().connect('changed', self.draft_changed)
        self.send.connect('clicked', self.send_message)
        self.composer.quote_close.connect('clicked', lambda *_a: self.cancel_reply())
        self.detail.append(self.composer.box)
        return self.detail

    def watch_size(self):
        """React to resizes without a per-frame tick callback."""
        surface = self.window.get_surface()
        if surface is None:
            self.window.add_tick_callback(self.responsive)
            return
        surface.connect('layout', lambda _surface, width, _height: self.layout(width < NARROW))
        self.apply_layout()

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

    def set_connection(self, state):
        tone, text = CONNECTION[state]
        self.connection_state = state
        self.status_bar.text.set_label(text)
        for name in ('ok', 'wait', 'off'):
            self.status_bar.dot.remove_css_class('ms-' + name)
        self.status_bar.dot.add_css_class('ms-' + tone)

    def set_notice(self, text, retry=False):
        self.notice.text.set_label(text)
        self.notice.box.set_visible(bool(text))
        self.notice.retry.set_visible(bool(text) and retry)
        self.notice.retry.set_sensitive(not self.retrying)

    def retry_connection(self, *_args):
        """Reconnect the saved phone through BlueFerry; never re-pairs or edits audio policy."""
        if self.retrying or self.closed:
            return
        self.retrying = True
        self.set_connection('reconnecting')
        self.set_notice(_('Reconnecting to your iPhone…'), retry=True)
        def work():
            if self.fixture:
                raise RuntimeError('Preview cannot reconnect')
            from connection import reconnect_existing
            return reconnect_existing(cancelled=lambda: self.closed)
        def finished(_status, error):
            self.retrying = False
            if error:
                self.set_notice(_('Could not reconnect. Keep your iPhone nearby with Bluetooth on, or choose Connect iPhone.'), retry=True)
                self.notice.text.set_tooltip_text(str(error))
            self.refresh(True)
        self.task(work, finished)

    def loaded(self, result, error):
        self.snapshot_pending = False
        if error:
            self.status = dict(map=False)
            self.last_map = False
            self.set_connection('offline')
            if not self.retrying:
                self.set_notice(_('Messages are offline. Keep your iPhone nearby with Bluetooth on. Use Connect iPhone if it needs setup.'), retry=True)
            self.update_composer()
            self.filter_rows()
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
        self.set_connection('ready' if self.last_map else 'reconnecting')
        if not self.retrying:
            if warning:
                self.set_notice(warning)
            elif not self.last_map:
                self.set_notice(_('Waiting for message access. Existing conversations remain available.'), retry=True)
            else:
                self.set_notice('')
            self.notice.text.set_tooltip_text(None)
        if self.last_refresh:
            stamp = datetime.now().astimezone().strftime('%X') if recent is not None else self.checked_text
            self.checked_text = stamp
            self.status_bar.box.set_tooltip_text(_('Recent iPhone inbox checked at %s. Up to 200 messages exposed by the phone; full iCloud history is not available.') % stamp)
        combined = resolve_contact_names(merge_inbox(threads, self.inbox), self.contact_names)
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
                    self.outbox.rekey(key, match['key'])
                    if self.model.selected == key:
                        self.model.selected = match['key']
                elif thread.get('new') or key in self.model.pending:
                    combined.append(thread)
        self.threads = {t['key']: t for t in combined}
        self.load_pins()
        self.load_contact_names(force=recent is not None)
        self.update_pin_note()
        self.populate()
        self.show_conversation()
        self.load_photos()

    def load_contact_names(self, force=False):
        """Page the cached phonebook so bare numbers show the person's name.

        Presentation only: resolve_contact_names never changes a thread's
        identity or recipients, and never guesses groups or ambiguous contacts.
        """
        if self.fixture or self.contacts_pending:
            return
        revision = (self.pin_profile, self.status.get('contacts'), self.status.get('contact_photo_revision'))
        if not force and revision == self.contact_revision:
            return
        self.contacts_pending = True
        def load():
            client = BackendClient()
            records = []
            for offset in range(0, 10000, 150):
                page = client.list_contacts(offset, 150)
                records.extend(page)
                if len(page) < 150:
                    break
            return contact_index(records)
        def loaded(index, error):
            self.contacts_pending = False
            if error or revision[0] != self.pin_profile:
                return
            self.contact_names, self.contact_revision = index, revision
            resolve_contact_names(self.threads.values(), index)
            self.populate()
            self.show_conversation()
        self.task(load, loaded)

    def photo_widget(self, data, px=AVATAR_PX['m']):
        return photo_frame(data, px)

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
            for registry, size in ((self.avatars, 'm'), (self.favorite_avatars, 'l')):
                for key, (slot, old) in list(registry.items()):
                    thread = self.threads.get(key, {})
                    data = photos.get((thread.get('recipients') or [''])[0])
                    if data and not thread.get('is_group') and (photo := photo_frame(data, AVATAR_PX[size])):
                        slot.remove(old)
                        slot.append(photo)
                        registry[key] = (slot, photo)
            self.heading_signature = None
            self.show_conversation()
            self.load_photos()
        self.task(load, loaded)

    def release_focus(self, container):
        """Drop keyboard focus from `container` before its children are destroyed.

        Selectable bubble text and list rows take focus. GTK 4.22's AT-SPI layer aborts the
        whole app when a focused widget is destroyed under it (SIGABRT in gtk_at_spi_root_to_ref),
        so any rebuild first moves focus out of the subtree. True means focus was inside it.
        """
        focus = self.window.get_focus()
        if focus is None or not (focus is container or focus.is_ancestor(container)):
            return False
        self.window.set_focus(None)
        return True

    def populate(self):
        order = ordered_keys(self.threads, self.pin_state)
        pinned = set(effective_pins(self.threads, self.pin_state))
        # Avoid destroying focus/selection when a status poll changes nothing.
        def last(thread):
            return thread['messages'][-1] if thread['messages'] else {}
        signatures = [(key, self.threads[key]['name'], key in pinned, self.threads[key].get('unread_count', 0),
                       last(self.threads[key]).get('body', ''), last(self.threads[key]).get('outgoing', False),
                       self.threads[key].get('last_ts', '')) for key in order]
        if signatures != getattr(self, 'row_signature', None):
            self.row_signature = signatures
            had_focus = self.release_focus(self.list) or self.release_focus(self.favorite_grid)
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
                address = (thread.get('recipients') or [''])[0]
                photo = None if thread.get('is_group') else self.photo_cache.get(address)
                row, slot, portrait = conversation_row(thread, photo=photo)
                row.key = key
                is_pinned = row.is_pinned = key in pinned
                self.rows[key] = row
                self.avatars[key] = (slot, portrait)
                self.attach_row_menu(row, key)
                self.list.append(row)
                if is_pinned:
                    button = Gtk.ToggleButton(tooltip_text=thread['name'])
                    button.add_css_class('flat');button.add_css_class('ms-favorite')
                    button.update_property([Gtk.AccessibleProperty.LABEL], [thread['name']])
                    tile, favorite_slot, favorite_portrait = favorite_tile(thread, photo)
                    button.set_child(tile)
                    def choose(_button, identity=key):
                        self.list.unselect_all()
                        self.selected(None, self.rows[identity])
                        self.sync_favorite_selection()
                    button.connect('clicked', choose)
                    self.favorite_buttons[key] = button
                    self.favorite_avatars[key] = (favorite_slot, favorite_portrait)
            if self.model.selected in self.rows:
                self.list.select_row(self.rows[self.model.selected])
                if had_focus:
                    self.rows[self.model.selected].grab_focus()
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
        self.pin_state = default_state()
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
        missing = unresolved_imports(self.pin_state, self.threads)
        self.pin_notice((_('Pins from Mac · %s') % stamp) + (_(' · %d need matching') % missing if missing else ''))
        self.pin_note.set_tooltip_text(_('This is the Mac’s exported pin order. Changes on your iPhone require a new export. Review Imported Pins to match missing conversations.'))
        if snapshot.get('source_modified_at'):
            modified = datetime.fromisoformat(snapshot['source_modified_at']).astimezone().strftime('%x %X')
            self.pin_note.set_tooltip_text(self.pin_note.get_tooltip_text() + '\n' + (_('Apple pin settings last changed: %s') % modified))
    def toggle_pin(self, key):
        if self.pin_pending:
            self.pin_notice(_('Favorites are still loading. Try again in a moment.'))
            return
        try:
            new_state = toggle_pin_state(key, self.threads, self.pin_state)
        except ValueError as error:
            self.pin_notice(str(error))
            return
        if self.fixture:
            self.pin_state = new_state
            self.update_pin_note()
            self.populate()
            return
        try:
            profile = profile_key()
        except ValueError:
            self.pin_notice(_('Connect an iPhone before saving favorites.'))
            return
        if profile != self.pin_profile or self.pin_store is None:
            self.pin_notice(_('Saved favorites are unavailable. Reopen Messages after connecting your iPhone.'))
            return
        self.pin_pending = True
        def persist():
            if profile_key() != profile:
                raise RuntimeError('The connected phone changed')
            return PinStore(profile).save(new_state)
        def saved(result, error):
            self.pin_pending = False
            if profile != self.pin_profile:
                return
            if error:
                self.pin_notice(_('Could not save that favorite. Your previous pins are kept.'))
                return
            self.pin_store, self.pin_state = PinStore(profile), result
            self.update_pin_note()
            self.populate()
            self.load_photos()
        self.task(persist, saved)
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
                new_state = matched_state(snapshot, selected, self.threads, previous=self.pin_state)
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

    def row_actions(self, thread):
        """(label, callback) pairs for a conversation's context menu."""
        actions = []
        if thread.get('unread_count') and not thread.get('inbox_only') and not thread.get('new'):
            actions.append((_('Mark as Read'), lambda: self.mark_read(thread)))
        if thread.get('recipients') and not thread.get('new'):
            pinned = thread['key'] in effective_pins(self.threads, self.pin_state)
            text = _('Remove from Favorites') if pinned else _('Add to Favorites')
            actions.append((text, lambda key=thread['key']: self.toggle_pin(key)))
        if thread.get('recipients'):
            address = ', '.join(thread['recipients'])
            actions.append((_('Copy Number or Address') if len(thread['recipients']) == 1 else _('Copy Numbers'),
                            lambda: self.window.get_display().get_clipboard().set(address)))
        return actions

    def attach_row_menu(self, row, key):
        """Right-click (or long-press) a conversation for quick actions."""
        def show(gesture, *_args):
            thread = self.threads.get(key)
            actions = self.row_actions(thread) if thread else []
            if not actions:
                return
            popover = Gtk.Popover(has_arrow=False, halign=Gtk.Align.START)
            popover.set_parent(row)
            menu = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
            for text, callback in actions:
                button = Gtk.Button(label=text, halign=Gtk.Align.FILL)
                button.add_css_class('flat')
                button.get_first_child().set_halign(Gtk.Align.START)
                button.connect('clicked', lambda _b, cb=callback: (popover.popdown(), cb()))
                menu.append(button)
            popover.set_child(menu)
            popover.connect('closed', lambda p: GLib.idle_add(p.unparent))
            x, y = _args[-2:] if len(_args) >= 2 else (0, 0)
            popover.set_pointing_to(Gdk.Rectangle(x=int(x), y=int(y), width=1, height=1))
            popover.popup()
        click = Gtk.GestureClick(button=3)
        click.connect('pressed', show)
        row.add_controller(click)
        press = Gtk.GestureLongPress()
        press.connect('pressed', lambda g, x, y: show(g, x, y))
        row.add_controller(press)

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
            self.release_focus(self.favorite_grid)
            clear(self.favorite_grid)
            for index, key in enumerate(favorites):
                self.favorite_grid.attach(self.favorite_buttons[key], index % 4, index // 4, 1, 1)
            if 0 < len(favorites) < 4:
                self.favorite_grid.attach(Gtk.Box(), 3, 0, 1, 1)
        self.favorites.set_visible(bool(favorites))
        self.favorite_separator.set_visible(bool(favorites) and ordinary)
        # Say why the list is blank; stay quiet while the first load is in flight.
        loading = not self.threads and self.connection_state in ('', 'connecting')
        self.empty_list.set_visible(not (ordinary or favorites) and not loading)
        self.empty_list.image.set_from_icon_name(icon_for('edit-find-symbolic') if query else icon_for('mail-message-new-symbolic'))
        self.empty_list.title.set_label(_('No Results') if query else _('No Conversations'))
        self.empty_list.detail.set_label(_('Try a different name or word.') if query else _('Start one with the pencil button above.'))

    def selected(self, _list, row):
        if row is None or row.key == self.model.selected:
            return
        self.model.selected = row.key
        self.sync_favorite_selection()
        self.rendered = None
        self.jump.set_visible(False)
        self.editor.get_buffer().set_text(self.model.drafts.get(row.key, ''))
        self.show_conversation()
        self.apply_layout()
        self.mark_read(self.threads[row.key])

    def mark_read(self, thread):
        """Ask BlueFerry to mark a thread read, once per newest message.

        Opening a conversation and new messages arriving in an open, focused
        one both come through here, so the phone is asked a single time.
        """
        if self.fixture or thread.get('inbox_only') or thread.get('new'):
            return
        key = thread['key']
        marker = (thread['messages'][-1].get('handle') if thread['messages'] else '', thread.get('unread_count', 0))
        if self.read_marked.get(key) == marker:
            return
        self.read_marked[key] = marker
        # A failed request must be retried on the next opportunity.
        self.task(lambda: BackendClient().mark_thread_read(key),
                  lambda _value, error: self.read_marked.pop(key, None) if error else None)

    def update_heading_avatar(self, thread):
        address = (thread.get('recipients') or [''])[0]
        data = None if thread.get('is_group') else self.photo_cache.get(address)
        signature = (thread['key'], thread['name'], bool(data))
        if signature == self.heading_signature:
            return
        self.heading_signature = signature
        clear(self.heading_slot)
        self.heading_slot.append(avatar(thread['name'], identity_of(thread), 's', thread.get('is_group'), data))

    def show_conversation(self):
        thread = self.threads.get(self.model.selected)
        self.heading.set_visible(thread is not None)
        if thread is None:
            self.stack.set_visible_child_name('welcome')
            self.update_composer()
            return
        self.stack.set_visible_child_name('messages')
        self.title.set_label(thread['name'])
        self.subtitle.set_label(', '.join(thread.get('recipients', [])))
        self.update_heading_avatar(thread)
        pending = self.outbox.visible(thread['key'], thread['messages'])
        pending_state = [(e.body, e.state) for e in pending]
        signature = (thread['key'], thread['messages'], pending_state)
        if signature != self.rendered:
            adjustment = self.message_scroll.get_vadjustment()
            near_bottom = adjustment.get_value() + adjustment.get_page_size() >= adjustment.get_upper() - 80
            switching = self.rendered is None or self.rendered[0] != thread['key']
            count = len(thread['messages']) + len(pending)
            grew = not switching and count > self.rendered_count
            self.rendered = (thread['key'], [dict(m) for m in thread['messages']], pending_state)
            self.rendered_count = count
            # Messages sent from this window show at once, dimmed until the phone confirms.
            stamp = datetime.now().astimezone().isoformat(timespec='seconds')
            shown = list(thread['messages']) + [dict(body=e.body, outgoing=True, timestamp=stamp, sender='', _state=e.state) for e in pending]
            self.stop_media_animation()
            had_focus = self.release_focus(self.messages)
            render_messages(self.messages, layout_messages(shown, bool(thread.get('is_group'))), self.media_card, self.reply_to_message)
            if not shown:
                self.messages.append(label(_('Say hello.'), 'dim-label', vexpand=True, valign=Gtk.Align.CENTER))
            if had_focus and self.editor.get_sensitive():
                self.editor.grab_focus()
            if near_bottom or switching or self.force_scroll:
                self.force_scroll = False
                GLib.idle_add(self.scroll_bottom)
            elif grew:
                self.jump.set_visible(True)
        # An open, focused conversation counts as read when new messages arrive.
        if thread.get('unread_count') and self.window.is_active():
            self.mark_read(thread)
        self.update_composer()

    # --- Link previews (images, GIFs, YouTube) -----------------------------
    # Fetching is bounded and public-HTTPS only; see message_media.py.

    def stop_media_animation(self):
        for timer in self.media_timers:
            GLib.source_remove(timer)
        self.media_timers.clear()

    def media_card(self, url, automatic=False):
        """A preview card for one link. Sent links load at once; incoming ones ask first."""
        video = bool(youtube_id(url))
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, halign=Gtk.Align.END if automatic else Gtk.Align.START, valign=Gtk.Align.START)
        card.add_css_class('ms-media')
        title = label(_('YouTube') if video else _('Image'), xalign=0)
        card.append(title)
        load = Gtk.Button(label=_('Load Preview'))
        card.append(load)
        card.append(Gtk.LinkButton(uri=url, label=_('Open YouTube') if video else _('Open Image')))

        def completed(preview):
            if not card.get_root():
                return
            if preview is None:
                title.set_label(_('Preview unavailable · open the link to view it.'))
                title.set_wrap(True)
                title.set_max_width_chars(30)
                load.set_sensitive(True)
                return
            adjustment = self.message_scroll.get_vadjustment()
            bottom = adjustment.get_value() + adjustment.get_page_size() >= adjustment.get_upper() - 80
            clear(card)
            width, height = preview.size
            picture = Gtk.Picture(can_shrink=True, content_fit=Gtk.ContentFit.CONTAIN, width_request=width, height_request=height)
            picture.update_property([Gtk.AccessibleProperty.LABEL], [preview.title])
            def frame(index):
                picture.set_paintable(Gdk.MemoryTexture.new(width, height, Gdk.MemoryFormat.R8G8B8A8, GLib.Bytes.new(preview.frames[index]), width * 4))
            frame(0)
            card.append(picture)
            card.append(label(preview.title, 'heading', xalign=0, wrap=True, max_width_chars=30))
            source_width, source_height = preview.source_size
            info = f'{source_width} × {source_height}'
            if len(preview.frames) > 1:
                info += ' · ' + _('%d frames') % len(preview.frames)
            card.append(label(info, 'dim-label', xalign=0))
            card.append(Gtk.LinkButton(uri=url, label=_('Watch on YouTube') if preview.kind == 'youtube' else _('Open Original')))
            if len(preview.frames) > 1 and Gtk.Settings.get_default().props.gtk_enable_animations:
                position = [0]
                def schedule(delay):
                    holder = {}
                    def tick():
                        self.media_timers.discard(holder['id'])
                        if self.closed or not picture.get_root():
                            return False
                        if picture.get_mapped():
                            position[0] = (position[0] + 1) % len(preview.frames)
                            frame(position[0])
                        schedule(preview.durations[position[0]] if picture.get_mapped() else 500)
                        return False
                    holder['id'] = GLib.timeout_add(delay, tick)
                    self.media_timers.add(holder['id'])
                schedule(preview.durations[0])
            if bottom:
                GLib.idle_add(self.scroll_bottom)

        def start(*_args):
            if url in self.media_cache:
                self.media_cache.move_to_end(url)
                GLib.idle_add(lambda: completed(self.media_cache.get(url)) or False)
                return
            if self.fixture:
                title.set_label(_('Preview fixture · no network request'))
                return
            if url in self.media_pending:
                self.media_pending[url].append(completed)
                load.set_sensitive(False)
                return
            if len(self.media_pending) >= 6:
                title.set_label(_('Other previews are loading. Try again shortly.'))
                return
            load.set_sensitive(False)
            title.set_label(_('Loading preview…'))
            self.media_pending[url] = [completed]
            future = self.media_pool.submit(load_preview, url)
            def fetched(result):
                try:
                    preview = result.result()
                except Exception:
                    preview = None
                def deliver():
                    callbacks = self.media_pending.pop(url, [])
                    if self.closed:
                        return False
                    if preview:
                        self.media_cache[url] = preview
                        while len(self.media_cache) > 8 or sum(p.memory for p in self.media_cache.values()) > 24 * 1024 * 1024:
                            self.media_cache.popitem(last=False)
                    for callback in callbacks:
                        callback(preview)
                    return False
                GLib.idle_add(deliver)
            future.add_done_callback(fetched)

        load.connect('clicked', start)
        if url in self.media_cache or automatic or url in self.media_pending:
            GLib.idle_add(lambda: start() or False)
        return card

    def scroll_bottom(self, *_args):
        if not self.closed:
            a = self.message_scroll.get_vadjustment()
            a.set_value(max(a.get_lower(), a.get_upper() - a.get_page_size()))
            self.jump.set_visible(False)
        return False

    def scrolled(self, adjustment):
        if adjustment.get_value() + adjustment.get_page_size() >= adjustment.get_upper() - 80:
            self.jump.set_visible(False)

    def draft_changed(self, buffer):
        if self.model.selected:
            self.model.drafts[self.model.selected] = buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)
            self.send_feedback.pop(self.model.selected, None)
        self.update_composer()

    def reply_to_message(self, message):
        """Start a quoted reply: the quote rides above whatever the user types next."""
        key = self.model.selected
        thread = self.threads.get(key)
        if not thread:
            return
        self.replying[key] = quote_snippet(message, bool(thread.get('is_group')))
        self.update_composer()
        self.editor.grab_focus()

    def cancel_reply(self):
        self.replying.pop(self.model.selected, None)
        self.update_composer()
        self.editor.grab_focus()

    def group_info_missing(self):
        """True when BlueFerry reports it cannot see which group a text belongs to.

        iOS gives MAP only the sender; group membership comes from ANCS notifications, so
        without them a group text is filed under whoever wrote it. Absent in the preview
        fixture, which never claims anything about the phone.
        """
        return self.status.get('ancs') is False

    def update_composer(self):
        key = self.model.selected
        thread = self.threads.get(key)
        pending = key in self.model.pending
        draft = self.model.drafts.get(key, '')
        self.editor.set_sensitive(bool(thread))
        self.send.set_sensitive(bool(thread and thread.get('reply_ready') and self.status.get('map') and draft.strip() and not pending))
        self.send.set_tooltip_text(_('Send Message (Enter)') if self.settings['enter_sends'] else _('Send Message (Ctrl+Enter)'))
        self.composer.send_revealer.set_reveal_child(bool(draft.strip()) or pending)
        quote = self.replying.get(key, '') if thread else ''
        self.composer.quote_label.set_label(_('Replying to %s') % quote if quote else '')
        self.composer.quote_revealer.set_reveal_child(bool(quote))
        self.composer.face.set_visible_child_name('busy' if pending else 'icon')
        (self.composer.spinner.start if pending else self.composer.spinner.stop)()
        if not thread:
            hint = _('Choose a conversation to write a message.')
        elif not thread.get('reply_ready'):
            hint = reply_block_reason(thread)
        elif not self.status.get('map'):
            hint = _('Offline · your draft is kept')
        else:
            hint = _('Message')
        self.composer.placeholder.set_label(hint)
        self.composer.placeholder.set_visible(not draft)
        text = ''
        if pending:
            text = _('Sending…')
        elif thread and not thread.get('reply_ready'):
            text = reply_block_reason(thread)
        elif thread:
            text = self.send_feedback.get(key) or (((_('Enter to send · Shift+Enter for a new line') if self.settings['enter_sends'] else _('Ctrl+Enter to send · Enter for a new line')) if self.status.get('map') else _('Offline · your draft is kept while this window is open')) if draft.strip() else '')
            if text and draft.strip() and self.status.get('map') and self.group_info_missing() and not thread.get('is_group') and key not in self.send_feedback:
                text += ' · ' + _('Sends to this person only')
        self.send_status.set_tooltip_text(GROUP_INFO_NOTE if self.group_info_missing() else None)
        self.send_status.set_label(text)
        self.composer.revealer.set_reveal_child(bool(text))

    def set_enter_sends(self, action, value):
        action.set_state(value)
        self.settings['enter_sends'] = value.get_boolean()
        if not self.fixture:
            try:
                messages_settings.save(self.settings)
            except OSError:
                # The switch still works for this session; it just will not be remembered.
                pass
        self.update_composer()

    def arm_enter(self, _controller, key, _code, modifiers):
        """Remember an unmodified Return so guard_newline can recognise its newline."""
        plain = not modifiers & (Gdk.ModifierType.SHIFT_MASK | Gdk.ModifierType.CONTROL_MASK
                                 | Gdk.ModifierType.ALT_MASK | Gdk.ModifierType.SUPER_MASK)
        if self.settings['enter_sends'] and plain and key in (Gdk.KEY_Return, Gdk.KEY_KP_Enter):
            self.enter_armed = True
            GLib.timeout_add(80, self.disarm_enter)
        return False

    def disarm_enter(self):
        self.enter_armed = False
        return False

    def guard_newline(self, buffer, _location, text, _length):
        if not (self.enter_armed and text == '\n'):
            return
        # Swallow the newline: this Return means "send", not "new line".
        self.enter_armed = False
        buffer.stop_emission_by_name('insert-text')
        GLib.idle_add(self.send_from_enter)

    def send_from_enter(self):
        if not self.closed and self.send.get_sensitive():
            self.send_message()
        return False

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
        if blocks_group_confirmation(thread):
            self.send_status.set_label(_('Add this group’s participants in Advanced BlueFerry before replying.'))
            self.composer.revealer.set_reveal_child(True)
            return
        if needs_group_confirmation(thread):
            # BlueFerry will not message a group until the user has seen who is in it.
            self.confirm_group_send(thread)
            return
        self.start_send(thread)

    def confirm_group_send(self, thread):
        """Show every recipient in a native dialog; send only on an explicit yes."""
        key, token = thread['key'], thread['confirmation_token']
        roster = group_roster(thread, self.contact_names)
        count = len(roster)
        secondary = (ngettext('This message will go to this %d person.', 'This message will go to these %d people.', count) % count
                     + ' ' + _('BlueFerry remembers the list and asks again only if the group changes.'))
        if thread.get('roster_changed') or thread.get('unexpected_sender'):
            secondary += ' ' + _('Someone who was not on the list has written to this group recently.')
        dialog = Granite.MessageDialog.with_image_from_icon_name(_('Reply to this group?'), secondary, icon_for('system-users', 'avatar-default'), Gtk.ButtonsType.NONE)
        dialog.set_transient_for(self.window)
        dialog.set_modal(True)
        people = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8, margin_top=6)
        for name, address in roster:
            row = Gtk.Box(spacing=10)
            row.append(avatar(name or address, address, 's'))
            texts = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, valign=Gtk.Align.CENTER)
            texts.append(label(name or address, xalign=0, ellipsize=Pango.EllipsizeMode.END))
            if name:
                texts.append(label(address, 'dim-label', xalign=0))
            row.append(texts)
            people.append(row)
        dialog.get_custom_bin().append(Gtk.ScrolledWindow(child=people, hscrollbar_policy=Gtk.PolicyType.NEVER, propagate_natural_height=True,
                                                          propagate_natural_width=True, min_content_width=320, max_content_height=220))
        dialog.add_button(_('Cancel'), Gtk.ResponseType.CANCEL)
        go = dialog.add_button(ngettext('Send to %d Person', 'Send to %d People', count) % count, Gtk.ResponseType.ACCEPT)
        go.add_css_class('suggested-action')
        # Enter must never confirm a group send by accident.
        dialog.set_default_response(Gtk.ResponseType.CANCEL)
        def responded(_dialog, response):
            dialog.destroy()
            self.group_dialog = None
            if response != Gtk.ResponseType.ACCEPT:
                return
            current = self.threads.get(key)
            if not current or current.get('confirmation_token') != token:
                self.send_status.set_label(_('The group changed. Review the participants and send again.'))
                self.composer.revealer.set_reveal_child(True)
                return
            self.start_send(current, confirm_group=True)
        dialog.connect('response', responded)
        self.group_dialog = dialog
        dialog.present()

    def start_send(self, thread, confirm_group=False):
        try:
            attempt = self.model.prepare(thread, confirm_group)
        except ValueError as error:
            self.send_status.set_label(str(error))
            self.composer.revealer.set_reveal_child(True)
            return
        # A reply carries its quote in the text, so the phone and the echo see the same body.
        wire = reply_body(self.replying.get(attempt.key, ''), attempt.body)
        entry = self.outbox.add(attempt.key, wire, thread['messages'])
        self.force_scroll = True
        self.update_composer()
        self.show_conversation()
        def send():
            if self.fixture:
                raise RuntimeError('Preview cannot send messages')
            client = BackendClient()
            if attempt.backend_thread:
                return client.send_to_thread(attempt.key, wire, confirm_group=attempt.confirm_group, expected_group_token=attempt.token)
            return client.send(attempt.recipient, wire)
        def sent(_transfer, error):
            self.model.finish(attempt, not error)
            if error:
                self.outbox.failed(entry)
            else:
                self.outbox.delivered(entry)
                self.replying.pop(attempt.key, None)
                if attempt.confirm_group and attempt.key in self.threads:
                    # BlueFerry remembered the confirmation; do not ask again before the refresh lands.
                    self.threads[attempt.key]['group_confirmed'] = True
            if self.model.selected == attempt.key:
                self.editor.get_buffer().set_text(self.model.drafts.get(attempt.key, ''))
            self.send_feedback[attempt.key] = (_('Could not send: %s. Your draft is kept; choose Send to retry.') % send_failure_reason(error)) if error else _('Sent through your iPhone')
            self.update_composer()
            self.show_conversation()
            self.refresh()
        self.task(send, sent)

    def show_shortcuts(self, *_args):
        """Every shortcut in one native list; accelerators come from the actions themselves."""
        sheet = Gtk.Window(title=_('Keyboard Shortcuts'), transient_for=self.window, modal=True, destroy_with_parent=True, default_width=380)
        header = Gtk.HeaderBar()
        header.set_title_widget(label(_('Keyboard Shortcuts'), 'title'))
        sheet.set_titlebar(header)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, margin_start=20, margin_end=20, margin_top=16, margin_bottom=20)
        rows = [(_('New Message'), 'app.new-message'), (_('Search Conversations'), 'app.search'), (_('Refresh iPhone Inbox'), 'app.refresh'),
                (_('Back to Conversations'), 'app.back'),
                (_('Send Message'), 'Return' if self.settings['enter_sends'] else '<Control>Return'),
                (_('New Line'), '<Shift>Return' if self.settings['enter_sends'] else 'Return')]
        for text, accel in rows:
            line = Gtk.Box(spacing=12)
            line.append(label(text, xalign=0, hexpand=True))
            # Read the accelerator from the app ourselves; Granite's own action lookup
            # showed nothing for these actions.
            accel_string = (self.get_accels_for_action(accel) or [''])[0] if accel.startswith('app.') else accel
            line.append(Granite.AccelLabel(label='', accel_string=accel_string))
            box.append(line)
        sheet.set_child(box)
        keys = Gtk.EventControllerKey()
        keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE)
        keys.connect('key-pressed', lambda _c, key, _code, _mods: sheet.close() or True if key == Gdk.KEY_Escape else False)
        sheet.add_controller(keys)
        sheet.present()
        return sheet

    def focus_search(self, *_args):
        if self.sidebar.get_visible():
            self.search.grab_focus()

    def new_message(self, *_args):
        dialog = Gtk.Window(title=_('New Message'), transient_for=self.window, modal=True, destroy_with_parent=True, default_width=440, default_height=500)
        header = Gtk.HeaderBar()
        header.set_title_widget(label(_('New Message'), 'title'))
        cancel = Gtk.Button(label=_('Cancel'))
        cancel.connect('clicked', lambda *_a: dialog.close())
        header.pack_start(cancel)
        dialog.set_titlebar(header)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10, margin_start=16, margin_end=16, margin_top=14, margin_bottom=16)
        entry = Gtk.SearchEntry(placeholder_text=_('Name, phone number, or email'))
        entry.update_property([Gtk.AccessibleProperty.LABEL], [_('To')])
        box.append(entry)
        hint = label(_('Choose a contact or enter a complete address.'), 'dim-label', wrap=True, xalign=0)
        box.append(hint)
        results = Gtk.ListBox(selection_mode=Gtk.SelectionMode.SINGLE, activate_on_single_click=True)
        results.add_css_class('navigation-sidebar')
        box.append(Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER, child=results))
        use = Gtk.Button(label=_('Start Conversation'), sensitive=False)
        use.add_css_class('suggested-action')
        box.append(use)
        dialog.set_child(box)
        dialog.set_default_widget(use)
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
        def add_result(name, address):
            row = Gtk.ListBoxRow()
            row.contact = (name, address)
            content = Gtk.Box(spacing=12, margin_top=6, margin_bottom=6, margin_start=4, margin_end=4)
            content.append(avatar(name or address, address, 's'))
            texts = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2, valign=Gtk.Align.CENTER)
            texts.append(label(name or address, xalign=0, ellipsize=Pango.EllipsizeMode.END))
            if name:
                texts.append(label(address, 'dim-label', xalign=0, ellipsize=Pango.EllipsizeMode.MIDDLE))
            content.append(texts)
            row.set_child(content)
            row.update_property([Gtk.AccessibleProperty.LABEL], [name + ', ' + address if name else address])
            results.append(row)
        def show_recents():
            clear(results)
            recent = [t for t in sorted(self.threads.values(), key=lambda t: t.get('last_ts', ''), reverse=True)
                      if not t.get('is_group') and len(t.get('recipients', [])) == 1 and not t.get('new')][:8]
            for thread in recent:
                add_result(thread['name'], thread['recipients'][0])
            hint.set_label(_('Recent conversations') if recent else _('Choose a contact or enter a complete address.'))
        def changed(*_a):
            state['generation'] += 1
            generation = state['generation']
            query = entry.get_text().strip()
            try:
                validate_recipient(query)
                use.set_sensitive(True)
            except ValueError:
                use.set_sensitive(False)
            if len(query) < 2:
                show_recents()
                return
            def found(matches, error):
                if not state['alive'] or generation != state['generation']:
                    return
                clear(results)
                hint.set_label(_('Could not search contacts.') if error else (_('Choose the address you want to message.') if matches else _('No matching contacts. A complete number or email works too.')))
                for name, address in (matches or [])[:30]:
                    add_result(name, address)
            self.task(lambda: [] if self.fixture else BackendClient().find_contacts(query), found)
        def activated(*_a):
            # A complete address typed by the user wins; otherwise take the first match.
            if use.get_sensitive():
                choose('', entry.get_text())
            elif (first := results.get_row_at_index(0)):
                choose(*first.contact)
        def entry_key(_c, key, _code, _mods):
            if key == Gdk.KEY_Down and (first := results.get_row_at_index(0)):
                results.select_row(first)
                first.grab_focus()
                return True
            return False
        def dialog_key(_c, key, _code, _mods):
            if key == Gdk.KEY_Escape:
                dialog.close()
                return True
            return False
        entry.connect('search-changed', changed)
        entry.connect('activate', activated)
        results.connect('row-activated', lambda _l, row: choose(*row.contact))
        use.connect('clicked', lambda *_a: choose('', entry.get_text()))
        for widget, handler, phase in ((entry, entry_key, Gtk.PropagationPhase.BUBBLE), (dialog, dialog_key, Gtk.PropagationPhase.CAPTURE)):
            controller = Gtk.EventControllerKey()
            controller.set_propagation_phase(phase)
            controller.connect('key-pressed', handler)
            widget.add_controller(controller)
        show_recents()
        dialog.present()
        entry.grab_focus()

    def responsive(self, *_args):
        self.apply_layout()
        return not self.closed

    def apply_layout(self):
        width = self.window.get_width() or self.window.get_default_size()[0]
        self.layout(width < NARROW)

    def layout(self, narrow):
        selected = bool(self.model.selected)
        self.sidebar.set_visible(not narrow or not selected)
        self.detail.set_visible(not narrow or selected)
        self.back.set_visible(narrow and selected)

    def go_back(self, *_args):
        self.model.selected = ''
        self.list.unselect_all()
        self.sync_favorite_selection()
        self.show_conversation()
        self.apply_layout()

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
        self.media_pool.shutdown(wait=False, cancel_futures=True)
        self.stop_media_animation()
        return False

    def sleep_changed(self, sleeping):
        if not sleeping and not self.closed:
            self.refresh(True)

    def preview(self):
        """Credential-free fixture used by tests and screenshots; it never touches Bluetooth."""
        now = datetime.now().astimezone()
        def at(**ago):
            return (now - timedelta(**ago)).isoformat(timespec='seconds')
        def msg(handle, body, outgoing, when, sender=''):
            return dict(handle=handle, body=body, outgoing=outgoing, timestamp=when, sender=sender)
        alex = [msg('demo-a', 'Did you get the new desktop running?', False, at(days=1, hours=1)),
                msg('demo-b', 'Setting it up now. Wish me luck.', True, at(days=1, hours=1, minutes=-2)),
                msg('demo1', 'Made it home! How is the new desktop?', False, at(hours=3)),
                msg('demo2', 'Even better now. Messages are right here with everything else.', True, at(hours=2, minutes=58)),
                msg('demo2b', 'Links work too: https://elementary.io/docs/human-interface-guidelines', True, at(hours=2, minutes=58, seconds=-20)),
                msg('demo3', 'That looks so much easier. See you for coffee tomorrow ☕', False, at(hours=2, minutes=55)),
                msg('demo5', 'Here is the whole plan so you have it in one place: we leave at nine, stop for coffee on the way, '
                    'get to the trailhead by eleven, eat at the overlook around one, and aim to be back at the car before the '
                    'light goes. Bring layers, water, and the good camera. Details and the map are at https://elementary.io/docs/learn.',
                    False, at(hours=2, minutes=40)),
                msg('demo6', '\ufffc', False, at(hours=2, minutes=39)),
                msg('demo7', 'Love it. Sending the photo from the last trip too.\ufffc', True, at(hours=2, minutes=20))]
        self.status = dict(map=True)
        self.set_connection('ready')
        self.threads = {
            'demo': dict(key='demo', name='Alex Morgan', recipients=['+15550100123'], reply_ready=True, messages=alex, last_ts=alex[-1]['timestamp']),
            'demo2': dict(key='demo2', name='A very long contact name that should fit comfortably', recipients=['+15550100124'], reply_ready=True,
                          messages=[msg('demo4', 'This is a preview conversation.', False, at(days=1))], last_ts=at(days=1)),
            'family': dict(key='family', name='Family', recipients=['+15550100126', '+15550100127'], is_group=True, reply_ready=True,
                           confirmation_token='fixture-roster', group_confirmed=False,
                           messages=[msg('fam1', 'Dinner at 7?', False, at(minutes=50), '+15550100126'),
                                     msg('fam2', 'I can bring dessert', False, at(minutes=48), '+15550100127'),
                                     msg('fam3', 'Perfect, see you both then', True, at(minutes=40))], last_ts=at(minutes=40)),
            'jordan': dict(key='jordan', name='Jordan Lee', recipients=['+15550100125'], reply_ready=True, unread_count=2,
                           messages=[msg('jl1', 'Running ten minutes late', False, at(minutes=12)), msg('jl2', 'Save me a seat?', False, at(minutes=11))], last_ts=at(minutes=11)),
            'sam': dict(key='sam', name='Sam Rivera', recipients=['+15550100128'], reply_ready=True,
                        messages=[msg('sr1', 'Sounds good, thanks!', True, at(days=3))], last_ts=at(days=3)),
        }
        self.populate()
        self.list.select_row(self.rows['demo'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--preview', action='store_true')
    parser.add_argument('--dark', action='store_true')
    parser.add_argument('--narrow', action='store_true')
    args = parser.parse_args()
    Messages(args.preview, args.dark, args.narrow).run([])
