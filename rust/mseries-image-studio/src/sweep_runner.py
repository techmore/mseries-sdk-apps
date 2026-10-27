"""Bounded, sequential prompt/step/model sweeps with persistent manifests."""
import json, os, sys, subprocess, time, uuid
from pathlib import Path
os.umask(0o077)

def supervised_child():
    import ctypes, signal, os
    libc=ctypes.CDLL(None,use_errno=True)
    if libc.prctl(1,signal.SIGTERM,0,0,0)!=0 or os.getppid()==1:
        os._exit(127)


base, recipe, plan = Path(sys.argv[1]), json.loads(sys.argv[2]), json.loads(sys.argv[3])
local_code, review_code = sys.argv[4], sys.argv[5]
lock, cancel = base/'batch.lock', base/'cancel.request'
owned=False
sweep_id='sweep-'+str(time.time_ns())
folder=base/'sweeps';folder.mkdir(parents=True,exist_ok=True)
manifest=folder/(sweep_id+'.json')
state=dict(id=sweep_id,status='Preparing',created_at_unix=int(time.time()),kind=plan['kind'],base_recipe=dict(recipe),plan=plan,runs=[])


def save():
    temp=manifest.with_suffix('.tmp');temp.write_text(json.dumps(state,indent=2));os.replace(temp,manifest)
    pointer=base/'latest-sweep.tmp';pointer.write_text(str(manifest));os.replace(pointer,base/'latest-sweep.txt')


try:
    fd=os.open(lock,os.O_CREAT|os.O_EXCL|os.O_WRONLY,0o600);os.write(fd,sweep_id.encode());os.close(fd);owned=True
    if (base/'generation.lock').exists():raise RuntimeError('A generation or review is active; wait before starting a sweep.')
    (base/'batch.pid').write_text(str(os.getpid()))
    if recipe['seed']<0:
        import secrets
        recipe['seed']=secrets.randbelow(2147483647)
    state['effective_seed']=recipe['seed']
    state['effective_base_recipe']=dict(recipe)
    variants=[]
    if plan['kind']=='repeat':
        count=int(plan.get('count',recipe.get('batch_size',1)))
        if not 1<=count<=8:raise RuntimeError('Image batches are limited to eight sequential runs.')
        for index in range(count):
            variants.append((f'Image {index+1} of {count}',dict(recipe,seed=(recipe['seed']+index)%2147483648)))
    elif plan['kind']=='lighting':
        for lighting in ['soft warm window light','dim amber fireplace light','warm desk lamp with subtle teal monitor light']:
            variants.append((lighting,dict(recipe,prompt=recipe['prompt']+'\nLighting variation: '+lighting)))
    elif plan['kind']=='steps':
        for steps in [8,16,24]:variants.append((str(steps)+' steps',dict(recipe,steps=steps)))
    elif plan['kind']=='models':
        for name in ['stable-diffusion-v1-5-pruned-emaonly-Q4_0.gguf','stable-diffusion-v1-5-pruned-emaonly-Q8_0.gguf']:
            model=base/'models'/name
            if model.is_file():variants.append((name,dict(recipe,model_id=str(model))))
        if len(variants)<2:raise RuntimeError('Install both SD 1.5 Q4 and Q8 to compare models.')
    else:raise RuntimeError('Unknown sweep type.')
    if not 1<=len(variants)<=8:raise RuntimeError('Sweeps are limited to eight runs.')
    if plan.get('review') and not all((base/'models'/name).is_file() for name in ['Qwen3VL-2B-Instruct-Q4_K_M.gguf','mmproj-Qwen3VL-2B-Instruct-Q8_0.gguf']):raise RuntimeError('Install the Qwen vision model and projector before enabling review.')
    env=dict(os.environ,MSERIES_SWEEP_ID=sweep_id)
    state['status']='Running';save()
    for index,(label,settings) in enumerate(variants):
        if cancel.exists():state['status']='Cancelled';break
        settings.update(sweep_id=sweep_id,sweep_variant=label,batch_size=1)
        row=dict(label=label,status='Running',settings=settings)
        state['runs'].append(row);save()
        result=subprocess.run([sys.executable,'-c',local_code,str(base),json.dumps(settings)],env=env,capture_output=True,text=True,preexec_fn=supervised_child)
        if result.returncode:
            stopped='Cancelled' if cancel.exists() else 'Failed'
            row.update(status=stopped,error=result.stderr.strip()[-2000:],exit_code=result.returncode);state['status']=stopped;save();break
        job=json.loads((base/'current-job.json').read_text())
        metadata=Path(job['recipe_path']);output=json.loads(metadata.read_text())
        if output.get('sweep_id')!=sweep_id or output.get('sweep_variant')!=label:
            raise RuntimeError('Generation returned a different sweep recipe; result was not attributed to this comparison.')
        if not Path(output['image_path']).is_file():raise RuntimeError('Generation completed without its saved image.')
        row.update(status='Complete',recipe_path=str(metadata),image_path=output['image_path'],elapsed_seconds=output.get('elapsed_seconds'),peak_memory_mib=output.get('peak_memory_mib'))
        save()
        if plan.get('review') and not cancel.exists():
            row['review_status']='Running';save()
            result=subprocess.run([sys.executable,'-c',review_code,str(base),str(metadata),plan.get('rubric','Evaluate prompt adherence, clothing, lighting, composition and visual artifacts.'),plan.get('reference','')],env=env,capture_output=True,text=True,preexec_fn=supervised_child)
            review_path=metadata.with_suffix('.review.json')
            if result.returncode or not review_path.exists():row.update(review_status='Cancelled' if cancel.exists() else 'Unavailable',review_error=result.stderr.strip()[-1000:])
            else:
                review=json.loads(review_path.read_text());row.update(review_status='Complete',review_path=str(review_path),overall_score=review.get('overall_score'))
            save()
    else:state['status']='Complete'
    if cancel.exists():state['status']='Cancelled'
    save()
except Exception as error:
    if owned:
        state.update(status='Failed' if state['runs'] else 'Unavailable',error=str(error))
        if state['runs'] and state['runs'][-1]['status']=='Running':state['runs'][-1].update(status='Failed',error=str(error))
        save()
    print(str(error),file=sys.stderr);sys.exit(1)
finally:
    if owned:
        (base/'batch.pid').unlink(missing_ok=True)
        lock.unlink(missing_ok=True)
