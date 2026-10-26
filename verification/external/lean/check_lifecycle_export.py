#!/usr/bin/env python3
"""Replay finite native lifecycle observations; universal model laws remain separate."""
from pathlib import Path
import re
from observer_export_replay import (ROOT, MAX64, array, boolean, fields, natural,
    require, sequence, variant, main as replay_main)

OPS = ('Append At Last Length EventInput EventRaw EventAction EventAge QueryInput '
       'QueryRaw QueryAge QueryAction Pair First Second Equal Select Add Rem Next Refused').split()
POSE_OPS = 'Unit Zero EmptyTrace Identity EventPose QueryPose'.split()
ERRORS = 'Arity Sort Missing Overflow TraceCapacity Allocation'.split()


def validate_adapter(data):
    """Version selects the representation; field absence never selects a version.

    V1 embeds old events with no pose port as None. V2 requires the explicit
    optional pose on every event, including literals and complete trace values.
    Mixed semantic versions need a per-definition value domain and are refused.
    """
    versions = set()
    for definition in sequence(data['definitions']):
        require(type(definition) is dict and 'semantics' in definition, 'Missing semantic adapter')
        fields(definition['semantics'], 'kind max_trace')
        version = definition['semantics']['kind']
        require(type(version) is str and version in ('AcquiredV1', 'AcquiredV2'), 'Unsupported semantic adapter')
        versions.add(version)
    require(len(versions) == 1, 'One explicit semantic adapter version is required')
    version = next(iter(versions))
    pending = [data]
    while pending:
        node = pending.pop()
        if type(node) is list:
            pending.extend(node)
        elif type(node) is dict:
            kind = node.get('kind')
            if kind in ('Event', 'Trace'):
                rows = [node.get('value')] if kind == 'Event' else sequence(node.get('events'))
                for row in rows:
                    require(type(row) is dict and ('pose' in row) == (version == 'AcquiredV2'),
                            'Event pose fields do not match declared adapter version')
            operation = node.get('operation')
            if version == 'AcquiredV1' and type(operation) is dict:
                require(operation.get('Primitive') not in POSE_OPS, 'Primitive requires AcquiredV2')
            pending.extend(node.values())
    return version


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


def source(item):
    fields(item, 'epoch raw_space owner shape')
    return '⟨' + ', '.join((wide(item['epoch']), wide(item['raw_space']),
        option(item['owner'], lineage), tag(item['shape'], ['Event', 'Query']))) + '⟩'


def event(item, query=False):
    keys = 'epoch raw_space owner raw input episode life_age action' + (' pose' if query or 'pose' in item else '')
    fields(item, keys)
    prefix = [wide(item['epoch']), wide(item['raw_space']), option(item['owner'], lineage),
              natural(item['raw']), natural(item['input'])]
    byte = lambda n: natural(n, 255)
    if query:
        suffix = [natural(item['episode']), natural(item['life_age']),
                  option(item['pose'], byte), option(item['action'], byte)]
    else:
        suffix = [byte(item['action']), natural(item['episode']), natural(item['life_age']),
                  option(item.get('pose'), byte)]
    return '⟨' + ', '.join(prefix + suffix) + '⟩'


def value(item, depth=0):
    require(depth <= 64, 'Value tree exceeds extraction depth limit')
    require(type(item) is dict and 'kind' in item, 'Expected structured value')
    kind = item['kind']
    if kind == 'Unit':
        fields(item, 'kind'); return '.unit'
    if kind == 'Pair':
        fields(item, 'kind first second')
        return f'(.pair {value(item["first"], depth + 1)} {value(item["second"], depth + 1)})'
    if kind == 'Trace':
        fields(item, 'kind events')
        return '(.trace ' + array(event(e) for e in sequence(item['events'])) + ')'
    fields(item, 'kind value')
    v = item['value']
    if kind == 'Nat': return '(.nat ' + natural(v) + ')'
    if kind == 'Bool': return '(.bool ' + boolean(v) + ')'
    if kind == 'Action': return '(.action ' + natural(v, 255) + ')'
    if kind in ('Event', 'Query'): return f'(.{kind.lower()} {event(v, kind == "Query")})'
    require(kind == 'Outcome' and type(v) is dict, 'Unknown value')
    if v.get('kind') == 'End':
        fields(v, 'kind terminal')
        out = '.terminal ' + tag(v['terminal'], ['Death', 'Time', 'Completed', 'Ended'])
    else:
        fields(v, 'kind key')
        out = tag(v['kind'], ['Next', 'Refused']) + ' ' + natural(v['key'])
    return '(.outcome (' + out + '))'


def code(item, depth=0):
    require(depth <= 64, 'Code tree exceeds extraction depth limit')
    fields(item, 'steps output library')
    steps = []
    for step in sequence(item['steps']):
        fields(step, 'operation arguments')
        kind, payload = variant(step['operation'])
        require(kind in ('Primitive', 'Call'), 'Unknown operation form')
        operation = '.primitive ' + tag(payload, OPS + POSE_OPS) if kind == 'Primitive' else '.call ' + natural(payload)
        args = []
        for operand in sequence(step['arguments']):
            k, data = variant(operand)
            require(k in ('Literal', 'Input', 'Register'), 'Unknown operand form')
            args.append(f'(.{k.lower()} ' + (value(data) if k == 'Literal' else natural(data)) + ')')
        steps.append('⟨' + operation + ', ' + array(args) + ', none⟩')
    return '(.mk ' + array(steps) + ' ' + option(item['output'], natural) + ' ' + array(
        code(child, depth + 1) for child in sequence(item['library'])) + ')'


def binding(item):
    fields(item, 'interface width slots')
    slots = []
    width = natural(item['width'])
    for slot in sequence(item['slots']):
        if slot == 'State': slots.append('.state')
        else:
            k, i = variant(slot)
            require(k == 'External' and int(natural(i)) < int(width), 'Invalid binding slot')
            slots.append('(.external ' + str(i) + ')')
    return f'⟨{source(item["interface"])}, {width}, {array(slots)}⟩'


def input_row(item):
    fields(item, 'interface values')
    return source(item['interface']), array(value(v) for v in sequence(item['values']))


def cut(item):
    fields(item, 'definition owner state value')
    value(item['value'])
    return '⟨' + ', '.join(natural(item[k]) for k in ('definition', 'owner', 'state')) + '⟩'


def prepared(item):
    fields(item, 'before after')
    fields(item['after'], 'state value')
    value(item['after']['value'])
    return '⟨' + cut(item['before']) + ', ' + natural(item['after']['state']) + '⟩'


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


def report(item):
    fields(item, 'stop registers dispatch_attempts expanded_attempts call_attempts halt')
    def halt(h):
        fields(h, 'path reason')
        return '⟨' + array(natural(n) for n in sequence(h['path'])) + ', ' + reason(h['reason']) + '⟩'
    return '⟨' + ', '.join([tag(item['stop'], ['Complete', 'Undefined', 'Budget']),
        array(value(v) for v in sequence(item['registers'])), natural(item['dispatch_attempts']),
        natural(item['expanded_attempts']), natural(item['call_attempts']), option(item['halt'], halt)]) + '⟩'


def refusal(item):
    require(type(item) is dict and 'kind' in item, 'Expected observer refusal')
    kind = item['kind']; phase = tag(item['phase'], ['Update', 'Read'])
    tag(item['classification'], ['Undefined', 'Resource', 'Poisoned'])
    base = 'kind phase classification'
    if kind == 'Execution':
        fields(item, base + ' report meter_error counter_capacity primitive_refusal')
        return f'(.execution {phase} {report(item["report"])} {boolean(item["meter_error"])} ' + \
            f'{boolean(item["counter_capacity"])} {option(item["primitive_refusal"], lambda x: tag(x, ["Undefined", "Resource"]))})'
    if kind == 'Work':
        fields(item, base + ' point error execution'); require(item['error'] is None, 'Expected unit meter error')
        return f'(.work {phase} ' + tag(item['point'], ['InputBinding', 'RootOutput']) + ' ' + option(item['execution'], report) + ')'
    if kind == 'MissingRootOutput':
        fields(item, base + ' execution'); return f'(.missingRootOutput {phase} {report(item["execution"])})'
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


def budget(item):
    fields(item, 'dispatch_limit work_remaining dispatches poisoned')
    limit = option(item['dispatch_limit'], natural)
    dispatches = natural(item['dispatches']); remaining = natural(item['work_remaining'])
    if item['dispatch_limit'] is not None:
        require(item['dispatches'] <= item['dispatch_limit'], 'Unreachable dispatch budget')
    return f'⟨{limit}, {remaining}, 0, {dispatches}, {boolean(item["poisoned"])}⟩'


def emit(data):
    fields(data, 'schema definitions ownership naming installation refusals')
    require(data['schema'] == 'synarc-observer-lifecycle-v1', 'Unknown lifecycle schema')
    adapter = validate_adapter(data)
    lines = ['import VerumExternalReplay.AcquiredObserver', 'import VerumExternalReplay.ObserverOwnership',
             'import VerumExternalReplay.ExactNames', 'import VerumExternalReplay.ObserverInstallation',
             'open AcquiredObserver', 'set_option warningAsError true', 'namespace FrozenObserverExport']
    claims, names, definitions = [], set(), {}

    def claim(equation, proof='rfl'):
        name = 'c' + str(len(claims)); claims.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')

    def named(row):
        name = row.get('name')
        require(type(name) is str and name and name not in names, 'Missing or duplicate observation name')
        names.add(name)

    owners, states, state_definitions = {}, {}, {}
    def register(c):
        cut(c); require(c['definition'] in definitions, 'Unknown definition identity')
        owner = c['owner']
        require(owners.setdefault(owner, c['definition']) == c['definition'], 'Owner definition changed')
        require(state_definitions.setdefault(c['state'], c['definition']) == c['definition'],
                'State definition changed')
        encoded = value(c['value'])
        if c['state'] in states: claim(f'({states[c["state"]]} : Value) = {encoded}')
        else: states[c['state']] = encoded

    for d in sequence(data['definitions']):
        fields(d, 'identity initial update read update_binding read_binding semantics')
        identity = natural(d['identity']); require(d['identity'] not in definitions, 'Duplicate definition')
        fields(d['semantics'], 'kind max_trace')
        require(d['semantics']['kind'] == adapter, 'Mixed semantic adapter versions')
        cap = natural(d['semantics']['max_trace']); prefix = 'd' + identity
        definitions[d['identity']] = (prefix, cap, d)
        lines.append(f'def {prefix}_initial : Value := {value(d["initial"])}')
        for phase in ('update', 'read'):
            lines.append(f'def {prefix}_{phase} : FiniteCode.Code Op Value := {code(d[phase])}')
            lines.append(f'def {prefix}_{phase}_binding : ObserverAdmission.Binding Source := {binding(d[phase + "_binding"])}')
    require(definitions, 'No actual definitions')

    for i, row in enumerate(sequence(data['ownership'])):
        named(row); kind = row['kind']
        if kind == 'fork':
            fields(row, 'name kind before after'); register(row['before'])
            claim(f'({array(str(n) for n in owners)} : List Nat).contains {natural(row["after"]["owner"])} = false')
            register(row['after'])
            claim(f'ObserverOwnership.fork ({cut(row["before"])} : ObserverOwnership.Cut Nat Nat Nat) {natural(row["after"]["owner"])} = {cut(row["after"])}')
            claim(f'({natural(row["before"]["owner"])} : Nat) ≠ {natural(row["after"]["owner"])}', 'decide')
            continue
        before, after = row['current_before'], row['current_after']; register(before); register(after)
        p, cap, definition = definitions[before['definition']]
        result = row['result']
        if kind == 'commit':
            fields(row, 'name kind current_before prepared result current_after')
            preparation = row['prepared']; register(preparation['before']); prep = prepared(preparation)
            state = preparation['after']; register({**preparation['before'], **state})
            if result['kind'] == 'Ok':
                fields(result, 'kind receipt'); fields(result['receipt'], 'before after')
                a, b = result['receipt']['before'], result['receipt']['after']; register(a); register(b)
                receipt = f'.ok ⟨{cut(a)}, {cut(b)}⟩'
            else:
                fields(result, 'kind reason prepared'); require(result['kind'] == 'Err', 'Unknown commit result')
                register(result['prepared']['before'])
                register({**result['prepared']['before'], **result['prepared']['after']})
                receipt = '.error (' + tag(result['reason'], ['ForeignInstance', 'StaleState']) + ', ' + prepared(result['prepared']) + ')'
            claim(f'ObserverOwnership.commit ({cut(before)} : ObserverOwnership.Cut Nat Nat Nat) {prep} = ⟨{cut(after)}, {receipt}⟩')
            continue
        require(kind in ('prepare', 'read'), 'Unknown ownership observation')
        fields(row, 'name kind current_before input budget_before result current_after budget_after')
        supplied, values = input_row(row['input']); initial_budget = budget(row['budget_before']); budget(row['budget_after'])
        phase = 'update' if kind == 'prepare' else 'read'; invocation = f'o{i}'
        lines.append(f'def {invocation} := ObserverAdmission.evaluate (apply {cap}) .{phase} '
                     f'{p}_{phase} {p}_{phase}_binding {value(before["value"])} {supplied} {values} {initial_budget}')
        proof = f'unfold {invocation} {p}_{phase} {p}_{phase}_binding\n  reduce_admission'
        if result['kind'] == 'Ok':
            if kind == 'prepare':
                fields(result, 'kind prepared execution'); prep = result['prepared']; prepared(prep); register(prep['before'])
                claim(f'({array(str(n) for n in states)} : List Nat).contains {natural(prep["after"]["state"])} = false')
                register({**prep['before'], **prep['after']})
                claim(f'({cut(prep["before"])} : ObserverOwnership.Cut Nat Nat Nat) = {cut(before)}')
                claim(f'({natural(prep["after"]["state"])} : Nat) ≠ {natural(before["state"])}', 'decide')
                val = value(prep['after']['value'])
            else:
                fields(result, 'kind cut value execution'); register(result['cut'])
                claim(f'({cut(result["cut"])} : ObserverOwnership.Cut Nat Nat Nat) = {cut(before)}')
                val = value(result['value'])
            expected = f'.ok ({val}, {report(result["execution"])})'
        else:
            fields(result, 'kind reason'); require(result['kind'] == 'Err', 'Unknown evaluation result')
            expected = '.error ' + refusal(result['reason'])
            claim(f'classification {refusal(result["reason"])} = ' + tag(result['reason']['classification'], ['Undefined', 'Resource', 'Poisoned']))
        claim(f'observe {invocation} = {expected}', proof)
        claim(f'({cut(before)} : ObserverOwnership.Cut Nat Nat Nat) = {cut(after)}')
        claim(f'{natural(row["budget_before"]["work_remaining"])} - {invocation}.budget.workUsed = {natural(row["budget_after"]["work_remaining"])}', proof)
        claim(f'{invocation}.budget.dispatches = {natural(row["budget_after"]["dispatches"])}', proof)
        claim(f'{invocation}.budget.poisoned = {boolean(row["budget_after"]["poisoned"])}', proof)
        require(row['budget_before']['dispatch_limit'] == row['budget_after']['dispatch_limit'], 'Dispatch limit changed')

    # Further sections are emitted by the same strict finite schema below.
    emit_naming(data['naming'], lines, claim, named)
    emit_installation(data['installation'], definitions, register, lines, claim, named)
    emit_classification(data['refusals'], claim, named)
    lines += ['#print axioms ' + n for n in claims] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', {'definitions': len(definitions),
        **{k: len(data[k]) for k in ('ownership', 'naming', 'installation', 'refusals')}, 'claims': len(claims)}


def namespace(item):
    fields(item, 'owner epoch capacity entries')
    natural(item['owner']); wide(item['epoch']); natural(item['capacity'])
    entries = []
    for entry in sequence(item['entries']):
        fields(entry, 'input value')
        entries.append(f'({natural(entry["input"])}, {value(entry["value"])})')
    return array(entries)


def emit_naming(rows, lines, claim, named):
    retained = {}
    for i, row in enumerate(sequence(rows)):
        named(row); fields(row, 'name before input value budget_before result after budget_after')
        before, after = row['before'], row['after']; keys, final = namespace(before), namespace(after)
        previous = retained.get(before['owner'])
        if previous is not None:
            require(all(previous[k] == before[k] for k in ('epoch', 'capacity')),
                    'Shared namespace metadata changed')
            previous_keys = namespace(previous)
            claim(f'({keys} : List (Nat × Value)).take {len(previous["entries"])} = {previous_keys}')
        retained[after['owner']] = after
        claim(f'({keys} : List (Nat × Value)).Nodup', 'decide')
        budget(row['budget_before']); budget(row['budget_after'])
        for field in ('owner', 'epoch', 'capacity'):
            encode = wide if field == 'epoch' else natural
            claim(f'({encode(before[field])} : Nat) = {encode(after[field])}')
        invocation = f'n{i}'
        lines.append(f'def {invocation} := ExactNames.metered (min {natural(before["capacity"])} (2^56)) '
                     f'({keys} : List (Nat × Value)) ({natural(row["input"])}, {value(row["value"])}) '
                     f'{natural(row["budget_before"]["work_remaining"])} {boolean(row["budget_before"]["poisoned"])}')
        r = row['result']
        if r['kind'] == 'Ok':
            fields(r, 'kind name'); result = '.ok ' + natural(r['name'])
        else:
            fields(r, 'kind reason'); require(r['kind'] == 'Err', 'Unknown naming result')
            reason = r['reason']
            require(type(reason) is dict and 'kind' in reason, 'Expected naming refusal')
            if reason['kind'] == 'NameCapacity':
                fields(reason, 'kind epoch'); require(reason['epoch'] == before['epoch'], 'Foreign namespace refusal')
                result = '.error .capacity'
            elif reason['kind'] == 'External':
                fields(reason, 'kind reason'); result = '.error ' + tag(reason['reason'], ['Work', 'Poisoned'])
            else: raise ValueError('Unsupported naming refusal; cannot infer allocator or mutex failure')
        claim(f'{invocation} = ({final}, {result}, {natural(row["budget_after"]["work_remaining"])})')
        require(row['budget_before']['dispatch_limit'] == row['budget_after']['dispatch_limit'], 'Dispatch limit changed')
        for field in ('dispatches', 'poisoned'):
            encode = boolean if field == 'poisoned' else natural
            typ = 'Bool' if field == 'poisoned' else 'Nat'
            claim(f'({encode(row["budget_before"][field])} : {typ}) = {encode(row["budget_after"][field])}')


def origin(item):
    fields(item, 'lineage epoch ordinal')
    return f'({lineage(item["lineage"])}, {wide(item["epoch"])}, {natural(item["ordinal"])})'


def history(item):
    fields(item, 'owner origin events'); natural(item['owner'])
    events = []
    for entry in sequence(item['events']):
        if entry == {'kind': 'Gap'}: events.append('none')
        else:
            interface, values = input_row(entry); events.append(f'(some ({interface}, {values}))')
    return f'⟨{origin(item["origin"])}, {array(events)}⟩'


def emit_installation(rows, definitions, register, lines, claim, named):
    admissions, histories, admission_histories = {}, {}, {}
    def retained_history(item):
        encoded = history(item)
        previous = histories.setdefault(item['owner'], encoded)
        typ = 'ObserverInstallation.History (Lineage × Nat × Nat) (Source × List Value)'
        claim(f'({encoded} : {typ}).origin = ({previous} : {typ}).origin')
        claim(f'({encoded} : {typ}).events = ({previous} : {typ}).events')
        return encoded
    for i, row in enumerate(sequence(rows)):
        named(row)
        common = 'name kind admission history cut known_histories'
        fields(row, common + (' installed replay' if row['kind'] == 'success' else
                             ' reason incomplete request current_before current_after'))
        require(row['kind'] in ('success', 'failure'), 'Unknown installation observation')
        admission = row['admission']; fields(admission, 'kind owner definition')
        require(admission['kind'] in ('Validated', 'Accepted'), 'Unknown admission owner')
        natural(admission['owner']); natural(admission['definition'])
        key = (admission['kind'], admission['owner'])
        require(admissions.setdefault(key, admission['definition']) == admission['definition'],
                'Retained admission definition changed')
        require(admission['definition'] in definitions, 'Unknown installed definition')
        p, cap, _ = definitions[admission['definition']]
        known = []
        phases, retained = [], []
        for old in sequence(row['known_histories']):
            fields(old, 'phase history'); require(old['phase'] in ('training', 'fresh'), 'Unknown history source')
            known.append(retained_history(old['history']))
            phases.append(old['phase']); retained.append((old['phase'], old['history']['owner']))
            require(old['history']['origin'] == row['history']['origin'], 'Unselected retained history origin')
        require(phases in ([], ['training'], ['fresh'], ['training', 'fresh']) and
                (admission['kind'] == 'Validated' or 'fresh' not in phases),
                'Retained history phases must match native installation order')
        history_key = (*key, origin(row['history']['origin']))
        require(admission_histories.setdefault(history_key, retained) == retained,
                'Retained admission histories changed')
        invocation = f'i{i}'
        lines.append(f'def {invocation}_receiver : ObserverInstallation.History '
                     f'(Lineage × Nat × Nat) (Source × List Value) := {retained_history(row["history"])}')
        lines.append(f'def {invocation}_known : List (ObserverInstallation.History '
                     f'(Lineage × Nat × Nat) (Source × List Value)) := {array(known)}')
        lines.append(f'def {invocation} := ObserverInstallation.install '
                     f'(fun state (event : Source × List Value) => run {cap} {p}_update {p}_update_binding state event.1 event.2) '
                     f'{p}_initial {invocation}_known {invocation}_receiver {natural(row["cut"])}')
        unfold = f'unfold {invocation} {invocation}_known {invocation}_receiver {p}_initial {p}_update {p}_update_binding\n'
        if row['kind'] == 'success':
            installed, replay = row['installed'], row['replay']; register(installed); register(replay)
            require(installed['definition'] == admission['definition'] == replay['definition'], 'Installed definition replaced')
            events = []
            for entry in row['history']['events'][:row['cut']]:
                interface, values = input_row(entry)
                events.append(f'({interface}, {values})')
            proof = unfold + '  rw [ObserverInstallation.success_exact]\n  refine ⟨by decide, ?_⟩\n'
            proof += f'  refine ⟨{array(events)}, rfl, ?_⟩\n'
            proof += '  simp only [PartialObservers.fold]\n  unfold AcquiredObserver.run\n'
            proof += '  simp [FiniteCode.bind, List.mapM, List.mapM.loop]\n  reduce_finite_code'
            claim(f'{invocation} = some {value(installed["value"])}', proof)
            claim(f'({value(installed["value"])} : Value) = {value(replay["value"])}')
            claim(f'({natural(installed["owner"])} : Nat) ≠ {natural(replay["owner"])}', 'decide')
        else:
            fields(row['request'], 'admission history cut')
            require(all(row[k] == row['request'][k] for k in ('admission', 'history', 'cut')),
                    'Installation refusal did not retain its request owners')
            register(row['current_before']); register(row['current_after'])
            claim(f'({cut(row["current_before"])} : ObserverOwnership.Cut Nat Nat Nat) = {cut(row["current_after"])}')
            error = row['reason']; require(type(error) is dict and 'kind' in error, 'Expected installation reason')
            kind = error['kind']
            # These structural refusals are independently decidable. Resource
            # or arbitrary replay errors need their own complete runtime row.
            if kind == 'CutBeyondHistory':
                fields(error, 'kind cut events')
                require(error['cut'] == row['cut'] and error['events'] == len(row['history']['events']), 'Wrong cut diagnostic')
                diagnostic = f'.cutBeyondHistory {natural(error["cut"])} {natural(error["events"])}'
            elif kind in ('Gap', 'HistoryChanged'):
                fields(error, 'kind event'); position = int(natural(error['event']))
                diagnostic = tag(kind, ['Gap', 'HistoryChanged']) + ' ' + str(position)
            else: raise ValueError('Unsupported installation refusal; retain full execution before checking it')
            claim(f'ObserverInstallation.structuralRefusal {invocation}_known {invocation}_receiver {natural(row["cut"])} = some ({diagnostic})')
            claim(f'{invocation} = none', unfold + '  rfl')
            require(row['incomplete'] is True, 'Structural installation failure must remain incomplete')


def emit_classification(rows, claim, named):
    for row in sequence(rows):
        named(row)
        fields(row, 'name kind domain reason_kind stop meter_error counter_capacity primitive_error primitive_refusal classification budget_before budget_after')
        require(row['kind'] == 'classification' and row['domain'] in ('canonical', 'acquired'), 'Unknown classification source')
        budget(row['budget_before']); budget(row['budget_after'])
        require(row['budget_before']['dispatch_limit'] == row['budget_after']['dispatch_limit'], 'Dispatch limit changed')
        # This finite classification format has no complete halt path/allowance
        # evidence for the cumulative u64-capacity case; refuse that extension.
        require(row['counter_capacity'] is False, 'Counter-capacity classification needs a full execution row')
        claim(f'({natural(row["budget_after"]["work_remaining"])} : Nat) ≤ {natural(row["budget_before"]["work_remaining"])}', 'decide')
        claim(f'({natural(row["budget_before"]["dispatches"])} : Nat) ≤ {natural(row["budget_after"]["dispatches"])}', 'decide')
        kind = row['reason_kind']; observed = tag(row['classification'], ['Undefined', 'Resource', 'Poisoned'])
        if kind == 'Execution':
            expression = 'executionClass ' + tag(row['stop'], ['Complete', 'Undefined', 'Budget']) + ' ' + \
                boolean(row['meter_error']) + ' ' + option(row['primitive_refusal'], lambda x: tag(x, ['Undefined', 'Resource']))
            if row['primitive_error'] is not None:
                require(row['domain'] == 'acquired', 'Canonical primitive interpretation requires its own adapter')
                claim('primitiveClass ' + tag(row['primitive_error'], ERRORS) + ' = ' + tag(row['primitive_refusal'], ['Undefined', 'Resource']))
            else: require(row['primitive_refusal'] is None, 'Unwitnessed primitive classification')
        else:
            require(row['stop'] is None and row['meter_error'] is (kind == 'Work') and row['primitive_error'] is None
                    and row['primitive_refusal'] is None, 'Unexpected execution fields')
            if kind == 'Poisoned': expression = '(.poisoned : RefusalClass)'
            elif kind in ('Work', 'Capacity'): expression = '(.resource : RefusalClass)'
            else:
                require(kind in ('Binding', 'MissingRootOutput'), 'Unknown refusal tag')
                expression = '(.undefined : RefusalClass)'
        claim(f'{expression} = {observed}')


def main():
    replay_main(emit=emit, modules=['TypedPrograms', 'FiniteCode', 'FiniteExecution',
        'FiniteExecutionProjection', 'ObserverAdmission', 'AcquiredObserver', 'ObserverOwnership',
        'ExactNames', 'ConservativeRefinement', 'ExactTransports', 'PartialObservers', 'ObserverInstallation'],
        producers=[Path(__file__).resolve()], description=__doc__,
        scope='Finite supplied lifecycle observations checked against ownership, acquired-program, '
              'installation and exact naming models for the explicitly declared 64-bit '
              'AcquiredV1 legacy or AcquiredV2 pose adapter. '
              'Not universal Rust ownership/memory correctness, '
              'authenticated provenance, learning correctness or default enrollment.')


if __name__ == '__main__':
    main()
