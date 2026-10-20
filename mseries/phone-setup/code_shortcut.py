"""Own one shortcut; compare parsed accelerators and preserve all other bindings."""
import json
from pathlib import Path
import sys
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gio, Gtk

PATH = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/mseries-verification-codes/'
SCHEMA = 'org.gnome.settings-daemon.plugins.media-keys'
CUSTOM = SCHEMA + '.custom-keybinding'
CANDIDATES = ('<Super><Shift>v', '<Super><Shift>o', '<Control><Alt>o')


def accelerator(value):
    valid, key, modifiers = Gtk.accelerator_parse(value)
    return (key, int(modifiers)) if valid and key else None


def choose(used, preferred=''):
    occupied = {accelerator(value) for value in used}
    return next((value for value in (preferred, *CANDIDATES)
                 if accelerator(value) and accelerator(value) not in occupied), '')


def settings(schema, path=None):
    if not Gio.SettingsSchemaSource.get_default().lookup(schema, True):
        return None
    return Gio.Settings.new_with_path(schema, path) if path else Gio.Settings.new(schema)


def install(launcher, root):
    keys = settings(SCHEMA)
    if not keys:
        print('Keyboard shortcut unavailable; open Verification Codes from Applications.')
        return
    paths = keys.get_strv('custom-keybindings')
    own = settings(CUSTOM, PATH)
    command = '"' + launcher + '" --show'
    if own.get_string('command') not in ('', command):
        print('Reserved shortcut path is already in use; existing binding preserved.')
        return
    used = []
    for path in paths:
        if path != PATH:
            used.append(settings(CUSTOM, path).get_string('binding'))
    for schema in ('org.gnome.desktop.wm.keybindings', SCHEMA, 'io.elementary.desktop.wm.keybindings'):
        source = settings(schema)
        if not source: continue
        for key in source.props.settings_schema.list_keys():
            value = source.get_value(key)
            if value.get_type_string() == 'as': used.extend(value.unpack())
            elif value.get_type_string() == 's': used.append(value.unpack())
    binding = choose(used, own.get_string('binding') if PATH in paths else '')
    if not binding:
        print('No proposed shortcut is free; open Verification Codes from Applications.')
        return
    own.set_string('name', 'Use Verification Code')
    own.set_string('command', command)
    own.set_string('binding', binding)
    if PATH not in paths: keys.set_strv('custom-keybindings', paths + [PATH])
    Gio.Settings.sync()
    metadata = Path(root) / 'verification-shortcut.json'
    metadata.write_text(json.dumps(dict(binding=binding, label=Gtk.accelerator_get_label(*accelerator(binding)), path=PATH))+'\n')
    metadata.chmod(0o600)
    print('Verification-code shortcut:', Gtk.accelerator_get_label(*accelerator(binding)))


def uninstall(launcher, root):
    keys = settings(SCHEMA)
    own = settings(CUSTOM, PATH)
    if keys and own and own.get_string('command') == '"'+launcher+'" --show':
        keys.set_strv('custom-keybindings', [p for p in keys.get_strv('custom-keybindings') if p != PATH])
        for key in ('name', 'command', 'binding'): own.reset(key)
        Gio.Settings.sync()
    (Path(root)/'verification-shortcut.json').unlink(missing_ok=True)


if __name__ == '__main__':
    {'install':install, 'uninstall':uninstall}[sys.argv[1]](*sys.argv[2:])
