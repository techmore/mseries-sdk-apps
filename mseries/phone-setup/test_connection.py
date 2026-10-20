"""Verify reconnection cannot silently pair, alter settings, or claim success."""
import unittest
from unittest.mock import patch, MagicMock
import connection


class Reconnect(unittest.TestCase):
    def setUp(self):
        self.settings = {'BLUEFERRY_MAC': 'AA:BB:CC:DD:EE:FF', 'BLUEFERRY_ADAPTER': 'hci0'}

    def test_no_target_never_restarts_a_service(self):
        with patch.object(connection.config, 'read_local_env', return_value={}), patch.object(connection.subprocess, 'run') as run:
            with self.assertRaises(RuntimeError): connection.reconnect_existing()
            run.assert_not_called()

    def exercise(self, status, paired=True, cancelled=False):
        proxy = MagicMock(); proxy.Get.return_value = paired
        with patch.object(connection.config, 'read_local_env', return_value=self.settings), patch.object(connection.dbus, 'SystemBus'), patch.object(connection.dbus, 'Interface', return_value=proxy), patch.object(connection.subprocess, 'run') as run, patch.object(connection, 'collect_status', return_value=status), patch.object(connection, 'record_recovery') as record:
            if not paired:
                with self.assertRaises(RuntimeError): connection.reconnect_existing()
                run.assert_not_called()
                return
            result = connection.reconnect_existing(timeout=0, cancelled=lambda: cancelled)
            if cancelled:
                run.assert_not_called();record.assert_not_called()
                self.assertIsNone(result)
                return
            run.assert_called_once_with(['systemctl', '--user', 'restart', 'blueferry.service'], check=True, capture_output=True, timeout=20)
            self.assertEqual(result, status)
            self.assertEqual(len(record.call_args_list), 1)
            return record.call_args.args[1]

    def test_missing_bond_never_restarts_or_repairs(self): self.exercise({}, paired=False)
    def test_cancelled_window_does_not_start_recovery(self): self.exercise({}, cancelled=True)
    def test_denied_remains_denied_after_restart(self):
        self.assertIn('still denies', self.exercise({'backend_running': True, 'map': False, 'connectivity_state': 'authorization-required'}))
    def test_timeout_is_not_success(self): self.assertIn('no verified', self.exercise({'map': False}))
    def test_only_map_success_records_connected(self): self.assertIn('connected after', self.exercise({'map': True}))


if __name__ == '__main__': unittest.main()
