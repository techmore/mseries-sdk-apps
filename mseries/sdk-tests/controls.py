#!/usr/bin/env python3
"""Exercise the generated settings app with actual keyboard and AT-SPI controls.
Run inside a fresh dbus-run-session/Xvfb, never on the owner's desktop.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import pyatspi

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
assert os.environ.get('MSDK_PRIVATE_TEST_BUS') == '1', 'requires an isolated test session'
assert not args.output.exists(), 'preserve prior receipts; choose a new output'
app_id = 'io.github.techmore.SdkSettings'
report = {'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
          'environment': 'isolated X11 / private D-Bus / synthetic XDG state', 'checks': []}

def wait(predicate, message, timeout=8):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(.04)
    raise AssertionError(message)

def walk(node, depth=0):
    if depth > 35:
        return
    yield node
    for child in node:
        if child is not None:
            yield from walk(child, depth + 1)

def focused(node):
    return any(item.getState().contains(pyatspi.STATE_FOCUSED) for item in walk(node))

def key(name):
    subprocess.run(['xdotool', 'key', '--clearmodifiers', name], check=True)
    time.sleep(.05)

def reach(node):
    for _ in range(32):
        if focused(node):
            return
        key('Tab')
    raise AssertionError(f'keyboard cannot reach {node.name!r}')

with tempfile.TemporaryDirectory(prefix='msdk-controls-') as directory:
    env = dict(os.environ, XDG_DATA_HOME=directory, XDG_CONFIG_HOME=directory + '/config')
    process = subprocess.Popen([str(args.binary.resolve())], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        desktop = pyatspi.Registry.getDesktop(0)
        def application():
            status = process.poll()
            if status is not None:
                raise RuntimeError(f'generated app exited before accessibility registration (exit {status}); see process_stderr')
            for app in desktop:
                if app.get_process_id() == process.pid:
                    return app
            return None
        app = wait(application, 'generated app did not publish an accessibility root')
        def target(name):
            # A native form exposes both its visible label and its named control.
            # Select the interactive accessible, not the preceding static label.
            roles = {'Turn on work mode': ('toggle button', 'check box', 'switch'),
                     'Schedule': ('combo box',), 'Reset to defaults': ('push button', 'button')}
            return wait(lambda: next((item for item in walk(app) if item.name == name and item.getRoleName() in roles[name]), None),
                        f'missing accessible control {name}')
        controls = {name: target(name) for name in ['Turn on work mode', 'Schedule', 'Reset to defaults']}
        roles = {name: item.getRoleName() for name, item in controls.items()}
        assert roles['Turn on work mode'] in ('toggle button', 'check box', 'switch'), roles
        assert roles['Schedule'] == 'combo box', roles
        assert roles['Reset to defaults'] in ('push button', 'button'), roles
        report['accessible_roles'] = roles
        report['checks'].append('live accessible names and roles')
        window = subprocess.check_output(['xdotool', 'search', '--sync', '--name', '^SDK Settings$'], text=True, timeout=8).splitlines()[0]
        subprocess.run(['xdotool', 'windowfocus', '--sync', window], check=True)
        reach(controls['Turn on work mode'])
        key('space')
        state_path = Path(directory) / app_id / 'state.json'
        def read_state():
            return json.loads(state_path.read_text()) if state_path.exists() else {}
        wait(lambda: read_state().get('enabled') is False, 'keyboard switch did not save false')
        report['checks'].append('Tab reaches switch; Space changes persisted state')
        reach(controls['Schedule'])
        key('space')
        key('Down')
        key('Return')
        wait(lambda: read_state().get('schedule') == 'Every day', 'keyboard dropdown did not save selection')
        report['checks'].append('Tab reaches dropdown; keyboard selection saves')
        reach(controls['Reset to defaults'])
        key('space')
        wait(lambda: read_state().get('enabled') is True and read_state().get('schedule') == 'Weekdays',
             'keyboard reset did not restore settings')
        report['checks'].append('Tab reaches reset; Space restores defaults')
        key('ctrl+q')
        process.wait(timeout=8)
        assert process.returncode == 0, process.returncode
        report['checks'].append('Ctrl+Q closes cleanly')
        report['result'] = 'pass'
    except Exception as error:
        report['result'] = 'fail'
        report['failure'] = str(error)
        raise
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        stdout, stderr = process.communicate(timeout=3)
        # Only this synthetic process's bounded logs are retained.
        report['process_stdout'] = stdout.decode(errors='replace')[-8000:]
        report['process_stderr'] = stderr.decode(errors='replace')[-8000:]
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
