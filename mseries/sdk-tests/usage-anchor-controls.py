#!/usr/bin/env python3
"""Private X11/AT-SPI probe for the candidate usage status anchor."""
import argparse
import datetime
import ctypes
import ctypes.util
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


def point(item):
    rect = item.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    assert rect.width > 0 and rect.height > 0, 'accessible control has no on-screen bounds'
    return str(rect.x + rect.width // 2), str(rect.y + rect.height // 2)


def move_point(position):
    subprocess.run(['xdotool', 'mousemove', '--sync', *map(str, position)], check=True)


def x11_pointer_child():
    # XQueryPointer reports the actual root child receiving pointer input,
    # including shaped popup windows; xdotool's WINDOW is the active window.
    x = ctypes.CDLL(ctypes.util.find_library('X11'))
    x.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x.XOpenDisplay.restype = ctypes.c_void_p
    x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    x.XDefaultRootWindow.restype = ctypes.c_ulong
    x.XQueryPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong),
                               ctypes.POINTER(ctypes.c_ulong), *([ctypes.POINTER(ctypes.c_int)] * 4),
                               ctypes.POINTER(ctypes.c_uint)]
    x.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = x.XOpenDisplay(None)
    if not display:
        raise RuntimeError('cannot open X display for pointer diagnostic')
    try:
        root, child = ctypes.c_ulong(), ctypes.c_ulong()
        rx, ry, wx, wy = (ctypes.c_int() for _ in range(4))
        mask = ctypes.c_uint()
        ok = x.XQueryPointer(display, x.XDefaultRootWindow(display), ctypes.byref(root), ctypes.byref(child),
                             ctypes.byref(rx), ctypes.byref(ry), ctypes.byref(wx), ctypes.byref(wy), ctypes.byref(mask))
        return {'ok': bool(ok), 'child': hex(child.value), 'root_x': rx.value, 'root_y': ry.value}
    finally:
        x.XCloseDisplay(display)


with tempfile.TemporaryDirectory(prefix='msdk-usage-anchor-') as directory:
    root = Path(directory)
    marker, executable = root / 'reads', root / 'fixture'
    geometry, popup_geometry = root / 'geometry.json', root / 'popup.json'
    observed = datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace('+00:00', 'Z')
    snapshot = {'schemaVersion': 1, 'entries': [{'provider': 'codex', 'updatedAt': observed,
                'windows': [{'key': 'week', 'label': '7 day', 'remaining': 75, 'resetsAt': ''}]}]}
    executable.write_text('#!/bin/sh\nprintf x >> ' + shlex.quote(str(marker)) + '\nprintf %s '
                          + shlex.quote(json.dumps(snapshot)) + '\n')
    executable.chmod(0o700)
    env = dict(os.environ, XDG_DATA_HOME=str(root / 'data'), XDG_CONFIG_HOME=str(root / 'config'),
               MSDK_ANCHOR_GEOMETRY=str(geometry), MSDK_POPUP_GEOMETRY=str(popup_geometry), MSDK_ANCHOR_TRACE="1")
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        subprocess.run(['xdotool', 'mousemove', '--sync', '0', '0'], check=True)
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
            app = wait(lambda: next((x for x in desktop if x.get_process_id() == process.pid), None), 'missing accessibility root')
            def button(name):
                return next((item for item in walk(app) if item.name == name and item.getRoleName() in ('button', 'push button')
                             and item.getState().contains(pyatspi.STATE_SHOWING)), None)
            anchor = wait(lambda: button('Usage'), 'missing usage anchor')
            outside = wait(lambda: button('Outside'), 'missing outside fixture control')
            window = subprocess.check_output(['xdotool', 'search', '--sync', '--name', '^Usage status anchor test$'], text=True, timeout=8).splitlines()[0]
            subprocess.run(['xdotool', 'windowfocus', '--sync', window], check=True)
            wait(geometry.exists, 'fixture did not export widget geometry')
            bounds = json.loads(geometry.read_text())
            window_geometry = dict(line.split('=', 1) for line in subprocess.check_output(
                ['xdotool', 'getwindowgeometry', '--shell', window], text=True).splitlines() if '=' in line)
            def gtk_center(rect):
                assert rect['width'] > 0 and rect['height'] > 0, 'GTK widget has no bounds'
                return (round(int(window_geometry['X']) + rect['x'] + rect['width'] / 2),
                        round(int(window_geometry['Y']) + rect['y'] + rect['height'] / 2))
            report['window_geometry'] = window_geometry
            report['gtk_anchor_bounds'] = bounds['anchor']
            report['gtk_anchor_center'] = gtk_center(bounds['anchor'])
            def reach(item):
                for _ in range(32):
                    if item.getState().contains(pyatspi.STATE_FOCUSED):
                        return
                    key('Tab')
                raise AssertionError(f'keyboard cannot reach {item.name}')
            reach(outside)
            time.sleep(.6)
            assert not marker.exists(), 'closed anchor must not read'
            report['anchor_center'] = list(point(anchor))
            move_point(gtk_center(bounds['anchor']))
            report['pointer_after_hover'] = subprocess.check_output(['xdotool', 'getmouselocation', '--shell'], text=True).strip()
            refresh = wait(lambda: button('Refresh usage'), 'hover did not open usage popup')
            wait(lambda: marker.exists() and marker.read_text() == 'x', 'hover did not trigger one read')
            assert outside.getState().contains(pyatspi.STATE_FOCUSED), 'hover must not steal focus'
            wait(popup_geometry.exists, 'fixture did not export popup geometry')
            popup_bounds = json.loads(popup_geometry.read_text())
            report['gtk_popup_bounds'] = popup_bounds
            report['x11_windows_before_travel'] = subprocess.check_output(
                ['xwininfo', '-root', '-tree'], text=True)
            move_point(gtk_center(popup_bounds))
            report['x11_pointer_child'] = x11_pointer_child()
            report['pointer_after_travel'] = subprocess.check_output(
                ['xdotool', 'getmouselocation', '--shell'], text=True).strip()
            time.sleep(.2)
            report['x11_windows_after_travel'] = subprocess.check_output(
                ['xwininfo', '-root', '-tree'], text=True)
            time.sleep(.45)
            assert button('Refresh usage') is not None, 'pointer travel closed popup'
            report['checks'].append('hover opens without stealing focus and pointer travel stays open')
            move_point(gtk_center(bounds['outside']))
            wait(lambda: button('Refresh usage') is None, 'unpinned popup did not close after pointer left')
            assert marker.read_text() == 'x', 'hover close must not duplicate the read'
            report['checks'].append('unpinned popup closes after pointer leaves')
            move_point(gtk_center(bounds['anchor']))
            wait(lambda: button('Refresh usage'), 'second hover did not reopen popup')
            wait(lambda: marker.read_text() == 'xx', 'second hover did not issue one read')
            subprocess.run(['xdotool', 'click', '1'], check=True)
            move_point(gtk_center(bounds['outside']))
            time.sleep(.65)
            assert button('Refresh usage') is not None, 'hover-to-click did not pin popup'
            assert marker.read_text() == 'xx', 'pinning must not duplicate read'
            subprocess.run(['xdotool', 'click', '1'], check=True)
            wait(lambda: button('Refresh usage') is None, 'outside click did not dismiss popup')
            time.sleep(.6)
            assert marker.read_text() == 'xx', 'closed popup must not poll'
            report['checks'].append('hover-to-click pins; outside click dismisses; closed popup does not poll')
            reach(anchor)
            key('space')
            wait(lambda: button('Refresh usage'), 'keyboard activation did not open popup')
            wait(lambda: marker.read_text() == 'xxx', 'keyboard opening did not issue one read')
            key('Escape')
            wait(lambda: button('Refresh usage') is None, 'Escape did not dismiss popup')
            wait(lambda: anchor.getState().contains(pyatspi.STATE_FOCUSED), 'Escape did not restore anchor focus')
            report['checks'].append('keyboard activation opens; Escape dismisses and restores focus')
            key('ctrl+q')
            process.wait(timeout=8)
            assert process.returncode == 0, process.returncode
            report['checks'].append('Ctrl+Q closes cleanly')
            report['result'] = 'pass'
        except Exception as error:
            report['result'] = 'fail'
            report['failure'] = str(error)
            report['synthetic_read_count'] = len(marker.read_text()) if marker.exists() else 0
            if app is not None:
                try:
                    report['accessible_nodes'] = [{'name': (item.name or '')[:160], 'role': item.getRoleName(),
                        'showing': item.getState().contains(pyatspi.STATE_SHOWING)} for item in walk(app)][:100]
                except Exception as diagnostic_error:
                    report['accessibility_diagnostic_error'] = type(diagnostic_error).__name__
            raise
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()
            stdout.seek(0); stderr.seek(0)
            report['process_stdout'] = stdout.read()[-8000:].decode(errors='replace')
            report['process_stderr'] = stderr.read()[-8000:].decode(errors='replace')
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps(report, indent=2))
