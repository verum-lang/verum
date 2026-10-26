#!/usr/bin/env python3
"""Check a pinned Rust observer export with the Lean kernel.

The Python layer only decodes data and emits constructors/equality claims. Code
lowering and evaluation use FiniteCode and TypedPrograms in Lean. No Rust VM is
replaced. Passing is finite conformance of the supplied observations, not a
universal Rust implementation proof or authentication of execution provenance.
"""
import argparse
import hashlib
import tempfile
import uuid
import json
import os
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


def code(obj, depth=0, *, declared_arity=None):
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
                                                  sequence(step['arguments'])) + ', ' +
                            ('none' if declared_arity is None else '(some ' + declared_arity + ')') + '⟩')
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


def option(data, encode):
    return 'none' if data is None else '(some ' + encode(data) + ')'


def slots(data):
    result = []
    for item in sequence(data):
        if item == 'State':
            result.append('.state')
        else:
            kind, n = variant(item)
            require(kind == 'External', 'Unknown binding slot')
            result.append('(.external ' + natural(n) + ')')
    return array(result)


def interface(data):
    fields(data, 'descriptor ports')
    d = data['descriptor']
    descriptor(d)
    ports = sequence(data['ports'])
    require(all(type(p) is str and p in PORTS for p in ports), 'Unknown port')
    desc = '⟨' + ', '.join([natural(d['input_epoch'], 2**128 - 1), natural(d['age_mod']),
        natural(d['events_mod']), '⟨' + ', '.join(boolean(b) for b in d['axes']) + '⟩',
        array(natural(n) for n in d['support'])]) + '⟩'
    return '⟨' + desc + ', ' + array('.' + p[0].lower() + p[1:] for p in ports) + '⟩'


def admission_binding(data):
    binding(data)  # Includes the constructor's slot bounds.
    return '⟨' + interface({k: data[k] for k in ('descriptor', 'ports')}) + ', ' + \
        natural(data['width']) + ', ' + slots(data['slots']) + '⟩'


def dispatch_limit(data):
    if data == 'Unlimited':
        return 'none'
    tag, n = variant(data)
    require(tag == 'Dispatches', 'Unknown execution limit')
    return '(some ' + natural(n) + ')'


def work_error(data):
    fields(data, 'used limit requested')
    return '⟨' + ', '.join(natural(data[k]) for k in ('used', 'limit', 'requested')) + '⟩'


def arithmetic_error(data):
    require(type(data) is dict and type(data.get('kind')) is str, 'Invalid arithmetic error')
    tag = data['kind']
    if tag in ('Overflow', 'ZeroDivisor'):
        fields(data, 'kind')
        return '.' + tag[0].lower() + tag[1:]
    if tag == 'ByteOverflow':
        fields(data, 'kind value')
        return '(.byteOverflow ' + natural(data['value']) + ')'
    if tag == 'Sort':
        fields(data, 'kind operation argument')
        return '(.sort ' + operation(data['operation']) + ' ' + natural(data['argument']) + ')'
    require(tag == 'Arity', 'Unknown arithmetic error')
    fields(data, 'kind operation expected actual')
    return '(.arity ' + operation(data['operation']) + ' ' + natural(data['expected']) + ' ' + natural(data['actual']) + ')'


def failure(data):
    require(type(data) is dict and type(data.get('kind')) is str, 'Invalid machine failure')
    tag = data['kind']
    if tag in ('MissingCallee', 'MissingCalleeOutput', 'Allowance', 'CounterCapacity'):
        fields(data, 'kind')
        return '.' + tag[0].lower() + tag[1:]
    if tag == 'MissingOperand':
        fields(data, 'kind argument')
        return '(.missingOperand ' + natural(data['argument']) + ')'
    if tag in ('MissingInput', 'MissingRegister'):
        fields(data, 'kind argument index')
        return '(.' + tag[0].lower() + tag[1:] + ' ' + natural(data['argument']) + ' ' + natural(data['index']) + ')'
    require(tag == 'Primitive', 'Unknown machine failure')
    fields(data, 'kind error')
    return '(.primitive ' + arithmetic_error(data['error']) + ')'


def halt(data):
    fields(data, 'path reason')
    return '⟨' + array(natural(n) for n in sequence(data['path'])) + ', ' + failure(data['reason']) + '⟩'


def observation(data):
    fields(data, 'stop registers dispatch_attempts expanded_attempts call_attempts halt')
    require(data['stop'] in ('Complete', 'Undefined', 'Budget'), 'Unknown execution stop')
    return '⟨.' + data['stop'].lower() + ', ' + array(value(v) for v in sequence(data['registers'])) + ', ' + \
        ', '.join(natural(data[k]) for k in ('dispatch_attempts', 'expanded_attempts', 'call_attempts')) + \
        ', ' + option(data['halt'], halt) + '⟩'


def root_output(data):
    require(type(data) is dict, 'Invalid root output')
    tag = data.get('status')
    if tag in ('Unattempted', 'Missing'):
        fields(data, 'status')
        return '.' + tag.lower()
    require(tag == 'Selected', 'Unknown root selection status')
    fields(data, 'status value')
    return '(.selected ' + value(data['value']) + ')'


def binding_error(data):
    tag = data.get('kind') if type(data) is dict else None
    if tag == 'Interface':
        fields(data, 'kind')
        return '.interface'
    if tag == 'Width':
        fields(data, 'kind expected actual')
        return '(.width ' + natural(data['expected']) + ' ' + natural(data['actual']) + ')'
    require(tag == 'InvalidSlot', 'Unknown binding refusal')
    fields(data, 'kind slot input width')
    return '(.invalidSlot ' + ' '.join(natural(data[k]) for k in ('slot', 'input', 'width')) + ')'


def observer_error(data):
    require(type(data) is dict and data.get('phase') in ('Read', 'Update'), 'Invalid observer phase')
    phase = '.' + data['phase'].lower()
    tag = data.get('kind')
    if tag == 'Poisoned':
        fields(data, 'kind phase')
        return '(.poisoned ' + phase + ')'
    if tag == 'Binding':
        fields(data, 'kind phase reason')
        return '(.binding ' + phase + ' ' + binding_error(data['reason']) + ')'
    if tag in ('Work', 'Capacity'):
        require(data.get('point') in ('InputBinding', 'RootOutput'), 'Unknown work point')
        point = '.' + data['point'][0].lower() + data['point'][1:]
        if tag == 'Capacity':
            fields(data, 'kind phase point')
            return '(.capacity ' + phase + ' ' + point + ')'
        fields(data, 'kind phase point error execution')
        return '(.work ' + phase + ' ' + point + ' ' + work_error(data['error']) + ' ' + \
            option(data['execution'], observation) + ')'
    if tag == 'Execution':
        fields(data, 'kind phase report meter_error counter_capacity')
        return '(.execution ' + phase + ' ' + observation(data['report']) + ' ' + \
            option(data['meter_error'], work_error) + ' ' + boolean(data['counter_capacity']) + ')'
    require(tag == 'MissingRootOutput', 'Unknown observer refusal')
    fields(data, 'kind phase execution')
    return '(.missingRootOutput ' + phase + ' ' + observation(data['execution']) + ')'


def code_view(data):
    if type(data) is dict and 'view' in data:
        fields(data, 'view program declared_arity')
        require(data['view'] == 'declared-arity-operand-hole', 'Unknown Code view')
        p = data['program']
        fields(p, 'steps output library')
        require(len(sequence(p['steps'])) == 1 and p['library'] == [],
                'Declared-arity view has exactly one instruction and no callees')
        return code(p, declared_arity=natural(data['declared_arity']))
    return code(data)


def description(data):
    fields(data, 'schema_version semantics descriptor initial update read update_binding read_binding')
    require(type(data['schema_version']) is int and data['schema_version'] == 1 and
            data['semantics'] == 'CheckedPairsV1', 'Unknown semantic version')
    descriptor(data['descriptor'])
    require(data['descriptor'] == data['update_binding']['descriptor'], 'Update descriptor mismatch')
    for key in ('read', 'update'):
        binding(data[key + '_binding'])
    value(data['initial'])
    return data


def emit_controls(data, lines, claim):
    fields(data, 'machine observer')
    names = set()

    def identity(c, kind):
        require(type(c['name']) is str and c['name'] and c['name'] not in names, 'Missing/duplicate control identity')
        names.add(c['name'])
        require(c['kind'] == kind, 'Unexpected control kind')

    for i, c in enumerate(sequence(data['machine'])):
        fields(c, 'name kind code inputs dispatch_limit work_limit work_used execution meter_error root')
        identity(c, 'machine')
        p, m = f'machine_code_{i}', f'machine_{i}'
        inputs = array(value(v) for v in sequence(c['inputs']))
        lines.append(f'def {p} : Code := {code_view(c["code"])}')
        lines.append(f'def {m} := FiniteExecution.execute apply {p} '
                     f'⟨{dispatch_limit(c["dispatch_limit"])}, {natural(c["work_limit"])}⟩ {inputs}')
        proof = f'unfold {m} {p}\n  reduce_execution'
        claim(m + '_execution', f'{m}.observation = {observation(c["execution"])}', proof)
        claim(m + '_work', f'{m}.usage.work = {natural(c["work_used"])}', proof)
        claim(m + '_meter', f'{m}.meterError = {option(c["meter_error"], work_error)}', proof)
        claim(m + '_root', f'FiniteExecution.selectRoot {p} {m} = {root_output(c["root"])}', proof)
        if c['execution']['stop'] != 'Budget':
            selected = c['root'].get('value') if c['root']['status'] == 'Selected' else None
            projection = f'change FiniteCode.run (FiniteExecutionProjection.partialApply apply) {p} {inputs} = _\n'
            projection += f'  have h : {m}.observation = {observation(c["execution"])} := by\n    unfold {m} {p}\n    reduce_execution\n'
            projection += '  have hs := congrArg FiniteExecution.Observation.stop h\n'
            projection += f'  have finished : {m}.stop ≠ .budget := by\n    change {m}.stop = .{c["execution"]["stop"].lower()} at hs\n    rw [hs]\n    decide\n'
            projection += f'  rw [FiniteExecutionProjection.execute_projection apply {p} '
            projection += f'⟨{dispatch_limit(c["dispatch_limit"])}, {natural(c["work_limit"])}⟩ {inputs} 0 finished]\n'
            projection += f'  unfold {m} {p} at *\n  reduce_execution'
            claim(m + '_typed_projection', f'run {p} {inputs} = {option(selected, value)}',
                  projection)

    for i, c in enumerate(sequence(data['observer'])):
        if c.get('kind') == 'binding-construction':
            fields(c, 'name kind width slots refusal')
            identity(c, 'binding-construction')
            claim(f'construction_{i}', f'ObserverAdmission.validateSlots {natural(c["width"])} '
                  f'{slots(c["slots"])} 0 = .error {binding_error(c["refusal"])}')
            continue
        identity(c, 'observer')
        common = 'name kind description supplied_interface inputs dispatch_limit work_limit work_used dispatches'
        if 'reference' in c:
            fields(c, common + ' reference refusal state')
            mode = 'root'
        elif 'read' in c:
            fields(c, common + ' before read update_refusal after')
            mode = 'read'
        else:
            fields(c, common + ' before read_refusal update_refusal after')
            mode = 'refusal'
        d = description(c['description'])
        prefix = f'observer_{i}'
        state = value(c['state'] if mode == 'root' else c['before'])
        supplied = interface(c['supplied_interface'])
        inputs = array(value(v) for v in sequence(c['inputs']))
        limit = dispatch_limit(c['dispatch_limit'])
        budget = '⟨' + limit + ', ' + natural(c['work_limit']) + ', 0, 0, false⟩'
        for phase in ('read', 'update'):
            lines.append(f'def {prefix}_{phase}_code : Code := {code(d[phase])}')
            lines.append(f'def {prefix}_{phase}_binding : ObserverAdmission.Binding ObserverInterfaces.Interface := '
                         + admission_binding(d[phase + '_binding']))
        claim(prefix + '_initial', f'({value(d["initial"])} : Value) = {state}')
        lines.append(f'def {prefix}_read := ObserverAdmission.evaluate apply .read '
                     f'{prefix}_read_code {prefix}_read_binding {state} {supplied} {inputs} {budget}')
        read_proof = f'unfold {prefix}_read {prefix}_read_code {prefix}_read_binding\n  reduce_admission'
        final = prefix + '_read'
        final_proof = read_proof
        if mode == 'read':
            fields(c['read'], 'value execution')
            expected = '.ok (' + value(c['read']['value']) + ', ' + observation(c['read']['execution']) + ')'
        else:
            expected = '.error ' + observer_error(c['refusal'] if mode == 'root' else c['read_refusal'])
        claim(prefix + '_read_result', f'{prefix}_read.observation = {expected}', read_proof)
        if mode != 'root':
            lines.append(f'def {prefix}_update := ObserverAdmission.evaluate apply .update '
                         f'{prefix}_update_code {prefix}_update_binding {state} {supplied} {inputs} {prefix}_read.budget')
            final = prefix + '_update'
            final_proof = f'unfold {prefix}_update {prefix}_read {prefix}_read_code {prefix}_read_binding '
            final_proof += f'{prefix}_update_code {prefix}_update_binding\n  reduce_admission'
            claim(prefix + '_update_result', f'{final}.observation = .error {observer_error(c["update_refusal"])}', final_proof)
            claim(prefix + '_state', f'({state} : Value) = {value(c["after"])}')
        else:
            ref = fields(c['reference'], 'value execution work_used dispatch_limit work_limit')
            # The reference has its own observed allowance, not one reconstructed
            # from the successful result or the subsequently refused invocation.
            lines.append(f'def {prefix}_reference := ObserverAdmission.evaluate apply .read '
                         f'{prefix}_read_code {prefix}_read_binding {state} {supplied} {inputs} '
                         f'⟨{dispatch_limit(ref["dispatch_limit"])}, {natural(ref["work_limit"])}, 0, 0, false⟩')
            proof = f'unfold {prefix}_reference {prefix}_read_code {prefix}_read_binding\n  reduce_admission'
            claim(prefix + '_reference_result', f'{prefix}_reference.observation = .ok '
                  f'({value(ref["value"])}, {observation(ref["execution"])})', proof)
            claim(prefix + '_reference_work', f'{prefix}_reference.budget.workUsed = {natural(ref["work_used"])}', proof)
        claim(prefix + '_work', f'{final}.budget.workUsed = {natural(c["work_used"])}', final_proof)
        claim(prefix + '_dispatches', f'{final}.budget.dispatches = {natural(c["dispatches"])}', final_proof)
    return {'machine_controls': len(data['machine']), 'observer_controls': len(data['observer'])}


def emit(data):
    has_controls = 'controls' in data
    fields(data, 'schema programs primitives metered_work' +
           (' controls_schema controls' if has_controls else ''))
    if has_controls:
        require(data['controls_schema'] == 'synarc-observer-execution-controls-v1', 'Unknown controls schema')
    require(data['schema'] == 'synarc-observer-rust-boundary-v1', 'Unknown export schema')
    natural(data['metered_work'])
    programs = sequence(data['programs'])
    primitives = sequence(data['primitives'])
    require(programs and primitives, 'Empty export is not conformance evidence')
    lines = ['import VerumExternalReplay.CheckedPairs',
             'import VerumExternalReplay.FiniteExecutionProjection',
             'import VerumExternalReplay.ObserverInterfaces', 'open CheckedPairs',
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
    extra_counts = emit_controls(data['controls'], lines, claim) if has_controls else {}
    # Every generated theorem's axiom roster is audited by the driver.
    lines += ['#print axioms ' + name for name in claims]
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', {
        'programs': len(programs), 'trace_rows': traces, 'primitive_cases': len(primitives),
        'claims': len(claims), **extra_counts}


def batches(source, size=8):
    """Bound each kernel invocation without weakening its acceptance deadline.

    Claims are closed equalities, independent of other claims. Every batch
    retains all code definitions and audits every included theorem's axioms.
    """
    blocks = source.strip().split('\n\n')
    header = [b for b in blocks if b.startswith(('import ', 'open ', 'set_option ', 'namespace '))]
    definitions = [b for b in blocks if b.startswith('def ')]
    claims = [b for b in blocks if b.startswith('theorem ')]
    for start in range(0, len(claims), size):
        group = claims[start:start + size]
        names = [b.split()[1] for b in group]
        yield ('\n\n'.join(header + definitions + group +
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


def verify(args):
    require(sha256(args.export) == args.sha256, 'Frozen export hash mismatch')
    require(len({p.resolve() for p in [args.output, args.export, args.report, args.provenance]}) == 4,
            'Output paths must be distinct')
    provenance = load(args.provenance)
    require(provenance['export_sha256'] == args.sha256, 'Provenance/export mismatch')
    provenance_hash = sha256(args.provenance)
    modules = ['TypedPrograms', 'FiniteCode', 'CheckedPairs', 'FiniteExecution',
               'ObserverAdmission', 'ObserverInterfaces', 'FiniteExecutionProjection']
    sources = [ROOT / 'VerumExternalReplay' / (m + '.lean') for m in modules]
    sources += [Path(__file__).resolve(), ROOT / 'check_typed_programs.py',
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
        require_success(run('lake', 'build', 'VerumExternalReplay.CheckedPairs',
                            'VerumExternalReplay.ObserverInterfaces',
                            'VerumExternalReplay.FiniteExecutionProjection', cwd=snapshot))
        results = []
        for index, (part, names) in enumerate(batches(text)):
            path = artifacts / f'part{index}.lean'
            require(path.resolve() not in {p.resolve() for p in
                    [args.output, args.export, args.report, args.provenance]}, 'Batch path collision')
            path.write_text(part)
            result = check(path, cwd=snapshot)
            rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
            require(len(rosters) == len(names), 'Missing theorem axiom audit')
            for roster in rosters:
                require(roster.endswith('depends on axioms: [propext, Quot.sound]') or
                        roster.endswith('does not depend on any axioms'), 'Unexpected theorem axioms')
            results.append({'path': str(path), 'claims': names, **result})
            print(f'Checked proof batch {index + 1}: {len(names)} claims', flush=True)
        require(checker_hashes == {str(p.relative_to(prefix)): sha256(p) for p in binaries},
                'Checker files changed during verification')
    require(sum(len(r['claims']) for r in results) == counts['claims'], 'Incomplete claim replay')
    live = {str(p.relative_to(ROOT)): sha256(p) for p in sources}
    require(all(before[str(p.relative_to(ROOT))] == live[str(p.relative_to(ROOT))]
                for p in sources if p.suffix == '.py') and sha256(args.export) == args.sha256
            and sha256(args.provenance) == provenance_hash, 'Producer/export changed during verification')
    report = {'scope': 'Finite Rust observations agree with checked typed-program lowering and '
              'independent CheckedPairsV1 semantics; supplied edge records also check exact '
              'refusals and structural resource accounting. Not universal Rust correspondence, '
              'native Verum replay, ownership/panic correctness, or execution provenance.',
              'export_sha256': args.sha256, 'source_sha256': before, 'counts': counts,
              'provenance_sha256': provenance_hash, 'reported_rust_provenance': provenance,
              'aggregate_source_sha256': sha256(aggregate),
              'aggregate_source': str(aggregate), 'model_sources': str(artifacts / 'model'),
              'model_build': 'private immutable source snapshot',
              'checkout_changed_since_snapshot': [name for name in before if before[name] != live[name]],
              'lean_version': version,
              'checker_sha256': checker_hashes,
              'proof_batches': results}
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
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
        report = verify(args)
    except Exception as error:
        atomic_report(args.report, {**invocation, 'status': 'failed',
                                   'error': f'{type(error).__name__}: {error}'[:8192]})
        raise
    atomic_report(args.report, {**invocation, 'status': 'verified', **report})
    print('Checked observer export: ' + json.dumps(report['counts']))


if __name__ == '__main__':
    main()
