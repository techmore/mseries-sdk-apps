#!/usr/bin/python3
"""Synthetic-only suite with private home, settings, D-Bus and X display."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--browser',action='store_true',help='Also launch an isolated Chromium/Brave profile (requires Python websockets)')
parser.add_argument('--screenshots',type=Path)
args=parser.parse_args()
here=Path(__file__).resolve().parent
with tempfile.TemporaryDirectory(prefix='mseries-code-check-') as folder:
    root=Path(folder)
    env=dict(os.environ)
    for key in ('AT_SPI_BUS_ADDRESS','GNOME_KEYRING_CONTROL'): env.pop(key,None)
    for key,name in [('HOME','home'),('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_CACHE_HOME','cache'),('XDG_RUNTIME_DIR','runtime')]:
        path=root/name;path.mkdir(mode=0o700);env[key]=str(path)
    # Expose only accessibility activation, not keyring/document/desktop portals.
    services=root/'services';services.mkdir()
    service=Path('/usr/share/dbus-1/services/org.a11y.Bus.service')
    shutil.copyfile(service, services/service.name)
    bus=root/'bus.conf'
    bus.write_text('''<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN" "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen><auth>EXTERNAL</auth>
<servicedir>'''+str(services)+'''</servicedir><policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>''')
    env.update(MSERIES_PRIVATE_CODE_TEST='1',MSERIES_TEST_CODE_BROWSER='1' if args.browser else '0',
               GDK_BACKEND='x11',GSK_RENDERER='cairo',GTK_A11Y='atspi',GIO_USE_VFS='local',
               GSETTINGS_BACKEND='memory',PYTHONWARNINGS='ignore::DeprecationWarning')
    if args.screenshots:
        env['MSERIES_CODE_SCREENSHOTS']=str(args.screenshots.resolve())
    result=subprocess.run(['dbus-run-session','--config-file='+str(bus),'--','xvfb-run','-a',
                           '/usr/bin/python3','-m','unittest','discover','-s',str(here),'-p','test_code*.py'],env=env,timeout=120)
raise SystemExit(result.returncode)
