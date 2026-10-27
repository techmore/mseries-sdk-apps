"""Reproducible user-local engine installation; never modifies desktop services."""
from pathlib import Path
import subprocess,sys,shutil,json,os
os.umask(0o077)
base=Path(sys.argv[1]);source=base/'engine-source';target=base/'engine'
revision='fd21ecfceef2665d2c5f2db8536e3202d864fd30'
base.mkdir(parents=True,exist_ok=True)
lock=base/'generation.lock'
owned=False
try:
    if (base/'batch.lock').exists():raise RuntimeError('A sweep is active; wait before compiling.')
    fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600);os.write(fd,str(os.getpid()).encode());os.close(fd);owned=True
    available=int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:')))*1024
    if available<9*1024**3:raise RuntimeError('Engine compilation needs 9 GiB available including the desktop reserve.')
    if shutil.disk_usage(base).free<5*1024**3:raise RuntimeError('Keep at least 5 GiB free before building the engine.')
    if not source.exists():
        subprocess.run(['git','clone','--no-checkout','https://github.com/leejet/stable-diffusion.cpp.git',str(source)],check=True)
        subprocess.run(['git','-C',str(source),'checkout','--detach',revision],check=True)
    actual=subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD'],text=True).strip()
    if actual!=revision:raise RuntimeError('The existing engine source uses another revision; it will not be overwritten.')
    subprocess.run(['git','-C',str(source),'diff','--quiet'],check=True)
    subprocess.run(['git','-C',str(source),'submodule','update','--init','--depth','1','ggml','thirdparty/libwebm','thirdparty/libwebp'],check=True)
    subprocess.run(['cmake','-S',str(source),'-B',str(source/'build'),'-DSD_VULKAN=ON','-DSD_BUILD_TESTS=OFF','-DCMAKE_BUILD_TYPE=Release'],check=True)
    subprocess.run(['cmake','--build',str(source/'build'),'--target','sd-cli','-j2'],check=True)
    target.mkdir(exist_ok=True)
    shutil.copy2(source/'build/bin/sd-cli',target/'sd-cli')
    shutil.copy2(source/'LICENSE',target/'LICENSE')
    (target/'source.json').write_text(json.dumps({'repository':'https://github.com/leejet/stable-diffusion.cpp','revision':revision,'backend':'Vulkan build','qualified':False},indent=2))
    print('Engine installed. Model and memory qualification are still required.')
except Exception as error:print('Engine setup failed: '+str(error),file=sys.stderr);sys.exit(1)

finally:
    if owned:lock.unlink(missing_ok=True)
