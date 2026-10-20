"""Run with GSETTINGS_BACKEND=memory; never alter owner shortcuts."""
import os
from pathlib import Path
import tempfile
import unittest
from code_shortcut import install, uninstall, settings, SCHEMA, CUSTOM, PATH


@unittest.skipUnless(os.environ.get('GSETTINGS_BACKEND') == 'memory', 'Requires memory settings backend')
class ShortcutTests(unittest.TestCase):
    def setUp(self):
        self.keys = settings(SCHEMA)
        self.keys.set_strv('custom-keybindings', [])
        self.own = settings(CUSTOM, PATH)
        for key in ('name','command','binding'): self.own.reset(key)
        self.folder = tempfile.TemporaryDirectory(prefix='mseries-shortcut-test-')
        self.addCleanup(self.folder.cleanup)
        self.launcher = '/synthetic/mseries-verification-codes'

    def test_install_upgrade_and_remove_preserve_other_shortcut(self):
        other_path = '/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/fixture/'
        other = settings(CUSTOM, other_path)
        other.set_string('binding','<Shift><Super>v')
        other.set_string('command','fixture-command')
        self.keys.set_strv('custom-keybindings', [other_path])
        install(self.launcher, self.folder.name)
        self.assertEqual(self.own.get_string('binding'), '<Super><Shift>o')
        install(self.launcher, self.folder.name)
        self.assertEqual(self.keys.get_strv('custom-keybindings'), [other_path, PATH])
        self.assertEqual((Path(self.folder.name)/'verification-shortcut.json').stat().st_mode & 0o777, 0o600)
        uninstall(self.launcher, self.folder.name)
        self.assertEqual(self.keys.get_strv('custom-keybindings'), [other_path])
        self.assertEqual(other.get_string('command'), 'fixture-command')
        self.assertEqual(self.own.get_string('command'), '')

    def test_collision_is_not_overwritten(self):
        self.own.set_string('command', 'owner-command')
        self.own.set_string('binding', '<Super>q')
        self.keys.set_strv('custom-keybindings', [PATH])
        install(self.launcher, self.folder.name)
        uninstall(self.launcher, self.folder.name)
        self.assertEqual(self.own.get_string('command'), 'owner-command')
        self.assertEqual(self.keys.get_strv('custom-keybindings'), [PATH])


if __name__ == '__main__':
    unittest.main()
