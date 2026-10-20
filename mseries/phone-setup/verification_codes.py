#!/usr/bin/python3
"""Near-pointer verification-code suggestions and explicit accessible-field fill."""
from concurrent.futures import ThreadPoolExecutor
import gettext
import json
import os
import subprocess
import sys
from pathlib import Path

# This small pointer-relative popup uses XWayland positioning; Messages remains
# native Wayland. GTK's Wayland toplevel API cannot choose screen coordinates.
os.environ.setdefault('GDK_BACKEND', 'x11')
os.environ.setdefault('GSK_RENDERER', 'cairo')
import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
gi.require_version('GdkX11', '4.0')
gi.require_version('Granite', '7.0')
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi, Gdk, GdkX11, Gio, GLib, Granite, Gtk
from blueferry.client import BackendClient
from code_offers import Offers
from code_fields import describe, insert

_ = gettext.gettext


class Codes(Gtk.Application):
    def __init__(self, fixture=False):
        super().__init__(application_id='io.elementary.mseries.VerificationCodes', flags=Gio.ApplicationFlags.HANDLES_COMMAND_LINE)
        self.fixture = fixture
        self.registry = Offers()
        self.pool = ThreadPoolExecutor(max_workers=1)
        self.initialized = False
        self.busy = False
        self.last_events = None
        self.target = None
        self.last_field = None
        self.shown = None
        self.selected_offer = None
        self.manual = False
        self.filling = False
        self.shortcut = _('Open Verification Codes from Applications')
        try:
            self.shortcut = json.loads((Path(__file__).parent/'verification-shortcut.json').read_text())['label']
        except (OSError, ValueError, KeyError):
            pass
        self.add_main_option('background', 0, GLib.OptionFlags.NONE, GLib.OptionArg.NONE, 'Watch for verification fields', None)
        self.add_main_option('show', 0, GLib.OptionFlags.NONE, GLib.OptionArg.NONE, 'Show available verification codes', None)

    def do_command_line(self, command):
        self.initialize()
        if not command.get_options_dict().contains('background'):
            self.manual = True
            if self.last_field:
                self.target = describe(self.last_field, allow_plain=True)
            self.render(True, activate=True)
            self.poll(force=True)
        return 0

    def initialize(self):
        if self.initialized:
            return
        self.initialized = True
        self.hold()
        if hasattr(Granite, 'init'):
            Granite.init()
        preferences = Granite.Settings.get_default()
        def scheme(*_args):
            Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme = preferences.get_prefers_color_scheme() == Granite.SettingsColorScheme.DARK
        preferences.connect('notify::prefers-color-scheme', scheme)
        scheme()
        self.window = Gtk.ApplicationWindow(application=self, title=_('Verification Code'), decorated=False, resizable=False, default_width=360)
        self.window.connect('close-request', self.dismiss)
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, margin_top=18, margin_bottom=18, margin_start=20, margin_end=20)
        heading = Gtk.Box(spacing=10)
        heading.append(Gtk.Image.new_from_icon_name('phone-symbolic'))
        title = Gtk.Label(label=_('Use Code from iPhone'), xalign=0, hexpand=True)
        title.add_css_class('heading');heading.append(title)
        close = Gtk.Button(icon_name='window-close-symbolic', tooltip_text=_('Dismiss (Esc)'))
        close.add_css_class('flat');close.connect('clicked', self.dismiss);heading.append(close)
        card.append(heading)
        self.source = Gtk.Label(xalign=0, wrap=True, max_width_chars=42)
        self.source.add_css_class('dim-label');card.append(self.source)
        self.code = Gtk.Label(selectable=True, xalign=0)
        self.code.add_css_class('title-1');card.append(self.code)
        self.destination = Gtk.Label(xalign=0, wrap=True, max_width_chars=42)
        card.append(self.destination)
        self.error = Gtk.Label(xalign=0, wrap=True, visible=False)
        self.error.add_css_class('error');card.append(self.error)
        actions = Gtk.Box(spacing=8)
        self.copy = Gtk.Button(label=_('Copy Code'))
        self.copy.connect('clicked', self.copy_code);actions.append(self.copy)
        self.use = Gtk.Button(label=_('Use Code'), hexpand=True)
        self.use.add_css_class('suggested-action')
        self.use.connect('clicked', self.use_code);actions.append(self.use)
        card.append(actions)
        self.hint = Gtk.Label(label=_('Enter to insert · Esc to dismiss'), xalign=0, wrap=True, max_width_chars=42)
        self.hint.add_css_class('dim-label');card.append(self.hint)
        self.window.set_child(card)
        self.window.set_default_widget(self.use)
        keys = Gtk.EventControllerKey();keys.connect('key-pressed', self.key_pressed);self.window.add_controller(keys)
        Atspi.set_timeout(200, 500)
        if not self.fixture:
            self.listener = Atspi.EventListener.new(self.focus_changed)
            self.listener.register('object:state-changed:focused')
            GLib.timeout_add_seconds(4, self.poll)
            self.poll(force=True)

    def task(self, work, completed):
        future = self.pool.submit(work)
        def done(result):
            try: value, error = result.result(), None
            except Exception: value, error = None, True
            GLib.idle_add(lambda: completed(value, error) or False)
        future.add_done_callback(done)

    def poll(self, force=False):
        if self.fixture or self.busy:
            return True
        self.busy = True
        previous = self.last_events
        def load():
            client = BackendClient();status = client.status()
            if not status.map:
                return None, None
            return status.events, client.recent('telecom/msg/INBOX', 20) if force or status.events != previous else None
        def loaded(value, error):
            self.busy = False
            if not error:
                self.last_events, messages = value
                if messages is not None:
                    self.registry.update(messages)
            self.render(self.manual)
        self.task(load, loaded)
        return True

    def focus_changed(self, event, *_args):
        if not event.detail1:
            return
        try:
            if event.source.get_application().get_process_id() == os.getpid():
                return
        except Exception:
            self.target = self.last_field = None
            self.window.set_visible(False)
            return
        self.last_field = event.source
        self.target = describe(event.source)
        self.manual = False
        self.render(False)

    def render(self, manual=False, activate=False):
        origin = self.target['origin'] if self.target else None
        offers = self.registry.available(origin)
        self.selected_offer = offers[0] if offers else None
        if not offers:
            self.code.set_label('')
            self.source.set_label(_('No recent verification code.'))
            self.destination.set_label(_('Request a fresh code on the sign-in screen, then try again.'))
            self.use.set_sensitive(False);self.copy.set_sensitive(False)
            self.hint.set_label(_('%s · Esc to dismiss') % self.shortcut)
            if manual:
                if activate or not self.window.get_visible(): self.present_popup()
            else:
                self.window.set_visible(False)
            return
        offer = offers[0]
        self.source.set_label(_('From %s · recent text') % offer.sender)
        self.code.set_label(offer.code)
        can_insert = bool(self.target and self.target.get('can_insert'))
        if can_insert:
            destination = _('Insert into “%s” in %s') % (self.target['name'], self.target['application'])
        elif self.target:
            destination = _('Copy, then paste into “%s” in %s.') % (self.target['name'], self.target['application'])
        else:
            destination = _('Focus a verification-code field and use the shortcut, or copy the code.')
        self.destination.set_label(destination)
        self.use.set_visible(can_insert)
        self.use.set_sensitive(can_insert and not self.filling)
        self.copy.set_hexpand(not can_insert)
        if can_insert: self.copy.remove_css_class('suggested-action')
        else: self.copy.add_css_class('suggested-action')
        self.copy.set_sensitive(True)
        self.window.set_default_widget(self.use if can_insert else self.copy)
        self.hint.set_label((_('%s → Enter to insert · Esc to dismiss') if can_insert else _('%s → Enter to copy · Esc to dismiss')) % self.shortcut)
        if self.target:
            identity = (self.target['pid'], self.target['path'], offer.id)
            if activate or (manual and not self.window.get_visible()) or identity != self.shown:
                self.shown = identity
                self.present_popup()
        elif activate or (manual and not self.window.get_visible()):
            self.present_popup()
        elif not manual:
            self.window.set_visible(False)

    def present_popup(self):
        self.error.set_visible(False)
        self.window.realize()
        surface = self.window.get_surface()
        if isinstance(surface, GdkX11.X11Surface):
            surface.set_skip_taskbar_hint(True);surface.set_skip_pager_hint(True)
            if not self.manual:
                surface.set_user_time(0)
        self.window.present()
        if self.manual:
            self.window.set_focus(self.use if self.target and self.target.get('can_insert') else self.copy)
        if isinstance(surface, GdkX11.X11Surface) and not self.fixture:
            xid = surface.get_xid()
            def location():
                result = subprocess.run(['xdotool', 'getmouselocation', '--shell'], check=True, capture_output=True, text=True, timeout=2)
                values = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
                geometry = subprocess.run(['xdotool', 'getdisplaygeometry'], check=True, capture_output=True, text=True, timeout=2).stdout.split()
                width, height = map(int, geometry)
                return max(0, min(int(values['X']) + 14, width - 390)), max(0, min(int(values['Y']) + 18, height - 290))
            def move(point, error):
                if not error:
                    subprocess.Popen(['xdotool', 'windowmove', str(xid), str(point[0]), str(point[1])], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    if self.manual:
                        subprocess.Popen(['xdotool', 'windowactivate', str(xid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.task(location, move)

    def use_code(self, *_args):
        if self.filling or not self.target or not self.target.get('can_insert') or not self.selected_offer:
            return
        target, identity = self.target, self.selected_offer.id
        offer = self.registry.get(identity, target['origin'])
        if not offer:
            self.render(self.manual);return
        self.use.set_sensitive(False)
        self.filling = True
        def work():
            # Revalidate the target immediately before using its editable API.
            # No keyboard injection, clipboard write or form submission occurs.
            if self.fixture:
                raise RuntimeError('Preview cannot fill real fields')
            if not self.target or self.target['source'] != target['source']:
                raise RuntimeError('The focused field changed')
            return insert(target, offer.code)
        def filled(value, error):
            self.filling = False
            if error:
                self.error.set_label(_('This app did not accept insertion. Copy Code is still available.'))
                self.error.set_visible(True)
                self.use.set_sensitive(bool(self.target))
            else:
                self.registry.consume(identity)
                self.window.set_visible(False)
                self.manual = False
        self.task(work, filled)

    def copy_code(self, *_args):
        if not self.selected_offer:
            return
        offer = self.registry.get(self.selected_offer.id, self.target['origin'] if self.target else None)
        if offer:
            self.window.get_clipboard().set(offer.code)
            self.dismiss()
        else:
            self.render(self.manual)

    def dismiss(self, *_args):
        self.window.set_visible(False)
        self.manual = False
        if self.target:
            try: self.target['source'].get_component_iface().grab_focus()
            except Exception: pass
        return True

    def key_pressed(self, _controller, key, _keycode, _mods):
        if key == Gdk.KEY_Escape:
            self.dismiss();return True
        return False


if __name__ == '__main__':
    Codes().run(sys.argv)
