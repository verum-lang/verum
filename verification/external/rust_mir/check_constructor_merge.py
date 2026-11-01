#!/usr/bin/env python3
"""Source-bound, universal constructor merge/frame correspondence in Lean.

Only the unchanged selected Rust method is covered. Caller alias validity,
Vec indexing, Arc operations, concurrency and the rest of wake/advance are not
proved here. The compiler, metadata and frontend remain trusted inputs.
"""
import argparse
import gzip
import io
import os
from pathlib import Path
import sys
import uuid

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT.parent / 'kani'))
from rust_replay import load, read, require, sha, write
from record_mir import Layout, RecordTranslation
from runtime import RUST_VERSION, execute, runtime_identity

FIXTURES = ROOT.parent / 'lean/fixtures'
MODELS = ['AppendHandoff', 'WakeHandoff']
HEADER = 'import VerumExternalReplay.WakeHandoff\nset_option warningAsError true\n\n'


def inflated(path):
    with gzip.GzipFile(fileobj=io.BytesIO(read(path))) as stream:
        data = stream.read(16 * 1024 * 1024 + 1)
    require(len(data) <= 16 * 1024 * 1024, 'Oversized compressed input')
    return data


def prepare(out):
    """Bind reviewed byte ranges to complete native inputs, never a source mirror."""
    manifest_bytes = read(ROOT / 'constructor-merge.json')
    manifest = load(manifest_bytes)
    require(manifest['format'] == 'verum-rust-record-source/1', 'Wrong selection schema')
    provenance_bytes = read(FIXTURES / manifest['provenance'])
    provenance = load(provenance_bytes)
    receipts = {}
    for name, digest in [('receipt', 'receipt_sha256'), ('build-receipt', 'build_receipt_sha256')]:
        data = inflated(FIXTURES / f'append-handoff-native-v5.{name}.json.gz')
        require(sha(data) == provenance[digest], 'Changed native receipt')
        receipts[name] = load(data)
        require(receipts[name]['binary'] == provenance['native_binary'], 'Binary identity mismatch')
        (out / f'native-{name}.json').write_bytes(data)
    require(receipts['receipt']['source'] == receipts['build-receipt']['source'], 'Build/run inputs differ')
    sources = {}
    for name, spec in manifest['sources'].items():
        require(Path(name).name == name and Path(spec['fixture']).name == spec['fixture'], 'Nonlocal source')
        data = inflated(FIXTURES / spec['fixture'])
        original = [s for s in provenance['sources'] if s['path'] == spec['native_path']]
        require(len(original) == 1 and original[0]['fixture'] == spec['fixture'] and
                len(data) == original[0]['bytes'] and sha(data) == original[0]['sha256'] ==
                spec['sha256'] == receipts['receipt']['source'][spec['native_path']]['sha256'],
                'Source is not the pinned native build input')
        sources[name] = data
        (out / name).write_bytes(data)
    intervals = {}
    def selected(spec):
        start, end = spec['start'], spec['end']
        data = sources[spec['source']]
        require(type(start) is int and type(end) is int and 0 <= start < end <= len(data), 'Invalid byte range')
        existing = intervals.setdefault(spec['source'], [])
        require(all(end <= a or start >= b for a, b in existing), 'Overlapping selections')
        existing.append((start, end))
        fragment = data[start:end]
        require(sha(fragment) == spec['sha256'], 'Changed reviewed declaration')
        return fragment
    declarations = b'\n\n'.join(selected(d) for d in manifest['declarations'])
    method = selected(manifest['method'])
    require(manifest['record'] == 'Constructor' and manifest['function'] == 'merge', 'Wrong proof target')
    assembled = b'use std::sync::Arc;\n\n' + declarations + b'\n\nimpl Constructor {\n' + method + b'\n}\n'
    (out / 'selection.json').write_bytes(manifest_bytes)
    (out / 'provenance.json').write_bytes(provenance_bytes)
    return assembled, method, {
        'selection_sha256': sha(manifest_bytes), 'provenance_sha256': sha(provenance_bytes),
        'native_run_status': receipts['receipt']['status'],
        'native_build_receipt_status': receipts['build-receipt']['status'],
        'premises': manifest['premises'],
    }


def specification(layout):
    """A complete typed projection: every non-wake field becomes part of the frame."""
    needed = {'phase': 'ConstructorPhase', 'scan': 'usize', 'newest': 'std::option::Option<usize>',
              'watermark': 'usize', 'observed': 'usize'}
    for name, kind in needed.items():
        require(layout.fields[layout.index(name)].kind == kind, 'Model field type mismatch')
    phase = layout.enums['ConstructorPhase']
    require(set(phase) == {'Merge', 'Freeze', 'Anchor', 'Bounds', 'Tuple', 'Reset', 'Carry', 'Idle'},
            'Unreviewed constructor phase')
    frame = [i for i, f in enumerate(layout.fields) if f.name not in needed]
    require(frame, 'Missing retained frame')
    field = lambda name: f'field{layout.index(name)}'
    frame_type = ' × '.join(layout.fields[i].lean for i in frame)
    values = ', '.join(f'c.field{i}' for i in frame)
    phase_cases = '\n'.join(f'    | {layout.enum_value("ConstructorPhase", name)} => "{name}"' for name in phase)
    implicit = f' {{{" ".join(layout.opaque)} : Type}}' if layout.opaque else ''
    expected = f'{{ before with {field("phase")} := {layout.enum_value("ConstructorPhase", "Merge")}, ' + \
               f'{field("scan")} := 0, {field("newest")} := none, {field("watermark")} := indexed }}'
    projection = f'''
def project{implicit} (c : {layout.record_type()}) : WakeHandoff.Constructor ({frame_type}) :=
  {{ phase := match c.{field('phase')} with
{phase_cases}
    scan := c.{field('scan')}, newest := c.{field('newest')}, watermark := c.{field('watermark')},
    observed := c.{field('observed')}, rest := ⟨{values}⟩ }}
'''
    proof = f'''
-- Full record equality includes the opaque Vec/Option<Tuple> fields; there is
-- no hypothesis assuming frame preservation and no finite state enumeration.
theorem full_record_exact{implicit} (before : {layout.record_type()}) (indexed : Nat) :
    extracted before indexed = ({expected}, Exit.returned) := by rfl

theorem model_exact{implicit} (before : {layout.record_type()}) (indexed : Nat) :
    project (extracted before indexed).1 = WakeHandoff.merge indexed (project before) := by rfl

theorem enabled_wake_exact{implicit} (before : {layout.record_type()}) (indexed : Nat)
    (enabled : WakeHandoff.needsWake indexed (project before) = true) :
    project (extracted before indexed).1 = WakeHandoff.wake indexed (project before) := by
  unfold WakeHandoff.wake
  exact (model_exact before indexed).trans (ite_eq_left enabled).symm

#print axioms full_record_exact
#print axioms model_exact
#print axioms enabled_wake_exact
'''
    # Every retained component has a distinct value, including a nonempty newest.
    concrete = []
    for i, f in enumerate(layout.fields):
        if f.kind == 'ConstructorPhase': concrete.append(layout.enum_value(f.kind, 'Idle'))
        elif f.kind == 'std::option::Option<usize>': concrete.append('(some 29)')
        else:
            require(f.kind in (None, 'usize'), 'Unreviewed counterexample carrier')
            concrete.append(str(11 + i))
    instance = 'Record' + ' Nat' * len(layout.opaque)
    witness = f'\ndef before : {instance} := ⟨{", ".join(concrete)}⟩\ndef indexed : Nat := 47\n'
    false_claim = f'\nexample : extracted before indexed = ({expected}, Exit.returned) := by decide_cbv\n'
    return projection, proof, witness + false_claim


NATIVE_CONTROL = b'''
fn main() {
    let child = Arc::new(Expression { owner: Arc::new(()), body: Body::Leaf(7),
        summary: Summary { outputs: Arc::from([13u64]), dependencies: 17 }, size: 19 });
    let mut c = Constructor { alternative: 23,
        axes: vec![Axis { domain: 29, consumed: 31, before: 37, after: 41, coordinate: 43 }],
        phase: ConstructorPhase::Idle, scan: 47, newest: Some(53), anchor: 59, carry: 61,
        watermark: 67, observed: 71, tuple: Some(Tuple { alternative: 73,
            children: vec![Arc::clone(&child)], outputs: vec![79], values: vec![83],
            phase: TuplePhase::Values, at: 89, case: 97, reset_from: 101, size: 103,
            dependencies: 107, valid: 109, possible: 113, zero_value: true }) };
    let axes = c.axes.as_ptr();
    let tuple = c.tuple.as_ref().unwrap();
    let children = tuple.children.as_ptr();
    let outputs = tuple.outputs.as_ptr();
    let values = tuple.values.as_ptr();
    for indexed in [0, 1, 47, usize::MAX] {
        c.merge(indexed);
        assert_eq!(c.watermark, indexed);
        assert!(c.phase == ConstructorPhase::Merge && c.scan == 0 && c.newest.is_none());
        assert_eq!((c.alternative,c.anchor,c.carry,c.observed), (23,59,61,71));
        assert_eq!(c.axes.as_ptr(), axes);
        assert_eq!(c.axes.len(), 1);
        let a = &c.axes[0];
        assert_eq!((a.domain,a.consumed,a.before,a.after,a.coordinate), (29,31,37,41,43));
        let t = c.tuple.as_ref().unwrap();
        assert_eq!((t.alternative,t.at,t.case,t.reset_from,t.size), (73,89,97,101,103));
        assert_eq!((t.dependencies,t.valid,t.possible,t.zero_value), (107,109,113,true));
        assert!(t.phase == TuplePhase::Values);
        assert_eq!(t.children.as_ptr(), children);
        assert_eq!(t.children.len(), 1);
        assert_eq!(t.outputs.as_ptr(), outputs);
        assert_eq!(t.values.as_ptr(), values);
        assert_eq!(t.outputs, [79]); assert_eq!(t.values, [83]);
        assert!(Arc::ptr_eq(&t.children[0], &child));
        assert_eq!(Arc::strong_count(&child), 2);
        assert_eq!(Arc::strong_count(&child.owner), 1);
    }
}
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', type=Path, required=True)
    parser.add_argument('--lean', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    inputs = [*ROOT.glob('*.py'), ROOT/'constructor-merge.json', ROOT.parent/'kani/rust_replay.py',
              *(ROOT.parent/f'lean/VerumExternalReplay/{name}.lean' for name in MODELS)]
    inputs = {p: read(p) for p in inputs}
    report = {'status': 'inconclusive', 'run_id': str(uuid.uuid4()), 'scope': __doc__, 'results': []}
    write(out/'receipt.json', report)
    try:
        source, method, provenance = prepare(out)
        report.update(provenance)
        rustc, lean = args.rustc.resolve(), args.lean.resolve()
        rustdoc = rustc.with_name('rustdoc')
        identities = {}
        for name, tool, expected in [('rust', rustc, RUST_VERSION),
                                     ('rustdoc', rustdoc, RUST_VERSION.replace('rustc ', 'rustdoc ')),
                                     ('lean', lean, 'Lean (version 4.35.0,')]:
            code, version, _ = execute([str(tool), '--version'], out, out/f'{name}-version.log')
            require(code == 0 and (version.startswith(expected) if name == 'lean' else version.strip() == expected),
                    'Unreviewed compiler/checker version')
            identities[tool] = runtime_identity(tool)
            write(out/f'{name}-toolchain.json', identities[tool])
        code, cfg, _ = execute([str(rustc), '--print', 'cfg'], out, out/'target.log')
        require(code == 0 and 'target_pointer_width="64"' in cfg.splitlines(), 'Expected a 64-bit target')
        env = {k:v for k,v in os.environ.items() if not k.startswith('LEAN_')}
        env['LEAN_PATH'] = str(out)
        checker = [str(lean), '-M', '1024', '--json']
        for name in MODELS:
            model = out/f'VerumExternalReplay/{name}.lean'
            model.parent.mkdir(exist_ok=True)
            model.write_bytes(inputs[ROOT.parent/f'lean/VerumExternalReplay/{name}.lean'])
            code, _, _ = execute([*checker, '-o', str(model.with_suffix('.olean')), str(model)], out, out/f'{name}.log', env)
            require(code == 0, 'Model failed to compile')
        native = source + NATIVE_CONTROL
        (out/'native.rs').write_bytes(native)
        code, _, _ = execute([str(rustc), '--edition=2021', '-C', 'overflow-checks=yes',
                              'native.rs', '-o', 'native'], out, out/'native-compile.log')
        require(code == 0, 'Native ownership control failed to compile')
        code, _, _ = execute([str(out/'native')], out, out/'native-run.log')
        require(code == 0 and read(out/'native.rs') == native, 'Native ownership control failed or changed')
        mutants = [
            ('wrong_watermark', b'self.watermark = indexed', b'self.watermark = 0'),
            ('missing_newest_reset', b'self.newest = None;', b''),
            ('missing_scan_reset', b'self.scan = 0;', b''),
            ('wrong_phase', b'self.phase = ConstructorPhase::Merge', b'self.phase = ConstructorPhase::Idle'),
            ('corrupted_observed', b'self.scan = 0;', b'self.scan = 0; self.observed = 0;'),
            ('corrupted_anchor', b'self.scan = 0;', b'self.scan = 0; self.anchor = 0;'),
        ]
        cases = [('exact', source, False)]
        for name, before, after in mutants:
            require(method.count(before) == 1 and source.count(method) == 1, 'Ambiguous mutation')
            cases.append((name, source.replace(method, method.replace(before, after)), True))
        for name, data, negative in cases:
            case = out/name
            case.mkdir()
            (case/'transaction.rs').write_bytes(data)
            command = [str(rustc), '--crate-type=lib', '--crate-name=transaction', '--edition=2021',
                       '--emit=mir', '-C', 'overflow-checks=yes', 'transaction.rs', '-o', 'transaction.mir']
            code, _, _ = execute(command, case, case/'compiler.log')
            require(code == 0, 'Rust MIR compilation failed')
            doc_command = [str(rustdoc), '--crate-name=transaction', '--edition=2021', '-Z', 'unstable-options',
                           '--document-private-items', '--output-format=json', 'transaction.rs', '-o', 'metadata']
            code, _, _ = execute(doc_command, case, case/'rustdoc.log')
            require(code == 0, 'Rust field metadata failed to compile')
            mir, metadata = read(case/'transaction.mir'), read(case/'metadata/transaction.json')
            layout = Layout(load(metadata), 'Constructor', 'merge')
            generated = RecordTranslation(mir.decode(), 'merge', layout).lean()
            projection, proof, counterexample = specification(layout)
            generated = HEADER + generated + (counterexample if negative else projection + proof)
            (case/'Replay.lean').write_text(generated)
            code, stdout, stderr = execute([*checker, str(case/'Replay.lean')], out, case/'lean.log', env)
            require(not stderr, 'Unexpected Lean stderr')
            diagnostics = [load(line) for line in stdout.splitlines() if line.strip()]
            errors = [d for d in diagnostics if d['severity'] == 'error']
            require(not any(d['severity'] == 'warning' for d in diagnostics), 'Lean warning')
            if negative:
                require(code == 1 and len(errors) == 1 and errors[0]['data'].startswith(
                    '`decide_cbv` failed: the proposition evaluates to `false`'),
                    'Mutation did not reach a semantic counterexample')
            else:
                require(code == 0 and not errors, 'Universal record correspondence failed')
                infos = [d['data'] for d in diagnostics if d['severity'] == 'information']
                require(infos == [f"'{n}' does not depend on any axioms" for n in
                                  ['full_record_exact', 'model_exact', 'enabled_wake_exact']],
                        'Unexpected theorem axiom roster')
            require(read(case/'transaction.rs') == data and read(case/'transaction.mir') == mir and
                    read(case/'metadata/transaction.json') == metadata and
                    (case/'Replay.lean').read_text() == generated, 'Replay input changed')
            report['results'].append({'case': name, 'source_sha256': sha(data), 'mir_sha256': sha(mir),
                'metadata_sha256': sha(metadata), 'lean_sha256': sha(generated.encode()),
                'compiler_command': command, 'metadata_command': doc_command, 'diagnostics': diagnostics})
            print(name, 'passed', flush=True)
        require(all(read(p) == data for p,data in inputs.items()), 'Verifier/model changed during replay')
        require(all(runtime_identity(tool) == identity for tool,identity in identities.items()), 'Toolchain changed')
        report['inputs'] = {str(p): sha(data) for p,data in inputs.items()}
        report['artifacts'] = {str(p.relative_to(out)): sha(read(p)) for p in out.rglob('*')
                               if p.is_file() and p.name != 'receipt.json'}
        report['status'] = 'verified'
    except BaseException as error:
        report['error'] = repr(error)
        raise
    finally:
        write(out/'receipt.json', report)


if __name__ == '__main__':
    main()
