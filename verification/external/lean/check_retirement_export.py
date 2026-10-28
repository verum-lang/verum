#!/usr/bin/env python3
"""Finite retirement boundaries from independent native snapshots, not arbitrary Rust executions."""
import json
from pathlib import Path
import check_lifecycle_export as life
import observer_trace_proofs
from observer_export_replay import array, boolean, fields, natural, require, sequence, main as replay_main


def literal(value):
    # Complete canonical data, never a digest substituted for structural equality.
    return json.dumps(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True))


def event(row):
    fields(row, 'lineage ordinal epoch raw_space raw situation action episode life_age pose inputs')
    life.lineage(row['lineage'])
    for k in ('epoch', 'raw_space'): life.wide(row[k])
    for k in ('ordinal', 'raw', 'situation', 'episode', 'life_age'): natural(row[k])
    natural(row['action'], 255); life.option(row['pose'], lambda x: natural(x, 255))
    epochs = []
    for pair in sequence(row['inputs']):
        require(type(pair) is list and len(pair) == 2, 'Invalid event input')
        life.wide(pair[0]); natural(pair[1]); epochs.append(int(pair[0]))
    require(epochs == sorted(set(epochs)), 'Event input epochs must be unique and sorted')


def history(value):
    fields(value, 'lineage length events'); life.lineage(value['lineage']); natural(value['length'])
    for row in sequence(value['events']): event(row)
    require(value['length'] == len(value['events']), 'Truncated history')
    require([r['ordinal'] for r in value['events']] == list(range(value['length'])), 'History gap')


def pairs(rows):
    keys = []
    for pair in sequence(rows):
        require(type(pair) is list and len(pair) == 2, 'Invalid dictionary entry')
        natural(pair[0]); natural(pair[1]); keys.append(pair[0])
    require(keys == sorted(set(keys)), 'Dictionary keys must be unique and sorted')


def tier(value, prefix=False):
    fields(value, 'boundary input_epoch birth dict next_ordinal dense_bits' +
           ('' if prefix else ' initial_dict overflows born_at'))
    for k in ('boundary', 'next_ordinal', 'dense_bits'): natural(value[k])
    life.option(value['input_epoch'], life.wide)
    if value['birth'] is not None:
        fields(value['birth'], 'lineage ordinal'); life.lineage(value['birth']['lineage']); natural(value['birth']['ordinal'])
    pairs(value['dict'])
    if not prefix:
        if value['initial_dict'] is not None: pairs(value['initial_dict'])
        natural(value['overflows']); natural(value['born_at'])


def corpus(rows):
    keys = []
    for group in sequence(rows):
        fields(group, 'key rows'); natural(group['key']); keys.append(group['key'])
        for row in sequence(group['rows']):
            fields(row, 'tick outcome ordinal raw_top cut')
            fields(row['tick'], 'key age events obs tower theta')
            for k in ('key', 'age', 'events', 'tower'): natural(row['tick'][k])
            boolean(row['tick']['obs']); natural(row['tick']['theta'], 255)
            outcome(row['outcome']); natural(row['ordinal']); life.option(row['raw_top'], natural); event(row['cut'])
    require(keys == sorted(set(keys)), 'Corpus keys must be unique and sorted')


def outcome(value):
    life.value(value); require(value['kind'] == 'Outcome', 'Expected actual world outcome')


def actual(value):
    if value is not None:
        fields(value, 'cut outcome'); event(value['cut']); outcome(value['outcome'])


def prefix_tier(value):
    return {k: value[k] for k in ('boundary', 'input_epoch', 'birth', 'dict', 'next_ordinal', 'dense_bits')}


def emit(data):
    fields(data, 'schema definitions cases')
    require(data['schema'] == 'synarc-observer-retirement-v1', 'Unknown retirement schema')
    require(life.validate_adapter(data) == 'AcquiredV2', 'Retirement requires AcquiredV2')
    base, _ = life.emit(dict(schema='synarc-observer-lifecycle-v1', definitions=data['definitions'],
                            ownership=[], naming=[], installation=[], refusals=[]))
    lines = ['import VerumExternalReplay.ObserverRetirement', 'import VerumExternalReplay.TraceReduction',
             base.removesuffix('end FrozenObserverExport\n')]
    names = []
    def claim(equation, proof='rfl'):
        name = 'r' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')
    def same(a, b): claim(f'({literal(a)} : String) = {literal(b)}')
    def eq(a, b): claim(f'({natural(a)} : Nat) = {natural(b)}')
    def le(a, b): claim(f'decide (({natural(a)} : Nat) ≤ {natural(b)}) = true')
    definitions = {d['identity']: d for d in data['definitions']}
    register, _, _ = life.cut_registry(definitions, claim)
    canonical_owners, canonical_states = {}, {}
    def cut(value):
        fields(value, 'kind value'); c = value['value']; life.cut(c)
        if value['kind'] == 'Acquired': register(c)
        else:
            require(value['kind'] == 'Canonical', 'Unknown owned cut')
            same(canonical_owners.setdefault(c['owner'], c['definition']), c['definition'])
            same(canonical_states.setdefault(c['state'], c['value']), c['value'])
            def scalar(v):
                require(v['kind'] in ('Nat', 'Bool', 'Pair'), 'Noncanonical clock value')
                life.value(v)
                if v['kind'] == 'Pair': scalar(v['first']); scalar(v['second'])
            scalar(c['value'])
    def floors(rows, prefix=False):
        for row in sequence(rows):
            fields(row, 'input_epoch cut' + ('' if prefix else ' output_epoch'))
            life.wide(row['input_epoch']); cut(row['cut'])
            if not prefix: life.option(row['output_epoch'], life.wide)
    def pending(value):
        if value is None: return
        fields(value, 'fault boundary actual'); fault = value['fault']
        fields(fault, 'index cut input refusal'); natural(fault['index']); register(fault['cut'])
        life.input_row(fault['input']); life.refusal(fault['refusal'])
        require(value['boundary'] in ('BeforeAct', 'AfterAct'), 'Unknown boundary'); actual(value['actual'])
    def receipt(value):
        fields(value, 'floor boundary ordinal fault history actual current_prefix retired_floors retired_tiers corpus work_before work_after journal_slots')
        fields(value['floor'], 'lineage input_epoch output_epoch'); life.lineage(value['floor']['lineage'])
        life.wide(value['floor']['input_epoch']); life.wide(value['floor']['output_epoch'])
        natural(value['ordinal']); history(value['history'])
        pending({k: value[k] for k in ('fault', 'boundary', 'actual')})
        fields(value['current_prefix'], 'epoch floors tiers'); life.wide(value['current_prefix']['epoch'])
        floors(value['current_prefix']['floors'], True); floors(value['retired_floors'])
        for t in sequence(value['current_prefix']['tiers']): tier(t, True)
        for t in sequence(value['retired_tiers']): tier(t)
        corpus(value['corpus'])
        for k in ('work_before', 'work_after', 'journal_slots'): natural(value[k])
    def snapshot(s):
        fields(s, 'epoch highest_epoch history physical floors tiers corpus pending journal used limit work')
        life.wide(s['epoch']); life.wide(s['highest_epoch']); history(s['history'])
        fields(s['physical'], 'raw pose episode life_age attempts')
        for k in ('raw', 'life_age', 'attempts'): natural(s['physical'][k])
        life.option(s['physical']['pose'], lambda x: natural(x, 255)); life.option(s['physical']['episode'], natural)
        floors(s['floors']); corpus(s['corpus']); pending(s['pending'])
        for t in sequence(s['tiers']): tier(t)
        bounds = [t['boundary'] for t in s['tiers']]
        require(bounds == sorted(bounds, reverse=True), 'Unordered tier chain')
        for r in sequence(s['journal']): receipt(r)
        for k in ('used', 'limit', 'work'): natural(s[k])
        eq(sum(r['journal_slots'] for r in s['journal']), s['used']); le(s['used'], s['limit'])
    def model_receipt(r, index):
        return f'⟨{life.lineage(r["floor"]["lineage"])}, {index}, {natural(r["fault"]["index"])}, .undefined, {natural(r["work_after"])}⟩'
    def model(s):
        layers = array(f'⟨{life.wide(f["input_epoch"])}, {literal(f)}⟩' for f in s['floors'])
        journal = array(model_receipt(r, i) for i, r in enumerate(s['journal']))
        # The model cut is the observed journal revision, not a native state ID.
        # Exact native instance/state ownership is checked separately below.
        physical = literal([s['physical'], s['history'], s['highest_epoch']])
        return f'({{ owner := {life.lineage(s["history"]["lineage"])}, cut := {len(s["journal"])}, language := {life.wide(s["epoch"])}, layers := {layers}, physical := {physical}, work := {natural(s["work"])}, receipts := {journal}, capacity := {natural(s["limit"])} }} : ObserverRetirement.State Lineage Nat String String)'
    committed = refused = before_act = after_act = surviving_prefix = 0
    rows = sequence(data['cases']); require(rows, 'No retirement boundaries')
    seen = set()
    for i, row in enumerate(rows):
        fields(row, 'name attempt precommit after result')
        require(type(row['name']) is str and row['name'] and row['name'] not in seen, 'Invalid boundary name'); seen.add(row['name'])
        before, pre, after = row['attempt'], row['precommit'], row['after']
        snapshot(before); snapshot(after)
        if pre is not None:
            snapshot(pre)
            same({k: v for k, v in before.items() if k != 'work'}, {k: v for k, v in pre.items() if k != 'work'})
            le(pre['work'], before['work'])
        for k in ('physical', 'history', 'highest_epoch', 'limit'): same(before[k], after[k])
        le(after['work'], before['work'])
        result = row['result']; require(type(result) is dict, 'Expected native result')
        if result.get('kind') == 'Refused':
            fields(result, 'kind resource'); boolean(result['resource'])
            require(result['resource'], 'Unsupported invalid retirement attempt')
            same({k: v for k, v in before.items() if k != 'work'}, {k: v for k, v in after.items() if k != 'work'})
            refused += 1; continue
        fields(result, 'kind'); require(result['kind'] == 'Committed' and pre is not None, 'Missing independent precommit snapshot')
        require(before['pending'] is not None and after['pending'] is None and after['journal'], 'Missing actual retirement receipt')
        r = after['journal'][-1]; p = before['pending']; index = r['fault']['index']
        require(index < len(before['floors']), 'Invalid retired floor')
        same(p, {k: r[k] for k in ('fault', 'boundary', 'actual')})
        same(after['journal'], before['journal'] + [r]); same(r['history'], before['history'])
        eq(r['ordinal'], before['history']['length']); same(r['floor']['lineage'], before['history']['lineage'])
        floor = before['floors'][index]; require(floor['cut']['kind'] == 'Acquired', 'Retiring floor is not acquired')
        same(r['fault']['cut'], floor['cut']['value']); same(r['floor']['input_epoch'], floor['input_epoch'])
        require(floor['output_epoch'] is not None, 'Missing acquired output epoch')
        same(r['floor']['output_epoch'], floor['output_epoch'])
        same(r['fault']['input']['interface']['epoch'], floor['input_epoch'])
        same(after['epoch'], floor['input_epoch']); same(after['floors'], before['floors'][:index])
        same(r['retired_floors'], before['floors'][index:]); same(r['corpus'], before['corpus']); same(after['corpus'], [])
        surviving = [t for t in before['tiers'] if t['boundary'] <= index]
        same(after['tiers'], surviving); same(r['retired_tiers'], [t for t in before['tiers'] if t['boundary'] > index])
        same(r['current_prefix'], dict(epoch=after['epoch'],
            floors=[{k: f[k] for k in ('input_epoch', 'cut')} for f in after['floors']],
            tiers=[prefix_tier(t) for t in surviving]))
        slots = 1 + len(before['floors']) + len(before['tiers']) + sum(len(t['dict']) for t in before['tiers']) + sum(len(g['rows']) for g in before['corpus'])
        eq(r['journal_slots'], slots); eq(after['used'], before['used'] + slots)
        eq(r['work_before'], before['work']); eq(r['work_after'], after['work']); eq(pre['work'] + slots, before['work']); le(after['work'], pre['work'])
        if r['boundary'] == 'BeforeAct': require(r['actual'] is None, 'BeforeAct cannot claim a committed action')
        else:
            require(r['actual'] is not None and before['history']['events'], 'AfterAct requires its actual committed event')
            same(r['actual']['cut'], before['history']['events'][-1])
            same(r['actual']['cut']['lineage'], before['history']['lineage'])
            matches = [x for g in before['corpus'] for x in g['rows'] if x['cut'] == r['actual']['cut'] and x['outcome'] == r['actual']['outcome']]
            require(len(matches) == 1, 'Actual outcome must have one exact retained corpus witness')
            if r['actual']['outcome']['value']['kind'] in ('Next', 'Refused'):
                eq(r['actual']['outcome']['value']['key'], before['physical']['raw'])
        fault = r['fault']; refusal = life.refusal(fault['refusal'])
        claim(f'classification {refusal} = .undefined')
        definition = definitions[fault['cut']['definition']]; prefix = 'd' + natural(definition['identity'])
        phase = 'read' if fault['refusal']['phase'] == 'Read' else 'update'
        supplied, values = life.input_row(fault['input']); invocation = f'fault_{i}'
        ports = fault['input']['values']
        require(len(ports) == 1 and ports[0]['kind'] == ('Query' if phase == 'read' else 'Event'),
                'Expected complete physical fault ports')
        ports = ports[0]['value']
        for key in ('raw', 'episode', 'life_age', 'pose'): same(ports[key], before['physical'][key])
        if phase == 'read': require(ports['action'] is None, 'A query cannot contain a delivered action')
        if ports['owner'] is not None: same(ports['owner'], before['history']['lineage'])
        lines.append(f'def {invocation} := ObserverAdmission.evaluate (apply {natural(definition["semantics"]["max_trace"])}) .{phase} {prefix}_{phase} {prefix}_{phase}_binding {life.value(fault["cut"]["value"])} {supplied} {values} ⟨none, 2^64-1, 0, 0, false⟩')
        claim(f'observe {invocation} = .error {refusal}', f'unfold {invocation} {prefix}_{phase} {prefix}_{phase}_binding\n  (conv => lhs; cbv) <;> rfl')
        claim(f'ObserverRetirement.retire {model(before)} {model_receipt(r, len(after["journal"])-1)} = some {model(after)}')
        committed += 1
        before_act += r['boundary'] == 'BeforeAct'; after_act += r['boundary'] == 'AfterAct'
        surviving_prefix += index > 0
    lines += ['#print axioms ' + n for n in names] + ['end FrozenObserverExport']
    source, trace_claims = observer_trace_proofs.share_traces(data, '\n\n'.join(lines) + '\n')
    return source, dict(cases=len(rows), committed=committed, refused=refused, before_act=before_act,
                        after_act=after_act, surviving_prefix=surviving_prefix,
                        trace_claims=trace_claims, claims=len(names)+trace_claims)


if __name__ == '__main__':
    replay_main(emit=emit, modules=life.MODELS + ['ObserverRetirement', 'TraceReduction'],
        producers=[Path(__file__).resolve(), *life.PRODUCERS, Path(observer_trace_proofs.__file__).resolve()],
        description=__doc__, scope='Finite independent retirement snapshots, exact owned Undefined replay, current prefix preservation, weighted journal admission, observed work, actual outcome conservation and append-only receipts. Not exporter authenticity, arbitrary Rust/Habitat semantics, allocation universality or original observer-failure work admission.')
