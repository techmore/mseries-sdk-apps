#!/usr/bin/python3
"""Native elementary phone setup over BlueFerry's existing pairing/backend API."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import threading
import time

os.environ.setdefault("GSK_RENDERER", "cairo")

import gi
gi.require_version('Gtk', '4.0')
gi.require_version('Granite', '7.0')
from gi.repository import Gio, GLib, Granite, Gtk
import dbus
from setup_core import connection_outcome, needs_ios27_audio_recovery
from phone_status import collect_status, record_recovery

APP_ID = 'io.elementary.mseries.PhoneSetup'


class PhoneSetup(Gtk.Application):
    def __init__(self, fixture=None, ios_version=""):
        super().__init__(application_id=APP_ID + ('.Preview' if fixture else ''), flags=Gio.ApplicationFlags.NON_UNIQUE if fixture else Gio.ApplicationFlags.DEFAULT_FLAGS)
        self.fixture = fixture
        self.worker_busy = False
        self.generation = 0
        self.pair_process = None
        self.cancel_pair = threading.Event()
        self.started = time.monotonic()
        self.audio_recovery_attempted = False
        self.poll_id = None
        self.devices = []
        self.state = 'welcome'
        self.last_status = {}
        self.ios_version = ios_version

    def do_activate(self):
        if self.get_active_window():
            self.get_active_window().present()
            return
        if hasattr(Granite, 'init'):
            Granite.init()
        preferences = Granite.Settings.get_default()
        def update_scheme(*_):
            Gtk.Settings.get_default().props.gtk_application_prefer_dark_theme = preferences.get_prefers_color_scheme() == Granite.SettingsColorScheme.DARK
        preferences.connect('notify::prefers-color-scheme', update_scheme)
        update_scheme()
        self.window = Gtk.ApplicationWindow(application=self, title='Connect iPhone', default_width=600, default_height=600)
        self.window.connect('close-request', self.close_requested)
        outer = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        header = Gtk.HeaderBar(show_title_buttons=True)
        outer.append(header)
        scroll = Gtk.ScrolledWindow(vexpand=True, hscrollbar_policy=Gtk.PolicyType.NEVER)
        body = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=18, margin_start=28, margin_end=28, margin_top=24, margin_bottom=24)
        scroll.set_child(body)
        outer.append(scroll)
        icon = Gtk.Image.new_from_icon_name('phone-symbolic')
        icon.set_pixel_size(48)
        body.append(icon)
        self.title = Gtk.Label(label='Connect your iPhone', wrap=True, xalign=0)
        self.title.add_css_class('title-1')
        body.append(self.title)
        self.description = Gtk.Label(wrap=True, xalign=0)
        body.append(self.description)
        self.ios_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12)
        self.ios_row.append(Gtk.Label(label='iOS version', xalign=0))
        self.ios = Gtk.Entry(placeholder_text='For example, 27.0.1', hexpand=True)
        self.ios.set_text(self.ios_version)
        self.ios_row.append(self.ios)
        body.append(self.ios_row)
        self.phones = Gtk.DropDown.new_from_strings([])
        self.phones.set_visible(False)
        body.append(self.phones)
        self.spinner = Gtk.Spinner(halign=Gtk.Align.CENTER)
        body.append(self.spinner)
        self.code = Gtk.Label(wrap=True)
        self.code.add_css_class('title-1')
        body.append(self.code)
        self.details = Gtk.Label(wrap=True, selectable=True, xalign=0)
        self.ios_help = Gtk.LinkButton.new_with_label('https://github.com/erikwb/blueferry/issues/263', 'iOS 27 connection report')
        self.ios_help.set_visible(False)
        body.append(self.ios_help)
        self.expander = Gtk.Expander(label='Connection details', child=self.details)
        body.append(self.expander)
        actions = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=12, halign=Gtk.Align.END)
        self.secondary = Gtk.Button(label='Check again', visible=False)
        self.secondary.connect('clicked', self.secondary_action)
        actions.append(self.secondary)
        self.next = Gtk.Button(label='Next')
        self.next.add_css_class('suggested-action')
        self.next.connect('clicked', self.advance)
        actions.append(self.next)
        body.append(actions)
        self.window.set_child(outer)
        self.show('welcome', 'Connect your iPhone', 'Read and reply to messages from this desktop. Setup checks Bluetooth, pairs your phone, and verifies message access. Your iPhone stays nearby; no relay Mac is needed.', 'Next')
        self.ios_row.set_visible(False)
        self.window.present()
        if not self.fixture:
            from blueferry import config as blueferry_config
            if blueferry_config.is_valid_mac(blueferry_config.read_local_env().get('BLUEFERRY_MAC', '')):
                GLib.idle_add(self.start_status)
        if self.fixture:
            if self.fixture == 'confirm':
                self.show('confirm', '1. Confirm on this computer', 'Compare the codes on both devices. If they match, choose Codes match here first. Then tap Pair on your iPhone.', 'Codes match')
                self.code.set_label('123456')
            else:
                status = {'map': self.fixture == 'ready', 'connectivity_state': 'authorization-required'}
                self.present_status(status)

    def close_requested(self, *_):
        self.generation += 1
        if self.poll_id:
            GLib.source_remove(self.poll_id)
            self.poll_id = None
        self.cancel_pair.set()
        if self.pair_process and self.pair_process.poll() is None:
            self.pair_process.terminate()
        return False

    def show(self, state, title, detail, action=None):
        self.state = state
        self.title.set_label(title)
        self.description.set_label(detail)
        self.next.set_label(action or 'Next')
        self.next.set_visible(action is not None)
        self.next.set_sensitive(not self.worker_busy)
        self.ios_row.set_visible(False)
        self.phones.set_visible(state == 'phone')
        self.code.set_visible(state == 'confirm')
        self.secondary.set_visible(state in ('permissions', 'blocked', 'error', 'confirm', 'pairing'))
        self.secondary.set_label('Cancel' if state in ('confirm', 'pairing') else 'Refresh status')
        self.ios_help.set_visible(self.ios_version.startswith('27.') and state in ('permissions', 'blocked'))

    def secondary_action(self, *_):
        if self.state in ('confirm', 'pairing'):
            self.generation += 1
            self.cancel_pair.set()
            self.answer_pair('no')
            if self.pair_process and self.pair_process.poll() is None:
                self.pair_process.terminate()
            self.show('error', 'Pairing cancelled', 'You can try again when your iPhone is ready.', 'Try again')
        else:
            self.start_status()

    def start_status(self):
        self.task(self.status, self.present_status)
        self.begin_polling()
        return False

    def task(self, operation, completed):
        if self.worker_busy or self.fixture:
            return
        self.worker_busy = True
        generation = self.generation
        self.spinner.start()
        self.next.set_sensitive(False)
        self.secondary.set_sensitive(False)
        def deliver(value, error):
            if generation != self.generation:
                return False
            self.worker_busy = False
            self.spinner.stop()
            self.next.set_sensitive(True)
            self.secondary.set_sensitive(True)
            if error:
                self.show('error', 'Setup needs attention', str(error), 'Try again')
            else:
                completed(value)
            return False
        def work():
            try:
                value, error = operation(), None
            except Exception as exc:
                value, error = None, str(exc)
            GLib.idle_add(deliver, value, error)
        threading.Thread(target=work, daemon=True).start()

    @staticmethod
    def command(*args, timeout=30):
        result = subprocess.run(['blueferry', *args], capture_output=True, text=True, timeout=timeout)
        try:
            data = json.loads(result.stdout)
        except ValueError:
            raise RuntimeError('The phone service did not return a valid response. Open BlueFerry for its detailed diagnostics.')
        if result.returncode or isinstance(data, dict) and data.get('error'):
            raise RuntimeError(data.get('error', 'The Bluetooth request failed.') if isinstance(data, dict) else 'The Bluetooth request failed.')
        return data

    @staticmethod
    def status():
        return collect_status()

    def advance(self, *_):
        if self.fixture:
            return
        if self.state in ('ready', 'blocked'):
            subprocess.Popen(['blueferry-gtk'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.quit()
        elif self.state == 'confirm':
            self.answer_pair('yes')
            self.show('pairing', '2. Confirm on your iPhone', 'Tap Pair on your iPhone. Then allow contacts and messages if the phone asks. Setup will check the connection automatically.')
        elif self.state == 'phone' and not self.devices:
            self.check_now()
        elif self.state == 'phone':
            selected = self.phones.get_selected()
            if selected < len(self.devices):
                self.pair(self.devices[selected])
        elif self.state == 'recovery':
            self.recover_audio()
        else:
            self.ios_version = self.ios.get_text().strip() or self.ios_version
            self.check_now()

    def check_now(self):
        if self.worker_busy:
            return
        self.show('checking', 'Checking your connection', 'Checking Bluetooth and the phone service…')
        def inspect():
            capability = self.command('pairing-compatibility-json')
            configuration = self.command('pairing-configuration-json', '--saved-only')
            if not capability.get('hardware_supported'):
                raise RuntimeError(capability.get('issue', 'A Bluetooth adapter is required.'))
            if not capability.get('powered'):
                bus = dbus.SystemBus()
                obj = bus.get_object('org.bluez', '/org/bluez/' + capability.get('adapter', 'hci0'))
                dbus.Interface(obj, 'org.freedesktop.DBus.Properties').Set('org.bluez.Adapter1', 'Powered', dbus.Boolean(True))
            if configuration.get('configured'):
                return 'status', self.status()
            return 'phones', self.command('pairing-devices-json', '--scan-seconds', '8', timeout=25)
        def done(value):
            kind, data = value
            if kind == 'status':
                self.present_status(data)
                self.begin_polling()
            else:
                self.devices = data
                self.phones.set_model(Gtk.StringList.new([d.get('alias') or d.get('name') or 'iPhone' for d in data]))
                self.show('phone', 'Choose your iPhone', 'Keep Settings → Bluetooth open on the iPhone, then choose Connect.' if data else 'No iPhone found yet. Keep its Bluetooth settings open, then scan again.', 'Connect' if data else 'Scan again')
                if data:
                    self.phones.set_selected(0)
        self.task(inspect, done)

    def answer_pair(self, answer):
        if self.pair_process and self.pair_process.poll() is None:
            try:
                self.pair_process.stdin.write(answer + '\n')
                self.pair_process.stdin.flush()
            except (OSError, ValueError):
                pass

    def pair(self, device):
        self.show('pairing', 'Connecting your iPhone', 'Keep the phone unlocked with its Bluetooth settings open.')
        self.cancel_pair.clear()
        generation = self.generation
        def event(data):
            if generation != self.generation:
                return False
            if data.get('event') == 'confirmation':
                if data.get('purpose') == 'bind':
                    # The user chose this phone and pressed Connect.
                    self.answer_pair('yes')
                else:
                    self.code.set_label(str(data.get('passkey', '')))
                    self.show('confirm', '1. Confirm on this computer', 'Compare the codes on both devices. If they match, choose Codes match here first. Then tap Pair on your iPhone.', 'Codes match')
            elif data.get('event') == 'display':
                self.code.set_label(str(data.get('passkey', '')))
                self.show('pairing', 'Confirm on your iPhone', 'Approve the code displayed on your iPhone.')
                self.code.set_visible(True)
            elif data.get('ok') is False:
                self.show('error', 'Pairing did not finish', data.get('error', 'Please try again.'), 'Try again')
            elif data.get('ok') is True:
                self.started = time.monotonic()
                self.begin_polling()
                self.check_now()
            return False
        def work():
            try:
                self.pair_process = subprocess.Popen(['blueferry', 'pairing-complete', device['mac'], '--interactive-agent'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
                finished = False
                for line in self.pair_process.stdout:
                    if self.cancel_pair.is_set():
                        break
                    try:
                        data = json.loads(line)
                    except ValueError:
                        continue
                    finished = finished or 'ok' in data
                    GLib.idle_add(event, data)
                self.pair_process.wait(timeout=5)
                if not finished and not self.cancel_pair.is_set():
                    GLib.idle_add(event, {'ok': False, 'error': 'The pairing helper ended without confirming a connection. Try again or open BlueFerry for its report.'})
            except Exception:
                GLib.idle_add(event, {'ok': False, 'error': 'The pairing helper stopped. Open BlueFerry for its detailed report.'})
        threading.Thread(target=work, daemon=True).start()

    def present_status(self, status):
        self.last_status = status
        history = status.get('recovery_history', {})
        self.audio_recovery_attempted = self.audio_recovery_attempted or 'ios27-hfp' in history
        if status.get('connectivity_state') == 'not-configured':
            if self.poll_id:
                GLib.source_remove(self.poll_id)
                self.poll_id = None
            self.show('error', 'Choose your iPhone to continue', 'Setup is not currently linked to a phone. Next checks existing bonds before offering a new pairing.', 'Next')
            return
        if not self.fixture:
            try:
                policy = (Path.home() / '.config/blueferry/local.env').read_text()
                if 'BLUEFERRY_KEEP_PHONE_AUDIO_ON_PHONE=false' in policy.splitlines():
                    self.audio_recovery_attempted = True
            except OSError:
                pass
        outcome = connection_outcome(status, time.monotonic() - self.started)
        observed = status.get('evidence', {})
        cached = status.get('contacts')
        detail = ('Bluetooth pairing: ' + observed.get('pairing', 'unknown') +
                  '\nMessages: ' + observed.get('messages', 'connected' if status.get('map') else 'unavailable') +
                  '\nContacts transport: ' + ('connected' if status.get('pbap') else 'unavailable') +
                  '\nContacts cached: ' + (str(cached) if cached is not None else 'unknown') +
                  '\niPhone switch positions: not observable from Linux' +
                  '\n' + str(status.get('connectivity_detail', '')))
        if status.get('last_message_result'):
            detail += '\nLast verified message result: ' + status['last_message_result'] + ' (' + status.get('last_message_result_at', '') + ')'
        if history:
            detail += '\nRecovery steps already tried: ' + ', '.join(history)
        self.details.set_label(detail)
        if not self.fixture and needs_ios27_audio_recovery(self.ios_version, status, self.audio_recovery_attempted):
            self.show('recovery', 'Prepare access for iOS 27', 'This iOS version may need the computer’s hands-free Bluetooth role before it offers message permissions. Next restores that role; your iPhone may offer this computer as an audio output. Setup will then check again.', 'Next')
        elif not self.fixture and observed.get('messages') == 'access-denied' and history:
            count = str(cached) + ' contacts are synced. ' if cached is not None else ''
            self.show('blocked', 'Your phone is connected; messages need access', count + 'The iPhone still denies message access. The recorded recovery steps have already been tried, so setup will not ask you to repeat pairing. Connection details show the current evidence and the last verified result.', 'Open BlueFerry')
        else:
            self.show(outcome.state, outcome.title, outcome.detail, 'Open Messages' if outcome.state == 'ready' else 'Open BlueFerry' if outcome.state == 'blocked' else None)
        if outcome.state == 'ready' and self.poll_id:
            GLib.source_remove(self.poll_id)
            self.poll_id = None

    def recover_audio(self):
        self.audio_recovery_attempted = True
        def operation():
            path = Path.home() / '.config/blueferry/local.env'
            rows = path.read_text().splitlines()
            rows = [line for line in rows if not line.startswith('BLUEFERRY_KEEP_PHONE_AUDIO_ON_PHONE=')]
            rows.append('BLUEFERRY_KEEP_PHONE_AUDIO_ON_PHONE=false')
            temporary = path.with_name('.local.env.phone-setup')
            fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'w') as stream:
                stream.write('\n'.join(rows) + '\n')
            os.replace(temporary, path)
            subprocess.run(['systemctl', '--user', 'restart', 'blueferry.service'], timeout=15, check=True, capture_output=True)
            record_recovery('ios27-hfp', 'Normal HFP role restored; message access must still be verified')
            return self.status()
        self.task(operation, self.present_status)

    def begin_polling(self):
        if self.poll_id or self.fixture:
            return
        def poll():
            if not self.worker_busy and self.state not in ('confirm', 'pairing', 'recovery', 'ready'):
                self.task(self.status, self.present_status)
            return GLib.SOURCE_CONTINUE
        self.poll_id = GLib.timeout_add_seconds(5, poll)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--preview', choices=['permissions', 'confirm', 'ready'])
    parser.add_argument('--ios-version', default='')
    arguments = parser.parse_args()
    PhoneSetup(arguments.preview, arguments.ios_version).run([])
