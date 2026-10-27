#!/usr/bin/env python3
"""Same external screenshot/quit probe for SDK and handwritten baseline apps."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

assert os.environ.get('DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
binary, screenshot = sys.argv[1:]
started = time.monotonic()
process = subprocess.Popen([binary])
try:
    window = None
    deadline = started + 10
    while time.monotonic() < deadline:
        result = subprocess.run(['xdotool', 'search', '--onlyvisible', '--pid', str(process.pid)], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            window = result.stdout.splitlines()[0]
            break
        if process.poll() is not None:
            break
        time.sleep(.04)
    if window is None:
        raise RuntimeError('no mapped app window within 10 seconds')
    mapped = time.monotonic() - started
    subprocess.run(['xdotool', 'windowsize', '--sync', window, '900', '600'], check=True)
    time.sleep(.5)
    subprocess.run(['import', '-window', window, screenshot], check=True, timeout=5)
    captured = time.monotonic() - started
    subprocess.run(['xdotool', 'windowfocus', '--sync', window], check=True)
    subprocess.run(['xdotool', 'key', '--clearmodifiers', 'ctrl+q'], check=True)
    try:
        process.wait(timeout=3)
        clean = process.returncode == 0
    except subprocess.TimeoutExpired:
        clean = False
    print(json.dumps({'mapped_seconds': mapped, 'captured_seconds': captured,
                      'frame_bytes': Path(screenshot).stat().st_size,
                      'ctrl_q_clean': clean, 'content_review': 'pending independent review'}))
    if not clean:
        raise SystemExit(1)
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
