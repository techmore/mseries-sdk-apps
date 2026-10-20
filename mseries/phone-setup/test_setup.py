import unittest
from setup_core import connection_outcome, needs_ios27_audio_recovery


class ConnectionStates(unittest.TestCase):
    def test_contact_connection_never_means_messages_ready(self):
        self.assertNotEqual(connection_outcome({'pbap': True, 'map': False}, 90).state, 'ready')

    def test_message_access_is_the_readiness_boundary(self):
        self.assertEqual(connection_outcome({'map': True, 'pbap': False}).state, 'ready')

    def test_authorization_requires_phone_action_even_after_waiting(self):
        result = connection_outcome({'connectivity_state': 'authorization-required'}, 500)
        self.assertEqual(result.state, 'permissions')
        self.assertIn('iPhone', result.title)

    def test_recovery_does_not_loop_or_change_unrelated_versions(self):
        status = {'connectivity_state': 'authorization-required'}
        self.assertTrue(needs_ios27_audio_recovery('27.0.1', status, False))
        self.assertFalse(needs_ios27_audio_recovery('27.0.1', status, True))
        self.assertFalse(needs_ios27_audio_recovery('18.7', status, False))

    def test_previous_denial_does_not_repeat_toggle_instructions(self):
        result = connection_outcome({'map': False, 'connectivity_state': 'reconnecting', 'evidence': {'messages': 'access-denied'}, 'recovery_history': {'clean-repair': {}}})
        self.assertEqual(result.state, 'blocked')
        self.assertEqual(result.action, 'Retry Connection')
        self.assertNotIn('Enable Receive Messages', result.detail)

    def test_inactive_service_does_not_blame_phone_permissions(self):
        result = connection_outcome({'map': None, 'connectivity_state': 'backend-inactive', 'last_message_result': 'access-denied', 'recovery_history': {'clean-repair': {}}})
        self.assertEqual(result.state, 'inactive')
        self.assertEqual(result.action, 'Retry Connection')

    def test_disconnected_phone_is_not_presented_as_live_permission_denial(self):
        result = connection_outcome({'bluetooth_connected': False, 'map': False, 'evidence': {'messages': 'access-denied'}, 'recovery_history': {'clean-repair': {}}})
        self.assertEqual(result.title, 'Your iPhone is disconnected')

    def test_timeout_does_not_claim_success(self):
        self.assertEqual(connection_outcome({'connectivity_state': 'reconnecting'}, 46).state, 'blocked')


if __name__ == '__main__':
    unittest.main()
