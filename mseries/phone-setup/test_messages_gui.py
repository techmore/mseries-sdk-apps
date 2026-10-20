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

    def test_theme_screenshots(self):
        destination = os.environ.get('MSERIES_MESSAGES_SCREENSHOTS')
        if not destination:
            self.skipTest('No screenshot output requested')
        from PIL import ImageGrab
        out = Path(destination);out.mkdir(parents=True, exist_ok=True)
        for dark in (False, True):
            Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme = dark
            self.app.window.set_default_size(1080, 720)
            settle(.4)
            ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save(out/('messages-dark.png' if dark else 'messages-light.png'))
        self.app.window.set_default_size(480, 680)
        settle(.4)
        ImageGrab.grab(xdisplay=os.environ['DISPLAY']).save(out/'messages-narrow.png')


if __name__ == '__main__':
    unittest.main()
