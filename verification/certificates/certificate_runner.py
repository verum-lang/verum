"""Shared native/independent replay gate for generated portable certificates."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(cases, description, sources):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument('--native', required=True, type=Path)
    parser.add_argument('--lean', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    args = parser.parse_args()
    native_path, lean_path = args.native.resolve(), args.lean.resolve()
    report = {'schema_version': 2, 'cases': [],
        'tools': {name: {'path': str(path), 'sha256': digest(path)}
            for name, path in [('native', native_path), ('independent_lean', lean_path)]},
        'producer_sources': {str(path): digest(path) for path in sources}}
    with tempfile.TemporaryDirectory(prefix='verum-certificates-') as directory:
        battery = {'schema_version': 1, 'certificates': [dict(id=name, term=cert['term'], claimed_type=cert['claimed_type']) for name, cert, _ in cases]}
        battery_path = Path(directory) / 'battery.json'
        battery_path.write_text(json.dumps(battery, separators=(',', ':')))
        lean_run = subprocess.run([str(lean_path), str(battery_path)], capture_output=True, text=True, timeout=60)
        if lean_run.returncode:
            raise RuntimeError('independent replay failed: ' + lean_run.stderr[:1000])
        verdicts = json.loads(lean_run.stdout)['verdicts']
        if [row['id'] for row in verdicts] != [name for name, _, _ in cases]:
            raise RuntimeError('independent replay changed or omitted certificate identities')
        for (name, cert, expected), lean in zip(cases, verdicts):
            judgment = {'profile': 'verum-native/1', 'statement': cert['claimed_type'], 'certificate': cert}
            encoded = json.dumps(judgment, separators=(',', ':')).encode()
            native_run = subprocess.run([str(native_path)], input=encoded, capture_output=True, timeout=10)
            native = json.loads(native_run.stdout)
            if native_run.returncode != (0 if native['accepted'] else 1):
                raise RuntimeError(name + ': native exit status disagrees with verdict')
            if native['accepted'] != expected or lean['ok'] != expected:
                raise RuntimeError(f'{name}: expected {expected}, native={native}, independent={lean}')
            if not expected and native['diagnostic']['stage'] not in ('substitution', 'normalization'):
                raise RuntimeError(name + ': rejection did not reach proof checking: ' + str(native['diagnostic']))
            report['cases'].append({'id': name, 'expected': expected, 'native': native, 'independent_lean': lean,
                'judgment_sha256': hashlib.sha256(encoded).hexdigest()})
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(f'checked {len(cases)} exact cases through bounded native admission and independent Lean replay')
