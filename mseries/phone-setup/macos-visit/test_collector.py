"""Mock native tools to exercise completion gates; this is not a macOS restore test."""
import base64
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('Collect-Everything.command')


class CollectorTests(unittest.TestCase):
    def run_case(self, mode='ok', twice=False):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            home = root / 'home'
            home.mkdir()
            drive = root / 'external'
            drive.mkdir()
            bindir = root / 'bin'
            bindir.mkdir()
            # Substitute platform executables only in the sandbox copy.
            script = SCRIPT.read_text().replace('export PATH="/usr/bin:/bin:/usr/sbin:/sbin:$PATH"',
                                                 'export PATH="$MOCK_BIN:/usr/bin:/bin"')
            script = script.replace('/usr/libexec/PlistBuddy', 'PlistBuddy').replace('/usr/bin/osascript', 'osascript')
            target = root / 'collector.sh'
            target.write_text(script)
            mock = bindir / 'mock'
            mock.write_text('''#!/usr/bin/env python3
import base64, hashlib, json, os, pathlib, subprocess, sys, tarfile
name=pathlib.Path(sys.argv[0]).name
args=sys.argv[1:]; mode=os.environ['CASE_MODE']
stage=pathlib.Path(os.environ['HOME'])/'Library/Application Support/MSeriesMacVisit'
if name=='uname': print('Darwin' if args[0]=='-s' else 'arm64')
elif name=='sw_vers': print('15.7')
elif name=='caffeinate' or name=='open' or name=='sync': pass
elif name=='df': print('Filesystem 1024-blocks Used Available Capacity Mounted on\\nx 999999999 0 999999999 0% /')
elif name=='diskutil': print('fixture-volume')
elif name=='PlistBuddy': print('exfat' if 'FilesystemType' in args[1] else 'false')
elif name=='osascript':
 if '-e' in args: print(os.environ['MOCK_DRIVE']+'/')
 else:
  sys.stdin.read()
  c=json.loads(pathlib.Path(args[-1]).read_text())
  if not c.get('machineId') or not c.get('hardwareModel') or c['storage'][0]['file']!='disk.img': sys.exit(1)
  print('Config verified')
elif name=='pbpaste':
 print(base64.b64encode((b'OABS\\0' if mode!='bad-hardware' else b'WRONG')+b'x'*120).decode())
elif name=='base64':
 try: sys.stdout.buffer.write(base64.b64decode(sys.stdin.buffer.read(),validate=False))
 except Exception: sys.exit(1)
elif name=='curl':
 out=pathlib.Path(args[args.index('-o')+1]); url=next(x for x in args if x.startswith('https:'))
 if 'macosvm' in url:
  tool=out.parent/'mock-vm'
  tool.write_text('#!/bin/bash\\nexec "$MOCK_BIN/macosvm" "$@"\\n'); tool.chmod(0o700)
  with tarfile.open(out,'w:gz') as t: t.add(tool,arcname='macosvm')
 else: out.write_bytes(b'fixture')
elif name=='unzip':
 dest=pathlib.Path(args[-1]); (dest/'Hardware.app').mkdir()
elif name=='macosvm':
 with (stage/'calls').open('a') as f: f.write(' '.join(args)+'\\n')
 if '-g' in args and mode=='bad-native-startup': sys.exit(1)
 if '--restore' in args:
  if mode=='bad-restore': sys.exit(1)
  pathlib.Path('disk.img').write_bytes(b'disk fixture')
  pathlib.Path('aux.img').write_bytes(b'x'*16400)
  pathlib.Path('vm.json').write_text(json.dumps({'machineId':'identity','hardwareModel':'model','storage':[{'type':'disk','file':'bad.img' if mode=='bad-config' else 'disk.img'},{'type':'aux','file':'aux.img'}]}))
elif name=='stat':
 p=pathlib.Path(args[-1]); print(68719476736 if p.name=='disk.img' else p.stat().st_size)
elif name=='shasum':
 if args[1]=='1': print('a1675f2c8412122a5e796981571b0269a966708e  image')
 elif '-c' in args and args[-1]=='-': sys.stdin.read()
 else: sys.exit(subprocess.call(['/usr/bin/shasum']+args))
elif name=='rsync':
 import shutil
 src=pathlib.Path(args[-2]); dst=pathlib.Path(args[-1]); shutil.copyfile(src,dst)
 if mode=='bad-transfer' and dst.name=='disk.img': dst.write_bytes(b'corrupt')
else: raise RuntimeError(name)
''')
            mock.chmod(0o755)
            for name in ('uname', 'sw_vers', 'caffeinate', 'open', 'sync', 'df', 'diskutil',
                         'PlistBuddy', 'osascript', 'pbpaste', 'base64', 'curl', 'unzip',
                         'macosvm', 'stat', 'shasum', 'rsync'):
                (bindir / name).symlink_to(mock)
            # Replace only the native read-only firmware input in the test copy.
            fw = root / 'firmware.bin'
            fw.write_bytes(b'firmware fixture')
            target.write_text(target.read_text().replace(
                '/System/Library/Frameworks/Virtualization.framework/Resources/AVPBooter.vmapple2.bin', str(fw)))
            env = dict(os.environ, HOME=str(home), MOCK_BIN=str(bindir), MOCK_DRIVE=str(drive), CASE_MODE=mode)
            def execute():
                return subprocess.run(['bash', str(target)], env=env, input='\nYES\n\n',
                                      text=True, capture_output=True, timeout=30)
            result = execute()
            receipt = drive/'MSeries-Linux-Return/COLLECTION-COMPLETE.txt'
            if mode == 'ok':
                self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
                self.assertTrue(receipt.is_file())
                self.assertNotIn(base64.b64encode(b'OABS\0'+b'x'*120).decode(), result.stdout)
                if twice:
                    result = execute()
                    self.assertEqual(result.returncode, 0, result.stdout+result.stderr)
                    calls = (home/'Library/Application Support/MSeriesMacVisit/calls').read_text().splitlines()
                    self.assertEqual(len(calls), 2, 'resume must not repeat restore or native startup')
            else:
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(receipt.exists(), result.stdout+result.stderr)

    def test_complete_and_resume(self): self.run_case(twice=True)
    def test_invalid_hardware_stops(self): self.run_case('bad-hardware')
    def test_restore_failure_stops(self): self.run_case('bad-restore')
    def test_native_startup_failure_stops(self): self.run_case('bad-native-startup')
    def test_config_failure_stops(self): self.run_case('bad-config')
    def test_corrupt_external_copy_stops(self): self.run_case('bad-transfer')


if __name__ == '__main__': unittest.main()
