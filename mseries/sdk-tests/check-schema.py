#!/usr/bin/env python3
"""Validate real SDK descriptions with an independent Draft 2020-12 validator."""
import argparse
import copy
import json
from pathlib import Path
import subprocess
import tempfile
import jsonschema

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--msdk', type=Path, required=True)
parser.add_argument('--rust-root', type=Path, required=True)
args = parser.parse_args()
cli = str(args.msdk.resolve())
rust = args.rust_root.resolve()
schema = json.loads(subprocess.check_output([cli, 'schema'], text=True))
jsonschema.Draft202012Validator.check_schema(schema)
validator = jsonschema.Draft202012Validator(schema)
inputs = sorted((rust / 'msdk/templates').glob('*/app.ui.json'))
inputs += sorted((rust / 'msdk/examples').glob('*.ui.json'))
inputs += sorted((rust / 'msdk/examples').glob('*.msdk'))
inputs += [rust / 'mseries-greeter/data/greeter.ui.json', rust / 'claude-pantheon-rs/sdk/t3.msdk']
inputs = [path for path in inputs if not path.name.startswith('._')]
for path in inputs:
    if path.suffix == '.msdk':
        doc = json.loads(subprocess.check_output([cli, 'json', str(path)], text=True))
    else:
        doc = json.loads(path.read_text())
    errors = list(validator.iter_errors(doc))
    if errors:
        raise AssertionError(f'{path}: ' + '\n'.join(f'{list(e.absolute_path)}: {e.message}' for e in errors))
base = json.loads((rust / 'msdk/templates/settings/app.ui.json').read_text())
metadata = copy.deepcopy(base)
metadata['app'].update(version='0.1.0', license='GPL-3.0-only', website='https://example.invalid')
validator.validate(metadata)
invalid = [
    {'set': 'enabled'},
    {'toggle': True},
    {'quit': False},
    {'when': 'enabled', 'do': True},
    {'set': 'enabled', 'to': True, 'toggle': 'enabled'},
    {'call': 'process.run', 'args': []},
    {'call': 'process.run', 'args': {'command': 'true'}, 'error': 7},
]
for action in invalid:
    doc = copy.deepcopy(base)
    doc['capabilities'].append('subprocess')
    doc['update']['SetEnabled'] = [action]
    assert list(validator.iter_errors(doc)), f'schema accepted malformed action: {action}'
    with tempfile.TemporaryDirectory(prefix='msdk-schema-') as directory:
        path = Path(directory) / 'invalid.ui.json'
        path.write_text(json.dumps(doc))
        result = subprocess.run([cli, 'check', '--json', str(path)], capture_output=True, text=True)
        assert result.returncode != 0, f'checker accepted malformed action: {action}'
print(json.dumps({'valid_descriptions': len(inputs) + 1, 'malformed_actions_rejected_by_both': len(invalid),
                  'validator': 'jsonschema.Draft202012Validator'}, indent=2))
