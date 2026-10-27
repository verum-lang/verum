"""Strict data decoding and immutable Lean replay shared by observer export formats."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import uuid
from check_typed_programs import ROOT, check, require_success, run, sha256

MAX64 = 2**64 - 1


def require(condition, message):
    if not condition:
        raise ValueError(message)


def fields(obj, names):
    require(type(obj) is dict and set(obj) == set(names.split()),
            f'Expected fields {names}')
    return obj


def natural(value, maximum=MAX64):
    require(type(value) is int and 0 <= value <= maximum, 'Out-of-range integer')
    return str(value)


def boolean(value):
    require(type(value) is bool, 'Expected Boolean')
    return str(value).lower()


def sequence(value):
    require(type(value) is list, 'Expected array')
    return value


def array(items):
    return '[' + ', '.join(items) + ']'


def variant(value):
    require(type(value) is dict and len(value) == 1, 'Expected tagged variant')
    return next(iter(value.items()))


def unique_object(pairs):
    result = {}
    for k, v in pairs:
        require(k not in result, f'Duplicate JSON key: {k}')
        result[k] = v
    return result


def load(path):
    require(path.stat().st_size <= 16 * 1024 * 1024, 'Export exceeds 16 MiB limit')
    return json.loads(path.read_text(), object_pairs_hook=unique_object)


def declarations(source):
    """The exact common prelude, compiled once within the private snapshot."""
    blocks = source.strip().split('\n\n')
    retained = [b for b in blocks if b.startswith(('import ', 'open ', 'set_option ',
        'namespace ', 'def ', '@[cbv_eval]\ntheorem ', 'attribute [cbv_opaque] '))]
    return '\n\n'.join(retained + ['end FrozenObserverExport']) + '\n'


def batches(source, size=8, shared_module=None):
    """Bound each kernel invocation without weakening its acceptance deadline.

    Claims are closed equalities. Shared trace lemmas are checked in the
    prelude (once when imported) and audited in bounded groups; no fact is assumed.
    """
    blocks = source.strip().split('\n\n')
    header = [b for b in blocks if b.startswith(('import ', 'open ', 'set_option ', 'namespace '))]
    shared = [b for b in blocks if b.startswith('@[cbv_eval]\ntheorem ')]
    definitions = [b for b in blocks if b.startswith(('def ', '@[cbv_eval]\ntheorem '))]
    attributes = [b for b in blocks if b.startswith('attribute [cbv_opaque] ')]
    prelude = header + definitions + attributes if shared_module is None else [
        'import ' + shared_module, *header]
    claims = [b for b in blocks if b.startswith('theorem ')]
    for start in range(0, len(claims), size):
        group = claims[start:start + size]
        names = [b.split()[1] for b in group]
        yield ('\n\n'.join(prelude + group +
               ['#print axioms ' + n for n in names] + ['end FrozenObserverExport']) + '\n', names)
    for start in range(0, len(shared), size):
        names = [b.split()[2] for b in shared[start:start + size]]
        yield ('\n\n'.join(prelude +
               ['#print axioms ' + n for n in names] + ['end FrozenObserverExport']) + '\n', names)


def atomic_report(path, data):
    """Replace the invocation status atomically; never retain a stale success."""
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, prefix='.observer-report-',
                                     delete=False) as stream:
        temporary = Path(stream.name)
        json.dump(data, stream, indent=2)
        stream.write('\n')
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def verify(args, *, emit, modules, producers, scope):
    require(sha256(args.export) == args.sha256, 'Frozen export hash mismatch')
    require(len({p.resolve() for p in [args.output, args.export, args.report, args.provenance]}) == 4,
            'Output paths must be distinct')
    provenance = load(args.provenance)
    require(provenance['export_sha256'] == args.sha256, 'Provenance/export mismatch')
    provenance_hash = sha256(args.provenance)
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [*producers, Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    captured = {p: p.read_bytes() for p in sources}
    before = {str(p.relative_to(ROOT)): hashlib.sha256(content).hexdigest()
              for p, content in captured.items()}
    require(before == {str(p.relative_to(ROOT)): sha256(p) for p in sources},
            'Sources changed while capturing verification snapshot')
    text, counts = emit(load(args.export))
    args.output = args.output.resolve()
    args.output.write_text(text)
    artifacts = Path(tempfile.mkdtemp(prefix=args.output.stem + '-', dir=args.output.parent))
    aggregate = artifacts / 'claims.lean'
    aggregate.write_text(text)
    for path, content in captured.items():
        destination = artifacts / 'model' / path.relative_to(ROOT)
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)
    # Compile and check one immutable source set with private build products.
    # Concurrent checkout edits cannot change the model halfway through replay.
    with tempfile.TemporaryDirectory(prefix='verum-observer-model-') as directory:
        snapshot = Path(directory)
        for path, content in captured.items():
            destination = snapshot / path.relative_to(ROOT)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(content)
        prefix = Path(require_success(run('lake', 'env', 'lean', '--print-prefix', cwd=snapshot)))
        version = require_success(run('lake', 'env', 'lean', '--version', cwd=snapshot))
        binaries = [prefix / 'bin/lean'] + sorted(p for p in (prefix / 'lib/lean').iterdir()
            if p.is_file() and p.suffix in {'.dylib', '.so', '.dll'})
        checker_hashes = {str(p.relative_to(prefix)): sha256(p) for p in binaries}
        require_success(run('lake', 'build', *('VerumExternalReplay.' + m for m in modules), cwd=snapshot))
        shared_module = 'FrozenObserverDeclarations'
        shared_text = declarations(text)
        shared_source = snapshot / (shared_module + '.lean')
        shared_source.write_text(shared_text)
        retained_shared = artifacts / 'shared-declarations.lean'
        retained_shared.write_text(shared_text)
        compiled_shared = snapshot / '.lake/build/lib/lean' / (shared_module + '.olean')
        require_success(run('lake', 'env', 'lean', '-o', str(compiled_shared), str(shared_source), cwd=snapshot))
        shared_hash = sha256(compiled_shared)
        results = []
        for index, (part, names) in enumerate(batches(text, shared_module=shared_module)):
            path = artifacts / f'part{index}.lean'
            require(path.resolve() not in {p.resolve() for p in
                    [args.output, args.export, args.report, args.provenance]}, 'Batch path collision')
            path.write_text(part)
            result = check(path, cwd=snapshot)
            rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
            require(len(rosters) == len(names), 'Missing theorem axiom audit')
            for roster in rosters:
                require(roster.endswith('depends on axioms: [propext, Quot.sound]') or
                        roster.endswith('depends on axioms: [propext]') or
                        roster.endswith('does not depend on any axioms'), 'Unexpected theorem axioms')
            results.append({'path': str(path), 'claims': names, **result})
            print(f'Checked proof batch {index + 1}: {len(names)} claims', flush=True)
        require(sha256(compiled_shared) == shared_hash and shared_source.read_text() == shared_text,
                'Shared declarations changed during verification')
        require(checker_hashes == {str(p.relative_to(prefix)): sha256(p) for p in binaries},
                'Checker files changed during verification')
    require(sum(len(r['claims']) for r in results) == counts['claims'], 'Incomplete claim replay')
    live = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require(all(before[str(p.relative_to(ROOT))] == live[str(p.relative_to(ROOT))]
                for p in sources if p.suffix == '.py') and sha256(args.export) == args.sha256
            and sha256(args.provenance) == provenance_hash, 'Producer/export changed during verification')
    report = {'scope': scope,
              'export_sha256': args.sha256, 'source_sha256': before, 'counts': counts,
              'provenance_sha256': provenance_hash, 'reported_rust_provenance': provenance,
              'aggregate_source_sha256': sha256(aggregate),
              'aggregate_source': str(aggregate), 'model_sources': str(artifacts / 'model'),
              'shared_declarations': {'source': str(retained_shared), 'source_sha256': sha256(retained_shared),
                                      'compiled_sha256': shared_hash},
              'model_build': 'private immutable source snapshot',
              'checkout_changed_since_snapshot': [name for name in before if before[name] != live[name]],
              'lean_version': version,
              'checker_sha256': checker_hashes,
              'proof_batches': results}
    return report


def main(*, emit, modules, producers, scope, description):
    parser = argparse.ArgumentParser(description=description)
    parser.add_argument('--export', type=Path, required=True)
    parser.add_argument('--sha256', required=True, help='Expected hash from the export owner')
    parser.add_argument('--provenance', type=Path, required=True,
                        help='Frozen source and Rust binary identities supplied by the owner')
    parser.add_argument('--output', type=Path, required=True, help='Retained Lean proof artifact')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    require(len({p.resolve() for p in [args.output, args.export, args.report, args.provenance]}) == 4,
            'Output paths must be distinct')
    invocation = {'run_id': uuid.uuid4().hex, 'requested_export_sha256': args.sha256}
    atomic_report(args.report, {**invocation, 'status': 'running'})
    try:
        report = verify(args, emit=emit, modules=modules, producers=producers, scope=scope)
    except Exception as error:
        atomic_report(args.report, {**invocation, 'status': 'failed',
                                   'error': f'{type(error).__name__}: {error}'[:8192]})
        raise
    atomic_report(args.report, {**invocation, 'status': 'verified', **report})
    print('Checked observer export: ' + json.dumps(report['counts']))


