"""Strict extraction of actual structural grammar/address/native codec witnesses."""
import json
import check_lifecycle_export as life
from observer_export_replay import array, fields, natural, require, sequence


def word(value):
    return array('true' if int(natural(bit, 1)) else 'false' for bit in sequence(value))


def address(value):
    fields(value, 'owner instructions output')
    rows = []
    for row in sequence(value['instructions']):
        fields(row, 'alternative operands')
        rows.append('⟨' + natural(row['alternative']) + ', ' + array(natural(n) for n in sequence(row['operands'])) + '⟩')
    return '⟨' + natural(value['owner']) + ', ' + array(rows) + ', ' + natural(value['output']) + '⟩'


def grammar(value):
    fields(value, 'owner inputs extent operations literals calls')
    extent = value['extent']
    require(type(extent) is dict, 'Expected structural extent')
    if extent.get('kind') == 'Finite':
        fields(extent, 'kind steps'); limit = '(some ' + natural(extent['steps']) + ')'
    else:
        fields(extent, 'kind'); require(extent['kind'] == 'AllFinite', 'Unknown structural extent'); limit = 'none'
    operations = []
    for row in sequence(value['operations']):
        fields(row, 'primitive arity')
        operations.append('(' + life.tag(row['primitive'], life.OPS + life.POSE_OPS) + ', ' + natural(row['arity']) + ')')
    literals = array(life.value(v) for v in sequence(value['literals']))
    calls = []
    for row in sequence(value['calls']):
        fields(row, 'arity program'); calls.append('(' + natural(row['arity']) + ', ' + life.code(row['program']) + ')')
    return '⟨' + ', '.join((natural(value['owner']), natural(value['inputs']), limit,
                            array(operations), literals, array(calls))) + '⟩'


def emit_witnesses(data, lines, claim):
    candidate = data['candidate']
    witnesses = candidate['structural_witnesses']
    if candidate['grammar']['source_order'] == 'legacy-indexed-v1':
        require(witnesses is None, 'Indexed source cannot carry structural witnesses')
        return 0
    require(candidate['grammar']['source_order'] == 'structural-odometer-v1', 'Unknown structural source')
    require(len(sequence(witnesses)) == 3, 'Missing structural witnesses')
    programs = [data['initialization']['program'], data['definition']['update'], data['definition']['read']]
    # External row width is not program arity: binding slots also insert state.
    widths = [0, len(data['definition']['update_binding']['slots']),
              len(data['definition']['read_binding']['slots'])]
    owners = {}
    for i, witness in enumerate(witnesses):
        fields(witness, 'language address word decoded_address consumed_bits materialized decoded_materialized audit')
        g, original, decoded = witness['language'], witness['address'], witness['decoded_address']
        encoded_grammar = grammar(g)
        name = 'structural' + str(i)
        lines += [f'def {name}_grammar : StructuralNames.Grammar Op Value := {encoded_grammar}',
                  f'def {name}_address : StructuralNames.Address := {address(original)}',
                  f'def {name}_decoded : StructuralNames.Address := {address(decoded)}',
                  f'def {name}_word : List Bool := {word(witness["word"])}']
        prior = owners.setdefault(g['owner'], g)
        # Same nominal owner means the SAME complete immutable catalogue.
        literal = lambda v: json.dumps(json.dumps(v, sort_keys=True, separators=(',', ':')))
        claim(f'({literal(prior)} : String) = {literal(g)}')
        claim(f'({natural(g["inputs"])} : Nat) = {natural(widths[i])}')
        claim(f'(0 : Nat) < {len(g["operations"]) + len(g["calls"])}', 'decide')
        ops = array(life.tag(o['primitive'], life.OPS + life.POSE_OPS) for o in g['operations'])
        vals = array(life.value(v) for v in g['literals'])
        claim(f'({ops} : List Op).Nodup', 'decide')
        claim(f'({vals} : List Value).Nodup', 'decide')
        claim(f'{name}_word = ({word(candidate["structural_code_words"][i])} : List Bool)')
        claim(f'({natural(witness["consumed_bits"])} : Nat) = {name}_word.length')
        # The original address and both Programs come from independent native
        # objects. Decode/encode/materialize are recomputed inside the kernel.
        claim(f'StructuralNames.decode {name}_grammar {name}_word = some {name}_address')
        claim(f'StructuralNames.decode {name}_grammar {name}_word = some {name}_decoded')
        claim(f'StructuralNames.encode {name}_grammar {name}_address = some {name}_word')
        claim(f'{name}_decoded = {name}_address')
        claim(f'StructuralNames.materialize {name}_grammar {name}_address = some {life.code(witness["materialized"])}')
        claim(f'StructuralNames.materialize {name}_grammar {name}_decoded = some {life.code(witness["decoded_materialized"])}')
        claim(f'({life.code(witness["materialized"])} : FiniteCode.Code Op Value) = {life.code(programs[i])}')
        claim(f'({life.code(witness["decoded_materialized"])} : FiniteCode.Code Op Value) = {life.code(programs[i])}')
        audit = witness['audit']; fields(audit, 'allowance decode_work materialize_work decoded_materialize_work')
        for k in ('decode_work', 'materialize_work', 'decoded_materialize_work'):
            claim(f'(0 : Nat) < {natural(audit[k])} ∧ {natural(audit[k])} ≤ {natural(audit["allowance"])}', 'decide')
    return len(witnesses)
