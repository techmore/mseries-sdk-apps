#!/usr/bin/env python3
"""Actual generated Home Storage keyboard flow; isolated Xvfb/D-Bus only."""
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
parser.add_argument('--screenshot', type=Path)
parser.add_argument('--width', type=int, default=900)
parser.add_argument('--config-home', type=Path)
args = parser.parse_args()
assert os.environ.get('MSDK_PRIVATE_TEST_BUS') == '1'
assert not args.output.exists(), 'preserve prior receipts'
binary = args.binary.resolve()
report = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
          'environment': 'private X11/D-Bus, synthetic HOME/XDG', 'checks': []}

def wait(predicate, message, timeout=8):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        time.sleep(.03)
    raise AssertionError(message)

def walk(node, depth=0):
    if depth > 35:
        return
    yield node
    for child in node:
        if child is not None:
            yield from walk(child, depth + 1)

def key(name):
    subprocess.run(['xdotool', 'key', '--clearmodifiers', name], check=True)
    time.sleep(.04)

with tempfile.TemporaryDirectory(prefix='msdk-home-controls-') as directory:
    # The first real command must fail because this synthetic folder is absent.
    # The harness fixes that condition, then retries in the SAME app process.
    home = Path(directory) / 'synthetic-home'
    config_home = str(args.config_home.resolve()) if args.config_home else directory + '/config'
    env = dict(os.environ, HOME=str(home), XDG_DATA_HOME=directory + '/data',
               XDG_CONFIG_HOME=config_home, XDG_CACHE_HOME=directory + '/cache')
    command = [str(binary)]
    if args.screenshot:
        assert not args.screenshot.exists()
        command += ['--screenshot', str(args.screenshot.resolve())]
    process = subprocess.Popen(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        desktop = pyatspi.Registry.getDesktop(0)
        app = wait(lambda: next((node for node in desktop if node.get_process_id() == process.pid), None),
                   'generated app did not publish accessibility root')
        def has_text(fragment):
            return any(fragment in (node.name or '') for node in walk(app))
        def button():
            return wait(lambda: next((node for node in walk(app) if node.name == 'Check again'
                        and node.getRoleName() in ('push button', 'button')), None), 'missing Check again button')
        window = subprocess.check_output(['xdotool', 'search', '--sync', '--name', '^Home Storage$'], text=True, timeout=8).splitlines()[0]
        subprocess.run(['xdotool', 'windowfocus', '--sync', window], check=True)
        subprocess.run(['xdotool', 'windowsize', '--sync', window, str(args.width), '400'], check=True)
        def activate():
            control = button()
            for _ in range(32):
                if any(node.getState().contains(pyatspi.STATE_FOCUSED) for node in walk(control)):
                    key('space')
                    return
                key('Tab')
            raise AssertionError('keyboard could not reach Check again')
        report['accessible_role'] = button().getRoleName()
        activate()
        wait(lambda: has_text('Storage could not be checked.'), 'real command failure was not shown')
        assert process.poll() is None
        button()
        report['checks'].append('keyboard starts actual df; absent synthetic HOME gives recoverable error and retry action')
        home.mkdir()
        activate()
        wait(lambda: has_text('Current filesystem usage. No files were changed.'), 'retry did not recover in the same app')
        wait(lambda: has_text('Available:') and has_text('Used space:'), 'successful result is missing labeled storage output')
        assert not has_text('Storage could not be checked.')
        report['checks'].append('keyboard retry after harness creates synthetic HOME succeeds in same process; stale error disappears')
        if args.screenshot:
            process.wait(timeout=8)
            assert process.returncode == 0
            assert args.screenshot.is_file() and args.screenshot.stat().st_size > 0
            report['checks'].append('actual generated executable captures recovered result and exits cleanly')
        else:
            key('ctrl+q')
            process.wait(timeout=8)
            assert process.returncode == 0
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
        # No filesystem output or home paths enter receipts. Diagnostics must
        # omit command data and private backend details even on real failure.
        report['stdout_empty'] = not stdout
        report['diagnostics_omit_synthetic_home'] = str(home).encode() not in stderr
        report['diagnostics'] = stderr.decode(errors='replace')[-8000:] if report['diagnostics_omit_synthetic_home'] else '[private data omitted]'
        if not report['diagnostics_omit_synthetic_home']:
            report['result'] = 'fail'
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')
assert report['result'] == 'pass', 'diagnostics leaked synthetic home path'
print(json.dumps(report, indent=2))
