#!/usr/bin/env python3
"""Fingerprint the exact SDK source/fixture closure independently of build caches."""
import argparse
import hashlib
import json
import os
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('rust_root', type=Path)
args = parser.parse_args()
root = args.rust_root.resolve()
files = []
for name in ('msdk', 'msdk-gtk', 'claude-pantheon-rs/sdk'):
    for directory, children, names in os.walk(root / name):
        children[:] = sorted(child for child in children if child not in ('target', '.git', '__pycache__'))
        for filename in sorted(names):
            if filename == 'Cargo.lock' or filename.startswith('._'):
                continue
            files.append(Path(directory) / filename)
files += [root / 'mseries-greeter/data/greeter.ui.json',
          root / 'pantheon-compositor/protocol/pantheon-desktop-shell-v1.xml']
manifest = {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(files)}
print(json.dumps(manifest, indent=2, sort_keys=True))
