"""Check rustdoc's projection against witnesses compiled in the executable cfg.

Rustdoc implicitly enables cfg(doc). It is not the authority for native field
order or exhaustive enum coverage. The compiler checks a generated, complete
record reconstruction and exhaustive matches against the unmodified types.
The witness is never executed and supplies no assumed behavioral contract.
"""
import re
from mir_to_lean import require, function_body, basic_blocks

RECORD_WITNESS = 'verumNativeRecordLayout'


def witness_source(layout):
    """Only names are suggested by rustdoc; the native compiler must resolve them."""
    record = layout.record
    fields = ', '.join(f'{field.name}: value.{field.name}' for field in layout.fields)
    source = f'\n#[allow(non_snake_case, dead_code)]\nfn {RECORD_WITNESS}(value: {record}) -> {record} {{\n    {record} {{ {fields} }}\n}}\n'
    for i, (kind, variants) in enumerate(layout.enums.items()):
        arms = ', '.join(f'{kind}::{name} => {j}usize' for j, name in enumerate(variants))
        source += f'\n#[allow(non_snake_case, dead_code)]\nfn verumNativeEnumLayout{i}(value: {kind}) -> usize {{\n    match value {{ {arms} }}\n}}\n'
    return source


def validate_layout(mir, layout):
    """MIR field numbers/types must agree with the named complete reconstruction.

The caller must compile witness_source(layout) appended to the actual source,
using exactly the native MIR options, and retain both inputs and outputs.
Compilation also enforces exhaustiveness of the generated enum matches.
Missing witnesses and unexpected lowering fail closed.
"""
    parameters, body = function_body(mir, RECORD_WITNESS, layout.record)
    require(parameters == '_1: ' + layout.record, 'Wrong native layout witness parameter')
    declarations, blocks = basic_blocks(body)
    require(set(blocks) == {'bb0'}, 'Unexpected layout witness control flow')
    types = {'_1': layout.record}
    for line in declarations.splitlines():
        line = line.strip()
        if not line or line == 'debug value => _1;': continue
        local = re.fullmatch(r'let (?:mut )?(_\d+): (.+);', line)
        require(local is not None and local[1] not in types, 'Unexpected layout witness local')
        types[local[1]] = local[2]
    require(types.get('_0') == layout.record, 'Wrong layout witness return')
    lines = blocks['bb0']
    require(len(lines) == len(layout.fields) + 2 and lines[-1] == 'return;', 'Incomplete native field witness')
    fields = {}
    for line in lines[:-2]:
        read = re.fullmatch(r'(_\d+) = (?:copy|move) \(_1\.(\d+): (.+)\);', line)
        require(read is not None and read[1] not in fields and read[1] not in ('_0', '_1')
                and types.get(read[1]) == read[3], 'Invalid native field witness')
        fields[read[1]] = int(read[2]), read[3]
    aggregate = re.fullmatch(r'_0 = ' + re.escape(layout.record) + r' \{ (.+) \};', lines[-2])
    require(aggregate is not None, 'Missing exhaustive native reconstruction')
    entries = aggregate[1].split(', ')
    require(len(entries) == len(layout.fields), 'Native field count differs from metadata')
    used = set()
    for i, (entry, field) in enumerate(zip(entries, layout.fields)):
        named = re.fullmatch(r'([A-Za-z][A-Za-z0-9_]*): (?:copy|move) (_\d+)', entry)
        require(named is not None and named[1] == field.name and named[2] in fields
                and named[2] not in used, 'Native field identity/order differs from metadata')
        used.add(named[2])
        index, kind = fields[named[2]]
        require(index == i, 'Native field index differs from metadata')
        require(field.kind is None or kind == field.kind, 'Native field type differs from metadata')
    require(used == set(fields) and set(types) == used | {'_0', '_1'}, 'Unaccounted native witness locals')
    for i, kind in enumerate(layout.enums):
        params, _ = function_body(mir, f'verumNativeEnumLayout{i}', 'usize')
        require(params == '_1: ' + kind, 'Missing exhaustive native enum witness')
