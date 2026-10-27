#!/usr/bin/env python3
"""Copy only the SDK dependency closure; never resolve the OS/compositor workspace."""
import argparse
from pathlib import Path
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('destination', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
dest = args.destination.resolve()
dest.mkdir(parents=True, exist_ok=False)
for name in ('msdk', 'msdk-gtk'):
    shutil.copytree(root / 'rust' / name, dest / name,
                    ignore=shutil.ignore_patterns('target', '._*', '.DS_Store'))
for relative in ('mseries-greeter/data/greeter.ui.json',
                 'claude-pantheon-rs/sdk',
                 'pantheon-compositor/protocol/pantheon-desktop-shell-v1.xml'):
    source = root / 'rust' / relative
    target = dest / relative
    target.parent.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        shutil.copytree(source, target)
    else:
        shutil.copy2(source, target)
(dest / 'Cargo.toml').write_text('[workspace]\nmembers = ["msdk", "msdk-gtk"]\nresolver = "3"\n')
# Retain the checked-in versions as the starting lock. Cargo may prune unrelated
# OS packages; keep the resulting SDK-only lock as qualification evidence.
shutil.copy2(root / 'rust/Cargo.lock', dest / 'Cargo.lock')
print(dest)
