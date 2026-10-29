#!/usr/bin/env python3
"""Finite correspondence of exported reservation and ordered sample projections.

Checks the observed source/origin/end projection, reserved/native-paid ordinal,
unchanged rows during materialization, and the selected-prefix/unscored-ID split.
Missing signatures, grammar addresses, criteria, capacity/refusal events, cut
projection inputs and actual validation state remain unchecked. Per-row source
is an exporter projection, not independent evidence of the retained cut owner.
"""
from pathlib import Path
import json
import acquired_export_codec
from acquired_export_codec import SENSORY, lineage, wide
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, module_closure, main as replay_main)

MODELS = module_closure(['PredeclaredEvidence'])
PRODUCERS = [Path(__file__).resolve(), Path(acquired_export_codec.__file__).resolve()]


def literal(value):
    return json.dumps(json.dumps(value, sort_keys=True, separators=(',', ':')))


def sample(value):
    fields(value, 'lineage ordinal'); lineage(value['lineage']); natural(value['ordinal'])


def emit(data):
    events = sequence(data)
    require(len(events) == 5, 'Expected all five declaration lifecycle snapshots')
    kinds = ['Declared', 'Buffered', 'OtherReservations', 'Opened', 'TrainingBoundary']
    lines = ['import VerumExternalReplay.PredeclaredEvidence',
        'open PredeclaredEvidence', 'set_option warningAsError true', 'set_option maxRecDepth 8192', 'namespace FrozenObserverExport']
    claims = []
    def claim(equation):
        name = 'c'+str(len(claims)); claims.append(name)
        lines.append(f'theorem {name} : {equation} := by decide_cbv')
    for index, (event, expected) in enumerate(zip(events, kinds)):
        fields(event, 'kind source origin declaration_end reserved_look paid_look next_reservation rows unscored validation_selected selected_training')
        require(event['kind'] == expected, 'Unexpected declaration lifecycle order')
        SENSORY.source(event['source']); fields(event['origin'], 'epoch lineage ordinal')
        wide(event['origin']['epoch']); lineage(event['origin']['lineage']); natural(event['origin']['ordinal'])
        sample(event['declaration_end'])
        reserved, next_ = wide(event['reserved_look']), wide(event['next_reservation'])
        if event['paid_look'] is not None: wide(event['paid_look'])
        boolean(event['validation_selected'])
        require(event['validation_selected'] is False, 'Actual validation evidence is unsupported by this export')
        owner = {key:event[key] for key in ('source','origin','declaration_end')}
        rows = sequence(event['rows'])
        for row in rows:
            fields(row, 'id source origin context outcome'); sample(row['id']); SENSORY.source(row['source'])
            natural(row['origin']); fields(row['context'], 'epoch input action')
            wide(row['context']['epoch']); natural(row['context']['input']); natural(row['context']['action'],255)
            SENSORY.value(row['outcome']); require(row['outcome']['kind'] == 'Outcome', 'Wrong row outcome sort')
        lines += [f'def owner{index} : String := {literal(owner)}',
            f'def rows{index} : List String := {array(literal(row) for row in rows)}',
            f'def buffer{index} : Buffer String String := (({reserved}, owner{index}), rows{index})']
        if index:
            claim(f'owner{index} = owner0')
            claim(f'({reserved} : Nat) = {wide(events[0]["reserved_look"])}')
        if index < 3:
            require(event['paid_look'] is None, 'Reservation must precede native paid admission')
        else:
            require(event['paid_look'] is not None, 'Missing native paid ordinal')
            claim(f'({wide(event["paid_look"])} : Nat) = {reserved}')
        if index < 4:
            require(event['selected_training'] is None and event['unscored'] is None, 'Premature training split')
        claim(f'({reserved} : Nat) < {next_}')

    declared, buffered, later, opened, boundary = events
    require(not declared['rows'], 'Declaration already includes trial rows')
    claim(f'reserve {wide(declared["reserved_look"])} owner0 = ({wide(declared["next_reservation"])}, buffer0.1)')
    claim(f'({wide(buffered["next_reservation"])} : Nat) = {wide(declared["next_reservation"])}')
    claim('appendBatch buffer0 rows1 = buffer1')
    claim('buffer2 = buffer1')
    claim(f'({wide(buffered["next_reservation"])} : Nat) ≤ {wide(later["next_reservation"])}')
    claim('buffer3 = buffer2')
    claim(f'({wide(opened["next_reservation"])} : Nat) = {wide(later["next_reservation"])}')
    claim(f'({wide(boundary["next_reservation"])} : Nat) = {wide(opened["next_reservation"])}')

    # Check every actual retained row against the frozen projection, and its
    # source order. This cannot establish the omitted cut-layer computation.
    rows = buffered['rows']
    for field, expected, render, carrier in [
        ('source', declared['source'], literal, 'String'),
        ('origin', declared['origin']['ordinal'], natural, 'Nat'),
    ]:
        claim(f'({array(render(row[field]) for row in rows)} : List {carrier}) = '
              f'List.replicate rows1.length {render(expected)}')
    claim(f'({array(literal(row["id"]["lineage"]) for row in rows)} : List String) = '
          f'List.replicate rows1.length {literal(declared["declaration_end"]["lineage"])}')
    claim(f'({array(wide(row["context"]["epoch"]) for row in rows)} : List Nat) = '
          f'List.replicate rows1.length {wide(declared["source"]["epoch"])}')
    ordinals = [row['id']['ordinal'] for row in rows]
    previous = [declared['declaration_end']['ordinal']]+ordinals
    adjacent = array(f'({natural(before)}, {natural(after)})' for before,after in zip(previous,ordinals))
    claim(f'({adjacent} : List (Nat × Nat)).all (fun pair => decide (pair.1 < pair.2)) = true')
    selected = natural(boundary['selected_training'])
    unscored = sequence(boundary['unscored'])
    for id_ in unscored: sample(id_)
    claim(f'({selected} : Nat) ≤ rows3.length')
    claim(f'rows4 = rows3.take {selected}')
    claim(f'rows4.length = {selected}')
    original_ids = array(literal(row['id']) for row in opened['rows'])
    claim(f'({array(literal(id_) for id_ in unscored)} : List String) = ({original_ids} : List String).drop {selected}')
    lines += ['#print axioms '+name for name in claims]+['end FrozenObserverExport']
    return '\n\n'.join(lines)+'\n', dict(claims=len(claims), snapshots=5,
        buffered_rows=len(buffered['rows']), selected_rows=boundary['selected_training'], unscored_ids=len(unscored),
        unchecked=['signature/family/criterion freeze', 'capacity/refusal transitions',
            'independent cut source and context projection', 'other reservation identities',
            'actual candidate selection and fresh validation', 'native affine ownership and whole-runtime refinement'],
        scope=__doc__)


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=PRODUCERS, scope=__doc__, description=__doc__)
