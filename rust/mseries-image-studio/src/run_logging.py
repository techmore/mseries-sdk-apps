"""Private, structured diagnostics for an individual render (stdlib only).

The recipe sidecar owns prompt text. This log records prompt hashes and lengths,
so diagnostics can be shared without copying the user's text into engine argv.
"""
import datetime
import hashlib
import json
import os
from pathlib import Path
import time


class RunLogger:
    def __init__(self, base, run_id):
        directory = Path(base) / 'logs'
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        # IDs originate in the worker, but never allow a caller to escape logs/.
        safe_id = ''.join(c for c in str(run_id) if c.isalnum() or c in '-_')
        if not safe_id:
            raise ValueError('A nonempty run ID is required')
        self.path = directory / (safe_id + '.jsonl')
        self.started = time.monotonic()
        self.last_sample = None
        self.sequence = 0
        self.error = None
        self.event('run_created', run_id=safe_id, worker_pid=os.getpid())

    def event(self, kind, **fields):
        """Best effort logging must never interrupt a render or hide its result."""
        self.sequence += 1
        record = dict(fields)
        record.update(timestamp=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                      elapsed_seconds=round(time.monotonic() - self.started, 3),
                      sequence=self.sequence, event=kind)
        try:
            fd = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
            with os.fdopen(fd, 'a', encoding='utf-8') as stream:
                stream.write(json.dumps(record, ensure_ascii=False, default=str) + '\n')
            self.error = None
        except OSError as error:
            self.error = str(error)

    @staticmethod
    def file_info(path):
        if path is None:
            return None
        path = Path(path)
        info = {'path': str(path)}
        try:
            stat = path.stat()
            info.update(size_bytes=stat.st_size, modified_ns=stat.st_mtime_ns)
            receipt = path.with_suffix(path.suffix + '.json')
            if receipt.is_file():
                info['receipt'] = json.loads(receipt.read_text())
        except (OSError, ValueError) as error:
            info['inspection_error'] = str(error)
        return info

    def snapshot(self, model=None, vae=None, engine=None):
        self.event('assets', model=self.file_info(model), vae=self.file_info(vae),
                   engine=self.file_info(engine))

    def launch(self, command):
        redacted = list(map(str, command))
        prompts = {}
        for index, arg in enumerate(redacted[:-1]):
            if arg in ('-p', '--prompt', '-n', '--negative-prompt'):
                value = redacted[index + 1]
                prompts[arg] = {'characters': len(value),
                                'sha256': hashlib.sha256(value.encode()).hexdigest()}
                redacted[index + 1] = '<prompt stored in recipe>'
        self.event('engine_launch', arguments=redacted, prompt_fingerprints=prompts)

    def sample(self, pid=None, force=False):
        now = time.monotonic()
        if not force and self.last_sample is not None and now - self.last_sample < 2:
            return
        self.last_sample = now
        memory = {}
        try:
            for line in Path('/proc/meminfo').read_text().splitlines():
                key, _, value = line.partition(':')
                if key in ('MemAvailable', 'MemTotal', 'SwapFree', 'SwapTotal'):
                    memory[key + '_bytes'] = int(value.split()[0]) * 1024
            if pid is not None:
                for line in Path(f'/proc/{int(pid)}/status').read_text().splitlines():
                    key, _, value = line.partition(':')
                    if key in ('VmRSS', 'VmHWM'):
                        memory[key + '_bytes'] = int(value.split()[0]) * 1024
        except (OSError, ValueError) as error:
            memory['inspection_error'] = str(error)
        self.event('memory', **memory)

    def finish(self, status, **fields):
        self.sample(force=True)
        self.event('run_finished', status=status, **fields)
