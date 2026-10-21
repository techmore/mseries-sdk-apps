"""Isolated GTK tests. Run with: xvfb-run -a python3 test_messages_gui.py

These open and close many windows and send key events, so they refuse to run on a
real desktop: the display must be an isolated virtual one and the X11 backend is
forced (see gui_safety.py). Nothing here ever appears on the user's screen.
"""
import os
import sys
import time
import unittest
from pathlib import Path

import gui_safety
SAFE, WHY = gui_safety.check(os.environ)
if not SAFE:
    if __name__ == '__main__':
        print('skipped: ' + WHY)
        sys.exit(0)
    raise unittest.SkipTest(WHY)
gui_safety.force_x11(os.environ)
from messages import Messages, Gtk, GLib, Gdk
from messages_widgets import photo_frame
from unittest.mock import patch


def settle(seconds=.25):
    context = GLib.MainContext.default()
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        while context.pending():
            context.iteration(False)
        time.sleep(.01)


def walk(widget):
    yield widget
    child = widget.get_first_child()
    while child:
        yield from walk(child)
        child = child.get_next_sibling()


def with_class(widget, css):
    return [w for w in walk(widget) if w.has_css_class(css)]


def msg(body, when, outgoing=False, sender='', handle=''):
    return dict(handle=handle or body, body=body, timestamp=when, outgoing=outgoing, sender=sender)


def run_inline(app):
    """Run worker tasks on the calling thread, mirroring the real error contract."""
    def task(work, completed):
        try:
            value, error = work(), None
        except Exception as exc:
            value, error = None, str(exc)
        completed(value, error)
    app.task = task


@unittest.skipUnless(os.environ.get('DISPLAY'), 'Requires isolated X display')
class MessagesUI(unittest.TestCase):
    def setUp(self):
        self.app = Messages(fixture=True)
        self.app.set_application_id('io.elementary.mseries.Messages.Test' + self._testMethodName.replace('_', ''))
        self.app.register()
        self.app.activate()
        settle()

    def tearDown(self):
        self.app.window.close()
        self.app.quit()
        settle(.05)

    def test_selection_draft_and_keyboard_focus(self):
        app = self.app
        app.editor.get_buffer().set_text('Unsent draft')
        app.list.select_row(app.rows['demo2'])
        self.assertEqual(app.model.selected, 'demo2')
        self.assertEqual(app.model.drafts['demo'], 'Unsent draft')
        app.list.select_row(app.rows['demo'])
        buffer = app.editor.get_buffer()
        self.assertEqual(buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True), 'Unsent draft')
        self.assertTrue(app.editor.grab_focus())
        self.assertTrue(app.send.get_sensitive())

    def test_unread_marker_stays_left_of_avatar_in_list_and_favorites(self):
        app=self.app
        app.threads['demo']['unread_count']=1
        app.populate()
        parent,avatar=app.avatars['demo']
        self.assertTrue(avatar.get_prev_sibling().get_first_child().get_visible())
        app.pin_state=dict(version=1,snapshot=None,choices=['demo'])
        app.populate()
        parent,avatar=app.favorite_avatars['demo']
        self.assertTrue(avatar.get_prev_sibling().get_first_child().get_visible())
        app.threads['demo']['unread_count']=0;app.populate()
        parent,avatar=app.favorite_avatars['demo']
        self.assertFalse(avatar.get_prev_sibling().get_first_child().get_visible())

    def test_media_card_renders_cached_gif_and_stops_animation_on_close(self):
        import io
        from PIL import Image
        from message_media import decode_image
        frames=[Image.new('RGB',(80,60),c) for c in ('red','blue')]
        out=io.BytesIO();frames[0].save(out,format='GIF',save_all=True,append_images=frames[1:],duration=100,loop=0)
        url='https://example.org/fixture.gif'
        self.app.media_cache[url]=decode_image(out.getvalue())
        self.app.threads['demo']['messages'].append(dict(body=url,outgoing=True,timestamp='Now'))
        self.app.show_conversation();settle(.3)
        line=self.app.messages.get_last_child()
        card=line.get_first_child().get_next_sibling()
        picture=card.get_first_child()
        self.assertIsInstance(picture,Gtk.Picture)
        self.assertEqual(picture.get_paintable().get_width(),80)
        self.assertTrue(self.app.media_timers)
        self.app.stop_media_animation()
        self.assertFalse(self.app.media_timers)

    def test_offline_retains_conversations_and_disables_send(self):
        app = self.app
        app.editor.get_buffer().set_text('Keep offline')
        app.loaded(None, 'Backend unavailable')
        self.assertFalse(app.send.get_sensitive())
        self.assertEqual(app.model.drafts['demo'], 'Keep offline')
        self.assertIn('demo', app.threads)

    def test_search_filters_body_and_recovers(self):
        app = self.app
        app.search.set_text('coffee');app.filter_rows()
        self.assertTrue(app.rows['demo'].get_visible())
        self.assertFalse(app.rows['demo2'].get_visible())
        app.search.set_text('');app.filter_rows()
        self.assertTrue(app.rows['demo2'].get_visible())

    def test_narrow_navigation_and_long_message(self):
        app = self.app
        app.window.set_default_size(480, 680)
        app.threads['demo']['messages'].append(dict(body='longword' * 50, outgoing=False, timestamp='Now'))
        app.show_conversation();settle(.4)
        self.assertLessEqual(app.window.get_width(), 500)
        self.assertFalse(app.sidebar.get_visible())
        self.assertTrue(app.detail.get_visible())
        self.assertTrue(app.back.get_visible())
        app.go_back();settle()
        self.assertTrue(app.sidebar.get_visible())
        self.assertFalse(app.detail.get_visible())

    def test_preview_cannot_send(self):
        app = self.app
        app.editor.get_buffer().set_text('Fixture only')
        app.send_message();settle(.5)
        self.assertEqual(app.model.drafts['demo'], 'Fixture only')
        self.assertIn('Could not send', app.send_status.get_text())

    def test_resume_requests_fresh_inbox_even_after_short_sleep(self):
        self.app.inbox_requested = False
        self.app.sleep_changed(False)
        self.assertTrue(self.app.inbox_requested)

    def test_resume_during_inflight_read_is_not_lost(self):
        app = self.app
        app.sleep_changed(False)
        epoch = app.inbox_request_epoch
        app.sleep_changed(False)
        app.loaded((dict(map=True), list(app.threads.values()), [], '', epoch), None)
        self.assertTrue(app.inbox_requested)

    def test_photo_avatar_is_bounded_and_invalid_data_falls_back(self):
        import gi
        gi.require_version('GdkPixbuf', '2.0')
        from gi.repository import GdkPixbuf
        pixbuf = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 512, 512)
        pixbuf.fill(0x3689e6ff)
        _ok, data = pixbuf.save_to_bufferv('png', [], [])
        self.assertIsNone(self.app.photo_widget(b'not an image'))
        self.app.photo_cache['+15550100123'] = bytes(data)
        self.app.row_signature = None
        self.app.populate();settle()
        avatar = self.app.avatars['demo'][1]
        self.assertLessEqual(avatar.get_width(), 50)
        self.assertLessEqual(avatar.get_height(), 50)

    def test_old_pinned_contact_photo_has_priority_over_newer_hidden_threads(self):
        from unittest.mock import patch
        app = self.app
        app.threads = {str(i): dict(key=str(i), name='Fixture contact', recipients=['+1555010%04d' % i], messages=[], last_ts='%04d' % i) for i in range(41)}
        app.pin_state['choices'] = ['0']
        app.status['contact_photos'] = True
        app.task = lambda work, completed: completed(work(), None)
        with patch('messages.BackendClient') as backend:
            backend.return_value.contact_photo.return_value = b''
            app.fixture = False
            try:
                app.load_photos()
            finally:
                app.fixture = True
            requested = [call.args[0] for call in backend.return_value.contact_photo.call_args_list]
        self.assertEqual(len(requested), 40)
        self.assertIn('+15550100000', requested)
        self.assertNotIn('+15550100001', requested)

    def pin_snapshot(self):
        from message_pins import SCHEMA
        return dict(schema=SCHEMA, version=1, source='macos-imcore', captured_at='2026-10-20T16:00:00Z',
                    pins=[dict(apple_identifier='opaque-group'), dict(apple_identifier='+15550100123')])

    def test_import_pin_order_keeps_draft_selection_and_group_choice(self):
        app = self.app
        app.editor.get_buffer().set_text('Keep while importing pins')
        snapshot = self.pin_snapshot()
        snapshot.update(source='macos-pin-cache', source_modified_at='2026-10-19T16:00:00Z')
        dialog = app.show_pin_review(snapshot);settle()
        dropdowns, apply, error = dialog.pin_controls
        self.assertEqual(dropdowns[0].get_selected(), 0, 'opaque group must not be guessed')
        self.assertEqual(dropdowns[1].get_selected(), 2, 'Alex is an exact individual match')
        dropdowns[0].set_selected(1)
        apply.emit('clicked');settle(.4)
        self.assertEqual(app.pin_state['choices'], ['demo2', 'demo'])
        self.assertEqual(app.list.get_row_at_index(0).key, 'demo2')
        self.assertEqual(app.model.selected, 'demo')
        self.assertEqual(app.model.drafts['demo'], 'Keep while importing pins')
        self.assertIn('Pins from Mac', app.pin_note.get_text())
        self.assertEqual(app.pin_state['snapshot']['source'], 'macos-pin-cache')
        self.assertIn('Apple pin settings last changed', app.pin_note.get_tooltip_text())
        self.assertIn('demo2', app.favorite_buttons)
        self.assertTrue(app.favorites.get_visible())

    def test_favorite_grid_four_across_divider_and_thread_selection(self):
        app=self.app
        keys=['demo']
        for index in range(4):
            key=f'fixture-favorite-{index}'
            app.threads[key]=dict(app.threads['demo'],key=key,name=f'Favorite {index}',messages=[])
            keys.append(key)
        app.pin_state=dict(version=1,snapshot=None,choices=keys)
        app.populate();settle()
        self.assertTrue(app.favorite_separator.get_visible())
        self.assertEqual(tuple(app.favorite_grid.query_child(app.favorite_buttons[keys[3]])), (3,0,1,1))
        self.assertEqual(tuple(app.favorite_grid.query_child(app.favorite_buttons[keys[4]])), (0,1,1,1))
        if destination := os.environ.get('MSERIES_MESSAGES_SCREENSHOTS'):
            from PIL import ImageGrab
            out=Path(destination);out.mkdir(parents=True,exist_ok=True)
            for dark in (False,True):
                Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme=dark;settle(.2)
                ImageGrab.grab(bbox=(0,0,app.window.get_width(),app.window.get_height()),xdisplay=os.environ['DISPLAY']).save(out/('favorites-dark.png' if dark else 'favorites-light.png'))
        app.editor.get_buffer().set_text('Preserve favorite draft')
        app.favorite_buttons[keys[4]].emit('clicked')
        self.assertEqual(app.model.selected,keys[4])
        self.assertEqual(app.model.drafts['demo'],'Preserve favorite draft')
        self.assertTrue(app.favorite_buttons[keys[4]].get_active())
        app.search.set_text('Favorite 3');app.filter_rows()
        self.assertFalse(app.favorite_separator.get_visible())
        self.assertEqual(tuple(app.favorite_grid.query_child(app.favorite_buttons[keys[4]])), (0,0,1,1))

    def test_duplicate_pin_match_keeps_existing_pins(self):
        dialog = self.app.show_pin_review(self.pin_snapshot());settle()
        dropdowns, apply, error = dialog.pin_controls
        dropdowns[0].set_selected(2);apply.emit('clicked');settle()
        self.assertTrue(error.get_visible())
        self.assertEqual(self.app.pin_state['choices'], [])
        self.assertTrue(apply.get_sensitive())
        dialog.close()

    def test_pin_save_failure_retains_previous_snapshot_and_retry(self):
        app = self.app
        dialog = app.show_pin_review(self.pin_snapshot());settle()
        dropdowns, apply, error = dialog.pin_controls
        # Isolate failure at the persistence boundary; no desktop keyring touched.
        app.task = lambda _work, completed: completed(None, 'keyring unavailable')
        apply.emit('clicked');settle()
        self.assertEqual(app.pin_state['choices'], [])
        self.assertTrue(error.get_visible());self.assertTrue(apply.get_sensitive())
        dialog.close()

    def test_cancel_pin_review_does_not_apply(self):
        dialog = self.app.show_pin_review(self.pin_snapshot());settle()
        dialog.close();settle()
        self.assertIsNone(self.app.pin_state['snapshot'])

    # --- Native experience -------------------------------------------------

    def test_runs_on_the_isolated_x11_display_never_a_live_session(self):
        display = self.app.window.get_display()
        self.assertIn('X11', type(display).__name__)
        self.assertEqual(display.get_name(), os.environ['DISPLAY'])
        self.assertNotIn('WAYLAND_DISPLAY', os.environ)
        self.assertEqual(os.environ['GDK_BACKEND'], 'x11')

    def test_stylesheet_parses_cleanly_on_this_toolkit(self):
        self.assertEqual(self.app.css_errors, [])

    def test_every_theme_colour_the_stylesheet_uses_resolves(self):
        import re
        context = self.app.window.get_style_context()
        # Names after @define-color lines, i.e. the ones our rules actually reference.
        rules = self.app.stylesheet.split('\n.ms-', 1)[1]
        names = sorted(set(re.findall(r'@([A-Za-z_]+)', rules)))
        self.assertIn('accent_bg_color', names)
        missing = [n for n in names if not context.lookup_color(n)[0]]
        self.assertEqual(missing, [], 'GTK silently ignores unknown colours, so check them explicitly')

    def test_accent_names_are_defined_when_the_theme_lacks_them(self):
        from messages_style import ACCENT_FALLBACKS, build_style
        css = build_style(ACCENT_FALLBACKS)
        self.assertTrue(css.startswith('@define-color accent_bg_color @theme_selected_bg_color;'))
        self.assertIn('@define-color accent_fg_color @theme_selected_fg_color;', css)
        self.assertNotIn('@define-color', build_style())
        for name in ACCENT_FALLBACKS:
            self.assertTrue(self.app.window.get_style_context().lookup_color(name)[0], name)
        # The alias mechanism itself: a name this theme lacks, defined the same way, resolves.
        provider = Gtk.CssProvider()
        provider.load_from_string('@define-color ms_probe_missing @theme_selected_bg_color;')
        display = self.app.window.get_display()
        Gtk.StyleContext.add_provider_for_display(display, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        try:
            ok, probe = self.app.window.get_style_context().lookup_color('ms_probe_missing')
            self.assertTrue(ok)
            self.assertEqual(probe.to_string(), self.app.window.get_style_context().lookup_color('theme_selected_bg_color')[1].to_string())
        finally:
            Gtk.StyleContext.remove_provider_for_display(display, provider)

    def test_missing_accent_probe_reports_only_undefined_names(self):
        from messages_style import missing_accent_names
        class Context:
            def __init__(self, known): self.known = known
            def lookup_color(self, name): return (name in self.known, None)
        self.assertEqual(missing_accent_names(Context({'theme_selected_bg_color'})), ['accent_bg_color', 'accent_fg_color'])
        self.assertEqual(missing_accent_names(Context({'accent_bg_color'})), ['accent_fg_color'])
        self.assertEqual(missing_accent_names(Context({'accent_bg_color', 'accent_fg_color'})), [])

    def test_avatars_never_expand_so_every_row_aligns(self):
        app = self.app
        left = set()
        for key in ('demo', 'jordan', 'family', 'sam'):
            slot, portrait = app.avatars[key]
            self.assertFalse(slot.get_hexpand(), key)
            ok, bounds = portrait.compute_bounds(app.rows[key])
            self.assertTrue(ok)
            left.add(round(bounds.get_x()))
        self.assertEqual(len(left), 1, 'avatars start at the same x in every row: %s' % left)

    def test_unread_dot_only_on_unread_conversations(self):
        app = self.app
        self.assertTrue(with_class(app.rows['jordan'], 'ms-unread')[0].get_visible())
        self.assertFalse(with_class(app.rows['sam'], 'ms-unread')[0].get_visible())

    def test_rows_show_preview_with_you_prefix_and_group_glyph(self):
        app = self.app
        previews = {k: with_class(app.rows[k], 'ms-preview')[0].get_label() for k in ('sam', 'jordan')}
        self.assertEqual(previews['sam'], 'You: Sounds good, thanks!')
        self.assertEqual(previews['jordan'], 'Save me a seat?')
        self.assertTrue(any(isinstance(w, Gtk.Image) for w in walk(app.avatars['family'][1])), 'groups use a people glyph')

    def test_conversation_groups_bubbles_into_runs_with_one_tail(self):
        app = self.app
        app.threads['demo']['messages'] = [
            msg('one', '2026-01-05T09:00:00'), msg('two', '2026-01-05T09:01:00'),
            msg('three', '2026-01-05T09:02:00', True), msg('four', '2026-01-05T09:03:00', True),
            msg('five', '2026-01-05T15:00:00')]
        app.rendered = None
        app.show_conversation();settle()
        self.assertEqual(len(with_class(app.messages, 'ms-stamp')), 2, 'a heading at the start and after the long silence')
        by_text = {b.get_text(): b for b in with_class(app.messages, 'ms-bubble')}
        self.assertTrue(by_text['one'].has_css_class('ms-first') and by_text['two'].has_css_class('ms-last'))
        self.assertTrue(by_text['three'].has_css_class('ms-outgoing') and by_text['four'].has_css_class('ms-last'))
        self.assertTrue(by_text['five'].has_css_class('ms-single'))

    def test_message_text_is_literal_and_only_safe_urls_become_links(self):
        app = self.app
        body = '<b>not bold</b> & https://example.com/a'
        app.threads['demo']['messages'] = [msg(body, '2026-01-05T09:00:00')]
        app.rendered = None
        app.show_conversation();settle()
        bubble = with_class(app.messages, 'ms-bubble')[0]
        self.assertEqual(bubble.get_text(), body)
        self.assertIn('<a href="https://example.com/a">', bubble.get_label())
        self.assertNotIn('<b>', bubble.get_label(), 'markup in a message must be escaped')

    def test_send_button_and_placeholder_follow_the_draft(self):
        app = self.app
        app.editor.get_buffer().set_text('');settle(.3)
        self.assertFalse(app.composer.send_revealer.get_reveal_child())
        self.assertTrue(app.composer.placeholder.get_visible())
        app.editor.get_buffer().set_text('Hello');settle(.3)
        self.assertTrue(app.composer.send_revealer.get_reveal_child())
        self.assertFalse(app.composer.placeholder.get_visible())
        self.assertTrue(app.composer.revealer.get_reveal_child(), 'keyboard hint appears with a draft')

    def test_sending_shows_a_dimmed_bubble_then_hands_over_to_the_backend_echo(self):
        from unittest.mock import patch
        app = self.app
        queued = []
        app.task = lambda work, completed: queued.append((work, completed))
        app.editor.get_buffer().set_text('Right behind you')
        app.send_message();settle()
        bubble = [b for b in with_class(app.messages, 'ms-bubble') if b.get_text() == 'Right behind you']
        self.assertEqual(len(bubble), 1)
        self.assertTrue(bubble[0].has_css_class('ms-sending'))
        self.assertEqual(app.composer.face.get_visible_child_name(), 'busy')
        self.assertFalse(app.send.get_sensitive(), 'no duplicate sends while pending')
        self.assertEqual(app.send_status.get_text(), 'Sending…')
        # The phone accepts the message: bubble stays, no longer dimmed.
        work, completed = queued[0]
        with patch('messages.BackendClient') as backend:
            backend.return_value.send_to_thread.return_value = 'transfer'
            app.fixture = False
            try:
                value, error = work(), None
            finally:
                app.fixture = True
        app.refresh = lambda *_a: True
        completed(value, error);settle()
        bubble = [b for b in with_class(app.messages, 'ms-bubble') if b.get_text() == 'Right behind you']
        self.assertEqual(len(bubble), 1)
        self.assertFalse(bubble[0].has_css_class('ms-sending'))
        self.assertEqual(app.model.drafts.get('demo', ''), '')
        self.assertEqual(app.send_status.get_text(), 'Sent through your iPhone')
        # BlueFerry history now contains it: exactly one bubble, never two.
        app.threads['demo']['messages'].append(msg('Right behind you', '2026-01-05T09:00:00', True))
        app.show_conversation();settle()
        self.assertEqual(sum(b.get_text() == 'Right behind you' for b in with_class(app.messages, 'ms-bubble')), 1)
        self.assertEqual(app.outbox.entries, [])

    def test_failed_send_removes_the_pending_bubble_and_keeps_the_draft(self):
        app = self.app
        run_inline(app)
        app.editor.get_buffer().set_text('Will fail')
        app.send_message();settle()
        self.assertEqual(app.model.drafts['demo'], 'Will fail')
        self.assertEqual(app.outbox.entries, [])
        self.assertFalse([b for b in with_class(app.messages, 'ms-bubble') if b.get_text() == 'Will fail'])
        self.assertIn('Could not send', app.send_status.get_text())
        self.assertEqual(app.composer.face.get_visible_child_name(), 'icon')

    def test_new_message_while_scrolled_up_offers_jump_to_latest(self):
        app = self.app
        app.threads['demo']['messages'] = [msg('filler %d ' % i + 'word ' * 20, '2026-01-05T09:%02d:00' % i) for i in range(40)]
        app.rendered = None
        app.show_conversation();settle(.5)
        adjustment = app.message_scroll.get_vadjustment()
        self.assertGreater(adjustment.get_upper(), adjustment.get_page_size() * 2)
        adjustment.set_value(0);settle()
        app.threads['demo']['messages'] = app.threads['demo']['messages'] + [msg('fresh arrival', '2026-01-05T10:30:00')]
        app.show_conversation();settle(.3)
        self.assertTrue(app.jump.get_visible())
        app.jump.emit('clicked');settle(.4)
        self.assertFalse(app.jump.get_visible())
        self.assertGreaterEqual(adjustment.get_value() + adjustment.get_page_size(), adjustment.get_upper() - 80)

    def test_empty_search_explains_itself_and_recovers(self):
        app = self.app
        self.assertFalse(app.empty_list.get_visible())
        app.search.set_text('zzz-nothing');app.filter_rows()
        self.assertTrue(app.empty_list.get_visible())
        self.assertEqual(app.empty_list.title.get_label(), 'No Results')
        app.search.set_text('');app.filter_rows()
        self.assertFalse(app.empty_list.get_visible())

    def test_heading_hides_until_a_conversation_is_open(self):
        app = self.app
        self.assertTrue(app.heading.get_visible())
        app.go_back();settle()
        self.assertFalse(app.heading.get_visible())
        self.assertEqual(app.stack.get_visible_child_name(), 'welcome')

    def test_welcome_pane_does_not_force_a_tall_window(self):
        minimum = self.app.window.get_child().measure(Gtk.Orientation.VERTICAL, -1)[0]
        self.assertLessEqual(minimum, 420, 'the window must still fit its 420px minimum height')

    def test_retry_connection_is_offered_when_offline_and_reconnects_in_place(self):
        from unittest.mock import patch
        app = self.app
        run_inline(app)
        app.refresh = lambda *_a: True
        app.loaded(None, 'Backend unavailable')
        self.assertTrue(app.notice.box.get_visible())
        self.assertTrue(app.notice.retry.get_visible())
        app.fixture = False
        try:
            with patch('connection.reconnect_existing', return_value={'map': True}) as reconnect:
                app.retry_connection()
        finally:
            app.fixture = True
        reconnect.assert_called_once()
        self.assertFalse(app.retrying)
        app.loaded((dict(map=True), list(app.threads.values()), [], '', app.inbox_request_epoch), None)
        self.assertFalse(app.notice.box.get_visible())
        self.assertEqual(app.connection.get_label(), 'iPhone connected · Messages ready')

    def test_failed_retry_keeps_the_retry_button_and_explains(self):
        from unittest.mock import patch
        app = self.app
        run_inline(app)
        app.refresh = lambda *_a: True
        app.loaded(None, 'Backend unavailable')
        app.fixture = False
        try:
            with patch('connection.reconnect_existing', side_effect=RuntimeError('The saved Bluetooth pairing is missing.')):
                app.retry_connection()
        finally:
            app.fixture = True
        self.assertIn('Could not reconnect', app.notice.text.get_label())
        self.assertIn('pairing is missing', app.notice.text.get_tooltip_text())
        self.assertTrue(app.notice.retry.get_visible() and app.notice.retry.get_sensitive())

    def test_shortcuts_are_registered_and_new_message_opens_the_sheet(self):
        app = self.app
        for name in ('new-message', 'search', 'refresh', 'back'):
            self.assertIsNotNone(app.lookup_action(name), name)
            self.assertTrue(app.get_accels_for_action('app.' + name), name)
        app.lookup_action('new-message').activate(None);settle(.3)
        sheets = [t for t in Gtk.Window.list_toplevels() if t is not app.window and t.get_title() == 'New Message']
        self.assertEqual(len(sheets), 1)
        sheets[0].close();settle()

    def test_new_message_sheet_lists_recents_and_enter_starts_a_typed_address(self):
        app = self.app
        app.new_message();settle(.3)
        sheet = next(t for t in Gtk.Window.list_toplevels() if t is not app.window and t.get_title() == 'New Message')
        rows = [w for w in walk(sheet) if isinstance(w, Gtk.ListBoxRow)]
        self.assertGreaterEqual(len(rows), 3, 'recent conversations appear before typing')
        self.assertNotIn('Family', [r.contact[0] for r in rows], 'groups are not offered as recipients')
        entry = next(w for w in walk(sheet) if isinstance(w, Gtk.SearchEntry))
        entry.set_text('+15550100199');settle(.3)
        entry.emit('activate');settle(.4)
        self.assertEqual(app.model.selected, 'new:+15550100199')
        self.assertFalse(sheet.is_visible())

    def test_new_message_sheet_keyboard_down_enters_results_and_escape_closes(self):
        app = self.app
        app.new_message();settle(.3)
        sheet = next(t for t in Gtk.Window.list_toplevels() if t is not app.window and t.get_title() == 'New Message')
        entry = next(w for w in walk(sheet) if isinstance(w, Gtk.SearchEntry))
        first = next(w for w in walk(sheet) if isinstance(w, Gtk.ListBoxRow))
        controllers = lambda widget: [c for c in widget.observe_controllers() if isinstance(c, Gtk.EventControllerKey)]
        self.assertTrue(any(c.emit('key-pressed', Gdk.KEY_Down, 0, 0) for c in controllers(entry)), 'Down is handled in the entry')
        self.assertTrue(first.is_selected())
        self.assertTrue(any(c.emit('key-pressed', Gdk.KEY_Escape, 0, 0) for c in controllers(sheet)), 'Escape is handled by the sheet')
        settle(.2)
        self.assertFalse(sheet.is_visible())

    def test_every_bubble_carries_its_full_time_as_a_tooltip(self):
        app = self.app
        app.threads['demo']['messages'] = [msg('hello', '2026-01-05T09:00:00')]
        app.rendered = None
        app.show_conversation();settle()
        self.assertIn('9:00 AM', with_class(app.messages, 'ms-bubble')[0].get_tooltip_text())

    def test_new_message_to_a_known_contact_reuses_their_conversation(self):
        app = self.app
        app.new_message();settle(.3)
        sheet = next(t for t in Gtk.Window.list_toplevels() if t is not app.window and t.get_title() == 'New Message')
        row = next(r for r in walk(sheet) if isinstance(r, Gtk.ListBoxRow) and getattr(r, 'contact', ('',))[0] == 'Jordan Lee')
        results = row.get_parent()
        results.emit('row-activated', row);settle(.4)
        self.assertEqual(app.model.selected, 'jordan')

    def test_non_square_photo_is_cropped_to_a_square_not_squashed(self):
        import gi
        gi.require_version('GdkPixbuf', '2.0')
        from gi.repository import GdkPixbuf
        wide = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, False, 8, 400, 100)
        wide.fill(0x3689e6ff)
        _ok, data = wide.save_to_bufferv('png', [], [])
        frame = photo_frame(bytes(data), 44)
        self.assertEqual(frame.measure(Gtk.Orientation.HORIZONTAL, -1)[1], 44)
        self.assertEqual(frame.measure(Gtk.Orientation.VERTICAL, -1)[1], 44)
        texture = frame.get_first_child().get_paintable()
        self.assertEqual((texture.get_intrinsic_width(), texture.get_intrinsic_height()), (88, 88))
        self.assertIsNone(photo_frame(b'', 44))
        self.assertIsNone(photo_frame(b'\x00' * 64, 44))


    # --- Group replies, failures, context menu, shortcuts -------------------

    def send_with(self, key, text, **client_methods):
        """Type text in a conversation and press Send against a fake client; return that client."""
        from unittest.mock import MagicMock
        app = self.app
        run_inline(app)
        app.refresh = lambda *_a: True
        app.list.select_row(app.rows[key]);settle(.2)
        app.editor.get_buffer().set_text(text)
        client = MagicMock()
        for name, value in client_methods.items():
            setattr(client, name, value)
        self.client = client
        app.fixture = False
        self.addCleanup(setattr, app, 'fixture', True)
        self._patch = patch('messages.BackendClient', return_value=client)
        self._patch.start()
        self.addCleanup(self._patch.stop)
        return client

    def test_unconfirmed_group_asks_who_will_receive_it_before_sending(self):
        from messages import Granite
        app = self.app
        client = self.send_with('family', 'Dinner is on me')
        app.send_message();settle(.3)
        dialog = app.group_dialog
        self.assertIsInstance(dialog, Granite.MessageDialog)
        client.send_to_thread.assert_not_called()
        shown = ' '.join(w.get_label() for w in walk(dialog) if isinstance(w, Gtk.Label))
        self.assertIn('+15550100126', shown)
        self.assertIn('+15550100127', shown)
        # The addresses must actually be readable, not ellipsized to nothing.
        for address in ('+15550100126', '+15550100127'):
            label = next(w for w in walk(dialog) if isinstance(w, Gtk.Label) and w.get_label() == address)
            self.assertGreaterEqual(label.get_width(), 80, address + ' must have room to show')
            self.assertFalse(label.get_layout().is_ellipsized(), address + ' is ellipsized')
        self.assertIn('Send to 2 People', shown)
        self.assertEqual(app.model.drafts['family'], 'Dinner is on me')
        dialog.emit('response', Gtk.ResponseType.CANCEL);settle(.2)
        client.send_to_thread.assert_not_called()
        self.assertIsNone(app.group_dialog)
        self.assertEqual(app.model.drafts['family'], 'Dinner is on me', 'cancel keeps the draft')

    def test_pressing_enter_in_the_group_dialog_cancels_rather_than_sends(self):
        app = self.app
        client = self.send_with('family', 'Careful')
        app.send_message();settle(.3)
        default = app.group_dialog.get_default_widget()
        self.assertIsNotNone(default)
        self.assertEqual(default.get_label(), 'Cancel', 'the default button must be the safe one')
        self.assertFalse(default.has_css_class('suggested-action'))
        app.group_dialog.emit('response', Gtk.ResponseType.CANCEL);settle(.2)
        client.send_to_thread.assert_not_called()

    def test_confirming_sends_once_with_the_token_and_is_not_asked_again(self):
        app = self.app
        client = self.send_with('family', 'First', send_to_thread=lambda *a, **k: 'transfer')
        client.send_to_thread = __import__('unittest.mock').mock.MagicMock(return_value='transfer')
        app.send_message();settle(.3)
        app.group_dialog.emit('response', Gtk.ResponseType.ACCEPT);settle(.4)
        client.send_to_thread.assert_called_once_with('family', 'First', confirm_group=True, expected_group_token='fixture-roster')
        self.assertTrue(app.threads['family']['group_confirmed'])
        self.assertEqual(app.send_status.get_text(), 'Sent through your iPhone')
        app.editor.get_buffer().set_text('Second')
        app.send_message();settle(.3)
        self.assertIsNone(app.group_dialog, 'already confirmed, so no second prompt')
        self.assertEqual(client.send_to_thread.call_args_list[-1].kwargs['confirm_group'], False)

    def test_group_that_changed_while_the_dialog_was_open_is_not_sent(self):
        app = self.app
        client = self.send_with('family', 'Hello')
        app.send_message();settle(.3)
        app.threads['family']['confirmation_token'] = 'roster-v2'
        app.group_dialog.emit('response', Gtk.ResponseType.ACCEPT);settle(.3)
        client.send_to_thread.assert_not_called()
        self.assertIn('group changed', app.send_status.get_text())
        self.assertEqual(app.model.drafts['family'], 'Hello')

    def test_group_with_an_incomplete_roster_points_to_advanced_blueferry(self):
        app = self.app
        client = self.send_with('family', 'Hello')
        app.threads['family']['participants_required'] = True
        app.send_message();settle(.2)
        self.assertIsNone(app.group_dialog)
        client.send_to_thread.assert_not_called()
        self.assertIn('Advanced BlueFerry', app.send_status.get_text())

    def test_failure_shows_the_daemons_reason_not_a_generic_line(self):
        app = self.app
        client = self.send_with('demo', 'Hello')
        client.send_to_thread.side_effect = Exception('OBEX connect failed: Forbidden')
        app.send_message();settle(.3)
        self.assertIn('Could not send', app.send_status.get_text())
        self.assertIn('isn’t allowing message access', app.send_status.get_text())
        self.assertEqual(app.model.drafts['demo'], 'Hello')

    def test_conversation_context_menu_offers_mark_read_and_copy(self):
        app = self.app
        unread = [text for text, _cb in app.row_actions(app.threads['jordan'])]
        read = [text for text, _cb in app.row_actions(app.threads['sam'])]
        self.assertEqual(unread, ['Mark as Read', 'Copy Number or Address'])
        self.assertEqual(read, ['Copy Number or Address'])
        self.assertEqual([t for t, _c in app.row_actions(app.threads['family'])], ['Copy Numbers'])
        row = app.rows['jordan']
        click = next(c for c in row.observe_controllers() if isinstance(c, Gtk.GestureClick))
        click.emit('pressed', 1, 12.0, 12.0);settle(.3)
        popovers = [w for w in walk(row) if isinstance(w, Gtk.Popover)]
        self.assertEqual(len(popovers), 1)
        self.assertEqual([b.get_label() for b in walk(popovers[0]) if isinstance(b, Gtk.Button)], unread)
        popovers[0].popdown();settle(.3)

    def test_copy_action_puts_the_number_on_the_clipboard(self):
        app = self.app
        copy = dict(app.row_actions(app.threads['sam']))['Copy Number or Address']
        copy();settle(.3)
        done = []
        app.window.get_display().get_clipboard().read_text_async(None, lambda cb, res: done.append(cb.read_text_finish(res)))
        end = time.monotonic() + 3
        while not done and time.monotonic() < end:
            settle(.1)
        self.assertEqual(done, ['+15550100128'])

    def test_shortcuts_window_lists_every_shortcut(self):
        from messages import Granite
        app = self.app
        sheet = app.show_shortcuts();settle(.3)
        accelerators = [w for w in walk(sheet) if isinstance(w, Granite.AccelLabel)]
        self.assertEqual(len(accelerators), 6)
        keycaps = [w.get_label() for w in walk(sheet) if isinstance(w, Gtk.Label) and w.get_label() in ('Ctrl', 'Enter', 'Return', 'F5', 'N', 'F', 'R', 'Left', 'Alt')]
        self.assertGreaterEqual(len(keycaps), 9, 'every row shows its keys: %s' % keycaps)
        for accelerator in accelerators:
            self.assertTrue(accelerator.get_first_child() is not None, 'an empty accelerator label draws nothing')
        text = ' '.join(w.get_label() for w in walk(sheet) if isinstance(w, Gtk.Label))
        for expected in ('New Message', 'Search Conversations', 'Refresh iPhone Inbox', 'Back to Conversations', 'Send Message', 'New Line'):
            self.assertIn(expected, text)
        sheet.close();settle(.2)


    # --- Enter to send (opt-in) --------------------------------------------

    def press(self, app, key, modifiers=Gdk.ModifierType(0)):
        """Deliver a key press to the editor's capture-phase controllers, as GTK would."""
        for controller in app.editor.observe_controllers():
            if isinstance(controller, Gtk.EventControllerKey) and controller.get_propagation_phase() == Gtk.PropagationPhase.CAPTURE:
                controller.emit('key-pressed', key, 0, modifiers)

    def enter_app(self, enabled=True):
        app = self.app
        app.settings['enter_sends'] = enabled
        sent = []
        app.send_message = lambda *_a: sent.append(app.editor.get_buffer().get_text(*app.editor.get_buffer().get_bounds(), True))
        app.editor.get_buffer().set_text('Hello there')
        settle(.2)
        return app, sent

    def draft(self, app):
        buffer = app.editor.get_buffer()
        return buffer.get_text(buffer.get_start_iter(), buffer.get_end_iter(), True)

    def test_enter_sends_when_the_switch_is_on_and_adds_no_newline(self):
        app, sent = self.enter_app()
        self.press(app, Gdk.KEY_Return)
        app.editor.get_buffer().insert_at_cursor('\n');settle(.2)
        self.assertEqual(sent, ['Hello there'], 'sent exactly once, without a trailing newline')
        self.assertEqual(self.draft(app), 'Hello there')

    def test_enter_still_adds_a_line_when_the_switch_is_off(self):
        app, sent = self.enter_app(enabled=False)
        self.press(app, Gdk.KEY_Return)
        app.editor.get_buffer().insert_at_cursor('\n');settle(.2)
        self.assertEqual(sent, [])
        self.assertEqual(self.draft(app), 'Hello there\n')

    def test_shift_enter_adds_a_line_instead_of_sending(self):
        app, sent = self.enter_app()
        self.press(app, Gdk.KEY_Return, Gdk.ModifierType.SHIFT_MASK)
        app.editor.get_buffer().insert_at_cursor('\n');settle(.2)
        self.assertEqual(sent, [])
        self.assertEqual(self.draft(app), 'Hello there\n')

    def test_enter_that_confirms_an_input_method_composition_never_sends(self):
        # An input method consumes Return to commit its candidate, so what arrives is
        # the composed text and not a newline. Sending here would send half a thought.
        app, sent = self.enter_app()
        self.press(app, Gdk.KEY_Return)
        app.editor.get_buffer().insert_at_cursor('あ');settle(.2)
        self.assertEqual(sent, [])
        self.assertEqual(self.draft(app), 'Hello thereあ')
        # And the armed state must not linger and turn a later pasted newline into a send.
        settle(.3)
        app.editor.get_buffer().insert_at_cursor('\n');settle(.2)
        self.assertEqual(sent, [])
        self.assertTrue(self.draft(app).endswith('\n'))

    def test_a_pasted_newline_without_a_return_press_is_just_text(self):
        app, sent = self.enter_app()
        app.editor.get_buffer().insert_at_cursor('\nsecond line');settle(.2)
        self.assertEqual(sent, [])
        self.assertEqual(self.draft(app), 'Hello there\nsecond line')

    def test_ctrl_enter_still_sends_in_both_modes(self):
        for enabled in (True, False):
            app, sent = self.enter_app(enabled)
            app.send.set_sensitive(True)
            app.key_pressed(None, Gdk.KEY_Return, 0, Gdk.ModifierType.CONTROL_MASK)
            self.assertEqual(len(sent), 1, 'enabled=%s' % enabled)

    def test_enter_on_an_empty_draft_does_nothing_at_all(self):
        app, sent = self.enter_app()
        app.editor.get_buffer().set_text('');settle(.2)
        self.press(app, Gdk.KEY_Return)
        app.editor.get_buffer().insert_at_cursor('\n');settle(.2)
        self.assertEqual(self.draft(app), '', 'the swallowed Return leaves no stray blank line')
        self.assertFalse(app.send.get_sensitive())
        self.assertEqual(sent, [], 'nothing to send, so nothing is sent')

    def test_menu_switch_persists_privately_and_updates_the_hint(self):
        import json, stat, tempfile
        from unittest.mock import patch
        folder = tempfile.mkdtemp()
        app = self.app
        app.fixture = False
        self.addCleanup(setattr, app, 'fixture', True)
        with patch.dict(os.environ, {'XDG_CONFIG_HOME': folder}):
            action = app.lookup_action('enter-sends')
            self.assertFalse(action.get_state().get_boolean())
            action.activate(None);settle(.2)
            self.assertTrue(action.get_state().get_boolean(), 'one activation toggles exactly once')
            path = os.path.join(folder, 'mseries', 'messages.json')
            self.assertEqual(json.load(open(path)), {'enter_sends': True})
            self.assertEqual(stat.S_IMODE(os.stat(path).st_mode), 0o600)
            app.editor.get_buffer().set_text('Draft');settle(.3)
            self.assertIn('Enter to send · Shift+Enter', app.send_status.get_text())
            self.assertIn('(Enter)', app.send.get_tooltip_text())
            action.activate(None);settle(.2)
            self.assertEqual(json.load(open(path)), {'enter_sends': False})
            self.assertIn('Ctrl+Enter to send', app.send_status.get_text())

    def test_fixture_never_touches_the_owners_preferences(self):
        import tempfile
        from unittest.mock import patch
        folder = tempfile.mkdtemp()
        with patch.dict(os.environ, {'XDG_CONFIG_HOME': folder}):
            self.app.lookup_action('enter-sends').activate(None);settle(.2)
        self.assertFalse(os.path.exists(os.path.join(folder, 'mseries')))

    @unittest.skipUnless(__import__('shutil').which('xdotool'), 'needs xdotool to send real key events')
    def test_real_key_events_end_to_end(self):
        import subprocess
        app, sent = self.enter_app()
        app.window.present();settle(.5)
        subprocess.run(['xdotool', 'search', '--name', 'Messages', 'windowfocus'], capture_output=True)
        app.editor.grab_focus();settle(.3)
        subprocess.run(['xdotool', 'key', 'Return'], check=True);settle(.6)
        self.assertEqual(sent, ['Hello there'], 'a real Return key press sends once with no newline')
        subprocess.run(['xdotool', 'key', 'shift+Return'], check=True);settle(.5)
        self.assertEqual(len(sent), 1)
        self.assertTrue(self.draft(app).endswith('\n'), 'a real Shift+Return adds a line')


    def test_theme_screenshots(self):
        destination = os.environ.get('MSERIES_MESSAGES_SCREENSHOTS')
        if not destination:
            self.skipTest('No screenshot output requested')
        from PIL import ImageGrab
        out = Path(destination);out.mkdir(parents=True, exist_ok=True)
        self.app.pin_state = dict(version=1, snapshot=self.pin_snapshot(), choices=['demo2', 'demo'])
        self.app.update_pin_note();self.app.populate()
        for dark in (False, True):
            Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme = dark
            self.app.window.set_default_size(1080, 720)
            settle(.4)
            ImageGrab.grab(bbox=(0, 0, self.app.window.get_width(), self.app.window.get_height()), xdisplay=os.environ['DISPLAY']).save(out/('messages-dark.png' if dark else 'messages-light.png'))
            dialog = self.app.show_pin_review(self.pin_snapshot());settle(.3)
            ImageGrab.grab(bbox=(0, 0, dialog.get_width(), dialog.get_height()), xdisplay=os.environ['DISPLAY']).save(out/('pin-import-dark.png' if dark else 'pin-import-light.png'))
            dialog.close();settle(.1)
        self.app.window.set_default_size(480, 680)
        settle(.4)
        ImageGrab.grab(bbox=(0, 0, self.app.window.get_width(), self.app.window.get_height()), xdisplay=os.environ['DISPLAY']).save(out/'messages-narrow.png')


@unittest.skipUnless(os.environ.get('DISPLAY'), 'Requires isolated X display')
class LiveDataPath(unittest.TestCase):
    """The real refresh/merge/read path, fed realistic BlueFerry objects by a fake client.

    Fixture mode skips refresh(), merge_inbox() and mark-as-read entirely, so
    this is the only place those are exercised together with the new views.
    """

    def setUp(self):
        from unittest.mock import patch
        from blueferry.models import BackendStatus, Thread

        class FakeClient:
            read = []
            fail = False

            def status(self):
                if FakeClient.fail:
                    raise RuntimeError('daemon unavailable')
                return BackendStatus(daemon=True, map=True, contacts=2)

            extra = False

            def threads(self):
                messages = [dict(handle='h0', body='Earlier', timestamp='2026-10-19T10:00:00', outgoing=True, read=True),
                            dict(handle='h1', body='Read this https://example.com/doc', timestamp='2026-10-20T10:00:00', outgoing=False, read=False)]
                if FakeClient.extra:
                    messages.append(dict(handle='h2', body='One more thing', timestamp='2026-10-20T10:01:00', outgoing=False, read=False))
                return [Thread.from_dict(dict(key='t1', name='Pat Doe', is_group=False, recipients=['+15550100150'],
                                              reply_ready=True, last_ts='2026-10-20T10:00:00', messages=messages))]

            def recent(self, folder, limit):
                return [dict(handle='inbox1', sender='+15550100151', contact_name='Casey Phone', body='Only on the phone',
                             timestamp='2026-10-20T09:00:00', read=False)]

            def contact_photo(self, address):
                return b''

            def mark_thread_read(self, key):
                FakeClient.read.append(key)
                return 1

        FakeClient.read, FakeClient.fail, FakeClient.extra = [], False, False
        self.client = FakeClient
        self.patches = [patch('messages.BackendClient', FakeClient), patch('messages.get_system_bus', side_effect=RuntimeError('no bus'))]
        for item in self.patches:
            item.start()
        self.app = Messages(fixture=False)
        self.app.set_application_id('io.elementary.mseries.Messages.Test' + self._testMethodName.replace('_', ''))
        self.app.register()
        # Keep the private pin keyring out of the picture entirely.
        self.app.load_pins = lambda: None
        self.app.activate()
        self.wait(lambda: 'inbox:+15550100151' in self.app.threads)

    def tearDown(self):
        self.app.window.close()
        self.app.quit()
        for item in self.patches:
            item.stop()
        settle(.05)

    def wait(self, condition, seconds=4):
        end = time.monotonic() + seconds
        while time.monotonic() < end and not condition():
            settle(.05)
        self.assertTrue(condition(), 'timed out waiting for the backend snapshot')

    def test_backend_and_phone_inbox_merge_into_native_rows(self):
        app = self.app
        self.assertEqual(app.connection.get_label(), 'iPhone connected · Messages ready')
        self.assertEqual(set(app.rows), {'t1', 'inbox:+15550100151'})
        self.assertTrue(with_class(app.rows['t1'], 'ms-unread')[0].get_visible(), 'unread comes from message read flags')
        self.assertEqual(with_class(app.rows['inbox:+15550100151'], 'ms-contact-name')[0].get_label(), 'Casey Phone')
        self.assertFalse(app.empty_list.get_visible())

    def test_opening_an_unread_thread_marks_it_read_and_renders_its_link(self):
        app = self.app
        app.list.select_row(app.rows['t1']);settle(.3)
        self.wait(lambda: self.client.read == ['t1'])
        bubbles = with_class(app.messages, 'ms-bubble')
        self.assertEqual([b.get_text() for b in bubbles], ['Earlier', 'Read this https://example.com/doc'])
        self.assertIn('<a href="https://example.com/doc">', bubbles[1].get_label())
        self.assertTrue(bubbles[0].has_css_class('ms-outgoing') and bubbles[1].has_css_class('ms-incoming'))

    def test_message_arriving_in_an_open_focused_conversation_is_marked_read_once(self):
        app = self.app
        app.window.is_active = lambda: True
        app.list.select_row(app.rows['t1']);settle(.3)
        self.wait(lambda: self.client.read == ['t1'])
        self.client.extra = True
        app.refresh(True)
        self.wait(lambda: self.client.read == ['t1', 't1'])
        app.refresh(True);settle(.6)
        self.assertEqual(self.client.read, ['t1', 't1'], 'the same newest message is not marked twice')

    def test_unfocused_window_does_not_mark_arrivals_read(self):
        app = self.app
        app.window.is_active = lambda: False
        app.list.select_row(app.rows['t1']);settle(.3)
        self.wait(lambda: self.client.read == ['t1'])
        self.client.extra = True
        app.refresh(True);settle(.8)
        self.assertEqual(self.client.read, ['t1'], 'only the explicit open counts while unfocused')

    def test_phone_only_conversation_is_readable_but_never_marked_read(self):
        app = self.app
        app.list.select_row(app.rows['inbox:+15550100151']);settle(.4)
        self.assertEqual(self.client.read, [])
        self.assertEqual([b.get_text() for b in with_class(app.messages, 'ms-bubble')], ['Only on the phone'])

    def test_daemon_failure_keeps_conversations_and_offers_retry(self):
        app = self.app
        self.client.fail = True
        app.refresh(True)
        self.wait(lambda: app.connection_state == 'offline')
        self.assertEqual(set(app.rows), {'t1', 'inbox:+15550100151'}, 'last known conversations stay on screen')
        self.assertTrue(app.notice.retry.get_visible())
        self.assertFalse(app.send.get_sensitive())
        self.client.fail = False
        app.refresh(True)
        self.wait(lambda: app.connection_state == 'ready')
        self.assertFalse(app.notice.box.get_visible())


if __name__ == '__main__':
    unittest.main()
