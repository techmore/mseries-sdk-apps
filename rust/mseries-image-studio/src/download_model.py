"""Pinned Hugging Face downloads with integrity receipts and progress."""
import sys,json,urllib.request,hashlib,os,shutil,time
from pathlib import Path
os.umask(0o077)
repo,name,directory=sys.argv[1:]
dest=Path(directory);dest.mkdir(parents=True,exist_ok=True)
part,final=dest/(name+'.part'),dest/name
created=False
progress=dest/'download-progress.json'

def publish(phase,received=0,total=0):
    temp=progress.with_suffix('.tmp')
    temp.write_text(json.dumps(dict(file=name,phase=phase,received=received,total=total)))
    os.replace(temp,progress)

try:
    if Path(name).name!=name:raise RuntimeError('Invalid model filename.')
    if (dest/'download.cancel').exists():raise RuntimeError('Download cancelled.')
    publish('Reading pinned model metadata')
    with urllib.request.urlopen('https://huggingface.co/api/models/'+repo+'?blobs=true',timeout=30) as response:info=json.load(response)
    entry=next(x for x in info['siblings'] if x['rfilename']==name)
    lfs=entry['lfs'];size=lfs['size'];expected=lfs['sha256']
    if final.exists():
        publish('Verifying existing file',0,size)
        with final.open('rb') as stream:actual=hashlib.file_digest(stream,'sha256').hexdigest()
        if final.stat().st_size!=size or actual!=expected:raise RuntimeError('The installed file does not match the source digest; it will not be overwritten.')
        publish('Already installed and verified',size,size)
        print('Already installed and verified.');sys.exit(0)
    if size>8*1024**3 or shutil.disk_usage(dest).free<size+2*1024**3:raise RuntimeError('Insufficient disk space with 2 GiB reserve')
    url='https://huggingface.co/'+repo+'/resolve/'+info['sha']+'/'+name
    digest=hashlib.sha256();received=0;last_update=0
    with urllib.request.urlopen(url,timeout=60) as response,part.open('xb') as stream:
        created=True
        while True:
            if (dest/'download.cancel').exists():raise RuntimeError('Download cancelled.')
            chunk=response.read(1024*1024)
            if not chunk:break
            stream.write(chunk);digest.update(chunk);received+=len(chunk)
            if received>size:raise RuntimeError('Remote file exceeded its declared size.')
            if time.monotonic()-last_update>1:
                publish('Downloading',received,size);last_update=time.monotonic()
    if received!=size or digest.hexdigest()!=expected:raise RuntimeError('Download integrity check failed')
    os.rename(part,final)
    final.with_suffix(final.suffix+'.json').write_text(json.dumps({'repo':repo,'revision':info['sha'],'sha256':expected,'size':size,'qualified':False},indent=2))
    publish('Downloaded and verified',size,size)
    print('Downloaded and verified. Inference qualification is still required.')
except Exception as error:
    if created:part.unlink(missing_ok=True)
    publish('Download unavailable: '+str(error))
    print('Download failed: '+str(error));sys.exit(1)
