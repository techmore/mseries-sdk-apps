"""Regression checks for status observability and backend activation."""
import unittest
from unittest.mock import patch
import phone_status


class ActivationBoundary(unittest.TestCase):
    @patch.object(phone_status, '_write_state')
    @patch.object(phone_status, '_read_state', return_value={})
    @patch.object(phone_status.config, 'read_local_env', return_value={})
    @patch.object(phone_status.dbus, 'SessionBus')
    def test_missing_target_does_not_activate_backend(self, session_bus, config, read, write):
        result = phone_status.collect_status()
        self.assertEqual(result['connectivity_state'], 'not-configured')
        session_bus.assert_not_called()

    def test_contact_access_does_not_prove_message_permission(self):
        result = phone_status.evidence({'pbap': True, 'contacts': 727, 'map': False, 'connectivity_state': 'authorization-required'}, {'paired': True})
        self.assertEqual(result['contacts_cached'], 727)
        self.assertEqual(result['messages'], 'access-denied')
        self.assertEqual(result['iphone_message_switch'], 'not-observable')

    def test_retry_does_not_erase_the_last_verified_denial(self):
        result = phone_status.evidence({'map': False, 'connectivity_state': 'reconnecting', 'last_message_result': 'access-denied'})
        self.assertEqual(result['messages'], 'access-denied')

    def test_inactive_backend_is_unknown_not_failed(self):
        result = phone_status.evidence({'map': None, 'pbap': None})
        self.assertEqual(result['messages'], 'unknown')
        self.assertEqual(result['pairing'], 'unknown')


if __name__ == '__main__':
    unittest.main()
