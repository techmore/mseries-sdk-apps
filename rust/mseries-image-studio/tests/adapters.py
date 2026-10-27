"""Offline tests for download integrity and admission; no models or engines run."""
import unittest, tempfile, json, hashlib, io, runpy, sys, contextlib
from pathlib import Path
from unittest.mock import patch
from types import SimpleNamespace
SRC=Path(__file__).resolve().parents[1]/'src'
class Adapters(unittest.TestCase):
 def download(self,root,content,sha):
  info={'sha':'revision','siblings':[{'rfilename':'model.gguf','lfs':{'size':len(content),'sha256':sha}}]}
  responses=iter([io.BytesIO(json.dumps(info).encode()),io.BytesIO(content)])
  with patch('shutil.disk_usage',return_value=SimpleNamespace(free=20*1024**3)),patch.object(sys,'argv',['download','repo','model.gguf',str(root)]),patch('urllib.request.urlopen',side_effect=lambda *a,**k:next(responses)),contextlib.redirect_stdout(io.StringIO()):
   runpy.run_path(str(SRC/'download_model.py'),run_name='__main__')
 def test_verified_download_records_revision_and_hash(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);content=b'test model';sha=hashlib.sha256(content).hexdigest();self.download(root,content,sha)
   self.assertEqual((root/'model.gguf').read_bytes(),content)
   self.assertEqual(json.loads((root/'model.gguf.json').read_text())['sha256'],sha)
 def test_corrupt_download_is_not_installed(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp)
   with self.assertRaises(SystemExit):self.download(root,b'bad','0'*64)
   self.assertFalse((root/'model.gguf').exists());self.assertFalse((root/'model.gguf.part').exists())
 def test_local_admission_refuses_low_memory_without_starting_worker(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);(root/'models').mkdir();model=root/'models/stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf';model.write_bytes(b'fixture')
   recipe=dict(model_id=str(model),lora_id='',width=512,height=512,batch_size=1,prompt='test')
   original=Path.read_text
   def read(path,*a,**k):return 'MemAvailable: 1000 kB\n' if str(path)=='/proc/meminfo' else original(path,*a,**k)
   with patch.object(sys,'argv',['local',str(root),json.dumps(recipe)]),patch.object(Path,'read_text',read),patch('subprocess.Popen') as launch,contextlib.redirect_stderr(io.StringIO()):
    with self.assertRaises(SystemExit):runpy.run_path(str(SRC/'local_generation.py'),run_name='__main__')
    launch.assert_not_called()
   self.assertFalse((root/'generation.lock').exists())
   self.assertIn('9 GiB',json.loads((root/'current-job.json').read_text())['status'])
 def test_fixture_subprocess_reference_preview_arguments_and_fresh_metadata(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);(root/'models').mkdir();(root/'engine').mkdir()
   model=root/'models/stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf';model.write_bytes(b'fixture weights')
   png=(SRC.parent/'tests/fixtures/tiny.png').read_bytes()
   reference=root/'portrait.png';reference.write_bytes(png)
   vae=root/'models/vae-ft-mse-840000-ema-pruned.safetensors';vae.write_bytes(b'fixture decoder')
   for asset in ['clip-vision-h-f16.gguf','ip-adapter-plus-face_sd15.safetensors','taesd.safetensors']:
    path=root/'models'/asset;path.write_bytes(b'fixture auxiliary weights');path.with_suffix(path.suffix+'.json').write_text(json.dumps({'sha256':'fixture only','source':'offline test'}))
   engine=root/'engine/sd-cli'
   engine.write_text("#!/usr/bin/env python3\nimport sys,time\nfrom pathlib import Path\na=sys.argv\np=a[a.index('--preview-path')+1]\nfor i in range(2):\n Path(p.replace('%03d',str(i).zfill(3))).write_bytes(bytes.fromhex("+repr(png.hex())+"))\n time.sleep(.45)\nPath(a[a.index('-o')+1]).write_bytes(bytes.fromhex("+repr(png.hex())+"))\n")
   engine.chmod(0o755)
   recipe=dict(model_id=str(model),lora_id='',width=512,height=512,batch_size=1,prompt='transport fixture, not an AI run',negative_prompt='',seed=-1,steps=2,cfg_scale=7,sampler='euler',reference_image=str(reference),reference_strength=.75,vae_path=str(vae),params_backend='disk',max_vram_gib=3,elapsed_seconds=9999,preview_count=9999,peak_memory_mib=9999,time_to_first_preview_seconds=9999,minimum_available_mib=9999,event_log_path='stale',engine_arguments=['stale'])
   original=Path.read_text
   def read(path,*a,**k):return 'MemAvailable: 12000000 kB\n' if str(path)=='/proc/meminfo' else original(path,*a,**k)
   original_write=Path.write_text;metadata_snapshots=[]
   def write(path,text,*a,**k):
    if path.parent==root/'images' and path.suffix=='.json':metadata_snapshots.append(json.loads(text))
    return original_write(path,text,*a,**k)
   with patch.object(sys,'argv',['local',str(root),json.dumps(recipe)]),patch.object(Path,'read_text',read),patch.object(Path,'write_text',write):
    runpy.run_path(str(SRC/'local_generation.py'),run_name='__main__')
   for key in ['elapsed_seconds','peak_memory_mib','preview_count','time_to_first_preview_seconds','minimum_available_mib','event_log_path','engine_arguments']:
    self.assertNotIn(key,metadata_snapshots[0],f'Previous run metric survived admission: {key}')
   job=json.loads((root/'current-job.json').read_text());saved=json.loads(Path(job['recipe_path']).read_text())
   self.assertFalse(job['running']);self.assertEqual(saved['preview_count'],2);self.assertGreaterEqual(saved['seed'],0)
   self.assertIsNotNone(saved['time_to_first_preview_seconds']);self.assertTrue(Path(saved['image_path']).is_file())
   args=saved['engine_arguments']
   def argument(flag):return args[args.index(flag)+1]
   self.assertEqual(argument('--preview'),'tae');self.assertEqual(argument('--preview-interval'),'3')
   self.assertIn('--taesd-preview-only',args);self.assertEqual(argument('--vae'),str(vae))
   self.assertEqual(argument('--params-backend'),'disk');self.assertEqual(float(argument('--max-vram')),3)
   self.assertEqual(argument('--backend'),'diffusion=vulkan0,te=cpu,clip_vision=cpu,vae=cpu')
   self.assertEqual(argument('--clip_vision'),str(root/'models/clip-vision-h-f16.gguf'))
   self.assertEqual(argument('--ip-adapter'),str(root/'models/ip-adapter-plus-face_sd15.safetensors'))
   self.assertEqual(float(argument('--ip-adapter-strength')),.75)
   digest=hashlib.sha256(png).hexdigest()
   self.assertEqual(saved['reference_sha256'],digest)
   preserved=Path(saved['reference_image']);self.assertEqual(preserved.parent,root/'references');self.assertEqual(preserved.read_bytes(),png)
   self.assertEqual(argument('--ip-adapter-image'),str(preserved))
   self.assertLess(saved['elapsed_seconds'],9999);self.assertLess(saved['time_to_first_preview_seconds'],9999)
   self.assertNotEqual(saved['event_log_path'],'stale');self.assertNotIn('stale',args)
   events=[json.loads(line) for line in Path(saved['event_log_path']).read_text().splitlines()]
   self.assertEqual(events[-1]['status'],'complete');self.assertTrue(any(event['event']=='preview' for event in events))
   self.assertFalse((root/'generation.lock').exists())
 def test_sweep_retains_fixed_seed_and_separate_results(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);(root/'models').mkdir();(root/'images').mkdir()
   for name in ['stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf','stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf']:(root/'models'/name).write_bytes(b'fixture')
   code="import sys,json,time;from pathlib import Path;b=Path(sys.argv[1]);r=json.loads(sys.argv[2]);p=b/'images'/('fixture-'+str(time.time_ns())+'.json');r['image_path']=str(p.with_suffix('.png'));p.with_suffix('.png').write_bytes(bytes.fromhex("+repr((SRC.parent/'tests/fixtures/tiny.png').read_bytes().hex())+"));p.write_text(json.dumps(r));(b/'current-job.json').write_text(json.dumps({'recipe_path':str(p)}))"
   recipe=dict(seed=424242,prompt='fixture')
   with patch.object(sys,'argv',['sweep',str(root),json.dumps(recipe),json.dumps({'kind':'models','review':False}),code,'']):
    runpy.run_path(str(SRC/'sweep_runner.py'),run_name='__main__')
   manifest=json.loads(Path((root/'latest-sweep.txt').read_text()).read_text())
   self.assertEqual(manifest['status'],'Complete');self.assertEqual(len(manifest['runs']),2)
   self.assertEqual([run['settings']['seed'] for run in manifest['runs']],[424242,424242])
   self.assertNotEqual(manifest['runs'][0]['recipe_path'],manifest['runs'][1]['recipe_path'])
   self.assertFalse((root/'batch.lock').exists())
if __name__=='__main__':unittest.main()
