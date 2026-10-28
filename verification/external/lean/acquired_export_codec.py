"""Strict value codecs selected explicitly for each lifecycle export.

No global mode switching: legacy callers retain the closed legacy vocabulary.
"""
from dataclasses import dataclass
import re
from observer_export_replay import array, boolean, fields, natural, require, sequence, variant

OPS = ('Append At Last Length EventInput EventRaw EventAction EventAge QueryInput '
       'QueryRaw QueryAge QueryAction Pair First Second Equal Select Add Rem Next Refused').split()


POSE_OPS = 'Unit Zero EmptyTrace Identity EventPose QueryPose'.split()


ERRORS = 'Arity Sort Missing Overflow TraceCapacity Allocation'.split()


SUSPEND_REASONS = 'Work GrantDispatch OriginalDispatch CounterCapacity'.split()


def tag(name, allowed):
    require(type(name) is str and name in allowed, 'Unknown variant: ' + str(name))
    return '.' + name[0].lower() + name[1:]


def option(item, encode):
    return 'none' if item is None else '(some ' + encode(item) + ')'


def wide(item):
    require(type(item) is str and re.fullmatch(r'0|[1-9][0-9]*', item), 'Expected canonical wide integer')
    return natural(int(item), 2**128 - 1)


def lineage(item):
    fields(item, 'authority birth')
    return '⟨' + natural(item['authority']) + ', ' + natural(item['birth']) + '⟩'


def reason(item):
    require(type(item) is dict and 'kind' in item, 'Expected failure reason')
    kind = item['kind']
    if kind == 'Primitive':
        fields(item, 'kind error'); return '(.primitive ' + tag(item['error'], ERRORS) + ')'
    if kind == 'MissingOperand':
        fields(item, 'kind argument'); return '(.missingOperand ' + natural(item['argument']) + ')'
    if kind in ('MissingInput', 'MissingRegister'):
        fields(item, 'kind argument index')
        return '(' + tag(kind, ['MissingInput', 'MissingRegister']) + ' ' + natural(item['argument']) + ' ' + natural(item['index']) + ')'
    fields(item, 'kind')
    return tag(kind, ['MissingCallee', 'MissingCalleeOutput', 'Allowance', 'CounterCapacity'])


def suspension(reason, dispatches, meter_error):
    """Decode observable progress only; no retained VM state is present here."""
    return '⟨' + ', '.join((tag(reason, SUSPEND_REASONS), natural(dispatches),
                           boolean(meter_error))) + '⟩'


def origin(item):
    fields(item, 'lineage epoch ordinal')
    return f'({lineage(item["lineage"])}, {wide(item["epoch"])}, {natural(item["ordinal"])})'



@dataclass(frozen=True)
class AcquiredDecoder:
    sensory: bool = False

    def bytes(self, value):
        return array(natural(byte, 255) for byte in sequence(value))

    def sensory_value(self, value):
        fields(value, 'schema body')
        return f'⟨{self.bytes(value["schema"])}, {self.bytes(value["body"])}⟩'

    def source(self, item):
        present = 'sensory' in item
        fields(item, 'epoch raw_space owner shape' + (' sensory' if present else ''))
        require(not present or (self.sensory and item['sensory'] is True),
                'Sensory presence requires the extended codec and canonical true')
        return '⟨' + ', '.join((wide(item['epoch']), wide(item['raw_space']),
            option(item['owner'], lineage), tag(item['shape'], ['Event', 'Query']),
            boolean(present))) + '⟩'


    def event(self, item, query=False):
        keys = 'epoch raw_space owner raw input episode life_age action' + (' pose' if query or 'pose' in item else '')
        present = 'sensory' in item
        fields(item, keys + (' sensory' if present else ''))
        require(not present or self.sensory, 'Sensory payload requires the extended codec')
        prefix = [wide(item['epoch']), wide(item['raw_space']), option(item['owner'], lineage),
                  natural(item['raw']), natural(item['input'])]
        byte = lambda n: natural(n, 255)
        if query:
            suffix = [natural(item['episode']), natural(item['life_age']),
                      option(item['pose'], byte), option(item['action'], byte)]
        else:
            suffix = [byte(item['action']), natural(item['episode']), natural(item['life_age']),
                      option(item.get('pose'), byte)]
        payload = '(some ' + self.sensory_value(item['sensory']) + ')' if present else 'none'
        return '⟨' + ', '.join(prefix + suffix + [payload]) + '⟩'


    def value(self, item, depth=0):
        require(depth <= 64, 'Value tree exceeds extraction depth limit')
        require(type(item) is dict and 'kind' in item, 'Expected structured value')
        kind = item['kind']
        if kind == 'Unit':
            fields(item, 'kind'); return '.unit'
        if kind == 'Pair':
            fields(item, 'kind first second')
            return f'(.pair {self.value(item["first"], depth + 1)} {self.value(item["second"], depth + 1)})'
        if kind == 'Trace':
            fields(item, 'kind events')
            return '(.trace ' + array(self.event(e) for e in sequence(item['events'])) + ')'
        fields(item, 'kind value')
        v = item['value']
        if kind == 'Nat': return '(.nat ' + natural(v) + ')'
        if kind == 'Bool': return '(.bool ' + boolean(v) + ')'
        if kind == 'Action': return '(.action ' + natural(v, 255) + ')'
        if kind == 'Bytes':
            require(self.sensory, 'Byte values require the extended codec')
            return '(.bytes ' + self.bytes(v) + ')'
        if kind in ('Event', 'Query'): return f'(.{kind.lower()} {self.event(v, kind == "Query")})'
        require(kind == 'Outcome' and type(v) is dict, 'Unknown value')
        if v.get('kind') == 'End':
            fields(v, 'kind terminal')
            out = '.terminal ' + tag(v['terminal'], ['Death', 'Time', 'Completed', 'Ended'])
        else:
            fields(v, 'kind key')
            out = tag(v['kind'], ['Next', 'Refused']) + ' ' + natural(v['key'])
        return '(.outcome (' + out + '))'


    def code(self, item, depth=0):
        require(depth <= 64, 'Code tree exceeds extraction depth limit')
        fields(item, 'steps output library')
        steps = []
        for step in sequence(item['steps']):
            fields(step, 'operation arguments')
            kind, payload = variant(step['operation'])
            require(kind in ('Primitive', 'Call'), 'Unknown operation form')
            allowed = OPS + POSE_OPS + (['EventSensory', 'QuerySensory'] if self.sensory else [])
            operation = '.primitive ' + tag(payload, allowed) if kind == 'Primitive' else '.call ' + natural(payload)
            args = []
            for operand in sequence(step['arguments']):
                k, data = variant(operand)
                require(k in ('Literal', 'Input', 'Register'), 'Unknown operand form')
                args.append(f'(.{k.lower()} ' + (self.value(data) if k == 'Literal' else natural(data)) + ')')
            steps.append('⟨' + operation + ', ' + array(args) + ', none⟩')
        return '(.mk ' + array(steps) + ' ' + option(item['output'], natural) + ' ' + array(
            self.code(child, depth + 1) for child in sequence(item['library'])) + ')'


    def binding(self, item):
        fields(item, 'interface width slots')
        slots = []
        width = natural(item['width'])
        for slot in sequence(item['slots']):
            if slot == 'State': slots.append('.state')
            else:
                k, i = variant(slot)
                require(k == 'External' and int(natural(i)) < int(width), 'Invalid binding slot')
                slots.append('(.external ' + str(i) + ')')
        return f'⟨{self.source(item["interface"])}, {width}, {array(slots)}⟩'


    def input_row(self, item):
        fields(item, 'interface values')
        return self.source(item['interface']), array(self.value(v) for v in sequence(item['values']))


    def cut(self, item):
        fields(item, 'definition owner state value')
        self.value(item['value'])
        return '⟨' + ', '.join(natural(item[k]) for k in ('definition', 'owner', 'state')) + '⟩'


    def prepared(self, item):
        fields(item, 'before after')
        fields(item['after'], 'state value')
        self.value(item['after']['value'])
        return '⟨' + self.cut(item['before']) + ', ' + natural(item['after']['state']) + '⟩'


    def report(self, item):
        fields(item, 'stop registers dispatch_attempts expanded_attempts call_attempts halt')
        def halt(h):
            fields(h, 'path reason')
            return '⟨' + array(natural(n) for n in sequence(h['path'])) + ', ' + reason(h['reason']) + '⟩'
        return '⟨' + ', '.join([tag(item['stop'], ['Complete', 'Undefined', 'Budget']),
            array(self.value(v) for v in sequence(item['registers'])), natural(item['dispatch_attempts']),
            natural(item['expanded_attempts']), natural(item['call_attempts']), option(item['halt'], halt)]) + '⟩'


    def refusal(self, item):
        require(type(item) is dict and 'kind' in item, 'Expected observer refusal')
        kind = item['kind']; phase = tag(item['phase'], ['Update', 'Read'])
        tag(item['classification'], ['Undefined', 'Resource', 'Poisoned'])
        base = 'kind phase classification'
        if kind == 'Suspended':
            fields(item, base + ' reason dispatches meter_error')
            encoded = suspension(item['reason'], item['dispatches'], item['meter_error'])
            require(item['meter_error'] is (item['reason'] == 'Work'), 'Incoherent suspension meter presence')
            return f'(.suspended {phase} {encoded})'
        if kind == 'Execution':
            fields(item, base + ' report meter_error counter_capacity primitive_refusal')
            return f'(.execution {phase} {self.report(item["report"])} {boolean(item["meter_error"])} ' + \
                f'{boolean(item["counter_capacity"])} {option(item["primitive_refusal"], lambda x: tag(x, ["Undefined", "Resource"]))})'
        if kind == 'Work':
            fields(item, base + ' point error execution'); require(item['error'] is None, 'Expected unit meter error')
            return f'(.work {phase} ' + tag(item['point'], ['InputBinding', 'RootOutput']) + ' ' + option(item['execution'], self.report) + ')'
        if kind == 'MissingRootOutput':
            fields(item, base + ' execution'); return f'(.missingRootOutput {phase} {self.report(item["execution"])})'
        if kind == 'Capacity':
            fields(item, base + ' point'); return f'(.capacity {phase} ' + tag(item['point'], ['InputBinding', 'RootOutput']) + ')'
        if kind == 'Poisoned':
            fields(item, base); return f'(.poisoned {phase})'
        require(kind == 'Binding', 'Unknown refusal')
        fields(item, base + ' reason'); r = item['reason']
        if r.get('kind') == 'Interface': fields(r, 'kind'); encoded = '.interface'
        elif r.get('kind') == 'Width':
            fields(r, 'kind expected actual'); encoded = f'(.width {natural(r["expected"])} {natural(r["actual"])})'
        else:
            fields(r, 'kind slot input width'); require(r['kind'] == 'InvalidSlot', 'Unknown binding refusal')
            encoded = '(.invalidSlot ' + ' '.join(natural(r[k]) for k in ('slot', 'input', 'width')) + ')'
        return f'(.binding {phase} {encoded})'


    def namespace(self, item):
        fields(item, 'owner epoch capacity entries')
        natural(item['owner']); wide(item['epoch']); natural(item['capacity'])
        entries = []
        for entry in sequence(item['entries']):
            fields(entry, 'input value')
            entries.append(f'({natural(entry["input"])}, {self.value(entry["value"])})')
        return array(entries)


    def history(self, item):
        fields(item, 'owner origin events'); natural(item['owner'])
        events = []
        for entry in sequence(item['events']):
            if entry == {'kind': 'Gap'}: events.append('none')
            else:
                interface, values = self.input_row(entry); events.append(f'(some ({interface}, {values}))')
        return f'⟨{origin(item["origin"])}, {array(events)}⟩'



LEGACY = AcquiredDecoder()
SENSORY = AcquiredDecoder(sensory=True)
