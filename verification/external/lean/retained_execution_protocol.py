"""Consume every native machine event and emit exact retained-model obligations.

Intermediate snapshots are checked under an explicit active-guard relation.
No callback is rerun in Python and no native field is discarded to repair a trace.
"""
import copy
from observer_export_replay import fields, require, sequence, boolean
import retained_execution_snapshot as schema
import retained_execution_encoding as lean


class Claims:
    def __init__(self):
        self.lines = ['import VerumExternalReplay.RetainedNative',
            'open RetainedExecution RetainedNative', 'set_option warningAsError true',
            'namespace FrozenObserverExport']
        self.count = 0

    def claim(self, proposition):
        binder = ' (onPanic : List Nat → Except RetainedNative.Error Nat)' if 'onPanic' in proposition else ''
        self.lines.append(f'theorem native_{self.count}{binder} :\n  {proposition} := by rfl')
        self.count += 1

    def text(self):
        return '\n\n'.join(self.lines + ['end FrozenObserverExport']) + '\n'


def same_snapshot(actual, expected, *, guard, storage=None):
    """Positive image of a known service phase, not normalization of evidence."""
    image = copy.deepcopy(expected)
    image['poisoned'] = guard
    if image['pending'] is not None and storage is not None:
        image['pending']['storage'] = storage
    require(actual == image, 'Native intermediate snapshot changed outside its transition')


def check_report(value):
    fields(value, 'stop registers dispatch expanded calls halt')
    require(value['stop'] in ('complete', 'undefined', 'budget'), 'Unknown report stop')
    schema.vector(value['registers'])
    for key in ('dispatch', 'expanded', 'calls'): schema.nat(value[key])
    require(schema.nat(value['expanded']) + schema.nat(value['calls']) == schema.nat(value['dispatch']),
            'Report counter partition differs')
    if value['halt'] is not None:
        fields(value['halt'], 'path reason'); schema.vector(value['halt']['path']); schema.failure(value['halt']['reason'])
    require((value['halt'] is None) == (value['stop'] == 'complete'), 'Report halt/stop mismatch')


class Service:
    def __init__(self, data, source, code_name, work, claims):
        fields(data, 'id budgetOwner budgetKind grantDispatch before after meter returned trace')
        schema.nat(data['id']); schema.nat(data['budgetOwner'])
        require(data['budgetKind'] == 'bounded_work_meter_with_dispatch_grant', 'Unsupported meter')
        fields(data['meter'], 'before after')
        self.data, self.source, self.code_name, self.claims = data, source, code_name, claims
        self.trace = sequence(data['trace']); self.index = 0
        self.current = data['before']; self.policy = self.current['policy']
        self.bound = schema.limit(self.current['originalLimit'])
        self.meter = schema.nat(data['meter']['before']); self.work = work
        self.dispatch_grant = schema.nat(data['grantDispatch'])
        self.start_dispatch = schema.nat(self.current['counters']['dispatch'])
        self.returned = False
        self.callbacks = []

    def entry(self):
        require(self.index < len(self.trace), 'Missing native event')
        entry = self.trace[self.index]; self.index += 1
        return entry

    def observe(self, kind, payload=None, *, expected=None, guard=True, storage=None, context=None):
        entry = self.entry(); fields(entry, 'kind event state')
        require(entry['kind'] == 'machine', 'Expected machine observation')
        event = entry['event']; payload = {} if payload is None else payload
        require(event == {'kind': kind, **payload}, f'Unexpected native event; expected {kind}')
        raw = entry['state']
        schema.snapshot(raw, self.source, policy=self.policy, original_limit=self.bound,
                        context=context or ('active' if guard else 'idle'))
        if expected is not None: same_snapshot(raw, expected, guard=guard, storage=storage)
        if kind.endswith('_request') or kind.endswith('_result'):
            if kind not in ('charge_request', 'charge_result'): self.callbacks.append(event)
        return raw

    def peek_event(self):
        require(self.index < len(self.trace), 'Missing callback observation')
        entry = self.trace[self.index]
        require(type(entry) is dict and entry.get('kind') == 'machine', 'Expected native callback')
        require(type(entry.get('event')) is dict, 'Missing native event payload')
        return entry['event']

    def dispatch_left(self, raw):
        used = schema.nat(raw['counters']['dispatch']) - self.start_dispatch
        require(0 <= used <= self.dispatch_grant, 'Service dispatch delta exceeds grant')
        return self.dispatch_grant - used

    def charge(self, units):
        payload = {'units': str(units), 'admitted': None}
        self.observe('charge_request', payload, expected=self.current, storage=self.storage)
        entry = self.entry(); fields(entry, 'kind units before after ok')
        require(entry['kind'] == 'meter' and entry['units'] == str(units), 'Meter request differs from actual charge')
        require(type(entry['ok']) is bool and schema.nat(entry['before']) == self.meter, 'Meter chronology changed')
        ok = units <= self.meter
        require(entry['ok'] == ok, 'Bounded meter admission changed')
        after = self.meter - units if ok else self.meter
        require(schema.nat(entry['after']) == after, 'Meter consumed wrong work or refunded work')
        self.meter = after
        if ok: self.work += units
        self.observe('charge_result', {'units': str(units), 'admitted': ok}, expected=self.current, storage=self.storage)
        return ok

    def gate(self, name, used, bound, passed):
        self.observe('gate', {'gate': name, 'used': str(used), 'bound': None if bound is None else str(bound),
            'passed': passed}, expected=self.current, storage=self.storage)
        return passed

    def terminal(self):
        event = self.peek_event(); require(event == {'kind': 'terminal'}, 'Missing terminal transition')
        dying = self.observe('terminal')
        require(dying['terminal'] is not None, 'Terminal marker has no terminal state')
        expected_pending = copy.deepcopy(self.current['pending'])
        if expected_pending is not None: expected_pending['storage'] = 'local'
        require(dying['pending'] == expected_pending, 'Dying local differs from the interrupted instruction')
        # Native run has now returned; the borrowed local is gone. Read that
        # actual snapshot rather than rewriting the dying-local observation.
        settled = self.observe('service_returned')
        require(settled['pending'] is None, 'Terminal local was not destroyed')
        image = copy.deepcopy(settled); image['pending'] = copy.deepcopy(dying['pending'])
        require(dying == image, 'Terminal destruction changed other retained fields')
        self.returned = True
        return settled

    def proof(self, before, work_before, meter_before, after, status, events):
        left = self.dispatch_left(before); right = self.dispatch_left(after)
        s = lean.state(before, work_before, logical=True)
        t = lean.state(after, self.work, logical=True)
        native = lean.state(after, self.work)
        self.claims.claim(f'RetainedNative.active ({t} : State Nat RetainedNative.Error) = {native}')
        self.claims.claim(f'tick {self.code_name} (interpret onPanic) .{self.policy} {s} ⟨{meter_before}, {left}⟩ = '
                          f'⟨{t}, ⟨{self.meter}, {right}⟩, {status}, {lean.array(events)}⟩')
        self.current = after

    def denied(self, before, work_before, meter_before, reason):
        if self.current['pending'] is not None:
            self.current = self.observe('pending_stored_pause', expected=self.current, storage='machine')
        self.proof(before, work_before, meter_before, self.current, f'(.paused .{lean.pause(reason)})', [])
        return {'kind': 'suspended', 'reason': reason, 'meterError': reason == 'Work'}

    def execute(self):
        poisoned = self.current['poisoned']
        self.observe('service_enter', expected=self.current, guard=poisoned,
                     context='unwound' if poisoned else 'idle')
        if poisoned:
            require(self.index == len(self.trace) and self.data['returned'] == {'kind': 'unwind'},
                    'Poisoned machine resumed or fabricated a normal return')
            require(self.data['after'] == self.current, 'Poisoned retry mutated state')
            require(schema.nat(self.data['meter']['after']) == self.meter, 'Poisoned retry charged work')
            return self.current, self.work, self.callbacks
        self.current = self.observe('service_active', expected=self.current)
        result = None
        while result is None:
            before = self.current; work_before = self.work; meter_before = self.meter
            events = []
            self.storage = None if before['pending'] is None else 'local'
            if before['terminal'] is not None:
                self.observe('terminal_cached', expected=before)
                result = {'kind': 'complete'}; break
            if not before['frames']:
                if not self.charge(1):
                    result = self.denied(before, work_before, meter_before, 'Work'); break
                after = self.observe('root_enter'); events = ['.root']
            elif before['pending'] is None:
                frame = before['frames'][-1]; scope, pc = frame['scope'], frame['pc']
                self.observe('instruction_request', {'scope': scope, 'pc': pc}, expected=before)
                ev = self.peek_event(); fields(ev, 'kind scope pc decoded')
                self.observe('instruction_result', {'scope': scope, 'pc': pc, 'decoded': ev['decoded']}, expected=before)
                d = ev['decoded']
                if d is not None:
                    fields(d, 'arity operation')
                    events = [f'(.decode {lean.values(scope)} {schema.nat(pc)} (some ({lean.decoded(d["operation"])}, {schema.nat(d["arity"])})))']
                    after = self.observe('pending_stored')
                    require(after['pending'] is not None and after['pending']['storage'] == 'machine', 'Decoded pending not stored')
                    require(after['pending']['decoded'] == d, 'Stored instruction differs from actual decode result')
                else:
                    events = [f'(.decode {lean.values(scope)} {schema.nat(pc)} none)']
                    if len(before['frames']) == 1:
                        after = self.terminal()
                    else:
                        self.observe('output_request', {'scope': scope}, expected=before)
                        ev = self.peek_event(); fields(ev, 'kind scope value')
                        self.observe('output_result', {'scope': scope, 'value': ev['value']}, expected=before)
                        selected = lean.option(ev['value'], lambda x: str(schema.nat(x)))
                        events.append(f'(.output {lean.values(scope)} {selected})')
                        after = self.terminal() if ev['value'] is None else self.observe('child_return')
            else:
                self.observe('pending_local', expected=before, storage='local')
                p = before['pending']; part = p['part']; arity = schema.nat(p['decoded']['arity'])
                call = 'call' in p['decoded']['operation']
                if part == 'start':
                    used = schema.nat(before['counters']['dispatch'])
                    for name, count, bound in [('original_dispatch', used, self.bound),
                            ('current_grant', used - self.start_dispatch, self.dispatch_grant),
                            ('counter_capacity', used, schema.MAX64)]:
                        if not self.gate(name, count, bound, bound is None or count < bound):
                            reason = {'original_dispatch': 'OriginalDispatch', 'current_grant': 'GrantDispatch', 'counter_capacity': 'CounterCapacity'}[name]
                            result = self.denied(before, work_before, meter_before, reason); break
                    if result is not None: break
                    units = arity + (4 if call else 2)
                    if self.policy == 'atomic':
                        passed = units <= schema.MAX64
                        self.observe('gate', {'gate': 'atomic_arity_capacity', 'arity': str(arity), 'overhead': str(4 if call else 2),
                            'capacity': str(schema.MAX64), 'passed': passed}, expected=before, storage='local')
                        if not passed:
                            result = self.denied(before, work_before, meter_before, 'CounterCapacity'); break
                    if self.policy != 'unmetered' and not self.charge(units if self.policy == 'atomic' else 1):
                        result = self.denied(before, work_before, meter_before, 'Work'); break
                    after = self.observe('dispatch_started'); events = [f'(.admit {boolean(call)})']
                elif part == 'arguments' and len(p['arguments']) == arity:
                    after = self.observe('arguments_complete')
                    events = ['(.phase .reserveReturn)' if call else '(.phase .primitive)']
                else:
                    if self.policy == 'phased' and not self.charge(1):
                        result = self.denied(before, work_before, meter_before, 'Work'); break
                    if part == 'arguments':
                        position = len(p['arguments']); payload = {'argument': str(position)}
                        self.observe('operand_request', payload, expected=before, storage='local')
                        ev = self.peek_event(); fields(ev, 'kind argument operand')
                        self.observe('operand_result', {**payload, 'operand': ev['operand']}, expected=before, storage='local')
                        owned = schema.code_at(self.source, schema.vector(p['scope']))
                        args = owned.instructions[schema.nat(p['pc'])].arguments
                        actual = None if ev['operand'] is None else schema.operand(ev['operand'])
                        require(actual == (args[position] if position < len(args) else None), 'Operand callback differs from fixed source')
                        next_event = self.peek_event()
                        if next_event['kind'] == 'operand_resolved':
                            fields(next_event, 'kind origin value')
                            origin = 'literal' if actual is not None and actual[0] == 'literal' else ev['operand']
                            self.observe('operand_resolved', {'origin': origin, 'value': next_event['value']}, expected=before, storage='local')
                            reply = f'(.ok {schema.nat(next_event["value"])})'
                            after = self.observe('argument_pushed')
                        else:
                            after = self.terminal(); reply = f'(.error {lean.failure(after["terminal"]["reason"])})'
                        events = [f'(.argument {lean.values(p["scope"])} {schema.nat(p["pc"])} {position} {reply})']
                    elif part == 'primitive':
                        handle = p['decoded']['operation']['primitive']
                        self.observe('primitive_request', {'operation': handle, 'arguments': p['arguments']}, expected=before, storage='local')
                        if self.index == len(self.trace):
                            require(handle['operation'] == 'panic', 'Unobserved callback result is not the frozen panic adapter')
                            require(self.data['returned'] == {'kind': 'unwind'}, 'Missing callback result without actual unwind')
                            after = self.data['after']
                            schema.snapshot(after, self.source, policy=self.policy, original_limit=self.bound, context='unwound')
                            self.claims.claim(f'abortPrimitive .{self.policy} {lean.state(before, work_before, logical=True)} '
                                f'⟨{meter_before}, {self.dispatch_left(before)}⟩ = some '
                                f'({lean.state(after, self.work)}, ⟨{self.meter}, {self.dispatch_left(after)}⟩)')
                            require(schema.nat(self.data['meter']['after']) == self.meter, 'Unwind refunded admitted work')
                            return after, self.work, self.callbacks
                        require(handle['operation'] != 'panic', 'Panic adapter fabricated a return value')
                        ev = self.peek_event(); fields(ev, 'kind result'); reply = ev['result']
                        require(type(reply) is dict and len(reply) == 1, 'Invalid primitive result')
                        self.observe('primitive_result', {'result': reply}, expected=before, storage='local')
                        if 'ok' in reply:
                            result_term = f'(.ok {schema.nat(reply["ok"])})'; after = self.observe('primitive_return')
                        else:
                            fields(reply, 'error'); schema.failure({'kind': 'primitive', 'error': reply['error']})
                            result_term = f'(.error {lean.primitive_error(reply["error"])})'; after = self.terminal()
                        events = [f'(.primitive {lean.values(p["scope"])} {schema.nat(p["pc"])} {result_term})']
                    elif part in ('reserve_return', 'move_arguments'):
                        after = self.observe('return_reserved' if part == 'reserve_return' else 'frame_reserved')
                        events = ['.reserveReturn' if part == 'reserve_return' else '.reserveFrame']
                    else:
                        require(part == 'enter_call', 'Unexpected execution phase')
                        callee = p['decoded']['operation']['call']
                        after = self.terminal() if callee is None else self.observe('call_enter')
                        events = [f'(.enterCall {lean.option(callee, lean.values)})']
                if after['terminal'] is None and after['pending'] is not None:
                    self.observe('pending_stored', expected=after, storage='machine')
                    # Retain the observed storage move as the next actual boundary.
                    after = self.trace[self.index - 1]['state']
            status = '.complete' if after['terminal'] is not None else '.advanced'
            self.proof(before, work_before, meter_before, after, status, events)
            if after['terminal'] is not None: result = {'kind': 'complete'}
        if not self.returned: self.observe('service_returned', expected=self.current, storage='machine')
        final = self.observe('service_leave', expected=self.current, guard=False, storage='machine')
        require(self.index == len(self.trace), 'Unconsumed native observations')
        require(self.data['returned'] == result, 'Native service result differs from admitted/refused transition')
        require(self.data['after'] == final, 'External service snapshot differs from actual leave')
        require(schema.nat(self.data['meter']['after']) == self.meter, 'Service meter balance differs')
        return final, self.work, self.callbacks
