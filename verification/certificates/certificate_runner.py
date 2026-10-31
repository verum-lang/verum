"""Shared native/independent replay gate for generated portable certificates."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path
from run_report import RunReport, atomic_write, require_completed


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


def require_replay_report(report, run_id, request, *, returncode):
    """Consumer boundary: current invocation plus every exact semantic control."""
    require_completed(report, run_id, request, returncode=returncode)
    rows = report.get('cases')
    expected = request['controls']
    if not expected or type(rows) is not list or len(rows) != len(expected):
        raise ValueError('missing or empty replay controls')
    if (report.get('tools') != request['tools']
            or report.get('producer_sources') != request['producer_sources']
            or report.get('artifacts', {}).get('battery.json') != request['battery_sha256']):
        raise ValueError('replay provenance changed')
    for row, control in zip(rows, expected):
        name, accepted, profile = control
        if (row['id'] != name or type(row['expected']) is not bool
                or row['expected'] != accepted
                or report['artifacts'].get(row['artifact']) != row['judgment_sha256']):
            raise ValueError('replay changed a control or artifact')
        validate_verdicts(name, row['native'], row['independent_lean'], accepted,
                         profile, row['native_returncode'])
    return report


def run(cases, description, sources, *, foundations=None):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument('--native', required=True, type=Path)
    parser.add_argument('--lean', required=True, type=Path)
    parser.add_argument('--report', required=True, type=Path)
    parser.add_argument('--export-dir', type=Path, help='write replayable exact judgments to a new directory')
    parser.add_argument('--run-id', help='fresh caller-generated UUID; never reuse on retry')
    args = parser.parse_args()
    with RunReport(args.report, args.run_id) as invocation:
        execute(cases, sources, foundations, args, invocation)


def execute(cases, sources, foundations, args, invocation):
    cases = list(cases)
    validate_cases(cases)
    sources = list(dict.fromkeys(Path(p).resolve() for p in [*sources, Path(__file__), Path(__file__).with_name('run_report.py')]))
    foundations = {} if foundations is None else foundations
    policies = {'intensional': ('verum-native/1', []),
        'function-extensionality': ('verum-native-funext/1', ['--foundation', 'function-extensionality'])}
    if set(foundations) - {name for name, _, _ in cases}:
        raise ValueError('foundation selected for an absent case')
    selected = [policies[foundations.get(name, 'intensional')] for name, _, _ in cases]
    exports = {}
    native_path, lean_path = args.native.resolve(), args.lean.resolve()
    report = {'cases': [],
        'tools': {name: {'path': str(path), 'sha256': digest(path)}
            for name, path in [('native', native_path), ('independent_lean', lean_path)]},
        'producer_sources': {str(path): digest(path) for path in sources}}
    with tempfile.TemporaryDirectory(prefix='verum-certificates-') as directory:
        battery = {'schema_version': 1, 'certificates': [dict(id=name, profile=policy[0], term=cert['term'], claimed_type=cert['claimed_type']) for (name, cert, _), policy in zip(cases, selected)]}
        battery_path = Path(directory) / 'battery.json'
        exports['battery.json'] = json.dumps(battery, separators=(',', ':')).encode()
        invocation.bind({'battery_sha256': hashlib.sha256(exports['battery.json']).hexdigest(),
            'controls': [[name, expected, policy[0]] for (name, _, expected), policy in zip(cases, selected)],
            'tools': report['tools'], 'producer_sources': report['producer_sources'],
            'export_dir': str(args.export_dir.resolve()) if args.export_dir else None})
        battery_path.write_bytes(exports['battery.json'])
        lean_run = subprocess.run([str(lean_path), str(battery_path)], capture_output=True, text=True, timeout=60)
        if lean_run.returncode:
            raise RuntimeError('independent replay failed: ' + lean_run.stderr[:1000])
        bundle = json.loads(lean_run.stdout)
        if type(bundle.get('schema_version')) is not int or bundle['schema_version'] != 1:
            raise RuntimeError('unsupported independent report schema')
        verdicts = bundle['verdicts']
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
            report['cases'].append({'id': name, 'artifact': filename, 'native_arguments': native_args, 'expected': expected, 'native': native, 'native_returncode': native_run.returncode, 'independent_lean': lean,
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
    completed = invocation.complete(report)
    require_replay_report(completed, invocation.run_id, invocation.request, returncode=0)
    if args.export_dir:
        atomic_write(args.export_dir / 'receipt.json', completed)
    print(f'checked {len(cases)} exact cases; run {invocation.run_id}; request {completed["run"]["request_sha256"]}')
