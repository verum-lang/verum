"""Shared native/independent replay gate for generated portable certificates."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

# A failed checker invocation is not a proof of an invalid certificate. These
# are the semantic error constructors of the two current checking protocols;
# new outcomes require explicit classification instead of an allow-by-default.
NATIVE_REJECTIONS = frozenset({
    'invalid_inductive', 'unbound_variable', 'not_a_type', 'not_a_function',
    'not_a_pair_type', 'not_an_identity', 'identity_endpoints_differ',
    'invalid_identity_motive', 'domain_mismatch', 'type_mismatch',
})
LEAN_REJECTIONS = frozenset({
    'InvalidInductive', 'UnboundVariable', 'NotAType', 'NotAFunction',
    'NotASigma', 'NotAnIdentity', 'NotAValidJMotive', 'DomainMismatch',
    'TypeMismatch', 'ClaimedTypeNotAType',
})


def validate_verdicts(name, native, lean, expected, profile, returncode):
    """Admit only complete matching proof verdicts, never operational failure."""
    def require(condition, message):
        if not condition:
            raise RuntimeError(name + ': ' + message)

    require(type(expected) is bool, 'expected verdict must be Boolean')
    require(type(native) is dict and type(lean) is dict, 'malformed checker report')
    require(type(native.get('accepted')) is bool and type(lean.get('ok')) is bool,
            'checker verdict must be Boolean')
    require(native.get('profile') == profile, 'worker changed the selected foundation')
    require(lean.get('id') == name, 'independent checker changed certificate identity')
    require(returncode == (0 if native['accepted'] else 1),
            'native exit status disagrees with verdict')
    require(native['accepted'] == expected and lean['ok'] == expected,
            'native and independent proof verdicts differ from the expected result')
    require(native.get('outcome') == ('accepted' if expected else 'rejected'),
            'unsupported, exhausted or inconclusive result is not a proof verdict')
    if expected:
        require(native.get('diagnostic') is None and 'error' not in lean,
                'accepted proof carries a failure diagnostic')
    else:
        diagnostic = native.get('diagnostic')
        require(type(diagnostic) is dict and diagnostic.get('stage') in ('substitution', 'normalization'),
                'rejection did not reach proof checking')
        require(diagnostic.get('code') in NATIVE_REJECTIONS,
                'native failure is not a semantic proof rejection')
        require(lean.get('error') in LEAN_REJECTIONS,
                'independent failure is not a semantic proof rejection')


def validate_cases(cases):
    if not cases:
        raise ValueError('certificate battery must not be empty')
    if any(type(name) is not str or not name or type(expected) is not bool
           for name, _, expected in cases):
        raise ValueError('each certificate requires a nonempty identity and Boolean expectation')
    if len({name for name, _, _ in cases}) != len(cases):
        raise ValueError('certificate identities must be unique')


def run(cases, description, sources, *, foundations=None):
    cases = list(cases)
    validate_cases(cases)
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument('--native', required=True, type=Path)
    parser.add_argument('--lean', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--export-dir', type=Path, help='write replayable exact judgments to a new directory')
    args = parser.parse_args()
    foundations = {} if foundations is None else foundations
    policies = {'intensional': ('verum-native/1', []),
        'function-extensionality': ('verum-native-funext/1', ['--foundation', 'function-extensionality'])}
    if set(foundations) - {name for name, _, _ in cases}:
        raise ValueError('foundation selected for an absent case')
    selected = [policies[foundations.get(name, 'intensional')] for name, _, _ in cases]
    exports = {}
    native_path, lean_path = args.native.resolve(), args.lean.resolve()
    report = {'schema_version': 2, 'cases': [],
        'tools': {name: {'path': str(path), 'sha256': digest(path)}
            for name, path in [('native', native_path), ('independent_lean', lean_path)]},
        'producer_sources': {str(path): digest(path) for path in sources}}
    with tempfile.TemporaryDirectory(prefix='verum-certificates-') as directory:
        battery = {'schema_version': 1, 'certificates': [dict(id=name, profile=policy[0], term=cert['term'], claimed_type=cert['claimed_type']) for (name, cert, _), policy in zip(cases, selected)]}
        battery_path = Path(directory) / 'battery.json'
        exports['battery.json'] = json.dumps(battery, separators=(',', ':')).encode()
        battery_path.write_bytes(exports['battery.json'])
        lean_run = subprocess.run([str(lean_path), str(battery_path)], capture_output=True, text=True, timeout=60)
        if lean_run.returncode:
            raise RuntimeError('independent replay failed: ' + lean_run.stderr[:1000])
        verdicts = json.loads(lean_run.stdout)['verdicts']
        if [row['id'] for row in verdicts] != [name for name, _, _ in cases]:
            raise RuntimeError('independent replay changed or omitted certificate identities')
        for index, ((name, cert, expected), lean, (profile, native_args)) in enumerate(zip(cases, verdicts, selected)):
            judgment = {'profile': profile, 'statement': cert['claimed_type'], 'certificate': cert}
            encoded = json.dumps(judgment, separators=(',', ':')).encode()
            native_run = subprocess.run([str(native_path), *native_args], input=encoded, capture_output=True, timeout=10)
            native = json.loads(native_run.stdout)
            validate_verdicts(name, native, lean, expected, profile, native_run.returncode)
            filename = f'{index:03d}.judgment.json'
            exports[filename] = encoded
            report['cases'].append({'id': name, 'artifact': filename, 'native_arguments': native_args, 'expected': expected, 'native': native, 'independent_lean': lean,
                'judgment_sha256': hashlib.sha256(encoded).hexdigest()})
    report['artifacts'] = {name: hashlib.sha256(content).hexdigest() for name, content in exports.items()}
    for tool in report['tools'].values():
        if digest(Path(tool['path'])) != tool['sha256']:
            raise RuntimeError('Checker executable changed during replay')
    if report['producer_sources'] != {str(path): digest(path) for path in sources}:
        raise RuntimeError('Producer or model source changed during replay')
    if args.export_dir:
        args.export_dir.mkdir(parents=True, exist_ok=False)
        for name, content in exports.items():
            (args.export_dir / name).write_bytes(content)
        (args.export_dir / 'receipt.json').write_text(json.dumps(report, indent=2) + '\n')
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(f'checked {len(cases)} exact cases through bounded native admission and independent Lean replay')
