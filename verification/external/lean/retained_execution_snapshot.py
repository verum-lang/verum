"""Strict decoding of native owned-program VM snapshots.

This validates the extraction boundary, not transition correspondence. Raw guard
and storage observations remain intact; only an explicit service context can
interpret transient poison. No native trace is accepted by this module alone.
"""
from dataclasses import dataclass
import re
from observer_export_replay import fields, require, sequence

MAX64 = 2**64 - 1
OPERATIONS = {'add', 'identity', 'reject', 'resource', 'panic'}
PARTS = {'start', 'arguments', 'primitive', 'reserve_return', 'move_arguments', 'enter_call'}


def nat(value):
    require(type(value) is str and re.fullmatch(r'0|[1-9][0-9]*', value) is not None,
            'Expected canonical native u64 string')
    require(len(value) <= 20 and int(value) <= MAX64, 'Native u64 out of range')
    return int(value)


def vector(value):
    return tuple(nat(x) for x in sequence(value))


def limit(value):
    require(type(value) is dict, 'Expected explicit execution limit')
    if value.get('kind') == 'unlimited':
        fields(value, 'kind'); return None
    fields(value, 'kind max')
    require(value['kind'] == 'dispatches', 'Unknown execution limit')
    return nat(value['max'])


def operand(value):
    require(type(value) is dict and len(value) == 1, 'Expected one operand variant')
    key = next(iter(value))
    require(key in ('input', 'register', 'literal'), 'Unknown operand variant')
    return key, nat(value[key])


@dataclass(frozen=True)
class Instruction:
    operation: tuple
    arguments: tuple


@dataclass(frozen=True)
class Program:
    instructions: tuple
    output: int | None
    library: tuple


def program(value, depth=0):
    require(depth <= 64, 'Program nesting exceeds extraction bound')
    fields(value, 'steps output library')
    steps = []
    for step in sequence(value['steps']):
        fields(step, 'operation arguments')
        op = step['operation']
        require(type(op) is dict and len(op) == 1, 'Expected one operation variant')
        if 'primitive' in op:
            require(type(op['primitive']) is str and op['primitive'] in OPERATIONS, 'Unknown primitive')
            decoded = ('primitive', op['primitive'])
        else:
            fields(op, 'call'); decoded = ('call', nat(op['call']))
        steps.append(Instruction(decoded, tuple(operand(a) for a in sequence(step['arguments']))))
    return Program(tuple(steps), None if value['output'] is None else nat(value['output']),
                   tuple(program(p, depth + 1) for p in sequence(value['library'])))


def code_at(source, scope):
    for index in scope:
        require(index < len(source.library), 'Scope has no source owner')
        source = source.library[index]
    return source


def resolve(arg, inputs, registers):
    kind, index = arg
    if kind == 'literal':
        return index
    values = inputs if kind == 'input' else registers
    require(index < len(values), 'Resolved prefix contains a missing operand')
    return values[index]


def failure(value):
    require(type(value) is dict, 'Expected explicit failure')
    kind = value.get('kind')
    if kind == 'missing_operand':
        fields(value, 'kind argument'); nat(value['argument'])
    elif kind in ('missing_input', 'missing_register'):
        fields(value, 'kind argument index'); nat(value['argument']); nat(value['index'])
    elif kind in ('missing_callee', 'missing_callee_output', 'allowance', 'counter_capacity'):
        fields(value, 'kind')
    else:
        fields(value, 'kind error'); require(kind == 'primitive', 'Unknown failure')
        error = value['error']; fields(error, 'kind declaredClassification')
        require(error['kind'] in ('arity', 'overflow', 'rejected', 'resource'), 'Unknown primitive failure')
        expected = 'resource' if error['kind'] == 'resource' else 'undefined'
        require(error['declaredClassification'] == expected, 'Primitive classification mismatch')


def snapshot(value, source, *, policy, original_limit, context):
    """Check actual fields without dropping or rewriting poison/storage metadata.

    Context is supplied by the event/service protocol checker, never inferred
    from the snapshot itself. Active and caught-unwind guards both remain true;
    this function does not equate their resume permissions.
    """
    require(context in ('idle', 'active', 'unwound'), 'Unknown service context')
    fields(value, 'policy originalLimit rootInputs poisoned counters terminal frames pending')
    require(policy in ('atomic', 'phased', 'unmetered') and value['policy'] == policy, 'Policy changed')
    require(limit(value['originalLimit']) == original_limit, 'Original allowance changed')
    require(type(value['poisoned']) is bool and value['poisoned'] == (context != 'idle'),
            'Poison guard disagrees with service context')
    counters = value['counters']; fields(counters, 'dispatch expanded calls')
    dispatch, expanded, calls = (nat(counters[k]) for k in ('dispatch', 'expanded', 'calls'))
    require(expanded + calls == dispatch, 'Counter partition changed')
    terminal = value['terminal']
    if terminal is not None:
        fields(terminal, 'stop reason')
        require(terminal['stop'] in ('complete', 'undefined'), 'Budget is a pause, not a machine terminal')
        require((terminal['reason'] is None) == (terminal['stop'] == 'complete'), 'Invalid terminal reason')
        if terminal['reason'] is not None:
            failure(terminal['reason'])
    frames = []
    for index, frame in enumerate(sequence(value['frames'])):
        fields(frame, 'scope inputs registers pc')
        scope = vector(frame['scope']); pc = nat(frame['pc']); regs = vector(frame['registers'])
        inputs = frame['inputs']; fields(inputs, 'kind values'); ins = vector(inputs['values'])
        require(inputs['kind'] == ('root' if index == 0 else 'call'), 'Frame input ownership kind changed')
        require(pc == len(regs), 'PC differs from completed register prefix')
        owned = code_at(source, scope)
        require(pc <= len(owned.instructions), 'PC exceeds source program')
        if index == 0:
            require(scope == (), 'Root scope changed')
        else:
            parent_scope, parent_ins, parent_regs, parent_code = frames[-1]
            parent_pc = len(parent_regs)
            require(parent_pc < len(parent_code.instructions), 'No suspended parent instruction')
            call = parent_code.instructions[parent_pc]
            require(call.operation[0] == 'call' and scope == parent_scope + (call.operation[1],),
                    'Child scope differs from suspended call')
            require(ins == tuple(resolve(a, parent_ins, parent_regs) for a in call.arguments),
                    'Callee inputs differ from resolved caller arguments')
        frames.append((scope, ins, regs, owned))
    root = value['rootInputs']
    if frames:
        require(root is None, 'Entered root still owns pending root inputs')
    else:
        require(root is not None and terminal is None, 'Unentered root lost inputs or became terminal')
        vector(root)
    pending = value['pending']
    if pending is None:
        return value
    fields(pending, 'storage scope pc decoded arguments part')
    require(bool(frames), 'Pending instruction has no frame')
    require(pending['storage'] in ('machine', 'local'), 'Unknown pending storage')
    require(context == 'active' or pending['storage'] == 'machine', 'Local pending escaped its service')
    scope, ins, regs, owned = frames[-1]
    require(vector(pending['scope']) == scope and nat(pending['pc']) == len(regs), 'Pending PC/scope changed')
    require(len(regs) < len(owned.instructions), 'Pending instruction absent from source')
    step = owned.instructions[len(regs)]
    decoded = pending['decoded']; fields(decoded, 'arity operation')
    require(nat(decoded['arity']) == len(step.arguments), 'Owned instruction arity changed')
    op = decoded['operation']
    if step.operation[0] == 'primitive':
        fields(op, 'primitive'); handle = op['primitive']; fields(handle, 'scope pc operation')
        require(vector(handle['scope']) == scope and nat(handle['pc']) == len(regs)
                and handle['operation'] == step.operation[1], 'Captured primitive identity changed')
    else:
        fields(op, 'call')
        target = None if op['call'] is None else vector(op['call'])
        expected = scope + (step.operation[1],) if step.operation[1] < len(owned.library) else None
        require(target == expected, 'Captured callee identity changed')
    args = vector(pending['arguments']); part = pending['part']
    require(type(part) is str and part in PARTS, 'Unknown pending phase')
    require(len(args) <= len(step.arguments), 'Argument prefix exceeds arity')
    require(args == tuple(resolve(a, ins, regs) for a in step.arguments[:len(args)]), 'Resolved argument prefix changed')
    if part == 'start':
        require(not args, 'Unstarted instruction has resolved arguments')
    elif part != 'arguments':
        require(len(args) == len(step.arguments), 'Instruction advanced before all arguments resolved')
        require((part == 'primitive') == (step.operation[0] == 'primitive'), 'Phase disagrees with decoded operation')
    require(terminal is None or terminal['stop'] == 'undefined', 'Completed machine retains pending instruction')
    if terminal is not None:
        reason = terminal['reason']['kind']
        require((part == 'primitive' and reason == 'primitive')
                or (part == 'enter_call' and reason == 'missing_callee')
                or (part == 'arguments' and reason in ('missing_operand', 'missing_input', 'missing_register')),
                'Local terminal reason disagrees with interrupted phase')
    return value
