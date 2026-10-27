#!/usr/bin/env python3
"""Build/install Image Studio and its native Applications launcher for this user."""
from pathlib import Path
import os,shutil,subprocess,sys
repo=Path(__file__).resolve().parents[2]
source=repo/'rust/mseries-image-studio/data'
data=Path(os.environ.get('XDG_DATA_HOME',str(Path.home()/'.local/share')))
bin_dir=Path.home()/'.local/bin'
libexec=Path.home()/'.local/libexec/mseries-image-studio'
target=Path(os.environ.get('CARGO_TARGET_DIR',str(Path.home()/'.cache/cargo-target')))
if '--no-build' not in sys.argv:
    subprocess.run(['cargo','build','--manifest-path',str(repo/'rust/Cargo.toml'),'-p','mseries-image-studio'],env=dict(os.environ,CARGO_TARGET_DIR=str(target)),check=True)
for directory in [bin_dir,libexec,data/'applications',data/'icons/hicolor/scalable/apps',data/'metainfo',data/'dbus-1/services']:
    directory.mkdir(parents=True,exist_ok=True)
shutil.copy2(target/'debug/mseries-image-studio',libexec/'mseries-image-studio.new')
os.replace(libexec/'mseries-image-studio.new',libexec/'mseries-image-studio')
# A stable executable path keeps desktop and D-Bus activation independent of builds.
wrapper=bin_dir/'mseries-image-studio'
wrapper.write_text('#!/bin/sh\nexport GSK_RENDERER="${GSK_RENDERER:-cairo}"\nexec '+"'"+str(libexec/'mseries-image-studio').replace("'","'\\''")+"'"+' "$@"\n')
wrapper.chmod(0o755)
quoted='"'+str(wrapper).replace('\\','\\\\').replace('"','\\"').replace('`','\\`').replace('$','\\$')+'"'
desktop=(source/'org.elementary.mseries.ImageStudio.desktop').read_text().replace('Exec=mseries-image-studio','Exec='+quoted)
(data/'applications/org.elementary.mseries.ImageStudio.desktop').write_text(desktop)
(data/'dbus-1/services/org.elementary.mseries.ImageStudio.service').write_text('[D-BUS Service]\nName=org.elementary.mseries.ImageStudio\nExec='+quoted+'\n')
shutil.copy2(source/'org.elementary.mseries.ImageStudio.svg',data/'icons/hicolor/scalable/apps/org.elementary.mseries.ImageStudio.svg')
shutil.copy2(source/'org.elementary.mseries.ImageStudio.metainfo.xml',data/'metainfo/org.elementary.mseries.ImageStudio.metainfo.xml')
for command,argument in [('update-desktop-database',data/'applications'),('gtk-update-icon-cache',data/'icons/hicolor')]:
    binary=shutil.which(command)
    if binary:subprocess.run([binary,str(argument)],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=False)
runtime=data/'mseries-image-studio/runtime';runtime.mkdir(parents=True,exist_ok=True)
shutil.copy2(repo/'rust/mseries-image-studio/src/run_logging.py',runtime/'run_logging.py')
print('Installed Image Studio in Applications with its icon and D-Bus launcher.')
