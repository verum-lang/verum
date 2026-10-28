"""Exact observed wrapper protocol around independently replayed machine steps.

Binding values and selected outputs generate Lean obligations. Nominal allocation
classes and every intermediate wrapper snapshot remain strict extraction checks.
"""
import copy
import json
from observer_export_replay import fields, require, sequence
from retained_execution_protocol import Service, check_report
import retained_execution_snapshot as schema
import retained_execution_encoding as lean


def validate_scalars(data):
    """Native integers are decimal strings; JSON numbers are never Booleans.

    Python's structural equality accepts 0 == False and 1 == True. Reject all
    numeric literals before comparing snapshots so guard/callback checks retain
    their exact JSON types. Field-specific decoders still validate each string.
    """
    pending = [data]
    while pending:
        value = pending.pop()
        if type(value) is dict:
            require(all(type(k) is str for k in value), 'Invalid native field name')
            pending.extend(value.values())
        elif type(value) is list: pending.extend(value)
        else: require(value is None or type(value) in (str, bool),
                      'Native scalar must be a string, Boolean or null')


def binding(data):
    fields(data, 'interface width slots'); schema.nat(data['interface']); width = schema.nat(data['width'])
    for slot in sequence(data['slots']):
        require(type(slot) is dict and len(slot) == 1, 'Invalid binding slot')
        if 'state' in slot: require(slot['state'] is True, 'State binding is not explicit')
        else:
            fields(slot, 'external'); require(schema.nat(slot['external']) < width, 'External binding exceeds declared width')


def binding_term(data):
    return lean.array('.state' if 'state' in s else f'(.external {schema.nat(s["external"])})' for s in data['slots'])


def terminal_term(classification):
    require(classification in ('Undefined', 'Resource', 'Poisoned', 'AlreadyReturned'),
            'Unknown terminal classification')
    return '.returned' if classification == 'AlreadyReturned' else f'(.failed {json.dumps(classification)})'


def lifecycle_term(state):
    terminal = terminal_term(state['terminalClass'])
    require(type(state['finished']) is bool and type(state['poisoned']) is bool, 'Invalid lifecycle guards')
    prior = f'(some {terminal})' if state['finished'] else 'none'
    return f'(⟨{str(state["poisoned"]).lower()}, {prior}⟩ : AffineService.State String)'


class Wrapper:
    def __init__(self, case, fixture, source, code_name, claims, owner, *, lifecycle=False):
        self.case, self.fixture, self.source, self.code_name, self.claims, self.owner = case, fixture, source, code_name, claims, owner
        self.current = copy.deepcopy(case['initial']); self.phase = case['phase']; self.work = 0; self.callbacks = []
        self.bind = fixture[self.phase + 'Binding']
        self.bound_owner = None
        self.lifecycle = lifecycle

    def guard(self, state):
        if state['poisoned'] or self.budget['poisoned'] or (state['finished'] and not self.lifecycle):
            result = self.refusal('Poisoned', 'poisoned'); gate = '.poisoned'
        elif state['finished']:
            result = self.refusal(state['terminalClass'], 'consumed', priorClassification=state['terminalClass'])
            gate = f'(.consumed {terminal_term(state["terminalClass"])})'
        else:
            result = None; gate = '.enter'
        if self.lifecycle:
            self.claims.claim(f'AffineService.gate {lifecycle_term(state)} '
                f'{str(self.budget["poisoned"]).lower()} = {gate}')
        return result

    def normal_return(self, result):
        if not self.lifecycle: return
        # Refusal class is not a lifecycle discriminator. In particular an
        # original-limit suspension still retains its owner despite no future
        # dispatch grant being able to make progress.
        if result['kind'] == 'complete': outcome = '.returned'
        elif result['refusal']['raw']['kind'] in ('work', 'suspended'): outcome = '.retained'
        else:
            require(result['refusal']['raw']['kind'] in ('binding', 'execution', 'missing_root_output'),
                    'Unexpected normal terminal refusal')
            outcome = f'(.failed {json.dumps(result["refusal"]["classification"])})'
        # RHS comes from the original native service_leave, not our transition.
        self.claims.claim(f'AffineService.settle {outcome} = {lifecycle_term(self.peek()["state"])}')

    def expect(self, kind, event, state):
        require(self.pos < len(self.trace), 'Missing wrapper observation')
        entry = self.trace[self.pos]; self.pos += 1; fields(entry, 'kind event state')
        require(entry['kind'] == kind and entry['event'] == event, f'Unexpected wrapper event; expected {kind}/{event["kind"]}')
        require(entry['state'] == state, f'Wrapper state changed outside {kind}/{event["kind"]}')
        if event['kind'] in ('root_output_request', 'root_output_result', 'classification_request', 'classification_result'):
            self.callbacks.append(event)
        return entry['state']

    def mark(self, kind, name, state):
        return self.expect(kind, {'kind': name}, state)

    def peek(self):
        require(self.pos < len(self.trace), 'Missing wrapper callback result')
        return self.trace[self.pos]

    def charge(self):
        entry = self.peek(); self.pos += 1; fields(entry, 'kind units before after ok')
        require(entry['kind'] == 'meter' and entry['units'] == '1' and entry['before'] == str(self.meter), 'Wrapper charge does not match actual meter')
        ok = self.meter >= 1
        require(type(entry['ok']) is bool and entry['ok'] == ok, 'Wrapper meter admission changed')
        if ok: self.meter -= 1
        require(entry['after'] == str(self.meter), 'Wrapper charge refunded or consumed wrong work')
        return ok

    def refusal(self, classification, kind, **payload):
        return {'kind': 'refused', 'refusal': {'classification': classification,
            'raw': {'kind': kind, 'phase': self.phase, **payload}}}

    def work_refusal(self, point):
        return self.refusal('Resource', 'work', point=point, meterError=True, execution=None)

    def bind_input(self):
        c = self.current
        if c['inspected'] is None:
            if not self.charge(): return self.work_refusal('InputBinding')
            if self.bind['interface'] != c['input']['interface']:
                return self.refusal('Undefined', 'binding', reason={'kind': 'interface'})
            if schema.nat(self.bind['width']) != len(c['input']['values']):
                return self.refusal('Undefined', 'binding', reason={'kind': 'width', 'expected': self.bind['width'], 'actual': str(len(c['input']['values']))})
            c['inspected'] = '0'; self.mark('call', 'interface_checked', c)
        while schema.nat(c['inspected']) < schema.nat(self.bind['width']):
            if not self.charge(): return self.work_refusal('InputBinding')
            c['inspected'] = str(schema.nat(c['inspected']) + 1); self.mark('call', 'input_inspected', c)
        while len(c['bound']) < len(self.bind['slots']):
            if not self.charge(): return self.work_refusal('InputBinding')
            entry = self.peek(); actual = entry.get('state', {}).get('bound')
            require(type(actual) is list and len(actual) == len(c['bound']) + 1 and actual[:-1] == c['bound'], 'Bound input prefix replaced')
            schema.vector(actual); c['bound'] = actual
            self.claims.claim(f'ObserverAdmission.bindValues {schema.nat(self.bind["width"])} {schema.nat(c["cut"]["value"])} '
                f'{lean.values(c["input"]["values"])} ({binding_term(self.bind)}.take {len(actual)}) 0 = Except.ok {lean.values(actual)}')
            self.mark('call', 'slot_bound', c)
        actual = self.peek()['state']['invocation']
        fields(actual, 'owner phase programOwner semanticsOwner inputs retainedInput originalLimit body paused finished poisoned machine'
               + (' terminalClass' if self.lifecycle else ''))
        bound = actual['inputs']; fields(bound, 'owner values')
        require(bound['values'] == c['bound'], 'Invocation did not receive actual bound inputs')
        self.owner(actual['owner'], 'invocation', (self.case['id'],), fresh=True)
        self.owner(bound['owner'], 'bound_input', bound['values'], fresh=True)
        self.bound_owner = copy.deepcopy(bound)
        c['invocation'] = {'owner': actual['owner'], 'phase': self.phase,
            'programOwner': self.fixture[self.phase + 'CodeOwner'], 'semanticsOwner': self.fixture['semanticsOwner'],
            'inputs': bound, 'retainedInput': bound, 'originalLimit': c['originalLimit'],
            'body': None, 'paused': None, 'finished': False, 'poisoned': False, 'machine': None}
        if self.lifecycle: c['invocation']['terminalClass'] = 'Undefined'
        c['bound'] = []; self.mark('call', 'invocation_bound', c)
        return None

    def machine(self, invocation, remaining):
        before = invocation['machine']; start = self.pos
        require(self.peek().get('kind') == 'machine' and self.peek().get('event') == {'kind': 'service_enter'}, 'Missing actual machine service')
        while self.pos < len(self.trace):
            entry = self.trace[self.pos]
            require(entry.get('kind') in ('machine', 'meter'), 'Machine left without service_leave or unwind')
            self.pos += 1
            if entry.get('event') == {'kind': 'service_leave'}: break
        segment = self.trace[start:self.pos]
        unwound = segment[-1].get('event') != {'kind': 'service_leave'}
        if unwound:
            require(self.service['returned'] == {'kind': 'unwind'}, 'Machine lacks return without actual unwind')
            after = self.service['after']['invocation']['machine']; returned = {'kind': 'unwind'}
        else:
            after = segment[-1]['state']
            if after['terminal'] is not None: returned = {'kind': 'complete'}
            else:
                raw = self.service['returned']['refusal']['raw']
                require(raw['kind'] == 'suspended', 'Paused machine not exposed as suspension')
                returned = {'kind': 'suspended', 'reason': raw['reason'], 'meterError': raw['meterError']}
        meters = [e for e in segment if e['kind'] == 'meter']; end = meters[-1]['after'] if meters else str(self.meter)
        # An extraction view of this exact nested event interval, never a new
        # execution or fabricated native return. Wrapper refusal/body evidence
        # supplies the completion boundary and is independently checked below.
        view = {'id': self.service['id'], 'budgetOwner': self.service['budgetOwner'],
            'budgetKind': 'bounded_work_meter_with_dispatch_grant', 'grantDispatch': str(remaining),
            'before': before, 'after': after, 'meter': {'before': str(self.meter), 'after': end},
            'returned': returned, 'trace': segment}
        after, self.work, callbacks = Service(view, self.source, self.code_name, self.work, self.claims).execute()
        self.callbacks.extend(callbacks); self.meter = schema.nat(end); invocation['machine'] = after
        if unwound:
            self.budget['poisoned'] = True
            return returned
        delta = schema.nat(after['counters']['dispatch']) - schema.nat(before['counters']['dispatch'])
        self.budget['dispatches'] = str(schema.nat(self.budget['dispatches']) + delta)
        if returned['kind'] == 'suspended':
            invocation['paused'] = returned['reason']
            return self.refusal('Resource', 'suspended', reason=returned['reason'],
                meterError=returned['meterError'], dispatches=after['counters']['dispatch'])
        invocation['machine'] = None; invocation['retainedInput'] = None
        if after['terminal']['stop'] == 'complete':
            report = self.peek()['state']['body']; check_report(report)
            self.claims.claim(f'RetainedNative.finish {lean.state(after, self.work)} none = {lean.report(report)}')
            invocation['body'] = report; self.mark('invocation', 'body_complete', invocation)
            return None
        invocation['finished'] = True
        report = self.service['returned']['refusal']['raw']['execution']; check_report(report)
        self.claims.claim(f'RetainedNative.finish {lean.state(after, self.work)} none = {lean.report(report)}')
        classification = None
        reason = report['halt']['reason']
        if reason['kind'] == 'primitive':
            error = reason['error']; classification = 'Resource' if error['kind'] == 'resource' else 'Undefined'
            self.expect('invocation', {'kind': 'classification_request', 'error': error}, invocation)
            self.expect('invocation', {'kind': 'classification_result', 'error': error, 'classification': classification}, invocation)
        if self.lifecycle: invocation['terminalClass'] = classification or 'Undefined'
        return self.refusal(classification or 'Undefined', 'execution', execution=report,
            meterError=False, counterCapacity=False, primitiveClassification=classification)

    def invoke(self, remaining):
        i = self.current['invocation']; self.mark('invocation', 'service_enter', i)
        refused = self.guard(i)
        if refused is not None: return refused
        i['poisoned'] = True; self.budget['poisoned'] = True; self.mark('invocation', 'service_active', i)
        result = None
        if i['body'] is None:
            if i['machine'] is None:
                ins = i['inputs']; require(ins == self.bound_owner, 'Bound input Arc was replaced')
                i['machine'] = {'policy': 'phased', 'originalLimit': i['originalLimit'], 'rootInputs': ins['values'],
                    'poisoned': False, 'counters': {'dispatch': '0', 'expanded': '0', 'calls': '0'},
                    'frames': [], 'pending': None, 'terminal': None}
                i['inputs'] = None; self.mark('invocation', 'machine_bound', i)
            i['paused'] = None
            result = self.machine(i, remaining)
            if result is not None and result['kind'] == 'unwind': return result
        if result is None:
            self.mark('invocation', 'root_output_charge_request', i)
            ok = self.charge(); self.expect('invocation', {'kind': 'root_output_charge_result', 'ok': ok}, i)
            if not ok: result = self.work_refusal('RootOutput')
            else:
                report = i['body']; require(report is not None and report['stop'] == 'complete', 'Root selection lacks completed body')
                i['body'] = None; i['finished'] = True
                self.expect('invocation', {'kind': 'root_output_request', 'body': report}, i)
                event = self.peek()['event']; fields(event, 'kind value'); value = event['value']
                self.expect('invocation', {'kind': 'root_output_result', 'value': value}, i)
                output = 'none' if self.source.output is None else f'(some {self.source.output})'
                self.claims.claim(f'FiniteExecution.output (V := Nat) {output} {lean.values(report["registers"])} = {lean.option(value, lambda x: str(schema.nat(x)))}')
                result = self.refusal('Undefined', 'missing_root_output', execution=report) if value is None else {
                    'kind': 'complete', 'result': {'kind': 'reading' if self.phase == 'read' else 'prepared',
                        'value': value, 'execution': report, 'sameCut': True}}
                if self.lifecycle and value is not None: i['terminalClass'] = 'AlreadyReturned'
        i['poisoned'] = False; self.budget['poisoned'] = False
        self.normal_return(result)
        self.mark('invocation', 'service_leave', i)
        return result

    def service_run(self, service):
        fields(service, 'id budgetOwner before after budgetBefore budgetAfter dispatchDelta meter returned trace')
        require(service['before'] == self.current, 'Call owner/state replaced between services')
        self.service = service; self.trace = sequence(service['trace']); self.pos = 0
        fields(service['meter'], 'before after'); self.meter = schema.nat(service['meter']['before'])
        self.budget = copy.deepcopy(service['budgetBefore']); fields(self.budget, 'limit dispatches poisoned')
        require(type(self.budget['poisoned']) is bool, 'Invalid budget guard')
        maximum = schema.limit(self.budget['limit']); maximum = schema.MAX64 if maximum is None else maximum
        dispatch_before = schema.nat(self.budget['dispatches']); require(dispatch_before <= maximum, 'Budget dispatch count exceeds allowance')
        remaining = maximum - dispatch_before
        self.mark('call', 'service_enter', self.current)
        result = self.guard(self.current)
        if result is None:
            if self.current['originalLimit'] is None: self.current['originalLimit'] = {'kind': 'dispatches', 'max': str(remaining)}
            self.current['poisoned'] = True; self.mark('call', 'service_active', self.current)
            result = None if self.current['invocation'] is not None else self.bind_input()
            if result is None: result = self.invoke(remaining)
            if result['kind'] != 'unwind':
                self.current['poisoned'] = False
                if self.lifecycle:
                    invocation = self.current['invocation']
                    consumed = (invocation['finished'] if invocation is not None else
                                result['kind'] == 'refused' and result['refusal']['raw']['kind'] == 'binding')
                    if result['kind'] == 'complete' or consumed:
                        self.current['finished'] = True
                        self.current['terminalClass'] = ('AlreadyReturned' if result['kind'] == 'complete'
                                                        else result['refusal']['classification'])
                    self.normal_return(result)
                elif result['kind'] == 'complete' or result['refusal']['classification'] == 'Undefined': self.current['finished'] = True
                self.mark('call', 'service_leave', self.current)
        require(self.pos == len(self.trace), 'Unconsumed wrapper observations')
        require(service['after'] == self.current, 'Wrapper external snapshot differs from actual transition')
        require(service['returned'] == result, 'Wrapper result differs from observed binding/body/selection')
        require(service['budgetAfter'] == self.budget, 'Budget guard/delta differs from actual machine progress')
        require(schema.nat(service['dispatchDelta']) == schema.nat(self.budget['dispatches']) - dispatch_before, 'Cumulative dispatches rebilled as a service delta')
        require(schema.nat(service['meter']['after']) == self.meter, 'Wrapper meter accounting differs')
        return result
