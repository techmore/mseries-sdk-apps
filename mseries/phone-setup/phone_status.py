#!/usr/bin/python3
"""Read-only live phone status. Never pair, activate a daemon, or guess iOS switches."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import tempfile

import dbus
from blueferry import config

STATE_DIR = Path(os.environ.get('XDG_STATE_HOME', str(Path.home() / '.local/state'))) / 'mseries-phone-setup'


def evidence(status, pairing=None):
    """Permission inference is distinct from directly observed transport state."""
    pairing = pairing or {}
    denied = status.get('connectivity_state') == 'authorization-required' or (status.get('map') is False and status.get('last_message_result') == 'access-denied')
    return {
        'pairing': 'paired' if pairing.get('paired') is True else 'not-paired' if pairing.get('paired') is False else 'unknown',
        'messages': 'connected' if status.get('map') is True else 'access-denied' if denied else 'unknown' if status.get('map') is None else 'not-connected',
        'contacts_profile': 'connected' if status.get('pbap') is True else 'unknown' if status.get('pbap') is None else 'not-connected',
        'contacts_cached': status.get('contacts'),
        'iphone_message_switch': 'not-observable',
        'iphone_contacts_switch': 'not-observable',
        'permission_source': 'transport-result; iOS does not expose switch positions',
    }


def _read_state(name):
    try:
        value = json.loads((STATE_DIR / name).read_text())
        return value if isinstance(value, dict) else {}
    except (OSError, ValueError):
        return {}


def _write_state(name, value):
    STATE_DIR.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.' + name, dir=STATE_DIR)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(value, stream, indent=2)
            stream.write('\n')
        os.replace(temporary, STATE_DIR / name)
    finally:
        Path(temporary).unlink(missing_ok=True)


def target_key():
    target = config.read_local_env().get('BLUEFERRY_MAC', '')
    return hashlib.sha256(target.upper().encode()).hexdigest() if config.is_valid_mac(target) else ''


def recovery_history():
    key = target_key()
    value = _read_state('recovery.json')
    return value.get('actions', {}) if key and value.get('target') == key else {}


def record_recovery(action, detail):
    key = target_key()
    if not key:
        return
    actions = recovery_history()
    actions[action] = {'at': datetime.now(timezone.utc).isoformat(), 'result': detail}
    _write_state('recovery.json', {'target': key, 'actions': actions})


def collect_status():
    now = datetime.now(timezone.utc).isoformat()
    values = config.read_local_env()
    target = values.get('BLUEFERRY_MAC', '')
    status = {'checked_at': now, 'backend_running': False, 'map': None, 'pbap': None, 'connectivity_state': 'not-configured'}
    paired = {}
    if config.is_valid_mac(target):
        status['connectivity_state'] = 'backend-inactive'
        session = None
        try:
            session = dbus.SessionBus(private=True)
            if session.name_has_owner('io.weirdware.BlueFerry'):
                # Do not GetObject or invoke service activation when no owner exists.
                obj = session.get_object('io.weirdware.BlueFerry', '/io/weirdware/BlueFerry', introspect=False)
                current = json.loads(dbus.Interface(obj, 'io.weirdware.BlueFerry.Messages1').GetStatus(timeout=5))
                # Select only non-content fields. No messages, addresses or contacts leave the backend.
                for key in ('map', 'pbap', 'ancs', 'contacts', 'connectivity_state', 'connectivity_detail', 'storage_state'):
                    status[key] = current.get(key)
                status['backend_running'] = True
        except (dbus.DBusException, ValueError) as error:
            status['connectivity_state'] = 'status-unavailable'
            status['error'] = type(error).__name__
        finally:
            if session is not None:
                session.close()
        system = None
        try:
            system = dbus.SystemBus(private=True)
            adapter = values.get('BLUEFERRY_ADAPTER', 'hci0')
            if not config.is_valid_adapter(adapter):
                adapter = 'hci0'
            obj = system.get_object('org.bluez', '/org/bluez/' + adapter + '/dev_' + target.replace(':', '_'), introspect=False)
            properties = dbus.Interface(obj, 'org.freedesktop.DBus.Properties').GetAll('org.bluez.Device1', timeout=3)
            paired['paired'] = bool(properties.get('Paired', False))
            status['bluetooth_connected'] = bool(properties.get('Connected', False))
        except dbus.DBusException as error:
            if error.get_dbus_name() in ('org.freedesktop.DBus.Error.UnknownObject', 'org.bluez.Error.DoesNotExist'):
                paired['paired'] = False
        finally:
            if system is not None:
                system.close()
    history = _read_state('transport-history.json')
    key = target_key()
    if history.get('target') != key:
        history = {'target': key}
    result = 'connected' if status.get('map') is True else 'access-denied' if status.get('connectivity_state') == 'authorization-required' else None
    if result is not None and result != history.get('last_message_result'):
        history.update(last_message_result=result, last_message_result_at=now)
        _write_state('transport-history.json', history)
    for field in ('last_message_result', 'last_message_result_at'):
        if field in history:
            status[field] = history[field]
    status['evidence'] = evidence(status, paired)
    status['recovery_history'] = recovery_history()
    previous = _read_state('status.json')
    changed = {k: v for k, v in status.items() if k != 'checked_at'} != {k: v for k, v in previous.items() if k != 'checked_at'}
    if changed:
        _write_state('status.json', status)
    return status


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--json', action='store_true')
    args = parser.parse_args()
    status = collect_status()
    if args.json:
        print(json.dumps(status, indent=2))
    else:
        state = status['evidence']
        print('Bluetooth pairing: ' + state['pairing'])
        print('Messages: ' + state['messages'])
        print('Contacts transport: ' + state['contacts_profile'])
        print('Contacts cached: ' + str(state['contacts_cached'] if state['contacts_cached'] is not None else 'unknown'))
        print('iPhone switch positions: not observable from Linux')
        print('Checked: ' + status['checked_at'])
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
