"""Isolated GTK tests. Run with GDK_BACKEND=x11 xvfb-run -a python3 ... ."""
import os
import time
import unittest
from pathlib import Path
from messages import Messages, Gtk, GLib, Gdk


def settle(seconds=.25):
    context = GLib.MainContext.default()
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        while context.pending():
            context.iteration(False)
        time.sleep(.01)


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


if __name__ == '__main__':
    unittest.main()
