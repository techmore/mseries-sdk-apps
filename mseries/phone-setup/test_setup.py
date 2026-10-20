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

    def test_timeout_does_not_claim_success(self):
        self.assertEqual(connection_outcome({'connectivity_state': 'reconnecting'}, 46).state, 'blocked')


if __name__ == '__main__':
    unittest.main()
