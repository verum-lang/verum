"""Typed, exclusive record updates from compiler MIR and rustdoc metadata.

Every record field is retained. Uninterpreted fields are universally quantified
frame values, never replaced by empty containers. Accessing such a field, calls,
additional references, drops, cyclic control flow and unsupported types fail
closed. Panic outcomes retain all preceding writes. Compiler metadata and this
frontend are trusted; generated judgments are independently kernel checked.
"""
from dataclasses import dataclass
import re
from mir_to_lean import Translation, Value, Overflow, require, function_body, basic_blocks, pair


def identifier(name):
    require(type(name) is str and re.fullmatch(r'[A-Za-z][A-Za-z0-9_]*', name), 'Unsupported identifier')
    return name


@dataclass(frozen=True)
class Field:
    name: str
    kind: str | None
    lean: str


class Layout:
    """Field order and nominal enum identities come from the compiler, not a mirror."""
    def __init__(self, metadata, record, function):
        require(metadata['format_version'] == 61, 'Unreviewed rustdoc format')
        self.record = identifier(record)
        index, paths = metadata['index'], metadata['paths']
        candidates = [item for item in index.values() if item['crate_id'] == 0 and
                      item['name'] == record and 'struct' in item['inner']]
        require(len(candidates) == 1, 'Ambiguous record identity')
        item = candidates[0]
        require(paths[str(item['id'])]['path'] == [index[str(metadata['root'])]['name'], record],
                'Only a root-module record is admitted')
        decl = item['inner']['struct']
        require(decl['generics'] == {'params': [], 'where_predicates': []}, 'Generic record unsupported')
        require(set(decl['kind']) == {'plain'} and not decl['kind']['plain']['has_stripped_fields'],
                'Incomplete or unsupported record layout')
        methods = []
        for impl_id in decl['impls']:
            impl = index[str(impl_id)]['inner']['impl']
            if impl['trait'] is not None: continue
            require(impl['for'].get('resolved_path', {}).get('id') == item['id']
                    and impl['generics'] == {'params': [], 'where_predicates': []},
                    'Mismatched or generic inherent implementation')
            for method_id in impl['items']:
                method = index[str(method_id)]
                if method['name'] == function: methods.append(method)
        require(len(methods) == 1, 'Method must belong to exactly one inherent implementation')
        signature = methods[0]['inner']['function']
        require(signature['header'] == {'is_const': False, 'is_unsafe': False, 'is_async': False, 'abi': 'Rust'}
                and signature['generics'] == {'params': [], 'where_predicates': []}
                and signature['has_body'] and not signature['sig']['is_c_variadic']
                and signature['sig']['output'] is None, 'Unsupported method signature')
        inputs = signature['sig']['inputs']
        require(inputs and inputs[0][1] == {'borrowed_ref': {'lifetime': None, 'is_mutable': True,
                                                         'type': {'generic': 'Self'}}},
                'Expected exclusive self parameter')
        self.parameters = []
        for _, ty in inputs[1:]:
            require(ty in ({'primitive': 'usize'}, {'primitive': 'bool'}), 'Unsupported method parameter')
            self.parameters.append(ty['primitive'])
        self.enums, self.fields, self.opaque = {}, [], []
        field_ids = decl['kind']['plain']['fields']
        require(0 < len(field_ids) <= 256 and len(set(field_ids)) == len(field_ids), 'Invalid field roster')
        for field_id in field_ids:
            field = index[str(field_id)]
            name = identifier(field['name'])
            require(name not in [f.name for f in self.fields], 'Duplicate field name')
            ty = field['inner']['struct_field']
            kind = ty.get('primitive')
            if kind in ('usize', 'bool'):
                lean = 'Nat' if kind == 'usize' else 'Bool'
            else:
                kind, lean = None, None
                resolved = ty.get('resolved_path')
                if resolved:
                    identity = paths[str(resolved['id'])]
                    if identity['crate_id'] != 0 and identity['kind'] == 'enum' and identity['path'] == ['core', 'option', 'Option'] and resolved['args'] == {
                        'angle_bracketed': {'args': [{'type': {'primitive': 'usize'}}], 'constraints': []}}:
                        kind, lean = 'std::option::Option<usize>', 'Option Nat'
                    elif identity['crate_id'] == 0 and identity['kind'] == 'enum' and resolved['args'] is None:
                        enum_item = index[str(resolved['id'])]
                        enum = enum_item['inner']['enum']
                        if enum['generics'] == {'params': [], 'where_predicates': []} and not enum['has_stripped_variants']:
                            variants = [index[str(v)] for v in enum['variants']]
                            if variants and all(v['inner']['variant'] == {'kind': 'plain', 'discriminant': None} for v in variants):
                                kind = identifier(enum_item['name'])
                                require(identity['path'] == [index[str(metadata['root'])]['name'], kind],
                                        'Unsupported enum qualification')
                                self.enums[kind] = [identifier(v['name']) for v in variants]
                                require(len(set(self.enums[kind])) == len(variants), 'Duplicate variant')
                                lean = 'Enum' + str(list(self.enums).index(kind))
                if lean is None:
                    lean = 'Frame' + str(len(self.fields))
                    self.opaque.append(lean)
            self.fields.append(Field(name, kind, lean))

    def index(self, name):
        return next(i for i, field in enumerate(self.fields) if field.name == name)

    def enum_value(self, kind, variant):
        require(kind in self.enums and variant in self.enums[kind], 'Unknown enum variant')
        return f'Enum{list(self.enums).index(kind)}.v{self.enums[kind].index(variant)}'

    def declarations(self):
        enums = ['inductive Enum' + str(i) + ' where\n' + '\n'.join('  | v'+str(j) for j in range(len(v)))
                 + '\n  deriving DecidableEq' for i, v in enumerate(self.enums.values())]
        parameters = f' ({" ".join(self.opaque)} : Type)' if self.opaque else ''
        record = 'structure Record' + parameters + ' where\n' + '\n'.join(
            f'  field{i} : {field.lean}' for i, field in enumerate(self.fields)) + '\n  deriving DecidableEq'
        return '\n\n'.join(enums + [record, 'inductive Exit where\n  | returned\n  | panicked\n  deriving DecidableEq'])

    def record_type(self):
        return 'Record' + ((' ' + ' '.join(self.opaque)) if self.opaque else '')


class RecordTranslation(Translation):
    def __init__(self, mir, function, layout, bits=64):
        require(type(bits) is int and 0 < bits <= 128, 'Invalid target width')
        parameters, body = function_body(mir, function, '()')
        expected = ['_1: &mut ' + layout.record] + [f'_{i+2}: {kind}' for i, kind in enumerate(layout.parameters)]
        require(parameters.split(', ') == expected, 'MIR/metadata parameter mismatch')
        self.layout, self.bits = layout, bits
        self.types = {'_1': '&mut ' + layout.record}
        self.arguments = {}
        for i, kind in enumerate(layout.parameters):
            name = f'_{i+2}'
            self.types[name] = kind
            self.arguments[name] = Value('arg'+str(i+2) + (' = true' if kind == 'bool' else ''), kind)
        locals, self.blocks = basic_blocks(body)
        for line in locals.splitlines():
            line = line.strip()
            if not line or re.fullmatch(r'debug \w+ => .+;', line) or re.fullmatch(r'scope \d+ \{|}', line): continue
            m = re.fullmatch(r'let (?:mut )?(_\d+): (.+);', line)
            require(m is not None and m[1] not in self.types, 'Unsupported or duplicate local declaration')
            require(m[2] in ('()', 'usize', 'bool', '(usize, bool)', 'std::option::Option<usize>')
                    or m[2] in layout.enums, 'Unsupported local type: '+m[2])
            self.types[m[1]] = m[2]
        require(self.types.get('_0') == '()', 'Return type mismatch')
        self.references, self.visited, self.nodes, self.fresh = set(), set(), 0, 0

    def field(self, owner, index, kind):
        index = int(index)
        require(owner == '_1' and index < len(self.layout.fields), 'Invalid record place')
        field = self.layout.fields[index]
        require(field.kind is not None and field.kind == kind, 'Uninterpreted or mistyped field access')
        return index

    def operand(self, text, environment):
        m = re.fullmatch(r'copy \(\(\*(_\d+)\)\.(\d+): (.+)\)', text)
        if m:
            index = self.field(*m.groups())
            value = f'{environment["state"]}.field{index}'
            return Value(value + (' = true' if m[3] == 'bool' else ''), m[3])
        if text in ('const true', 'const false'):
            return Value('True' if text == 'const true' else 'False', 'bool')
        if text == 'const ()': return Value('()', '()')
        return super().operand(text, environment)

    def expression(self, text, environment):
        if text == 'Option::<usize>::None': return Value('none', 'std::option::Option<usize>')
        m = re.fullmatch(r'Option::<usize>::Some\((.+)\)', text)
        if m:
            value = self.operand(m[1], environment)
            require(value.kind == 'usize', 'Option payload type mismatch')
            return Value(f'(some {value.text})', 'std::option::Option<usize>')
        m = re.fullmatch(r'([A-Za-z][A-Za-z0-9_]*)::([A-Za-z][A-Za-z0-9_]*)', text)
        if m: return Value(self.layout.enum_value(*m.groups()), m[1])
        return super().expression(text, environment)

    def block(self, name, environment, stack=()):
        require(name in self.blocks and name not in stack, 'Missing block or cyclic control flow')
        self.nodes += 1
        require(self.nodes <= 4096, 'Control-flow expansion limit')
        self.visited.add(name)
        environment = dict(environment)
        lines = self.blocks[name]
        require(lines, 'Empty basic block')
        prefix, pending = '', None
        for line in lines[:-1]:
            require(pending is None, 'Overflow tuple must be checked immediately')
            m = re.fullmatch(r'(.*?) = (.*);', line)
            require(m is not None, 'Unsupported statement: '+line)
            target, source = m.groups()
            value = self.expression(source, environment)
            if source.startswith('Rem('):
                # Capture operands before a local assignment can shadow its divisor.
                require(target in self.types, 'Remainder must be evaluated in a local')
                _, denominator = (self.operand(x, environment) for x in pair(source[4:-1]))
                prefix += f'if {denominator.text} = 0 then ({environment["state"]}, Exit.panicked) else\n'
            if target in self.types:
                if isinstance(value, Overflow):
                    require(self.types[target] == '(usize, bool)', 'Overflow tuple type mismatch')
                    pending = target, value
                else:
                    require(self.types[target] == value.kind, 'Local type mismatch')
                    # Reads capture the state at this instruction, before any later write.
                    environment[target] = value
            else:
                place = re.fullmatch(r'\(\(\*(_\d+)\)\.(\d+): (.+)\)', target)
                require(place is not None, 'Unsupported assignment target: '+target)
                index = self.field(*place.groups())
                require(isinstance(value, Value) and value.kind == place[3], 'Field value type mismatch')
                encoded = f'(decide ({value.text}))' if value.kind == 'bool' else value.text
                self.fresh += 1
                state = f'state{self.fresh}'
                prefix += f'let {state} := {{ {environment["state"]} with field{index} := {encoded} }}\n'
                environment['state'] = state
        advance = lambda target: self.block(target, environment, (*stack, name))
        failed = f'({environment["state"]}, Exit.panicked)'
        last = lines[-1]
        if pending is not None:
            local, value = pending
            m = re.fullmatch(r'assert\(!move \('+re.escape(local)+r'\.1: bool\), "(?:[^"\\]|\\.)*"(?:, .*)?\) -> \[success: (bb\d+), unwind continue\];', last)
            require(m is not None, 'Overflow check absent or changed')
            a, b = value.left.text, value.right.text
            guard, result = (f'{b} ≤ {a}', f'({a} - {b})') if value.operator == 'SubWithOverflow' else (
                f'{a} + {b} < {2**self.bits}', f'({a} + {b})')
            environment[(local, 0)] = Value(result, 'usize')
            environment[(local, 1)] = Value('False', 'bool')
            return prefix + f'if {guard} then\n{advance(m[1])}\nelse {failed}'
        if last == 'return;': return prefix + f'({environment["state"]}, Exit.returned)'
        m = re.fullmatch(r'goto -> (bb\d+);', last)
        if m: return prefix + advance(m[1])
        m = re.fullmatch(r'switchInt\((.*)\) -> \[0: (bb\d+), otherwise: (bb\d+)\];', last)
        if m:
            value = self.operand(m[1], environment)
            require(value.kind == 'bool', 'Only Boolean switches supported')
            return prefix + f'if {value.text} then\n{advance(m[3])}\nelse\n{advance(m[2])}'
        m = re.fullmatch(r'assert\(!(.*?), "(?:[^"\\]|\\.)*"(?:, .*)?\) -> \[success: (bb\d+), unwind continue\];', last)
        if m:
            value = self.operand(m[1], environment)
            require(value.kind == 'bool', 'Non-Boolean assertion')
            return prefix + f'if {value.text} then {failed} else\n{advance(m[2])}'
        raise ValueError('Unsupported terminator: '+last)

    def lean(self):
        body = self.block('bb0', {**self.arguments, 'state': 'initial'})
        require(self.visited == self.blocks.keys(), 'Unvisited basic blocks')
        frame = f' {{{" ".join(self.layout.opaque)} : Type}}' if self.layout.opaque else ''
        params = ''
        for i, kind in enumerate(self.layout.parameters):
            name = f'arg{i+2}'
            if not re.search(r'\b'+name+r'\b', body): name = '_' + name
            params += f' ({name} : {"Nat" if kind == "usize" else "Bool"})'
        return self.layout.declarations() + '\n\ndef extracted' + frame + f' (initial : {self.layout.record_type()})' + params + \
            f' : ({self.layout.record_type()}) × Exit :=\n' + '\n'.join('  '+line for line in body.splitlines())+'\n'
