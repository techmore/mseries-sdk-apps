"""Private D-Bus, temporary home and keyring; never run against owner state."""
import os
import unittest
from message_pins import PinStore, matched_state
from test_message_pins import snapshot, threads


@unittest.skipUnless(os.environ.get('MSERIES_PRIVATE_PIN_TEST') == '1', 'Requires private bus/home')
class KeyringTests(unittest.TestCase):
    def test_real_secret_service_roundtrip_and_phone_isolation(self):
        import gi
        gi.require_version('Secret', '1')
        from gi.repository import Secret
        # test-pin-keyring.sh provisions the default login collection through
        # gnome-keyring-daemon --unlock. Creating it again through libsecret
        # triggers a collection prompt and is unsupported in this headless bus.
        first, second = PinStore('fixture-phone-one'), PinStore('fixture-phone-two')
        self.assertIsNone(first.load()['snapshot'])
        state = matched_state(snapshot(), ['alex', None], threads())
        self.assertEqual(first.save(state), state)
        self.assertEqual(first.load(), state)
        self.assertIsNone(second.load()['snapshot'])
        # libsecret owns encryption; no app-created plaintext preferences file.
        self.assertFalse(os.path.exists(os.path.join(os.environ['XDG_DATA_HOME'], 'mseries')))


if __name__ == '__main__': unittest.main()
