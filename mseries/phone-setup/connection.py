"""Reconnect a saved phone without changing its bond or audio policy."""
import subprocess
import time
import dbus
from blueferry import config
from phone_status import collect_status, record_recovery


def reconnect_existing(timeout=15, cancelled=lambda: False):
    values = config.read_local_env()
    target = values.get('BLUEFERRY_MAC', '')
    adapter = values.get('BLUEFERRY_ADAPTER', 'hci0')
    if not config.is_valid_mac(target) or not config.is_valid_adapter(adapter):
        raise RuntimeError('Choose your iPhone before reconnecting.')
    bus = dbus.SystemBus(private=True)
    try:
        path = '/org/bluez/' + adapter + '/dev_' + target.replace(':', '_')
        obj = bus.get_object('org.bluez', path, introspect=False)
        paired = dbus.Interface(obj, 'org.freedesktop.DBus.Properties').Get('org.bluez.Device1', 'Paired', timeout=3)
        if not paired:
            raise RuntimeError('The saved Bluetooth pairing is missing. Open BlueFerry to pair your phone.')
    finally:
        bus.close()
    if cancelled():
        return None
    subprocess.run(['systemctl', '--user', 'restart', 'blueferry.service'],
                   check=True, capture_output=True, timeout=20)
    deadline = time.monotonic() + timeout
    latest = {}
    while not cancelled():
        latest = collect_status()
        if latest.get('map') is True:
            result = 'Message access connected after restarting the existing phone service'
            break
        if latest.get('backend_running') and latest.get('connectivity_state') == 'authorization-required':
            result = 'Existing bond retained; iPhone still denies message access after service restart'
            break
        if time.monotonic() >= deadline:
            result = 'Existing bond retained; no verified message access before reconnect timeout'
            break
        time.sleep(0.5)
    else:
        return None
    record_recovery('native-reconnect', result)
    return collect_status()
