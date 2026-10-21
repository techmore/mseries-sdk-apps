"""Small per-user preferences for Messages.

Stored as a tiny JSON file of on/off switches, outside BlueFerry's encrypted
history. Only the keys in DEFAULTS are ever read or written and only booleans
are accepted, so a hand-edited or corrupt file cannot inject anything and no
message content, address or account detail is ever saved here.
"""
import json
import os
from pathlib import Path
import tempfile

DEFAULTS = {'enter_sends': False}


def config_path():
    base = os.environ.get('XDG_CONFIG_HOME') or str(Path.home() / '.config')
    return Path(base) / 'mseries' / 'messages.json'


def load(path=None):
    """The saved switches over DEFAULTS; a missing or unreadable file means defaults."""
    values = dict(DEFAULTS)
    try:
        saved = json.loads(Path(path or config_path()).read_text())
    except (OSError, ValueError):
        return values
    if isinstance(saved, dict):
        for key in DEFAULTS:
            if isinstance(saved.get(key), bool):
                values[key] = saved[key]
    return values


def save(values, path=None):
    """Write only the known boolean switches, atomically, readable by the owner only."""
    target = Path(path or config_path())
    clean = {key: bool(values.get(key, default)) for key, default in DEFAULTS.items()}
    target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(dir=target.parent, prefix='.messages-', suffix='.tmp')
    try:
        with os.fdopen(handle, 'w') as out:
            json.dump(clean, out, indent=2)
            out.write('\n')
        os.chmod(temporary, 0o600)
        os.replace(temporary, target)
    except BaseException:
        try:
            os.unlink(temporary)
        except OSError:
            pass
        raise
    return clean
