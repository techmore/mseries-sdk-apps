"""Run in dbus-run-session + Xvfb: synthetic entries only, no owner accounts."""
import os
import subprocess
import sys
import time
import unittest
from datetime import datetime, timezone
import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi, Gio, GLib, Gtk


def target_app():
    app = Gtk.Application(application_id='io.elementary.mseries.CodeFieldFixture')
    def activate(app):
        window = Gtk.ApplicationWindow(application=app, title='Code field fixture')
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        entry = Gtk.Entry()
        entry.update_property([Gtk.AccessibleProperty.LABEL], ['Verification code'])
        box.append(entry)
        password = Gtk.Entry(visibility=False)
        password.update_property([Gtk.AccessibleProperty.LABEL], ['Password'])
        box.append(password)
        submits = Gtk.Label(label='Submitted 0')
        def submitted(*_): submits.set_label('Submitted 1')
        entry.connect('activate', submitted)
        box.append(submits)
        window.set_child(box);window.present();entry.grab_focus()
    app.connect('activate', activate);app.run([])


def settle(seconds=.2):
    context = GLib.MainContext.default()
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        while context.pending(): context.iteration(False)
        time.sleep(.01)


def find(node, name, depth=0, predicate=None):
    if depth > 12:
        return None
    try:
        node.clear_cache()
        if node.get_name() == name and (predicate is None or predicate(node)): return node
        for index in range(min(node.get_child_count(), 100)):
            if result := find(node.get_child_at_index(index), name, depth+1, predicate): return result
    except Exception:
        return None


@unittest.skipUnless(os.environ.get('MSERIES_PRIVATE_CODE_TEST') == '1', 'Requires isolated D-Bus and Xvfb')
class FieldTests(unittest.TestCase):
    def setUp(self):
        from verification_codes import Codes
        self.process = subprocess.Popen([sys.executable, __file__, '--target'], stdout=subprocess.DEVNULL)
        self.app = Codes(fixture=True)
        self.app.set_application_id('io.elementary.mseries.Codes.Test' + self._testMethodName.replace('_', ''))
        self.app.register();self.app.initialize()
        self.desktop = Atspi.get_desktop(0)
        self.entry = None
        for _ in range(30):
            settle(.1)
            self.entry = find(self.desktop, 'Verification code', predicate=lambda node: node.get_application().get_process_id() == self.process.pid and node.get_role() in (Atspi.Role.ENTRY, Atspi.Role.TEXT))
            if self.entry: break
        self.assertIsNotNone(self.entry)

    def tearDown(self):
        self.app.window.destroy();self.app.pool.shutdown(wait=True, cancel_futures=True)
        self.app.quit();self.process.terminate();self.process.wait(timeout=5);settle(.05)

    def test_native_field_fill_uses_editable_api_without_submit(self):
        from code_fields import describe
        target = describe(self.entry)
        self.assertIsNotNone(target)
        self.app.target = target
        stamp = datetime.now(timezone.utc).isoformat()
        self.app.registry.update([dict(handle='fixture', timestamp=stamp, sender='Fixture', body='Your verification code is 246810.')])
        self.app.manual = True;self.app.render(True)
        # Only this test's isolated bus exposes the synthetic target. Normal
        # preview mode refuses all insertion, including if run on the real bus.
        self.app.fixture = False
        # Exercise Return in the popup, not just the button signal. Key events
        # are injected only into this isolated fixture display, never at runtime.
        xid=self.app.window.get_surface().get_xid()
        subprocess.run(['xdotool','windowfocus',str(xid),'key','Return'],check=True,timeout=3)
        for _ in range(30):
            settle(.1)
            text = Atspi.Text.get_text(self.entry, 0, -1)
            if text: break
        self.assertEqual(text, '246810')
        self.assertIsNotNone(find(self.desktop, 'Submitted 0'))
        self.assertEqual(self.app.registry.available(), [])

    def test_password_and_changed_field_rejected(self):
        from code_fields import describe
        password = find(self.desktop, 'Password')
        self.assertIsNotNone(password)
        self.assertIsNone(describe(password))
        self.app.target = describe(self.entry)
        event = type('Focus', (), dict(source=password, detail1=1))()
        self.app.focus_changed(event)
        self.assertIsNone(self.app.target)

    def test_expired_offer_does_not_touch_field(self):
        from code_fields import describe
        self.app.target = describe(self.entry)
        stamp = datetime.now(timezone.utc)
        self.app.registry.update([dict(handle='fixture', timestamp=stamp.isoformat(), body='Your verification code is 246810.')])
        self.app.render(True)
        self.app.registry.now = lambda: stamp.timestamp() + 301
        self.app.use_code();settle()
        self.assertEqual(Atspi.Text.get_text(self.entry, 0, -1), '')

    def test_manual_empty_state_survives_poll_and_keyboard_dismisses(self):
        from gi.repository import Gdk
        self.app.manual=True;self.app.render(True);settle(.1)
        self.assertTrue(self.app.window.get_visible())
        self.app.render(True);settle(.1)
        self.assertTrue(self.app.window.get_visible())
        self.assertFalse(self.app.use.get_sensitive())
        self.assertTrue(self.app.key_pressed(None, Gdk.KEY_Escape, 0, 0))
        self.assertFalse(self.app.window.get_visible())

    def test_fixture_theme_screenshots(self):
        destination = os.environ.get('MSERIES_CODE_SCREENSHOTS')
        if not destination: self.skipTest('No screenshot output requested')
        from pathlib import Path
        from PIL import ImageGrab
        from code_fields import describe
        out=Path(destination);out.mkdir(parents=True,exist_ok=True)
        self.app.target=describe(self.entry)
        self.app.registry.update([dict(handle='fixture', timestamp=datetime.now(timezone.utc).isoformat(), sender='Example Service', body='Your verification code is 246810.')])
        self.app.shortcut='Shift+Super+V'
        for dark in (False,True):
            Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme=dark
            self.app.manual=True;self.app.render(True);settle(.3)
            ImageGrab.grab(bbox=(0,0,self.app.window.get_width(),self.app.window.get_height()),xdisplay=os.environ['DISPLAY']).save(out/('codes-dark.png' if dark else 'codes-light.png'))
        self.app.target['can_insert']=False
        self.app.render(True);settle(.3)
        ImageGrab.grab(bbox=(0,0,self.app.window.get_width(),self.app.window.get_height()),xdisplay=os.environ['DISPLAY']).save(out/'codes-copy.png')


if __name__ == '__main__':
    if '--target' in sys.argv: target_app()
    else: unittest.main()
