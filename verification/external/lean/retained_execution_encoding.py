"""Lossless finite Lean terms for the explicitly named native u64 adapter."""
from observer_export_replay import array, boolean
from retained_execution_snapshot import nat, vector, limit


def option(value, encode):
    return 'none' if value is None else '(some ' + encode(value) + ')'


def values(xs):
    return array(str(nat(v)) for v in xs)


def primitive_error(error):
    return '.' + error['kind']


def failure(error):
    kind = error['kind']
    names = {'missing_callee': 'missingCallee', 'missing_callee_output': 'missingCalleeOutput',
             'counter_capacity': 'counterCapacity', 'allowance': 'allowance'}
    if kind in names: return '.' + names[kind]
    if kind == 'primitive': return '(.primitive ' + primitive_error(error['error']) + ')'
    name = {'missing_operand': 'missingOperand', 'missing_input': 'missingInput',
            'missing_register': 'missingRegister'}[kind]
    args = [str(nat(error['argument']))]
    if kind != 'missing_operand': args.append(str(nat(error['index'])))
    return '(.' + name + ' ' + ' '.join(args) + ')'


def decoded(operation):
    if 'call' in operation:
        return '(.call ' + option(operation['call'], values) + ')'
    handle = operation['primitive']
    return '(.primitive ' + values(handle['scope']) + ' ' + str(nat(handle['pc'])) + ')'


def code(source):
    def step(s):
        op, arg = s.operation
        operation = f'(.primitive .{arg})' if op == 'primitive' else f'(.call {arg})'
        args = array(f'(.{kind} {value})' for kind, value in s.arguments)
        arity = 'none' if s.declared_arity is None else f'(some {s.declared_arity})'
        return f'⟨{operation}, {args}, {arity}⟩'
    output = 'none' if source.output is None else f'(some {source.output})'
    return f'(.mk {array(step(s) for s in source.instructions)} {output} {array(code(c) for c in source.library)})'


def pending(p):
    part = {'reserve_return': 'reserveReturn', 'move_arguments': 'moveArguments', 'enter_call': 'enterCall'}.get(p['part'], p['part'])
    return (f'⟨{values(p["scope"])}, {nat(p["pc"])}, {decoded(p["decoded"]["operation"])}, '
            f'{nat(p["decoded"]["arity"])}, {values(p["arguments"])}, .{part}⟩')


def state(raw, work, *, logical=False):
    def frame(f):
        return f'⟨{values(f["scope"])}, .{f["inputs"]["kind"]}, {values(f["inputs"]["values"])}, {values(f["registers"])}⟩'
    def terminal(t):
        return f'(.{t["stop"]}, {option(t["reason"], failure)})'
    bound = limit(raw['originalLimit']); bound = 'none' if bound is None else f'(some {bound})'
    guard = False if logical else raw['poisoned']
    c = raw['counters']
    return (f'(⟨{bound}, ⟨{option(raw["rootInputs"], values)}, '
            f'{array(frame(f) for f in reversed(raw["frames"]))}, {option(raw["pending"], pending)}, '
            f'{option(raw["terminal"], terminal)}, {boolean(guard)}⟩, '
            f'⟨{work}, {nat(c["dispatch"])}, {nat(c["expanded"])}, {nat(c["calls"])}⟩⟩ : State Nat RetainedNative.Error)')


def report(r):
    def halt(h): return f'⟨{values(h["path"])}, {failure(h["reason"])}⟩'
    return (f'⟨.{r["stop"]}, {values(r["registers"])}, {nat(r["dispatch"])}, '
            f'{nat(r["expanded"])}, {nat(r["calls"])}, {option(r["halt"], halt)}⟩')


def pause(name):
    return {'Work': 'work', 'OriginalDispatch': 'originalDispatch',
            'GrantDispatch': 'grantDispatch', 'CounterCapacity': 'counterCapacity'}[name]
