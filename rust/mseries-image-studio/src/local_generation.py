"""Supervised single-image engine adapter, with genuine intermediate previews."""
import json, sys, os, time, subprocess, resource, secrets, signal, hashlib, math
from pathlib import Path
os.umask(0o077)


def supervised_child():
    import ctypes, signal, os
    libc=ctypes.CDLL(None,use_errno=True)
    if libc.prctl(1,signal.SIGTERM,0,0,0)!=0 or os.getppid()==1:
        os._exit(127)


base = Path(sys.argv[1])
recipe = json.loads(sys.argv[2])
# A repeated recipe carries settings, never the previous run's measurements.
for key in ('image_path', 'elapsed_seconds', 'peak_memory_mib', 'preview_count',
            'time_to_first_preview_seconds', 'minimum_available_mib',
            'sampling_seconds', 'post_sampling_seconds', 'decoder_seconds',
            'engine_arguments', 'event_log_path', 'logging_finalization_error'):
    recipe.pop(key, None)
recipe['created_at_unix'] = int(time.time())
for key, value in dict(vae_on_cpu=True, vae_tile_size=256, cpu_threads=4,
                       params_backend='disk', max_vram_gib=3.0, preview_interval=3,
                       reference_image='', reference_strength=0.8).items():
    recipe.setdefault(key,value)
job, lock = base / 'current-job.json', base / 'generation.lock'
started, preview_count, first_preview = time.monotonic(), 0, None
sample_step = 0
proc, owned = None, False
decode_started = None
images = base / 'images'
images.mkdir(parents=True, exist_ok=True)
name = 'local-' + str(time.time_ns())
output, metadata = images / (name + '.png'), images / (name + '.json')
preview_dir = base / 'previews' / name
preview_dir.mkdir(parents=True)
cancel = base / 'cancel.request'
import runpy, re
logging_file=base/'runtime/run_logging.py'
if not logging_file.exists() and '__file__' in globals():logging_file=Path(__file__).with_name('run_logging.py')
if 'RunLogger' not in globals():
    RunLogger=runpy.run_path(str(logging_file))['RunLogger']
logger=RunLogger(base,name)



def available():
    return int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines()
                    if x.startswith('MemAvailable:'))) * 1024


def publish(status, running, image='', **extra):
    data = dict(status=status, running=running,
                engine='stable-diffusion.cpp · local Vulkan build · backend details in run log',
                prompt=recipe['prompt'], image_path=str(image), recipe_path=str(metadata),
                width=recipe.get('width'), height=recipe.get('height'), seed=recipe.get('seed'),
                cfg_scale=recipe.get('cfg_scale'), preview_method=recipe.get('preview_method','proj'),
                event_log_path=str(logger.path), preview_count=preview_count, step=sample_step, steps=recipe.get('steps',20), model=str(recipe.get('model_id','')), sampler=recipe.get('sampler','euler'), scheduler=recipe.get('scheduler','discrete'), elapsed_seconds=round(time.monotonic()-started, 2), **extra)
    temp = job.with_suffix('.tmp')
    temp.write_text(json.dumps(data))
    os.replace(temp, job)


def stop_child():
    if proc and proc.poll() is None:
        os.killpg(proc.pid, signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()


try:
    batch = base / 'batch.lock'
    if batch.exists() and batch.read_text() != os.environ.get('MSERIES_SWEEP_ID', ''):
        raise RuntimeError('A sweep is active; wait for it or cancel it first.')
    fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    os.write(fd, str(os.getpid()).encode())
    os.close(fd)
    owned = True
    if cancel.exists():
        raise RuntimeError('Cancelled.')
    model = Path(recipe['model_id']).resolve()
    models = {
        'stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf': 3.0,
        'stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf': 3.5,
        'sd_xl_turbo_1.0_fp16.safetensors': 6.0,
        'Realistic_Vision_V6.0_NV_B1_fp16.safetensors': 4.0,
    }
    if model.name not in models or model.parent != (base/'models').resolve() or not model.is_file():
        raise RuntimeError('Choose an installed model from Models, then load its starter defaults.')
    if recipe.get('lora_id'):
        raise RuntimeError('LoRA rendering is not qualified yet. Clear the adapter for this run.')
    if recipe['width'] not in (512,768,1024) or recipe['height'] not in (512,768,1024) or recipe['batch_size'] != 1:
        raise RuntimeError('Use 512/768/1024 dimensions and one image at a time; Compare handles sequential batches.')
    if recipe.get('sampler', 'euler') not in ('euler', 'euler_a', 'dpm++2m', 'dpm++2m_sde'):
        raise RuntimeError('Select Euler, Euler ancestral, DPM++ 2M or DPM++ 2M SDE.')
    if not 1 <= recipe.get('steps', 20) <= 50 or not 0 <= recipe.get('cfg_scale', 7) <= 15:
        raise RuntimeError('Steps or guidance are outside the supported range.')
    if 'turbo' in model.name and (recipe.get('steps',20) > 4 or recipe.get('cfg_scale',7) != 1):
        raise RuntimeError('Load SDXL Turbo defaults: 1–4 steps, engine CFG 1, no negative prompt.')
    reference = recipe.get('reference_image', '')
    reference_weights = []
    if reference:
        if 'turbo' in model.name:
            raise RuntimeError('This reference adapter supports SD 1.5 models, not SDXL Turbo.')
        reference_path = Path(reference).expanduser().resolve()
        if not reference_path.is_file() or reference_path.stat().st_size > 32*1024**2:
            raise RuntimeError('Choose a reference image under 32 MiB.')
        from PIL import Image
        with Image.open(reference_path) as reference_pixels:
            if reference_pixels.format not in ('PNG', 'JPEG') or max(reference_pixels.size)>8192:
                raise RuntimeError('Reference must be a PNG or JPEG up to 8192 pixels per side.')
            reference_pixels.verify()
        reference_bytes = reference_path.read_bytes()
        digest = hashlib.sha256(reference_bytes).hexdigest()
        references = base/'references'; references.mkdir(exist_ok=True)
        preserved = references/(digest+reference_path.suffix.lower())
        if preserved.exists():
            if hashlib.sha256(preserved.read_bytes()).hexdigest()!=digest:
                raise RuntimeError('Saved reference failed its image hash check.')
        else:
            temporary=references/(digest+'.'+name+'.tmp')
            try:
                with temporary.open('xb') as snapshot:snapshot.write(reference_bytes)
                os.replace(temporary,preserved)
            finally:temporary.unlink(missing_ok=True)
        recipe['reference_image'] = str(preserved)
        recipe['reference_sha256'] = digest
        strength = float(recipe.get('reference_strength',0.8))
        if not math.isfinite(strength) or not 0 <= strength <= 2:raise RuntimeError('Reference strength must be between 0 and 2.')
        reference_weights = [base/'models/clip-vision-h-f16.gguf',base/'models/ip-adapter-plus-face_sd15.safetensors']
        if not all(p.is_file() for p in reference_weights):
            raise RuntimeError('Install the SD 1.5 Plus Face reference bundle first.')
        recipe['reference_receipts'] = [json.loads(p.with_suffix(p.suffix+'.json').read_text()) for p in reference_weights]
    params_backend = recipe.get('params_backend','disk')
    if params_backend not in ('disk','cpu','auto'):raise RuntimeError('Unsupported weight storage backend.')
    vram_budget = float(recipe.get('max_vram_gib',3.0))
    if not 2 <= vram_budget <= 8:raise RuntimeError('GPU budget must be between 2 and 8 GiB.')
    # Disk-backed weights avoid retaining a second source copy on unified memory.
    # This remains an estimate: the continuous reserve check is authoritative.
    working = max(vram_budget,3.0) if params_backend=='disk' else models[model.name]
    required = (6 + working + (0.5 if reference and params_backend=='disk' else 1.3 if reference else 0) + max(0,recipe['width']*recipe['height']/262144-1)*0.5) * 1024**3
    if available() < required:
        raise RuntimeError(f'This calibration needs {required/1024**3:g} GiB available, including the 6 GiB desktop reserve.')
    binary = base / 'engine/sd-cli'
    if not binary.is_file():
        raise RuntimeError('Install the local image engine using the documented engine setup.')
    if recipe['seed'] < 0:
        recipe['seed'] = secrets.randbelow(2147483647)
    recipe.update(engine_id='stable-diffusion.cpp local Vulkan build',
                  settings_note='Supervised local calibration. Intermediate images are projected latents. Peak memory is process RSS, excluding GPU allocations. Backend selection is retained in the run log.')
    tae = base/'models/taesd.safetensors'
    use_tae = tae.is_file() and 'turbo' not in model.name
    recipe['preview_method'] = 'tae' if use_tae else 'proj'
    recipe['preview_interval'] = int(recipe.get('preview_interval',3 if use_tae else 1))
    if not 1 <= recipe['preview_interval'] <= 10:raise RuntimeError('Preview interval must be between 1 and 10 steps.')
    recipe['settings_note'] = ('Intermediate previews use the approximate TAESD decoder; the final image uses the selected full VAE. ' if use_tae else 'Intermediate previews are projected latents. ') + 'Peak memory is process RSS, excluding GPU allocations. Backend selection is retained in the run log.'
    if use_tae:recipe['preview_receipt'] = json.loads(tae.with_suffix('.safetensors.json').read_text())
    receipt = model.with_suffix(model.suffix+'.json')
    if receipt.is_file():
        recipe['model_receipt'] = json.loads(receipt.read_text())
    source = base/'engine/source.json'
    if source.is_file():
        recipe['engine_receipt'] = json.loads(source.read_text())
    metadata.write_text(json.dumps(recipe, indent=2))
    publish('Loading local model…', True)
    command = [str(binary), '-m', str(model), '-p', recipe['prompt'], '-n', recipe['negative_prompt'],
               '-W', str(recipe['width']), '-H', str(recipe['height']), '--steps', str(recipe.get('steps',20)),
               '--cfg-scale', str(recipe.get('cfg_scale',7)), '--sampling-method', recipe.get('sampler','euler'),
               '--seed', str(recipe['seed']), '--preview', recipe['preview_method'], '--preview-interval', str(recipe['preview_interval']),
               '--preview-path', str(preview_dir/'frame-%03d.png'),
               '-t', str(recipe.get('cpu_threads',4)), '-o', str(output)]
    if recipe['cpu_threads'] not in range(1,17):raise RuntimeError('Choose between 1 and 16 CPU threads.')
    backend='diffusion=vulkan0,te=cpu,clip_vision=cpu,vae='+('cpu' if recipe['vae_on_cpu'] else 'vulkan0')
    command += ['--backend',backend]
    if use_tae:command += ['--taesd',str(tae),'--taesd-preview-only']
    scheduler=recipe.get('scheduler','discrete')
    if scheduler not in ('discrete','karras'):raise RuntimeError('Unsupported scheduler.')
    tile = recipe.get('vae_tile_size',256)
    if tile not in range(64,513,64):raise RuntimeError('Decoder tile size must be 64–512 pixels in multiples of 64.')
    command += ['--scheduler',scheduler,'--vae-tiling','--vae-tile-size',str(tile)]
    command += ['--max-vram',str(vram_budget)]
    if params_backend != 'auto':command += ['--params-backend',params_backend]
    if reference:
        command += ['--clip_vision',str(reference_weights[0]),'--ip-adapter',str(reference_weights[1]),
                    '--ip-adapter-image',recipe['reference_image'],'--ip-adapter-strength',str(strength)]
    vae=recipe.get('vae_path','')
    if vae:
        vae_file=Path(vae).resolve()
        if vae_file.parent!=(base/'models').resolve() or not vae_file.is_file():raise RuntimeError('Select an installed VAE.')
        command += ['--vae',str(vae_file)]
        receipt=vae_file.with_suffix(vae_file.suffix+'.json')
        if receipt.is_file():recipe['vae_receipt']=json.loads(receipt.read_text())
    if 'Realistic_Vision' in model.name and not vae:raise RuntimeError('Realistic Vision requires its separate VAE decoder.')
    logger.snapshot(model=model,vae=recipe.get('vae_path') or None,engine=binary)
    logger.launch(command)
    recipe['event_log_path']=str(logger.path)
    recipe['engine_arguments'] = command
    metadata.write_text(json.dumps(recipe, indent=2))
    minimum_available = available()
    with (images/(name+'.log')).open('w') as log:
        proc = subprocess.Popen(command, stdout=log, stderr=log, start_new_session=True, preexec_fn=supervised_child)
        last = None
        last_decode = None
        while proc.poll() is None:
            logger.sample(proc.pid)
            free = available()
            minimum_available = min(minimum_available, free)
            if free < 6*1024**3:
                stop_child()
                raise RuntimeError('Stopped to preserve the 6 GiB desktop memory reserve.')
            if cancel.exists():
                stop_child()
                raise RuntimeError('Cancelled by user.')
            frames = sorted(preview_dir.glob('frame-*.png'))
            try:
                with (images/(name+'.log')).open('rb') as progress_log:
                    progress_log.seek(0,2); length=progress_log.tell(); progress_log.seek(max(0,length-4096))
                    tail=progress_log.read().decode(errors='replace')
                samples=re.findall(r'(\d+)/'+str(recipe.get('steps',20))+r' - [^\r\n]*?(?:s/it|it/s)',tail)
                if samples:sample_step=max(sample_step,int(samples[-1]))
                if decode_started is None and 'decoding 1 latents' in tail:
                    decode_started=time.monotonic()
                    sample_step=recipe.get('steps',20)
                    logger.event('decoder_started')
                    publish('Decoding final image…',True,last or '')
            except OSError:tail=''
            if frames and frames[-1] != last and frames[-1].read_bytes().endswith(b'IEND\xaeB`\x82'):
                last = frames[-1]
                preview_count = len(frames)
                if first_preview is None:
                    first_preview = round(time.monotonic()-started,3)
                logger.event('preview',preview_count=preview_count,image_path=str(last))
                publish('Rendering · preview '+str(preview_count), True, last)
            if time.monotonic()-started > 900:
                stop_child()
                raise RuntimeError('Render stopped after the 15 minute calibration limit.')
            if sample_step >= recipe.get('steps',20):
                try:
                    with (images/(name+'.log')).open('rb') as progress_log:
                        progress_log.seek(0,2); length=progress_log.tell(); progress_log.seek(max(0,length-4096))
                        tail=progress_log.read().decode(errors='replace')
                    decode_tail=tail.rsplit('decoding 1 latents',1)[-1] if 'decoding 1 latents' in tail else ''
                    matches=re.findall(r'(\d+)/(\d+) - [^\r\n]*?(?:s/it|it/s)(?: - ETA (\d+:\d+))?',decode_tail)
                    if matches and matches[-1]!=last_decode:
                        last_decode=matches[-1];current,total,eta=last_decode
                        publish(f'Decoding final image · tile {current}/{total}'+(f' · engine ETA {eta}' if eta else ''),True,last or '')
                        logger.event('decode_progress',tile=int(current),tiles=int(total),engine_eta=eta)
                except OSError:pass
            time.sleep(.25)
    if proc.returncode:
        raise RuntimeError('Image engine failed. Details: '+str(images/(name+'.log')))
    if not output.is_file() or not output.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'):
        raise RuntimeError('Engine returned no valid PNG.')
    recipe.update(image_path=str(output), elapsed_seconds=round(time.monotonic()-started,3),
                  peak_memory_mib=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss//1024,
                  preview_count=len(list(preview_dir.glob('frame-*.png'))),
                  time_to_first_preview_seconds=first_preview,
                  minimum_available_mib=minimum_available//1048576)
    engine_log=(images/(name+'.log')).read_text(errors='replace')
    sampling=re.search(r'sampling completed, taking ([\d.]+)s',engine_log)
    full=re.search(r'generate_image completed in ([\d.]+)s',engine_log)
    decoder=re.search(r'decode_first_stage completed, taking ([\d.]+)s',engine_log)
    if decoder:recipe['decoder_seconds']=float(decoder[1])
    if sampling:recipe['sampling_seconds']=float(sampling[1])
    if sampling and full:recipe['post_sampling_seconds']=round(float(full[1])-float(sampling[1]),3)
    if not decoder and decode_started is not None:recipe['decoder_seconds']=round(time.monotonic()-decode_started,3)
    metadata.write_text(json.dumps(recipe, indent=2))
    logger.finish('complete',output=str(output),preview_count=recipe['preview_count'])
    sample_step=recipe.get('steps',20)
    publish('Complete · local render', False, output)
except Exception as error:
    logger.finish('failed',error=str(error))
    stop_child()
    if owned:
        recipe.update(run_status='Cancelled' if cancel.exists() else 'Failed',error=str(error),elapsed_seconds=round(time.monotonic()-started,3))
        metadata.write_text(json.dumps(recipe,indent=2))
        publish('Generation unavailable: '+str(error), False)
    print(str(error), file=sys.stderr)
    sys.exit(1)
finally:
    if owned:
        lock.unlink(missing_ok=True)
