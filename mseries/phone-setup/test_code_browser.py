"""Optional real Chromium/Brave bridge test on a private bus/display/profile."""
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import time
import unittest
from urllib.request import urlopen

import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
from test_code_fields_gui import find, settle


@unittest.skipUnless(os.environ.get('MSERIES_PRIVATE_CODE_TEST') == '1', 'Requires isolated D-Bus and Xvfb')
@unittest.skipUnless(os.environ.get('MSERIES_TEST_CODE_BROWSER') == '1', 'Browser check is opt-in')
class BrowserFieldTests(unittest.TestCase):
    def test_actual_browser_without_edit_api_offers_copy_without_submit(self):
        binary = shutil.which('brave-browser') or shutil.which('chromium')
        if not binary:
            self.skipTest('Chromium/Brave is not installed')
        from websockets.sync.client import connect
        from code_fields import describe, insert
        with tempfile.TemporaryDirectory(prefix='mseries-code-browser-') as folder:
            root = Path(folder)
            page = root / 'code.html'
            page.write_text('''<!doctype html><title>Private verification fixture</title>
              <form onsubmit="window.submits++;return false">
              <label>Verification code<input id="otp" autocomplete="one-time-code" maxlength="6"></label>
              <label>Password<input type="password" id="password"></label>
              <button>Continue</button></form>
              <script>window.inputs=0;window.submits=0;
              otp.oninput=()=>window.inputs++;otp.focus();</script>''')
            process = subprocess.Popen([binary, '--user-data-dir='+str(root/'profile'),
                '--no-first-run', '--no-default-browser-check', '--disable-sync',
                '--disable-background-networking', '--force-renderer-accessibility',
                '--ozone-platform=x11', '--remote-debugging-port=0', page.as_uri()],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            try:
                port_file = root/'profile'/'DevToolsActivePort'
                for _ in range(100):
                    if port_file.exists(): break
                    settle(.1)
                self.assertTrue(port_file.exists(), 'Private browser did not start')
                port = int(port_file.read_text().splitlines()[0])
                with urlopen(f'http://127.0.0.1:{port}/json/list', timeout=3) as response:
                    pages = json.load(response)
                target_page = next(p for p in pages if p.get('url') == page.as_uri())
                with connect(target_page['webSocketDebuggerUrl'], open_timeout=3) as socket:
                    counter = 0
                    def evaluate(expression):
                        nonlocal counter
                        counter += 1
                        socket.send(json.dumps(dict(id=counter, method='Runtime.evaluate', params=dict(expression=expression, returnByValue=True))))
                        while True:
                            result = json.loads(socket.recv(timeout=3))
                            if result.get('id') == counter:
                                self.assertNotIn('exceptionDetails', result.get('result', {}))
                                return result['result']['result'].get('value')
                    for _ in range(50):
                        if evaluate('!!document.getElementById("otp")'): break
                        settle(.1)
                    self.assertTrue(evaluate('!!document.getElementById("otp")'), 'Fixture page did not load')
                    evaluate('document.getElementById("otp").focus()')
                    entry = None
                    for _ in range(50):
                        settle(.1)
                        entry = find(Atspi.get_desktop(0), 'Verification code', predicate=lambda node: describe(node) is not None)
                        if entry and describe(entry): break
                    self.assertIsNotNone(entry, 'Browser did not expose code field')
                    target = describe(entry)
                    self.assertIsNotNone(target)
                    # Chromium currently exports Text, not EditableText. Do not
                    # pretend filling works or silently inject global keystrokes.
                    self.assertFalse(target['can_insert'])
                    with self.assertRaises(RuntimeError): insert(target, '246810')
                    from datetime import datetime, timezone
                    from verification_codes import Codes
                    app = Codes(fixture=True)
                    app.register();app.initialize();app.target=target
                    try:
                        app.registry.update([dict(handle='fixture', timestamp=datetime.now(timezone.utc).isoformat(), body='Your verification code is 246810.')])
                        app.render(True)
                        self.assertFalse(app.use.get_visible())
                        self.assertEqual(app.window.get_default_widget(), app.copy)
                        app.copy.emit('clicked')
                        self.assertFalse(app.window.get_visible())
                        copied=[]
                        app.window.get_clipboard().read_text_async(None, lambda clipboard,result: copied.append(clipboard.read_text_finish(result)))
                        for _ in range(20):
                            settle(.05)
                            if copied: break
                        self.assertEqual(copied,['246810'])
                    finally:
                        app.window.destroy();app.pool.shutdown(wait=True, cancel_futures=True);app.quit()
                    self.assertEqual(evaluate('otp.value'), '')
                    self.assertEqual(evaluate('window.inputs'), 0)
                    self.assertEqual(evaluate('window.submits'), 0)
                    evaluate('password.focus()');settle(.1)
                    password = find(Atspi.get_desktop(0), 'Password', predicate=lambda node: node.get_role() == Atspi.Role.PASSWORD_TEXT)
                    self.assertIsNotNone(password)
                    self.assertIsNone(describe(password))
            finally:
                os.killpg(process.pid, signal.SIGTERM)
                try: process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL);process.wait(timeout=3)


if __name__ == '__main__':
    unittest.main()
