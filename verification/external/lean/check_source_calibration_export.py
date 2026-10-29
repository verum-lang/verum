#!/usr/bin/env python3
"""Finite source-calibration correspondence, conditional on exported projections.

Checks exact source/owner metadata, support membership, bounded preflight refusal,
retained profile changes and service stuttering. Initial states are supplied,
not reconstructed histories. Missing prepared tokens, cut layers and materializer
spines are reported explicitly; no full runtime refinement or admission claim.
"""
from collections import Counter
from pathlib import Path
import json
import acquired_export_codec
from acquired_export_codec import SENSORY, lineage, option, tag, wide
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, module_closure, main as replay_main)

MODELS = module_closure(['SourceCalibration'])
PRODUCERS = [Path(__file__).resolve(), Path(acquired_export_codec.__file__).resolve()]
REFUSALS = ('Capacity ForeignSource ForeignContext ForeignOutcome BeforeOrigin '
            'Duplicate OutOfOrder StalePreparation Arithmetic Incomplete').split()
PROFILE = 'identity source origin revision last_novel end first_unretained maximum cells support'
SNAPSHOT = 'identity source origin revision last_novel end maximum support'
MAX64 = 2**64 - 1


def key(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


def sample(value):
    fields(value, 'lineage ordinal')
    return '⟨' + lineage(value['lineage']) + ', ' + natural(value['ordinal']) + '⟩'


def origin(value):
    fields(value, 'lineage epoch ordinal')
    return '⟨' + ', '.join((lineage(value['lineage']), wide(value['epoch']), natural(value['ordinal']))) + '⟩'


def context(value):
    fields(value, 'epoch input action')
    return '⟨' + ', '.join((wide(value['epoch']), natural(value['input']), natural(value['action'], 255))) + '⟩'


def support(rows):
    seen = set(); result = []
    for row in sequence(rows):
        fields(row, 'context values'); c = context(row['context'])
        require(c not in seen, 'Duplicate support context'); seen.add(c)
        values = set(); require(sequence(row['values']), 'Empty support context')
        for value in row['values']:
            v = SENSORY.value(value)
            require(v not in values, 'Duplicate support value'); values.add(v)
            result.append('(' + c + ', ' + v + ')')
    return array(result)


def profile(value):
    fields(value, PROFILE)
    return '⟨' + ', '.join([natural(value['identity']), option(value['source'], SENSORY.source),
        option(value['origin'], origin), natural(value['revision']), option(value['last_novel'], sample),
        option(value['end'], sample), option(value['first_unretained'], sample), natural(value['maximum']),
        natural(value['cells']), support(value['support'])]) + '⟩'


def snapshot(value):
    fields(value, SNAPSHOT)
    natural(value['identity']); SENSORY.source(value['source']); origin(value['origin'])
    natural(value['revision']); option(value['last_novel'], sample); option(value['end'], sample)
    natural(value['maximum']); support(value['support'])


def materializer(value):
    fields(value, 'processed cells pending support')
    natural(value['processed']); natural(value['cells']); boolean(value['pending']); support(value['support'])


def emit(data):
    fields(data, 'schema scope identity_scope events')
    require(data['schema'] == 'synarc-source-profile-lifecycle-v1', 'Unknown calibration schema')
    lines = ['import VerumExternalReplay.SourceCalibration', 'open SourceCalibration AcquiredObserver',
             'set_option warningAsError true', 'namespace FrozenObserverExport']
    claims = []
    def claim(equation):
        name = 'c' + str(len(claims)); claims.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  decide_cbv')
    checked, unchecked = Counter(), Counter()
    owners, attempts, declared, observed_snapshots = {}, {}, {}, set()

    def agenda(value, initial=False):
        fields(value, 'source_owner profile current workers retired next_look next_source first_unretained halted capacity observations')
        owner = natural(value['source_owner']); p = value['profile']; encoded = profile(p)
        natural(value['capacity']); wide(value['next_look']); wide(value['next_source'])
        option(value['first_unretained'], sample); boolean(value['halted'])
        for observation in sequence(value['observations']):
            fields(observation, 'id outcome routing_complete')
            sample(observation['id']); SENSORY.value(observation['outcome']); boolean(observation['routing_complete'])
        if p['source'] is not None and p['origin'] is not None:
            observed_snapshots.add(key({k: p[k] for k in SNAPSHOT.split()}))
        for attempt in [value['current'], *sequence(value['workers']), *sequence(value['retired'])]:
            fields(attempt, 'identity profile declared look next_source snapshot materializer alphabets')
            identity = natural(attempt['identity']); natural(attempt['profile'])
            require(attempts.setdefault(identity, owner) == owner, 'Attempt moved to a foreign source owner')
            option(attempt['look'], wide); wide(attempt['next_source']); support(attempt['alphabets'])
            if attempt['materializer'] is not None: materializer(attempt['materializer'])
            if attempt['declared'] is not None:
                address = attempt['declared']; fields(address, 'role depth evidence_round imported composition')
                require(address['role'] in ('Categorical', 'Predictive'), 'Unknown declaration role')
                natural(address['depth']); natural(address['evidence_round']); option(address['imported'], natural)
                if address['composition'] is not None:
                    fields(address['composition'], 'nesting body_inputs')
                    natural(address['composition']['nesting']); natural(address['composition']['body_inputs'])
            s = attempt['snapshot']
            if s is not None:
                snapshot(s)
                require(s['identity'] == attempt['profile'] and attempt['declared'] is not None,
                        'Snapshot has no matching declared owner')
                require(declared.setdefault(identity, s) == s, 'Immutable declared snapshot changed')
                if key(s) not in observed_snapshots:
                    require(initial, 'Declaration does not retain an observed profile')
                    unchecked['InitialSnapshotOrigin'] += 1
                    observed_snapshots.add(key(s))
        return encoded

    for event in sequence(data['events']):
        require(type(event) is dict, 'Expected lifecycle event')
        kind = event.get('kind')
        if kind in ('Physical', 'Service', 'ImportDeclaration'):
            extra = {'Physical': 'name input projected_context', 'Service': 'name grant remaining result',
                     'ImportDeclaration': 'bytes decoded'}[kind]
            fields(event, 'kind before after ' + extra)
            before, after = event['before'], event['after']; owner = before['source_owner']
            require(owner == after['source_owner'], 'Source owner changed')
            initial = owner not in owners
            if not initial: require(before == owners[owner], 'Discontinuous source-owner state')
            b = agenda(before, initial); a = agenda(after)
            pb, pa = before['profile'], after['profile']
            if kind == 'Physical':
                inp = event['input']; fields(inp, 'id epoch raw_space event outcome')
                id_ = sample(inp['id']); wide(inp['epoch']); wide(inp['raw_space'])
                fields(inp['event'], 'raw_state situation action')
                natural(inp['event']['raw_state']); natural(inp['event']['situation'])
                natural(inp['event']['action'], 255)
                c = context(event['projected_context']); v = SENSORY.value(inp['outcome'])
                require(inp['outcome']['kind'] == 'Outcome', 'Physical outcome has wrong sort')
                require(pa['source'] is not None and pa['origin'] is not None, 'Missing source binding')
                require(pa['source']['raw_space'] == inp['raw_space'] and
                        pa['source']['owner'] == inp['id']['lineage'], 'Foreign physical source')
                require(event['projected_context']['action'] == inp['event']['action'], 'Changed physical action')
                bound = {**pb, 'source': pa['source'], 'origin': pa['origin']}; encoded_bound = profile(bound)
                claim(f'bind {b} {SENSORY.source(pa["source"])} {origin(pa["origin"])} = .ok {encoded_bound}')
                call = f'observe {MAX64} {natural(before["capacity"])} {natural(pa["identity"])} {encoded_bound} {id_} {c} {v}'
                if pa['first_unretained'] is None:
                    claim(f'resultAgrees ({call}) {a} = true')
                    novel = not any(row['context'] == event['projected_context'] and inp['outcome'] in row['values']
                                    for row in pb['support'])
                    require((pa['identity'] != pb['identity']) == novel, 'Novelty owner was reused or spuriously replaced')
                else:
                    expected = {**bound, 'first_unretained': inp['id']}
                    claim(f'({a} : Profile) = {profile(expected)}')
                    unchecked['PhysicalRetentionCause'] += 1
                unchecked['CutLayerProjection'] += 1
            else:
                claim(f'({b} : Profile) = {a}')
                if kind == 'Service':
                    natural(event['grant']); natural(event['remaining'], event['grant'])
                    result = event['result']; require(type(result) is dict, 'Missing service result')
                    if result.get('kind') == 'Profile':
                        fields(result, 'kind reason'); tag(result['reason'], REFUSALS)
                    else:
                        fields(result, 'kind'); require(result['kind'] in ('Ok', 'OtherOpeningStop'), 'Unknown service result')
                    unchecked['AgendaServiceResult'] += 1
                else:
                    for byte in sequence(event['bytes']): natural(byte, 255)
                    require(type(event['decoded']) is dict, 'Missing decoded import'); unchecked['ImportSemantics'] += 1
            owners[owner] = after; checked[kind] += 1
        elif kind in ('Preflight', 'Commit'):
            has_input, refused = 'input' in event, 'refusal' in event
            fields(event, 'kind name before after' + (' input' if has_input else '') + (' refusal' if refused else ''))
            b, a = profile(event['before']), profile(event['after'])
            if refused:
                reason = tag(event['refusal'], REFUSALS)
                claim(f'({b} : Profile) = {a}')
            if kind == 'Preflight':
                require(has_input and refused, 'Expected recorded preflight refusal')
                inp = event['input']; fields(inp, 'id context outcome capacity')
                claim(f'prepare {MAX64} {natural(inp["capacity"])} {b} {sample(inp["id"])} '
                      f'{context(inp["context"])} {SENSORY.value(inp["outcome"])} = .error {reason}')
                checked['PreflightRefusal'] += 1
            else:
                if has_input:
                    inp = event['input']; fields(inp, 'id context outcome')
                    claim(f'resultAgrees (observe {MAX64} {MAX64} {natural(event["after"]["identity"])} '
                          f'{b} {sample(inp["id"])} {context(inp["context"])} {SENSORY.value(inp["outcome"])}) {a} = true')
                    checked['CommitPayloadTransition'] += 1
                else: require(refused, 'Commit lacks payload and refusal')
                unchecked['PreparedTokenAuthorization'] += 1
        elif kind == 'Materialize':
            fields(event, 'kind name grant remaining capacity before after complete' + (' refusal' if 'refusal' in event else ''))
            b, a = event['before'], event['after']; materializer(b); materializer(a)
            grant, remaining = event['grant'], event['remaining']
            natural(grant); natural(remaining, grant); natural(event['capacity'])
            require(a['processed'] == b['processed'] + grant - remaining, 'Unpaid materializer progress')
            if grant == remaining: require(a == b, 'Unpaid materializer state change')
            if event['complete'] is not None:
                boolean(event['complete']); require(event['complete'] == (not a['pending']), 'Wrong completion flag')
            if event.get('refusal') is not None: tag(event['refusal'], REFUSALS)
            checked['MaterializerAccounting'] += 1; unchecked['MaterializerSnapshotTraversal'] += 1
        else: raise ValueError('Unknown calibration event')
    require(claims, 'No calibration claims')
    lines += ['#print axioms ' + name for name in claims] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', dict(claims=len(claims), checked=dict(checked), unchecked=dict(unchecked),
        supplied_initial_agendas=len(owners), scope=__doc__)


def main():
    replay_main(emit=emit, modules=MODELS, producers=PRODUCERS, description=__doc__, scope=__doc__)


if __name__ == '__main__': main()
