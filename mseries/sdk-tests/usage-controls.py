#!/usr/bin/env python3
"""Private synthetic keyboard/AT-SPI refresh, failure and retry probe."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shlex
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
report = {'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
          'environment': 'isolated X11 / private D-Bus / synthetic data', 'checks': []}


def walk(node, depth=0):
    if depth > 35:
        return
    yield node
    for child in node:
        if child is not None:
            yield from walk(child, depth + 1)


def key(name):
    subprocess.run(['xdotool', 'key', '--clearmodifiers', name], check=True)
    time.sleep(.05)


with tempfile.TemporaryDirectory(prefix='msdk-usage-controls-') as directory:
    root = Path(directory)
    marker, fail, executable = root / 'reads', root / 'fail', root / 'fixture'
    observed = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace('+00:00', 'Z')
    snapshot = {'schemaVersion': 1, 'entries': [{'provider': 'codex', 'updatedAt': observed,
                'windows': [{'key': 'week', 'label': '7 day', 'remaining': 75, 'resetsAt': ''}]}]}
    executable.write_text('#!/bin/sh\nprintf x >> ' + shlex.quote(str(marker)) + '\n'
                          + 'if [ -f ' + shlex.quote(str(fail)) + ' ]; then\n'
                          + "printf 'synthetic-private-error' >&2\nprintf 'invalid-json'\nelse\nprintf '%s' "
                          + shlex.quote(json.dumps(snapshot)) + '\nfi\n')
    executable.chmod(0o700)
    env = dict(os.environ, XDG_DATA_HOME=str(root / 'data'), XDG_CONFIG_HOME=str(root / 'config'))
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        process = subprocess.Popen([str(args.binary.resolve()), str(executable)], env=env, stdout=stdout, stderr=stderr)
        app = None
        try:
            def wait(predicate, message, timeout=8):
                end = time.monotonic() + timeout
                while time.monotonic() < end:
                    if process.poll() is not None:
                        raise RuntimeError(f'fixture app exited early ({process.returncode}); see process_stderr')
                    value = predicate()
                    if value:
                        return value
                    time.sleep(.04)
                raise AssertionError(message)

            desktop = pyatspi.Registry.getDesktop(0)
            def application():
                return next((app for app in desktop if app.get_process_id() == process.pid), None)
            app = wait(application, 'fixture app did not publish an accessibility root')
            def named(name):
                return next((item for item in walk(app) if item.name == name), None)
            button = wait(lambda: next((item for item in walk(app) if item.name == 'Refresh usage'
                         and item.getRoleName() in ('push button', 'button')), None), 'refresh button is not accessible')
            report['refresh_accessible_role'] = button.getRoleName()
            wait(lambda: named('Refresh complete'), 'initial synthetic read did not finish')
            assert marker.read_text() == 'x', 'opening must issue exactly one read'
            window = subprocess.check_output(['xdotool', 'search', '--sync', '--name', '^Usage control test$'], text=True, timeout=8).splitlines()[0]
            subprocess.run(['xdotool', 'windowfocus', '--sync', window], check=True)
            def reach():
                for _ in range(32):
                    if button.getState().contains(pyatspi.STATE_FOCUSED):
                        return
                    key('Tab')
                raise AssertionError('keyboard cannot reach refresh')
            reach()
            report['checks'].append('Tab reaches the named refresh button')
            fail.write_text('fail')
            key('space')
            wait(lambda: named('Retry when ready'), 'keyboard refresh did not show failure')
            assert marker.read_text() == 'xx', 'keyboard refresh must issue one read'
            wait(lambda: named('Stale'), 'last-known data was not marked stale')
            assert any('Couldn\'t refresh usage' in item.name for item in walk(app) if item.name)
            assert not any('synthetic-private-error' in item.name for item in walk(app) if item.name)
            report['checks'].append('Space refreshes; failure keeps stale data and hides stderr')
            fail.unlink()
            wait(lambda: button.getState().contains(pyatspi.STATE_SENSITIVE), 'refresh did not re-enable')
            reach()
            key('space')
            wait(lambda: named('Refresh complete'), 'keyboard retry did not recover')
            assert marker.read_text() == 'xxx', 'keyboard retry must issue one read'
            report['checks'].append('keyboard retry recovers')
            key('ctrl+q')
            process.wait(timeout=8)
            assert process.returncode == 0, process.returncode
            report['checks'].append('Ctrl+Q closes cleanly')
            report['result'] = 'pass'
        except Exception as error:
            report['result'] = 'fail'
            report['failure'] = str(error)
            if app is not None:
                try:
                    report['accessible_nodes'] = [
                        {'name': (item.name or '')[:160], 'role': item.getRoleName()}
                        for item in walk(app)
                    ][:100]
                except Exception as diagnostic_error:
                    report['accessibility_diagnostic_error'] = type(diagnostic_error).__name__
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            stdout.seek(0); stderr.seek(0)
            report['process_stdout'] = stdout.read()[-8000:].decode(errors='replace')
            report['process_stderr'] = stderr.read()[-8000:].decode(errors='replace')
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
