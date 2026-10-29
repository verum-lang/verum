#!/usr/bin/env python3
"""Finite correspondence of frozen declarations and selection-frontier snapshots.

Checks declaration identity, native paid ordinals, retained Cut source/origin,
capacity, exact ordered prefixes, independent input/action projections and
preselection refusal versus fresh-row admission. Issued criterion semantics,
exact family cardinality and native affine ownership remain separate obligations. A null
signature records the absence of a signature, never evidence of a present one.
"""
from pathlib import Path
import json
import acquired_export_codec
from acquired_export_codec import SENSORY, lineage, origin, wide
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, module_closure, main as replay_main)

MODELS = module_closure(['PredeclaredEvidence'])
PRODUCERS = [Path(__file__).resolve(), Path(acquired_export_codec.__file__).resolve()]
DECLARATION = 'source origin declaration_end capacity signature address declared_criterion risk_denominator'.split()
SNAPSHOT = ('kind source origin declaration_end reserved_look paid_look next_reservation rows unscored '
    'validation_from collecting_validation selected_training capacity signature address family_count '
    'declared_criterion risk_denominator issued_criterion')
DECLARED = ['Declared', 'Buffered', 'OtherReservations', 'Opened', 'TrainingBoundary']
SELECTED = ['Selected', 'OldRowRefused', 'ActualFreshRow']


def literal(value):
    return json.dumps(json.dumps(value, sort_keys=True, separators=(',', ':')))


def sample(value):
    fields(value, 'lineage ordinal'); lineage(value['lineage']); natural(value['ordinal'])


def context(value):
    fields(value, 'epoch input action')
    wide(value['epoch']); natural(value['input']); natural(value['action'],255)


def row(value):
    fields(value, 'id source origin context outcome actual_cut_epoch selected_input_epoch derived_context')
    sample(value['id']); SENSORY.source(value['source']); origin(value['origin'])
    context(value['context']); context(value['derived_context'])
    wide(value['actual_cut_epoch']); wide(value['selected_input_epoch'])
    SENSORY.value(value['outcome']); require(value['outcome']['kind'] == 'Outcome', 'Wrong row outcome sort')


def snapshot(event):
    require(type(event) is dict, 'Expected snapshot')
    refused = event.get('kind') == 'OldRowRefused'
    fields(event, SNAPSHOT + (' attempted_row observed_result' if refused else ''))
    if refused:
        row(event['attempted_row'])
        result=event['observed_result']; fields(result,'kind reason')
        require(result['kind'] in ('Ok','Err') and type(result['reason']) is str, 'Invalid observed result')
    SENSORY.source(event['source']); origin(event['origin']); sample(event['declaration_end'])
    natural(event['capacity']); wide(event['reserved_look']); wide(event['next_reservation'])
    wide(event['risk_denominator']); boolean(event['collecting_validation'])
    for key, decode in [('paid_look',wide), ('family_count',wide), ('signature',natural),
                        ('selected_training',natural), ('validation_from',sample)]:
        if event[key] is not None: decode(event[key])
    require(event['declared_criterion'] in ('ExactPredictiveV1','ExactResidualPredictiveV1'), 'Unknown criterion')
    require(event['issued_criterion'] is None or type(event['issued_criterion']) is str, 'Expected issued criterion text')
    address = event['address']
    fields(address, 'role depth evidence_round imported composition')
    require(address['role'] in ('Categorical','Predictive'), 'Unknown source role')
    natural(address['depth']); natural(address['evidence_round'])
    if address['imported'] is not None: natural(address['imported'])
    if address['composition'] is not None:
        fields(address['composition'], 'nesting body_inputs')
        natural(address['composition']['nesting']); natural(address['composition']['body_inputs'])
    if event['unscored'] is not None:
        for id_ in sequence(event['unscored']): sample(id_)
    for value in sequence(event['rows']): row(value)


def emit(data):
    events = sequence(data)
    for event in events: snapshot(event)
    kinds = [event['kind'] for event in events]
    require(kinds in (DECLARED, SELECTED), 'Expected complete declaration or selection lifecycle')
    validation = kinds == SELECTED
    lines = ['import VerumExternalReplay.PredeclaredEvidence',
        'open PredeclaredEvidence', 'set_option warningAsError true', 'set_option maxRecDepth 8192',
        'namespace FrozenObserverExport']
    claims = []
    def claim(equation):
        name = 'c'+str(len(claims)); claims.append(name)
        lines.append(f'theorem {name} : {equation} := by decide_cbv')
    def same(field, index, reference=0):
        claim(f'({literal(events[index][field])} : String) = {literal(events[reference][field])}')
    for index, event in enumerate(events):
        reserved, next_ = wide(event['reserved_look']), wide(event['next_reservation'])
        owner = {key:event[key] for key in DECLARATION}
        lines += [f'def owner{index} : String := {literal(owner)}',
            f'def rows{index} : List String := {array(literal(row) for row in event["rows"])}',
            f'def buffer{index} : Buffer String String := (({reserved}, owner{index}), rows{index})']
        if index:
            claim(f'owner{index} = owner0'); same('reserved_look',index)
        claim(f'({reserved} : Nat) < {next_}')
        claim(f'0 < ({wide(event["risk_denominator"])} : Nat)')
        claim(f'rows{index}.length ≤ {natural(event["capacity"])}')
        if event['family_count'] is not None: claim(f'0 < ({wide(event["family_count"])} : Nat)')
        if validation or index >= 3:
            require(event['paid_look'] is not None, 'Missing native paid ordinal')
            claim(f'({wide(event["paid_look"])} : Nat) = {reserved}')
        else:
            require(event['paid_look'] is None and event['family_count'] is None and event['issued_criterion'] is None,
                    'Reservation must precede native paid admission')
        require(event['collecting_validation'] is validation, 'Wrong intake phase')
        if validation:
            require(event['validation_from'] is not None, 'Missing physical selection frontier')
        else:
            require(event['validation_from'] is None, 'Unexpected selection frontier during training')
            if index < 4:
                require(event['selected_training'] is None and event['unscored'] is None, 'Premature training split')

    if validation:
        selected, refused, received = events
        same_state = lambda event: {k:v for k,v in event.items()
            if k not in ('kind','rows','attempted_row','observed_result')}
        for index in (1,2):
            claim(f'({literal(same_state(events[index]))} : String) = {literal(same_state(selected))}')
        claim('rows0 = []'); claim('buffer1 = buffer0')
        require(len(received['rows']) == 1, 'Expected the first actual validation row')
        require(selected['selected_training'] is not None and selected['family_count'] is not None,
                'Missing selected training evidence')
        claim(f'0 < ({natural(selected["selected_training"])} : Nat)')
        frontier = selected['validation_from']
        claim(f'({literal(frontier["lineage"])} : String) = {literal(selected["declaration_end"]["lineage"])}')
        claim(f'({natural(selected["declaration_end"]["ordinal"])} : Nat) < {natural(frontier["ordinal"])}')
        row = received['rows'][0]
        # Encode the whole source + origin + lineage as ownership, so the
        # exact admission theorem cannot silently forget a coordinate.
        owner = lambda source, start, id_: literal(dict(source=source,origin=start,lineage=id_['lineage']))
        lines += [f'def selectedDeclaration : Exact.Declaration String String String String := '
            f'⟨{owner(selected["source"],selected["origin"],selected["declaration_end"])}, '
            f'{natural(selected["origin"]["ordinal"])}, {natural(selected["declaration_end"]["ordinal"])}, '
            f'{literal(selected["signature"])}, {literal(selected["address"])}, '
            f'{literal(dict(criterion=selected["declared_criterion"],risk=selected["risk_denominator"]))}⟩',
            f'def selectedBuffer : Exact.OwnedBuffer String String String String String := '
            f'(({wide(selected["reserved_look"])}, selectedDeclaration), [])']
        ordinal = natural(row['id']['ordinal'])
        claim(f'(Exact.receiveValidation {natural(selected["capacity"])} {natural(frontier["ordinal"])} '
            f'selectedBuffer {owner(row["source"],row["origin"],row["id"])} {ordinal} {literal(row)}).toOption = '
            f'some (appendBatch selectedBuffer [({ordinal}, {literal(row)})])')
        attempted = refused['attempted_row']
        old_ordinal = natural(attempted['id']['ordinal'])
        claim(f'({old_ordinal} : Nat) < {natural(frontier["ordinal"])}')
        claim(f'({literal(refused["observed_result"]["kind"])} : String) = {literal("Err")}')
        claim(f'(Exact.receive {natural(selected["capacity"])} selectedBuffer '
            f'{owner(attempted["source"],attempted["origin"],attempted["id"])} '
            f'{old_ordinal} {literal(attempted)}).toOption = '
            f'some (appendBatch selectedBuffer [({old_ordinal}, {literal(attempted)})])')
        claim(f'(match Exact.receiveValidation {natural(selected["capacity"])} {natural(frontier["ordinal"])} '
            f'selectedBuffer {owner(attempted["source"],attempted["origin"],attempted["id"])} '
            f'{old_ordinal} {literal(attempted)} with '
            f'| .error retained => some retained | .ok _ => none) = some selectedBuffer')
        # Include attempted coordinates in the provenance checks even though
        # they must never become part of the accepted validation prefix.
        checked_rows = received['rows']+[attempted]; row_index = 2
    else:
        declared, buffered, later, opened, boundary = events
        require(not declared['rows'], 'Declaration already includes trial rows')
        require(opened['family_count'] is not None and opened['issued_criterion'] is not None,
                'Missing materialized family or criterion')
        same('family_count',4,3); same('issued_criterion',4,3)
        claim(f'reserve {wide(declared["reserved_look"])} owner0 = ({wide(declared["next_reservation"])}, buffer0.1)')
        same('next_reservation',1)
        claim('appendBatch buffer0 rows1 = buffer1'); claim('buffer2 = buffer1')
        claim(f'({wide(buffered["next_reservation"])} : Nat) ≤ {wide(later["next_reservation"])}')
        claim('buffer3 = buffer2'); same('next_reservation',3,2); same('next_reservation',4,3)
        selected = natural(boundary['selected_training'])
        unscored = sequence(boundary['unscored'])
        claim(f'({selected} : Nat) ≤ rows3.length')
        claim(f'rows4 = rows3.take {selected}'); claim(f'rows4.length = {selected}')
        original_ids = array(literal(row['id']) for row in opened['rows'])
        claim(f'({array(literal(id_) for id_ in unscored)} : List String) = ({original_ids} : List String).drop {selected}')
        checked_rows = buffered['rows']; row_index = 1

    accepted_rows = events[row_index]['rows']
    lines.append(f'def checkedRows : List String := {array(literal(row) for row in checked_rows)}')
    first = events[0]
    for field in ('source','origin'):
        claim(f'({array(literal(row[field]) for row in checked_rows)} : List String) = '
              f'List.replicate checkedRows.length {literal(first[field])}')
    claim(f'({array(literal(row["id"]["lineage"]) for row in checked_rows)} : List String) = '
          f'List.replicate checkedRows.length {literal(first["declaration_end"]["lineage"])}')
    claim(f'({array(wide(row["context"]["epoch"]) for row in checked_rows)} : List Nat) = '
          f'List.replicate checkedRows.length {wide(first["source"]["epoch"])}')
    claim(f'({array(literal(row["context"]) for row in checked_rows)} : List String) = '
          f'{array(literal(row["derived_context"]) for row in checked_rows)}')
    for field in ('actual_cut_epoch','selected_input_epoch'):
        claim(f'({array(wide(row[field]) for row in checked_rows)} : List Nat) = '
              f'List.replicate checkedRows.length {wide(first["source"]["epoch"])}')
    ordinals = [row['id']['ordinal'] for row in accepted_rows]
    previous = [first['declaration_end']['ordinal']]+ordinals
    adjacent = array(f'({natural(before)}, {natural(after)})' for before,after in zip(previous,ordinals))
    claim(f'({adjacent} : List (Nat × Nat)).all (fun pair => decide (pair.1 < pair.2)) = true')
    lines += ['#print axioms '+name for name in claims]+['end FrozenObserverExport']
    return '\n\n'.join(lines)+'\n', dict(claims=len(claims), snapshots=len(events),
        buffered_rows=len(accepted_rows), selected_rows=events[-1]['selected_training'],
        unscored_ids=len(events[-1]['unscored']), validation=validation,
        unchecked=['refusal diagnostic text and cross-epoch replacements',
            'issued criterion semantics and exact family cardinality', 'other reservation identities',
            'native affine ownership and whole-runtime refinement'], scope=__doc__)


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=PRODUCERS, scope=__doc__, description=__doc__)
