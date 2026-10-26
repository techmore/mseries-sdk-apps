"""Native adapter boundaries, using private config and inert commands only."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time
import unittest

ADAPTER = Path(__file__).resolve().parents[1] / 'bin/mseries-fleet'

class NativeAdapterTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.root = Path(self.folder.name)
        self.env = os.environ | {'HOME': str(self.root), 'XDG_CONFIG_HOME': str(self.root/'config')}
    def tearDown(self):
        self.folder.cleanup()
    def run_adapter(self, *args):
        return subprocess.run([str(ADAPTER), *args], env=self.env, capture_output=True, text=True, timeout=5)
    def test_settings_are_explicit_and_preserve_alias_boundaries(self):
        self.assertEqual(self.run_adapter('list').returncode, 0)
        self.assertEqual(self.run_adapter('add','Build server','build-server').returncode, 0)
        bad = self.run_adapter('add','bad','-oProxyCommand=x')
        self.assertNotEqual(bad.returncode, 0)
        self.assertNotEqual(self.run_adapter('add','duplicate','build-server').returncode, 0)
        result = json.loads(self.run_adapter('list').stdout)
        self.assertEqual([c['id'] for c in result['connectors']], ['local','build-server'])
        path = self.root/'config/fleet-shepherd/connectors.json'
        self.assertEqual(path.stat().st_mode & 0o777,0o600)
        self.assertEqual(self.run_adapter('remove','build-server').returncode,0)
    def test_symlink_destination_is_not_written(self):
        parent=self.root/'config/fleet-shepherd'
        parent.mkdir(parents=True)
        victim=self.root/'victim'
        victim.write_text('{"schemaVersion":1,"connectors":[]}')
        (parent/'connectors.json').symlink_to(victim)
        self.assertNotEqual(self.run_adapter('add','Build','build').returncode,0)
        self.assertEqual(victim.read_text(),'{"schemaVersion":1,"connectors":[]}')
    def test_concurrent_additions_do_not_lose_a_machine(self):
        jobs=[subprocess.Popen([str(ADAPTER),'add',name,name],env=self.env,stdout=subprocess.PIPE) for name in ['alpha','beta']]
        for job in jobs:
            job.communicate(timeout=5)
            self.assertEqual(job.returncode,0)
        result=json.loads(self.run_adapter('list').stdout)
        self.assertEqual({c['id'] for c in result['connectors']},{'local','alpha','beta'})
    def test_cancel_terminates_owned_probe_group(self):
        tools=self.root/'tools';tools.mkdir()
        pidfile=self.root/'probe-pid'
        herdr=tools/'herdr'
        herdr.write_text('#!/bin/sh\necho $$ > "$PROBE_PID"\nexec sleep 60\n')
        herdr.chmod(0o755)
        env=self.env|{'PATH':str(tools)+':'+os.environ['PATH'],'PROBE_PID':str(pidfile)}
        process=subprocess.Popen([str(ADAPTER),'snapshot'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        try:
            deadline=time.monotonic()+5
            while not pidfile.exists():
                self.assertIsNone(process.poll())
                self.assertLess(time.monotonic(),deadline)
                time.sleep(.02)
            child=int(pidfile.read_text())
            process.send_signal(signal.SIGTERM)
            process.wait(timeout=5)
            with self.assertRaises(ProcessLookupError): os.kill(child,0)
        finally:
            if process.poll() is None: process.kill();process.wait()
