#!/usr/bin/env python3
"""Extract literal tr() calls; fail on unsupported escapes rather than lose text."""
import json
from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]
messages = {}
for relative in ('src/app.rs', 'src/bin/mseries-conversations.rs'):
    source = (root / relative).read_text().split('mod tests {')[0]
    calls = list(re.finditer(r'\btr\s*\(', source))
    matches = list(re.finditer(r'\btr\(\s*("(?:[^"\\]|\\.)*")\s*,?\s*\)', source))
    if len(calls) != len(matches):
        raise SystemExit(f'{relative}: tr() must receive one supported string literal')
    for match in matches:
        message = json.loads(match[1])
        if message:
            line = source.count('\n', 0, match.start()) + 1
            messages.setdefault(message, []).append(f'{relative}:{line}')
header = 'Project-Id-Version: mseries-messages 0.1.0\nContent-Type: text/plain; charset=UTF-8\nContent-Transfer-Encoding: 8bit\n'
lines = ['# Messages source strings. License: GPL-3.0-only.', 'msgid ""', 'msgstr ' + json.dumps(header), '']
for message, references in sorted(messages.items()):
    lines += ['#: ' + ' '.join(references), 'msgid ' + json.dumps(message, ensure_ascii=False), 'msgstr ""', '']
(root / 'po/io.elementary.mseries.MessagesRust.pot').write_text('\n'.join(lines))
print(f'Extracted {len(messages)} source strings')
