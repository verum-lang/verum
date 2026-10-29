#!/usr/bin/env python3
"""Replay declared-support KT scoring of supplied native common-window readings.

Program debug strings and query/history metadata are not a VM semantics export.
The packet supplies shared-window identity; this adapter does not reconstruct
allocation identity, prove preregistration, or certify admission or the native
price-weighted Comparison stage. Price fields are checked for internal coherence.
"""
from pathlib import Path
import re
from observer_export_replay import (array, boolean, fields, module_closure,
    natural, require, sequence, main as replay_main)

MODULES = module_closure(['CommonWindow'])
SCOPE = (__doc__.split('\n\n')[0] + ' No VM/read causality, preregistration, admission, '
         'or native weighted-comparison claim; original price fields checked for coherence only.')
MASS_FIELDS = ('parent_kt old_child_kt new_child_kt old_native_compression_ratio '
               'new_native_compression_ratio child_ratio').split()


def decimal(value):
    require(type(value) is str and len(value) <= 4096 and
            re.fullmatch(r'0|[1-9][0-9]*', value), 'Expected canonical natural decimal')
    return value


def positive_decimal(value):
    value = decimal(value)
    require(value != '0', 'Expected positive denominator')
    return value


def fraction(value):
    fields(value, 'numerator denominator')
    return f'(⟨{decimal(value["numerator"])},{positive_decimal(value["denominator"])},by decide⟩ : Fraction)'


def support(value):
    require(type(value) is dict and value, 'Missing declared support')
    result = {}
    for context, labels in value.items():
        context = int(decimal(context)); natural(context, 255)
        labels = sequence(labels)
        for label in labels: natural(label, 255)
        require(labels and len(set(labels)) == len(labels), 'Empty or duplicated alphabet')
        result[context] = labels
    return result


def emit(data):
    fields(data, 'schema descriptive_only window_identity declaration_identity same_window_owner '
        'support old_training_support new_training_support ordered_rows old_definition new_definition '
        'old_original_price new_original_price ' + ' '.join(MASS_FIELDS))
    require(data['schema'] == 'synarc-common-window-v1', 'Unsupported common-window export schema')
    natural(data['window_identity']); natural(data['declaration_identity'])
    declared = support(data['support'])
    training = [support(data[name + '_training_support']) for name in ('old', 'new')]
    rows = sequence(data['ordered_rows'])
    definitions = []; claims = [boolean(data['descriptive_only']), boolean(data['same_window_owner'])]
    claims.append(f'decide (0 < {len(rows)})')
    for row in rows:
        fields(row, 'origin ordinal cut context outcome query_interface query_values old_reading new_reading')
        for name in ('origin', 'context', 'outcome', 'query_interface'): natural(row[name], 255)
        for name in ('ordinal', 'cut', 'old_reading', 'new_reading'): natural(row[name])
        for value in sequence(row['query_values']): natural(value)
    # These identify supplied observations; cuts need not be monotone and can
    # revisit a retained history. Their program/query correspondence is separate.
    identities = array(f'({r["origin"]},{r["ordinal"]})' for r in rows)
    claims.append(f'decide (({identities} : List (Nat × Nat)).Nodup)')
    contexts = array(natural(c) for c in declared)
    claims.append(f'({array(natural(r["context"]) for r in rows)} : List Nat).all '
                  f'(fun c => ({contexts} : List Nat).contains c)')
    for table in training:
        claims.append(f'({array(natural(c) for c in table)} : List Nat).all '
                      f'(fun c => ({contexts} : List Nat).contains c)')
    for side in ('old', 'new'):
        definition = data[side + '_definition']; fields(definition, 'initial update read')
        natural(definition['initial'])
        for name in ('update', 'read'):
            require(type(definition[name]) is str, 'Expected program debug description')
    parts = {name: [] for name in ('parent', 'old', 'new')}
    for index, (context, labels) in enumerate(sorted(declared.items())):
        alphabet = f'a{index}'
        definitions.append(f'def {alphabet} : Alphabet Nat := '
            f'⟨{array(natural(y) for y in labels)},by decide_cbv,by decide⟩')
        # The producer declares the union of the original supports. Include
        # unseen labels; growing support from the scored window is not allowed.
        original = training[0].get(context, []) + training[1].get(context, [])
        originals = f'({array(natural(y) for y in original)} : List Nat)'
        claims += [f'{alphabet}.labels.all (fun y => {originals}.contains y)',
                   f'{originals}.all (fun y => {alphabet}.labels.contains y)']
        local = [row for row in rows if row['context'] == context]
        for side in parts:
            cells = array(f'({0 if side == "parent" else row[side + "_reading"]},{row["outcome"]})'
                          for row in local)
            name = f'{side}{index}'
            definitions.append(f'def {name} : Option Fraction := '
                f'recordedMass {alphabet} ({cells} : List (Nat × Nat))')
            parts[side].append(name)
    definitions.append('def combined (parts : List (Option Fraction)) : Option Fraction :=\n'
        '  parts.foldlM (fun accumulated part => part.map (mul accumulated)) one')
    definitions.append('def agrees (actual : Option Fraction) (expected : Fraction) : Bool :=\n'
        '  match actual with | none => false | some mass => decide (Equal mass expected)')
    for name in MASS_FIELDS:
        definitions.append(f'def {name} : Fraction := {fraction(data[name])}')
    for side in parts:
        name = 'parent_kt' if side == 'parent' else side + '_child_kt'
        claims.append(f'agrees (combined {array(parts[side])}) {name}')
        claims.append(f'decide (0 < {name}.numerator)')
    for side in ('old', 'new'):
        claims.append(f'decide (Equal (mul {side}_native_compression_ratio parent_kt) {side}_child_kt)')
        price = data[side + '_original_price']
        fields(price, 'authority mass_denominator name_denominator paid_look')
        name = positive_decimal(price['name_denominator'])
        mass = positive_decimal(price['mass_denominator'])
        if price['authority'] == 'ExplicitUniformSingleUse':
            require(price['paid_look'] is None, 'Single-use price cannot invent a paid look')
            look = '1'
        else:
            require(price['authority'] == 'JointPaidLook', 'Unknown original price authority')
            paid = decimal(price['paid_look'])
            look = f'(({paid}+1)*({paid}+2))'
        definitions.append(f'def {side}_price : Price := '
            f'⟨unit {name} (by decide),unit {look} (by decide),Nat.zero_lt_one,Nat.zero_lt_one⟩')
        claims.append(f'decide (Equal {side}_price.mass (unit {mass} (by decide)))')
    claims.append('decide (Equal (mul child_ratio old_child_kt) new_child_kt)')
    lines = ['import VerumExternalReplay.CommonWindow', 'open ExactMass CommonWindow',
             'set_option warningAsError true', 'set_option maxRecDepth 65536',
             'set_option maxHeartbeats 30000000', 'set_option cbv.maxSteps 10000000',
             'namespace FrozenObserverExport', *definitions]
    for start in range(0, len(claims), 32):
        lines.append(f'theorem c{start} : ({array(claims[start:start+32])} : List Bool).all id = true := by decide_cbv')
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', dict(rows=len(rows), contexts=len(declared),
        conditions=len(claims), claims=(len(claims)+31)//32)


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODULES, producers=[Path(__file__).resolve()],
                scope=SCOPE, description=__doc__)
