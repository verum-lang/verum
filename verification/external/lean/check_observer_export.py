#!/usr/bin/env python3
"""Check a pinned Rust observer export with the Lean kernel.

The Python layer only decodes data and emits constructors/equality claims. Code
lowering and evaluation use FiniteCode and TypedPrograms in Lean. No Rust VM is
replaced. Passing is finite conformance of the supplied observations, not a
universal Rust implementation proof or authentication of execution provenance.
"""
import argparse
import json
from pathlib import Path
import re
from check_typed_programs import ROOT, check, require_success, run, sha256

OPS = {name: name[0].lower() + name[1:] for name in
       ('First', 'Second', 'CheckedAdd', 'Select', 'Rem', 'PackByte', 'Pair', 'Equal')}
PORTS = {'Carrier', 'Key', 'LifeAge', 'UnderClock', 'Pose'}
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


def value(obj):
    kind, data = variant(obj)
    if kind == 'Nat':
        return f'(.nat {natural(data)})'
    if kind == 'Bool':
        return f'(.bool {boolean(data)})'
    require(kind == 'Pair' and type(data) is list and len(data) == 2, 'Invalid Value')
    return f'(.pair {value(data[0])} {value(data[1])})'


def operation(op):
    require(type(op) is str and op in OPS, 'Unknown operation')
    return '.' + OPS[op]


def operand(obj):
    kind, data = variant(obj)
    if kind == 'Literal':
        return f'(.literal {value(data)})'
    require(kind in {'Input', 'Register'}, 'Unknown operand')
    return f'(.{kind.lower()} {natural(data)})'


def code(obj, depth=0):
    require(depth <= 64, 'Code tree exceeds extraction depth limit')
    fields(obj, 'steps output library')
    instructions = []
    for step in sequence(obj['steps']):
        fields(step, 'operation arguments')
        kind, data = variant(step['operation'])
        if kind == 'Primitive':
            op = '.primitive ' + operation(data)
        else:
            require(kind == 'Call', 'Unknown dispatch')
            op = '.call ' + natural(data)
        instructions.append('⟨' + op + ', ' + array(operand(a) for a in
                                                  sequence(step['arguments'])) + '⟩')
    output = 'none' if obj['output'] is None else '(some ' + natural(obj['output']) + ')'
    return '(.mk ' + array(instructions) + ' ' + output + ' ' + array(
        code(p, depth + 1) for p in sequence(obj['library'])) + ')'


def descriptor(obj):
    fields(obj, 'input_epoch age_mod events_mod axes support')
    natural(obj['input_epoch'], 2**128 - 1)
    natural(obj['age_mod'], 255)
    natural(obj['events_mod'], 255)
    axes = sequence(obj['axes'])
    require(len(axes) == 3, 'Expected three axes')
    for b in axes:
        boolean(b)
    require(sum(axes) <= 1, 'Incompatible axes')
    support = sequence(obj['support'])
    for n in support:
        natural(n)
    require(support == sorted(set(support)), 'Descriptor support is not canonical')


def binding(obj):
    fields(obj, 'descriptor ports width slots')
    descriptor(obj['descriptor'])
    width = natural(obj['width'])
    ports = sequence(obj['ports'])
    require(all(type(p) is str and p in PORTS for p in ports), 'Unknown port')
    require(len(ports) == obj['width'], 'Port/width mismatch')
    slots = []
    for s in sequence(obj['slots']):
        if s == 'State':
            slots.append('.state')
        else:
            kind, n = variant(s)
            require(kind == 'External', 'Unknown binding slot')
            natural(n)
            require(n < obj['width'], 'Binding index outside declared width')
            slots.append('(.external ' + str(n) + ')')
    return width + ' ' + array(slots)


def refusal(text):
    require(type(text) is str, 'Expected error string')
    if text in ('Overflow', 'ZeroDivisor'):
        return '.' + text[0].lower() + text[1:]
    if m := re.fullmatch(r'ByteOverflow \{ value: (\d+) \}', text):
        return '(.byteOverflow ' + natural(int(m[1])) + ')'
    if m := re.fullmatch(r'Sort \{ operation: (\w+), argument: (\d+) \}', text):
        return '(.sort ' + operation(m[1]) + ' ' + natural(int(m[2])) + ')'
    m = re.fullmatch(r'Arity \{ operation: (\w+), expected: (\d+), actual: (\d+) \}', text)
    require(m is not None, 'Unknown refusal; do not erase it to semantic failure')
    return '(.arity ' + operation(m[1]) + ' ' + natural(int(m[2])) + ' ' + natural(int(m[3])) + ')'


def unique_object(pairs):
    result = {}
    for k, v in pairs:
        require(k not in result, f'Duplicate JSON key: {k}')
        result[k] = v
    return result


def load(path):
    require(path.stat().st_size <= 16 * 1024 * 1024, 'Export exceeds 16 MiB limit')
    return json.loads(path.read_text(), object_pairs_hook=unique_object)


def emit(data):
    fields(data, 'schema programs primitives metered_work')
    require(data['schema'] == 'synarc-observer-rust-boundary-v1', 'Unknown export schema')
    natural(data['metered_work'])
    programs = sequence(data['programs'])
    primitives = sequence(data['primitives'])
    require(programs and primitives, 'Empty export is not conformance evidence')
    lines = ['import VerumExternalReplay.CheckedPairs', 'open CheckedPairs',
             'set_option warningAsError true', 'namespace FrozenObserverExport']
    claims = []

    def claim(name, equation, proof='rfl'):
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')
        claims.append(name)

    traces = 0
    for i, program in enumerate(programs):
        fields(program, 'description executions')
        d = fields(program['description'],
                   'schema_version semantics descriptor initial update read update_binding read_binding')
        require(type(d['schema_version']) is int and d['schema_version'] == 1
                and d['semantics'] == 'CheckedPairsV1', 'Unknown semantic version')
        descriptor(d['descriptor'])
        require(d['descriptor'] == d['update_binding']['descriptor'],
                'Top-level descriptor differs from actual update binding')
        # Each binding is retained; do not substitute the update descriptor for read.
        bindings = {k: binding(d[k + '_binding']) for k in ('update', 'read')}
        for k in bindings:
            lines.append(f'def p{i}_{k} : Code := {code(d[k])}')
        previous = d['initial']
        rows = sequence(program['executions'])
        require(rows, 'Program has no execution observations')
        for j, row in enumerate(rows):
            fields(row, 'state query read event next_state')
            prefix = f'p{i}_t{j}'
            claim(prefix + '_cut', f'( {value(previous)} : Value) = {value(row["state"])}')
            event = array('(.bool ' + boolean(b) + ')' for b in sequence(row['event']))
            query = array('(.nat ' + natural(n) + ')' for n in sequence(row['query']))
            for kind, external, result in [('update', event, row['next_state']),
                                           ('read', query, row['read'])]:
                equation = f'(FiniteCode.bind {bindings[kind]} {value(row["state"])} {external}).bind '
                equation += f'(run p{i}_{kind}) = some {value(result)}'
                claim(prefix + '_' + kind, equation,
                      f'simp [FiniteCode.bind, List.mapM, List.mapM.loop]\n  unfold p{i}_{kind} run\n  reduce_finite_code')
            previous = row['next_state']
            traces += 1
    for i, p in enumerate(primitives):
        fields(p, 'operation arguments result error')
        if p['result'] is None:
            expected = '.error ' + refusal(p['error'])
        else:
            require(p['error'] is None, 'Both success and failure supplied')
            expected = '.ok ' + value(p['result'])
        claim(f'primitive_{i}', 'apply ' + operation(p['operation']) + ' ' +
              array(value(a) for a in sequence(p['arguments'])) + ' = ' + expected)
    # Every generated theorem's axiom roster is audited by the driver.
    lines += ['#print axioms ' + name for name in claims]
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', {
        'programs': len(programs), 'trace_rows': traces, 'primitive_cases': len(primitives),
        'claims': len(claims)}


def batches(source, size=8):
    """Bound each kernel invocation without weakening its acceptance deadline.

    Claims are closed equalities, independent of other claims. Every batch
    retains all code definitions and audits every included theorem's axioms.
    """
    blocks = source.strip().split('\n\n')
    definitions = [b for b in blocks if b.startswith('def ')]
    claims = [b for b in blocks if b.startswith('theorem ')]
    for start in range(0, len(claims), size):
        group = claims[start:start + size]
        names = [b.split()[1] for b in group]
        yield ('\n\n'.join(blocks[:4] + definitions + group +
               ['#print axioms ' + n for n in names] + ['end FrozenObserverExport']) + '\n', names)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--export', type=Path, required=True)
    parser.add_argument('--sha256', required=True, help='Expected hash from the export owner')
    parser.add_argument('--provenance', type=Path, required=True,
                        help='Frozen source and Rust binary identities supplied by the owner')
    parser.add_argument('--output', type=Path, required=True, help='Retained Lean proof artifact')
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    require(sha256(args.export) == args.sha256, 'Frozen export hash mismatch')
    require(len({p.resolve() for p in [args.output, args.export, args.report, args.provenance]}) == 4,
            'Output paths must be distinct')
    provenance = load(args.provenance)
    require(provenance['export_sha256'] == args.sha256, 'Provenance/export mismatch')
    provenance_hash = sha256(args.provenance)
    modules = ['TypedPrograms', 'FiniteCode', 'CheckedPairs']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
                ROOT / 'lean-toolchain', ROOT / 'lakefile.toml']
    before = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    text, counts = emit(load(args.export))
    args.output.write_text(text)
    require_success(run('lake', 'build', 'VerumExternalReplay.CheckedPairs'))
    results = []
    for index, (part, names) in enumerate(batches(text)):
        path = args.output.with_name(args.output.stem + f'.part{index}.lean')
        require(path.resolve() not in {p.resolve() for p in
                [args.output, args.export, args.report, args.provenance]}, 'Batch path collision')
        path.write_text(part)
        result = check(path)
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        require(len(rosters) == len(names), 'Missing theorem axiom audit')
        for roster in rosters:
            require(roster.endswith('depends on axioms: [propext, Quot.sound]') or
                    roster.endswith('does not depend on any axioms'), 'Unexpected theorem axioms')
        results.append({'path': str(path), 'claims': names, **result})
        print(f'Checked proof batch {index + 1}: {len(names)} claims', flush=True)
    require(sum(len(r['claims']) for r in results) == counts['claims'], 'Incomplete claim replay')
    require(before == {str(p.relative_to(ROOT)): sha256(p) for p in sources}
            and sha256(args.export) == args.sha256 and sha256(args.provenance) == provenance_hash,
            'Inputs changed during verification')
    prefix = Path(require_success(run('lake', 'env', 'lean', '--print-prefix')))
    binaries = [prefix / 'bin/lean'] + sorted(p for p in (prefix / 'lib/lean').iterdir()
        if p.is_file() and p.suffix in {'.dylib', '.so', '.dll'})
    report = {'scope': 'Finite Rust observations agree with checked typed-program lowering and '
              'independent CheckedPairsV1 semantics. Not universal Rust correspondence, '
              'native Verum replay, resource/ownership correctness, or execution provenance.',
              'export_sha256': args.sha256, 'source_sha256': before, 'counts': counts,
              'provenance_sha256': provenance_hash, 'reported_rust_provenance': provenance,
              'aggregate_source_sha256': sha256(args.output),
              'lean_version': require_success(run('lake', 'env', 'lean', '--version')),
              'checker_sha256': {str(p.relative_to(prefix)): sha256(p) for p in binaries},
              'proof_batches': results}
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print('Checked observer export: ' + json.dumps(counts))


if __name__ == '__main__':
    main()
