#!/usr/bin/env python3
"""Replay finite signature catalogue transitions against an independent Lean model.

Checks source/semantic/phase selection, nominal body references, exact catalogue
order, work/capacity pauses and constructor catalogue ownership. Other lifecycle
events are explicitly counted as unchecked. This is neither VM execution replay
nor universal implementation refinement or empirical admission verification.
"""
from pathlib import Path
from collections import Counter
import json
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, variant, module_closure, main as replay_main)
from acquired_export_codec import SENSORY, OPS, POSE_OPS, option, origin
import acquired_export_codec

MODELS = module_closure(['SignatureCatalogues'])
PRODUCERS = [Path(__file__).resolve(), Path(acquired_export_codec.__file__).resolve()]


def triple(items, encode):
    require(len(sequence(items)) == 3, 'Exactly three ordered phases required')
    return '⟨' + ', '.join(encode(item) for item in items) + '⟩'


def program(item, depth=0):
    """Validate complete syntax without lowering control flow to eager code."""
    require(depth <= 64, 'Program exceeds extraction depth')
    fields(item, 'steps output library')
    option(item['output'], natural)
    for step in sequence(item['steps']):
        fields(step, 'operation arguments')
        kind, payload = variant(step['operation'])
        if kind == 'Primitive':
            require(type(payload) is str and payload in OPS + POSE_OPS + ['EventSensory', 'QuerySensory'],
                    'Unknown acquired primitive')
        elif kind == 'Branch':
            fields(payload, 'yes no'); natural(payload['yes']); natural(payload['no'])
        else:
            require(kind in ('Call', 'Fold'), 'Unknown control operation'); natural(payload)
        for operand in sequence(step['arguments']):
            kind, payload = variant(operand)
            if kind == 'Literal': SENSORY.value(payload)
            else:
                require(kind in ('Input', 'Register'), 'Unknown operand'); natural(payload)
    for child in sequence(item['library']): program(child, depth + 1)


def table(rows, expected):
    result = {}
    for row in sequence(rows):
        fields(row, expected); natural(row['identity'])
        require(row['identity'] not in result, 'Duplicate nominal identity')
        result[row['identity']] = row
    return result


def emit(data):
    fields(data, 'schema scope identity_scope catalogue_mapping definitions signatures bodies events')
    require(data['schema'] == 'synarc-acquired-signature-lifecycle-v1', 'Unknown signature schema')
    definitions = table(data['definitions'],
        'identity semantics initial update read update_binding read_binding')
    bodies = table(data['bodies'], 'identity program')
    signatures = table(data['signatures'],
        'identity version parent dependency validated_owner definition origin parts candidate definition_receipt initialization')
    require(definitions and bodies and signatures, 'Missing native owner tables')
    for body in bodies.values(): program(body['program'])
    for d in definitions.values():
        fields(d['semantics'], 'kind max_trace')
        expected = 'AcquiredV3' if d['read_binding']['interface'].get('sensory') is True else 'AcquiredV2'
        require(d['semantics']['kind'] == expected, 'Semantic adapter/source mismatch')
        natural(d['semantics']['max_trace']); SENSORY.value(d['initial'])
        for phase in ('update', 'read'):
            SENSORY.binding(d[phase + '_binding']); program(d[phase])

    def signature_ref(identity):
        natural(identity); require(identity in signatures, 'Unknown signature owner')
        return str(identity)

    def body_ref(identity):
        natural(identity); require(identity in bodies, 'Unknown body owner')
        return str(identity)

    entries = []
    for s in signatures.values():
        natural(s['version']); natural(s['validated_owner']); origin(s['origin'])
        option(s['parent'], signature_ref); option(s['dependency'], signature_ref)
        natural(s['definition'])
        require(s['definition'] in definitions, 'Unknown definition owner')
        d = definitions[s['definition']]
        parts = triple(s['parts'], body_ref)
        for phase, index in [('update', 1), ('read', 2)]:
            require(bodies[s['parts'][index]]['program'] == d[phase],
                    'Retained body does not match its definition')
        # Equal printed programs do not merge nominal owners. The node's actual
        # parent and retained part IDs select the catalogue entries below.
        def slots(binding):
            return array('.state' if slot == 'State' else '(.external ' + natural(slot['External']) + ')'
                         for slot in binding['slots'])
        u, r = d['update_binding'], d['read_binding']
        entries.append('⟨' + ', '.join([natural(s['identity']), natural(s['version']),
            option(s['parent'], signature_ref), parts, SENSORY.source(u['interface']),
            SENSORY.source(r['interface']), natural(d['semantics']['max_trace']),
            natural(u['width']), natural(r['width']), slots(u), slots(r)]) + '⟩')
        seen = set(); at = s['identity']
        while at is not None:
            require(at not in seen, 'Cyclic signature parent'); seen.add(at)
            current = signatures[at]
            parent = current['parent']
            if parent is not None:
                require(current['version'] == signatures[parent]['version'] + 1,
                        'Signature version does not follow its parent')
            else: require(current['version'] == 1, 'Initial signature version must be one')
            at = parent

    lines = ['import VerumExternalReplay.SignatureCatalogues',
             'open SignatureCatalogues', 'set_option warningAsError true',
             'namespace FrozenObserverExport', 'def entries : List Entry := ' + array(entries)]
    claims = []
    def claim(equation, proof='decide_cbv'):
        name = 'c' + str(len(claims)); claims.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')

    def selection(s):
        fields(s, 'version included exclusion'); natural(s['version'])
        require(s['exclusion'] in (None, 'Source', 'Semantics', 'Ports'), 'Unknown exclusion')
        return '⟨' + natural(s['version']) + ', ' + triple(s['included'], boolean) + ', ' + option(
            s['exclusion'], lambda value: '.' + value.lower()) + '⟩'

    def library(state):
        fields(state, 'next pending phase_order calls frozen catalogue_versions cells taken selections')
        require(state['phase_order'] == ['initial', 'update', 'read'], 'Phase order changed')
        def pending(p):
            fields(p, 'signature part included parts'); natural(p['part'], 3)
            require(p['parts'] == signatures[int(signature_ref(p['signature']))]['parts'],
                    'Pending body owner changed')
            return '⟨' + ', '.join([signature_ref(p['signature']), natural(p['part']),
                triple(p['included'], boolean), triple(p['parts'], body_ref)]) + '⟩'
        def calls(rows, indexed):
            result = []
            for index, row in enumerate(sequence(rows)):
                fields(row, ('index ' if indexed else '') + 'arity body')
                if indexed:
                    natural(row['index']); require(row['index'] == index, 'Noncontiguous catalogue index')
                result.append('⟨' + natural(row['arity']) + ', ' + body_ref(row['body']) + '⟩')
            return array(result)
        selections = array(selection(s) for s in sequence(state['selections']))
        versions = triple(state['catalogue_versions'], lambda xs: array(natural(v) for v in sequence(xs)))
        claim(f'({versions} : Triple (List Nat)) = ⟨' + ', '.join(
            f'({selections} : List Selection).filterMap (fun s => if s.included.get {phase} then some s.version else none)'
            for phase in range(3)) + '⟩')
        return '⟨' + ', '.join([option(state['next'], signature_ref), option(state['pending'], pending),
            triple(state['calls'], lambda xs: calls(xs, True)),
            option(state['frozen'], lambda xs: triple(xs, lambda rows: calls(rows, False))),
            natural(state['cells']), boolean(state['taken']), selections]) + '⟩'

    errors = {'library catalogue already handed off': 'taken',
        'signature catalogue retention capacity': 'retention',
        'signature call retention capacity': 'callRetention',
        'signature selection retention capacity': 'selectionRetention',
        'signature frozen catalogue capacity': 'frozenRetention'}
    checked, unchecked = Counter(), Counter()
    retained = {}
    key = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':'))
    for event in sequence(data['events']):
        require(type(event) is dict and type(event.get('name')) is str and event['name'], 'Missing event name')
        kind = event.get('kind')
        if kind == 'LibraryAdvance':
            fields(event, 'name kind input before after result work_before work_after')
            config = event['input']; fields(config, 'source max_trace predictive capacity')
            natural(event['work_before'], 100000); natural(event['work_after'], event['work_before'])
            context = '⟨' + ', '.join([SENSORY.source(config['source']), natural(config['max_trace']), boolean(config['predictive'])]) + '⟩'
            result = event['result']
            if result.get('kind') == 'Ok':
                fields(result, 'kind complete'); outcome = '.ok ' + boolean(result['complete'])
            else:
                fields(result, 'kind reason'); require(result['kind'] == 'Err' and result['reason'] in errors,
                    'Unmodeled library failure; allocation and capacity are distinct')
                outcome = '.error .' + errors[result['reason']]
            before, after = library(event['before']), library(event['after'])
            before_key = key(event['before'])
            raw = event['before']
            if raw['next'] is not None and raw['pending'] is None and not raw['selections']:
                claim(f'({before} : Library) = Library.new {signature_ref(raw["next"])}')
            else:
                # Capacity may be raised on retry; source, semantics and ports
                # remain those of the retained consumer declaration.
                declaration = {k: v for k, v in config.items() if k != 'capacity'}
                require(key(declaration) in retained.get(before_key, set()),
                        'Unobserved or rebound retained catalogue')
            claim(f'advance entries {context} {natural(config["capacity"])} {natural(event["work_before"])} '
                  f'{before} = ({after}, {natural(event["work_after"])}, {outcome})')
            declaration = {k: v for k, v in config.items() if k != 'capacity'}
            retained.setdefault(key(event['after']), set()).add(key(declaration))
            checked[kind] += 1
        elif kind == 'Constructor':
            fields(event, 'name kind input before after result')
            fields(event['input'], 'source max_trace depth bounds')
            SENSORY.source(event['input']['source']); natural(event['input']['max_trace'])
            natural(event['input']['depth'])
            if event['input']['bounds'] is not None:
                fields(event['input']['bounds'], 'nesting body_inputs')
                natural(event['input']['bounds']['nesting']); natural(event['input']['bounds']['body_inputs'])
            declaration = dict(source=event['input']['source'], max_trace=event['input']['max_trace'], predictive=False)
            require(key(declaration) in retained.get(key(event['before']), set()),
                    'Constructor does not own an observed categorical catalogue')
            before, after = library(event['before']), library(event['after'])
            result = event['result']; require(type(result) is dict, 'Missing constructor result')
            if result.get('kind') == 'Err':
                fields(result, 'kind reason'); require(type(result['reason']) is str and result['reason'], 'Missing refusal')
                claim(f'({before} : Library) = {after}')
            else:
                fields(result, 'kind'); require(result['kind'] == 'Ok', 'Unknown constructor result')
                claim(f'({before} : Library).frozen.isSome = true ∧ ({before} : Library).taken = false', 'decide')
                claim(f'({after} : Library) = {{ ({before} : Library) with taken := true }}')
            retained.setdefault(key(event['after']), set()).add(key(declaration))
            checked['ConstructorOwnership'] += 1
        else:
            require(kind in ('Link', 'Publish', 'Observe', 'Queue', 'Snapshot', 'Freeze', 'WorkSummary', 'Execution'),
                    'Unknown lifecycle event')
            unchecked[kind] += 1
    require(checked['LibraryAdvance'] > 0, 'No catalogue transitions')
    lines += ['#print axioms ' + name for name in claims] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', dict(claims=len(claims), checked=dict(checked),
        unchecked=dict(unchecked), signatures=len(signatures), bodies=len(bodies))


def main():
    replay_main(emit=emit, modules=MODELS, producers=PRODUCERS,
        description=__doc__, scope=__doc__)


if __name__ == '__main__': main()
