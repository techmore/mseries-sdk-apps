#!/usr/bin/env python3
"""Fixed tool surface inside a non-root, offline, disposable SDK benchmark VM.

Model output is data. It never selects a host command or writes SDK/runner files.
Generated Rust executes only in the separately isolated benchmark container.
"""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path('/work/bench-app')
SDK = Path('/work/sdk')
TARGET = Path('/work/bench-target')
MODE = os.environ.get('MSDK_BENCH_MODE', 'sdk')

def project_path(name, write=False):
    if not isinstance(name, str) or not name or len(name) > 240:
        raise ValueError('a relative project path is required')
    path = Path(name)
    if path.is_absolute() or '..' in path.parts or any(part.startswith('.') for part in path.parts):
        raise ValueError('absolute, parent and hidden paths are not allowed')
    resolved = (ROOT / path).resolve()
    if not resolved.is_relative_to(ROOT.resolve()):
        raise ValueError('path leaves the project')
    if any(part in ('evidence', 'target') for part in path.parts):
        raise ValueError('runner evidence and build outputs are not model inputs')
    if write and not (path.name in ('Cargo.toml', 'Cargo.lock', 'LICENSE', 'README.md')
                      or path.suffix in ('.json', '.msdk', '.rs', '.svg', '.xml', '.desktop')):
        raise ValueError('unsupported source file type')
    return resolved

def run(argv, cwd=ROOT, seconds=120):
    env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='2',
               CARGO_TARGET_DIR=str(TARGET), GDK_BACKEND='x11', GSK_RENDERER='cairo')
    started = time.monotonic()
    with tempfile.TemporaryFile() as output:
        process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=output,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        reason = None
        try:
            while True:
                # Preserve the leader's identity until all group signals finish.
                status = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                if status is not None:
                    break
                if output.tell() > 2 * 1024 * 1024:
                    reason = 'output-limit'
                    break
                if time.monotonic() - started >= seconds:
                    reason = 'timeout'
                    break
                time.sleep(.02)
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            code = process.wait()
        output.seek(0)
        text = output.read(16000).decode(errors='replace')
        return {'code': code, 'limit': reason, 'seconds': time.monotonic() - started,
                'output': text, 'argv': argv}

def binary_path(project, requested):
    metadata = subprocess.check_output(['/usr/bin/cargo', 'metadata', '--offline', '--no-deps',
                '--format-version', '1', '--manifest-path', str(project / 'Cargo.toml')],
                text=True, env=dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_TARGET_DIR=str(TARGET)))
    binaries = [target['name'] for package in json.loads(metadata)['packages']
                for target in package['targets'] if 'bin' in target['kind']]
    selected = requested or (binaries[0] if len(binaries) == 1 else '')
    if selected not in binaries or '/' in selected:
        raise ValueError('select one built Cargo binary')
    return TARGET / 'debug' / selected

def execute(request):
    action = request.get('action')
    if action == 'list':
        files = [str(path.relative_to(ROOT)) for path in ROOT.rglob('*')
                 if path.is_file() and not any(part in ('evidence', 'target', '.bench-state')
                 for part in path.relative_to(ROOT).parts)]
        return {'files': sorted(files)[:80]}
    if action == 'read':
        name = request.get('path', '')
        if name.startswith('sdk/'):
            if MODE != 'sdk':
                raise ValueError('SDK source is not a baseline tool')
            path = (SDK / name[4:]).resolve()
            if not path.is_relative_to(SDK.resolve()) or 'target' in path.parts:
                raise ValueError('path leaves SDK source')
        else:
            path = project_path(name)
        lines = path.read_text().splitlines()
        first = max(0, min(int(request.get('start', 0)), len(lines)))
        return {'text': '\n'.join(lines[first:first + 160])[:16000], 'line_count': len(lines)}
    if action == 'write':
        files = request.get('files')
        if not isinstance(files, dict) or not 1 <= len(files) <= 16:
            raise ValueError('write files must be an object of 1..16 path:text entries')
        checked = []
        for name, content in files.items():
            if not isinstance(content, str) or len(content.encode()) > 100000:
                raise ValueError('each source file is limited to 100000 bytes')
            checked.append((project_path(name, write=True), content))
        if sum(len(text.encode()) for _, text in checked) > 500000:
            raise ValueError('source write exceeds the batch limit')
        for path, content in checked:
            path.parent.mkdir(parents=True, exist_ok=True)
            if path.is_symlink():
                raise ValueError('symlink output is not permitted')
            path.write_text(content)
        return {'written': list(files)}
    if action != 'run':
        raise ValueError('supported actions: list, read, write, run, done')
    command = request.get('command')
    project = ROOT if request.get('project', '.') == '.' else project_path(request['project'])
    if command in ('build', 'test', 'clippy', 'fmt'):
        if not (project / 'Cargo.toml').is_file():
            raise ValueError('project needs Cargo.toml')
        args = ['/usr/bin/cargo', command, '--manifest-path', str(project / 'Cargo.toml')]
        if command != 'fmt':
            args += ['--offline']
        else:
            args += ['--', '--check']
        if command == 'clippy':
            args += ['--all-targets', '--', '-D', 'warnings']
        return run(args, project)
    if command == 'launch':
        binary = binary_path(project, request.get('binary'))
        shot = ROOT / 'evidence' / f'frame-{time.time_ns()}.png'
        shot.parent.mkdir(exist_ok=True)
        result = run(['xvfb-run', '-a', 'dbus-run-session', '--', '/usr/bin/python3',
                      '/opt/msdk-bench/capture.py', str(binary), str(shot)], project, 30)
        result['frame'] = str(shot.relative_to(ROOT)) if shot.is_file() else None
        return result
    if MODE != 'sdk':
        raise ValueError('SDK commands are not baseline tools')
    cli = '/work/target/debug/msdk'
    if command == 'describe':
        topic = request.get('topic')
        return run([cli, 'describe'] + ([str(topic)] if topic else []))
    if command in ('check', 'render', 'gate'):
        path = project_path(request.get('path', 'app.ui.json'))
        args = [cli, command, str(path)]
        if command == 'gate' and 'gates' in request:
            args.append(str(project_path(request['gates'])))
        return run(args)
    if command == 'new':
        return run([cli, 'new', str(request.get('template', 'list-detail')),
                    str(project_path(request.get('path', 'starter')))])
    if command == 'templates':
        return run([cli, 'templates'])
    if command == 'gen':
        return run([cli, 'gen', str(project_path(request.get('path', 'app.ui.json'))),
                    str(project_path(request.get('output', 'generated'))), '--sdk-root', str(SDK)])
    if command == 'pack':
        binary = binary_path(project, request.get('binary'))
        package = ROOT / 'evidence' / f'package-{time.time_ns()}.deb'
        package.parent.mkdir(exist_ok=True)
        result = run([cli, 'pack', str(project), '--binary', str(binary), '--output', str(package)], project)
        result['package'] = str(package.relative_to(ROOT)) if package.is_file() else None
        return result
    raise ValueError('unknown fixed command')

if __name__ == '__main__':
    try:
        status = dict(line.split(':', 1) for line in Path('/proc/self/status').read_text().splitlines() if ':' in line)
        if (os.geteuid() != 2000 or set(os.listdir('/sys/class/net')) != {'lo'}
                or int(status['CapEff'].strip(), 16) or int(status['CapAmb'].strip(), 16)):
            raise RuntimeError('benchmark worker requires uid2000, zero effective/ambient capabilities and loopback-only network')
        request = json.load(sys.stdin)
        if not isinstance(request, dict):
            raise ValueError('request must be an object')
        print(json.dumps({'ok': True, 'result': execute(request)}))
    except Exception as error:
        print(json.dumps({'ok': False, 'error': str(error)}))
