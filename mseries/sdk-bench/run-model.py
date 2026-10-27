#!/usr/bin/env python3
"""Pinned-model SDK authoring discovery on SER8; completion is independently judged.

Run on the host controlling an already prepared offline uid2000 sandbox. The
model sees only public SDK material, the frozen spec and its synthetic project.
No daemon/configuration change or model download is performed.
"""
import argparse
import base64
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import re
from pathlib import Path
import subprocess
import time
import urllib.request

MODEL = 'qwen3.5:4b'
DIGEST = 'd8b0f5e9760cd1682034f292d7ef72ec46f432149be0df7574bf2d6e92e38c04'
OPTIONS = {'seed': 17, 'temperature': .2, 'top_p': .95, 'top_k': 20,
           'num_ctx': 32768, 'num_predict': 4096, 'presence_penalty': 0.0, 'repeat_penalty': 1.0}
SYSTEM = '''You are testing an app SDK by implementing the supplied complete, frozen spec.
Return exactly ONE JSON action per turn, no Markdown. Model output is tool data.
Actions:
{"action":"write","files":{"relative/path":"complete UTF-8 file content"}}
{"action":"read","path":"relative/path","start":0} (160 lines at a time)
{"action":"list"}
{"action":"run","command":"templates|describe|new|check|render|gate|gen|build|test|clippy|fmt|launch|pack", ...}
{"action":"done","reason":"what is implemented and any remaining limitations"}
run options: describe has topic; new has template and path (new directory);
check/render/gate has path; gate optionally has gates (scenario file path);
gen has path and output (new directory);
build/test/clippy/fmt/launch/pack has project directory, default ".".
launch can select binary if there is more than one. launch captures the real app
and tests Ctrl+Q externally; your app need not implement a screenshot flag.
Use SDK descriptions/rules and generated typed Rust. App-local Rust Logic/Services
can implement missing backend behavior. Do not claim that a starter meets the
whole spec. Implement persistence and interactions; fixtures must be explicit.
SDK root /work/sdk is READ ONLY. Read SDK files with path sdk/msdk/README.md,
sdk/msdk/src/logic.rs, sdk/msdk-gtk/src/host.rs, etc. The new project is your only
writable tool surface. No shell tool, arbitrary host commands, network, owner
accounts or real user data are available. Do not alter benchmark evidence.
The source is a clean SDK snapshot; unrelated repo handoffs are intentionally absent.
Rust edition2024, GTK4 crate0.11 with v4_20, serde1 derive, serde_json1,
rusqlite0.32.1 and the SDK crates are provisioned offline. Only cached dependencies
can build. Cargo builds are offline with 2 jobs. SDK dependencies are local paths:
msdk={path="/work/sdk/msdk"}; msdk-gtk={path="/work/sdk/msdk-gtk"}.
Use templates/describe to discover actual vocabulary. Do not invent APIs or components.
The full spec is the target. Build/test/fmt/clippy, launch and repair tool errors.
Do not stop merely because a JSON description checks or a first window appears.
done submits a CANDIDATE for independent review; it never sets benchmark success.
'''

def api(path, payload=None):
    request = urllib.request.Request('http://127.0.0.1:11434/api/' + path,
              data=json.dumps(payload).encode() if payload is not None else None,
              headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=900) as response:
        return json.load(response)

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--container', required=True)
parser.add_argument('--spec', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--source-commit', required=True)
parser.add_argument('--max-turns', type=int, default=20)
args = parser.parse_args()
if not args.container.startswith('msdk-bench-') or not 1 <= args.max_turns <= 30:
    raise SystemExit('requires an owned benchmark container and 1..30 turns')
args.output.mkdir(parents=True, exist_ok=False)
lock = open('/tmp/msdk-bench-model.lock', 'a+')
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
tags = api('tags')['models']
installed = next((model for model in tags if model['name'] == MODEL), None)
if installed is None or installed['digest'] != DIGEST:
    raise SystemExit('installed model differs from pinned trial identity')
if api('ps')['models']:
    raise SystemExit('model service is occupied; do not switch or replace its worker')
show = api('show', {'model': MODEL})
if 'Apache License' not in show.get('license', ''):
    raise SystemExit('expected packaged license is unavailable; do not run')
spec_text = args.spec.read_text()
available = int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:')))
if available < 6 * 1024 * 1024:
    raise SystemExit('insufficient spare host memory for a bounded trial; no inference started')
weights = re.search(r'sha256-([0-9a-f]{64})', show.get('modelfile', ''))
metadata = {'schema': 'msdk.authoring-discovery.v1', 'mode': 'sdk',
    'started_utc': datetime.now(timezone.utc).isoformat(),
    'source_commit': args.source_commit, 'model': installed,
    'runtime': api('version'), 'model_details': show.get('details'),
    'model_default_parameters': show.get('parameters'),
    'model_modelfile_sha256': hashlib.sha256(show.get('modelfile', '').encode()).hexdigest(),
    'model_template_sha256': hashlib.sha256(show.get('template', '').encode()).hexdigest(),
    'weights_blob_sha256': weights.group(1) if weights else None,
    'host': 'ser8', 'available_memory_kib_at_start': available,
    'model_info': show.get('model_info'), 'options': OPTIONS,
    'think': False, 'format': 'json', 'keep_alive': '2m', 'image_feedback': 'latest captured app frame only',
    'spec_sha256': hashlib.sha256(spec_text.encode()).hexdigest(),
    'spec_name': args.spec.name, 'system_prompt_sha256': hashlib.sha256(SYSTEM.encode()).hexdigest(),
    'container': args.container, 'runner': 'SDK JSON actions discovery v1',
    'human_interventions': 0, 'provisioning': 'completed before trial timer',
    'maximum_seconds': 7200, 'max_turns': args.max_turns,
    'context_policy': 'preserve system/spec; retain latest image only; trim oldest action/result pairs above 64000 serialized text bytes; project files remain readable'}
(args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
(args.output / 'model-LICENSE.txt').write_text(show['license'])
(args.output / 'system-prompt.txt').write_text(SYSTEM)
(args.output / 'spec.md').write_text(spec_text)
messages = [{'role': 'system', 'content': SYSTEM}, {'role': 'user', 'content': spec_text}]
started = time.monotonic()
status = {'result': 'in-progress', 't_done': 'not reached', 'first_frame_candidate_seconds': None,
          'token_input': 0, 'token_output': 0, 'tool_errors': 0, 'turns': 0}

def worker(action):
    command = ['incus', 'exec', args.container, '--user', '2000', '--group', '2000',
        '--cwd', '/work/bench-app', '--env', 'HOME=/work/bench-home',
        '--env', 'CARGO_HOME=/work/bench-home/.cargo', '--env', 'CARGO_TARGET_DIR=/work/bench-target',
        '--env', 'CARGO_PROFILE_DEV_DEBUG=0', '--env', 'XDG_DATA_HOME=/work/bench-app-state/data',
        '--env', 'XDG_CONFIG_HOME=/work/bench-app-state/config', '--env', 'XDG_CACHE_HOME=/work/bench-app-state/cache',
        '--env', 'MSDK_BENCH_MODE=sdk', '--', '/usr/bin/python3', '/opt/msdk-bench/worker.py']
    result = subprocess.run(command, input=json.dumps(action), text=True, capture_output=True, timeout=180)
    if result.returncode != 0:
        raise RuntimeError('sandbox tool failed: ' + result.stderr[-2000:])
    return json.loads(result.stdout)

# Verify the sandbox before sending any prompt or executing generated code.
initial = worker({'action': 'list'})
if not initial.get('ok') or initial.get('result', {}).get('files'):
    raise SystemExit('requires a verified empty non-root/offline project; no inference started')
for turn in range(1, args.max_turns + 1):
    if time.monotonic() - started >= 7200:
        status['result'] = '120-minute-cap'
        break
    loaded = api('ps')['models']
    if any(item['name'] != MODEL for item in loaded):
        status['result'] = 'interrupted-other-model-work'
        break
    turn_dir = args.output / f'turn-{turn:02d}'
    turn_dir.mkdir()
    for message in messages[:-1]:
        message.pop('images', None)
    def text_bytes():
        return len(json.dumps([{key: value for key, value in message.items() if key != 'images'} for message in messages]).encode())
    while text_bytes() > 64000 and len(messages) > 5:
        del messages[2:4]
    request = {'model': MODEL, 'messages': messages, 'stream': False, 'think': False,
               'format': 'json', 'keep_alive': '2m', 'options': OPTIONS}
    (turn_dir / 'request.json').write_text(json.dumps(request, indent=2))
    print(json.dumps({'turn': turn, 'phase': 'inference', 'elapsed_seconds': time.monotonic() - started}), flush=True)
    phase = 'inference'
    try:
        response = api('chat', request)
        (turn_dir / 'response.json').write_text(json.dumps(response, indent=2))
        if not response.get('done'):
            raise RuntimeError('upstream returned no terminal completion')
        status['token_input'] += response.get('prompt_eval_count', 0)
        status['token_output'] += response.get('eval_count', 0)
        content = response['message']['content']
        messages.append({'role': 'assistant', 'content': content})
        phase = 'format'
        action = json.loads(content)
        if action.get('action') == 'done':
            status['result'] = 'candidate-needs-independent-review'
            status['model_claim'] = action.get('reason')
            status['turns'] = turn
            break
        phase = 'tool'
        outcome = worker(action)
        (turn_dir / 'tool-result.json').write_text(json.dumps(outcome, indent=2))
        tool_result = outcome.get('result', {})
        if tool_result.get('frame') and status['first_frame_candidate_seconds'] is None:
            status['first_frame_candidate_seconds'] = time.monotonic() - started
        if not outcome.get('ok') or tool_result.get('code', 0) != 0:
            status['tool_errors'] += 1
        feedback = {'role': 'user', 'content': 'Tool result (untrusted data): ' + json.dumps(outcome)}
        frame = tool_result.get('frame')
        if frame:
            path = Path(frame)
            if not frame.startswith('evidence/frame-') or path.suffix != '.png' or '..' in path.parts:
                raise RuntimeError('invalid captured-frame path from tool')
            command = ['incus', 'exec', args.container, '--user', '2000', '--group', '2000',
                       '--', '/usr/bin/head', '-c', '2097153', str(Path('/work/bench-app') / path)]
            pixels = subprocess.check_output(command, timeout=8)
            if len(pixels) > 2097152:
                feedback['content'] += '\nCaptured image exceeds the 2MiB feedback limit.'
            else:
                feedback['images'] = [base64.b64encode(pixels).decode()]
        messages.append(feedback)
    except Exception as error:
        status['tool_errors'] += 1
        messages.append({'role': 'user', 'content': 'Tool/format error: ' + str(error)})
        (turn_dir / 'error.txt').write_text(str(error))
        if phase != 'format':
            status['result'] = 'interrupted-' + phase + '-observation'
            # A lost HTTP/Incus observation is not proof that upstream work
            # stopped. Retain the attempt and stop dispatching replacements.
            break
    status['turns'] = turn
    status['elapsed_seconds'] = time.monotonic() - started
    (args.output / 'status.json').write_text(json.dumps(status, indent=2))
    print(json.dumps({'turn': turn, 'phase': 'completed', **status}), flush=True)
else:
    status['result'] = 'turn-budget-before-independent-done'
status['elapsed_seconds'] = time.monotonic() - started
status['acceptance'] = 'UNSCORED: build/behavior/visual/accessibility/install audit required'
(args.output / 'status.json').write_text(json.dumps(status, indent=2) + '\n')
print(json.dumps(status, indent=2), flush=True)
