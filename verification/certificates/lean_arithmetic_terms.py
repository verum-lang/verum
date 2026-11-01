"""Untrusted structural export of arithmetic certificates into Lean terms.

This checks the *proof term* with Lean's kernel, separately from executing the
reference checker. It deliberately supports only the explicit arithmetic data
fragment below. Unsupported syntax is refused, never replaced by an axiom.
"""
import json


PRELUDE = '''import Init
set_option warningAsError true
set_option maxRecDepth 4096
set_option maxHeartbeats 8000000
namespace PortableArithmetic

-- Type-valued identity matches the portable calculus; Prop-valued Eq would
-- change the universe of motives. There is no proof irrelevance assumption.
inductive Identity {A : Type u} (x : A) : A → Type u where
  | refl : Identity x x

def transport {A : Type u} {x y : A} (motive : A → Type v)
    (base : motive x) (path : Identity x y) : motive y :=
  match path with
  | .refl => base

'''


class Exporter:
    def __init__(self):
        self.counter = 0

    def fresh(self):
        self.counter += 1
        return f'_v{self.counter}'

    def data(self, term, context):
        """Recognize the complete declaration, never just its display name."""
        if set(term) == {'Var'}:
            index = term['Var']
            if type(index) is not int or not 0 <= index < len(context):
                raise ValueError('unbound data alias')
            kind = context[index][1]
            if kind is None:
                raise ValueError('data is not a closed arithmetic declaration')
            return kind
        if set(term) != {'Data'}:
            raise ValueError('unsupported arithmetic data expression')
        decl = term['Data']
        if set(decl) != {'name', 'level', 'constructors'} or decl['level'] != {'Concrete': 0}:
            raise ValueError('unsupported arithmetic data declaration')
        constructors = decl['constructors']
        if decl['name'] == 'Nat' and constructors == [[], ['Recursive']]:
            return 'Nat'
        if decl['name'] == 'Unit' and constructors == [[]]:
            return 'Unit'
        if decl['name'] == 'Empty' and constructors == []:
            return 'Empty'
        if (decl['name'] == 'List' and len(constructors) == 2 and constructors[0] == []
                and len(constructors[1]) == 2 and constructors[1][1] == 'Recursive'
                and set(constructors[1][0]) == {'Value'}):
            return ('List', self.term(constructors[1][0]['Value'], context))
        raise ValueError('unsupported arithmetic constructors')

    def term(self, term, context=()):
        if not isinstance(term, dict) or len(term) != 1:
            raise ValueError('expected exactly one term constructor')
        kind, value = next(iter(term.items()))
        emit = lambda t: self.term(t, context)
        if kind == 'Var':
            if type(value) is not int or not 0 <= value < len(context):
                raise ValueError('unbound arithmetic variable')
            return context[value][0]
        if kind == 'Universe':
            if (not isinstance(value, dict) or set(value) != {'Concrete'}
                    or type(value['Concrete']) is not int or not 0 <= value['Concrete'] <= 32):
                raise ValueError('unsupported arithmetic universe')
            return f'Type {value["Concrete"]}'
        if kind in ('Pi', 'Lam', 'Sigma'):
            if not isinstance(value, list) or len(value) != 2:
                raise ValueError('malformed arithmetic binder')
            name = self.fresh()
            domain = emit(value[0])
            body = self.term(value[1], ((name, None),) + context)
            if kind == 'Pi':
                return f'(({name} : {domain}) → {body})'
            if kind == 'Lam':
                return f'(fun ({name} : {domain}) => {body})'
            return f'(Sigma (fun ({name} : {domain}) => {body}))'
        if kind == 'Let':
            if set(value) != {'value', 'body'}:
                raise ValueError('malformed arithmetic let')
            name = self.fresh()
            definition = emit(value['value'])
            # Only a complete Data declaration, or an already recognized
            # alias, can provide constructor/eliminator identity.
            data_kind = None
            if set(value['value']) in ({'Data'}, {'Var'}):
                try:
                    data_kind = self.data(value['value'], context)
                except ValueError:
                    pass
            body = self.term(value['body'], ((name, data_kind),) + context)
            return f'(let {name} := {definition};\n{body})'
        if kind == 'Data':
            data_kind = self.data(term, context)
            return f'(List {data_kind[1]})' if isinstance(data_kind, tuple) else data_kind
        if kind == 'Construct':
            if set(value) != {'data', 'constructor', 'args'}:
                raise ValueError('malformed arithmetic constructor')
            data_kind = self.data(value['data'], context)
            choices = {'Nat': [('Nat.zero', 0), ('Nat.succ', 1)],
                       'Unit': [('Unit.unit', 0)], 'Empty': [],
                       'List': [('List.nil', 0), ('List.cons', 2)]}
            constructors = choices['List' if isinstance(data_kind, tuple) else data_kind]
            index = value['constructor']
            if type(index) is not int or not 0 <= index < len(constructors):
                raise ValueError('invalid arithmetic constructor index')
            constructor, arity = constructors[index]
            if len(value['args']) != arity:
                raise ValueError('invalid arithmetic constructor arity')
            args = ' '.join(f'({emit(arg)})' for arg in value['args'])
            return f'(({constructor} {args}) : {emit(value["data"])})'
        if kind == 'Induct':
            if set(value) != {'scrutinee', 'motive', 'cases'} or set(value['motive']) != {'Lam'}:
                raise ValueError('unsupported arithmetic induction motive')
            data_kind = self.data(value['motive']['Lam'][0], context)
            name = 'List' if isinstance(data_kind, tuple) else data_kind
            arity = {'Nat': 2, 'List': 2, 'Unit': 1, 'Empty': 0}[name]
            if len(value['cases']) != arity:
                raise ValueError('invalid arithmetic induction cases')
            cases = ' '.join(f'({emit(case)})' for case in value['cases'])
            return f'({name}.rec (motive := {emit(value["motive"])}) {cases} ({emit(value["scrutinee"])}))'
        if kind in ('App', 'Pair'):
            if not isinstance(value, list) or len(value) != 2:
                raise ValueError('malformed arithmetic pair/application')
            a, b = map(emit, value)
            return f'(({a}) ({b}))' if kind == 'App' else f'⟨{a}, {b}⟩'
        if kind in ('Fst', 'Snd'):
            return f'(({emit(value)}).{1 if kind == "Fst" else 2})'
        if kind == 'Id':
            if set(value) != {'ty', 'lhs', 'rhs'}:
                raise ValueError('malformed arithmetic identity')
            return f'(Identity (A := {emit(value["ty"])}) ({emit(value["lhs"])}) ({emit(value["rhs"])}))'
        if kind == 'Refl':
            return f'(Identity.refl (x := {emit(value)}))'
        if kind == 'J':
            if set(value) != {'motive', 'base', 'scrutinee'}:
                raise ValueError('malformed arithmetic transport')
            return f'(transport ({emit(value["motive"])}) ({emit(value["base"])}) ({emit(value["scrutinee"])}))'
        raise ValueError('unsupported arithmetic term: ' + kind)


def source(cases):
    parts = [PRELUDE]
    for index, (name, certificate, _) in enumerate(cases):
        # JSON round-trip forces the exact portable wire representation.
        certificate = json.loads(json.dumps(certificate))
        exporter = Exporter()
        claim = exporter.term(certificate['claimed_type'])
        proof = exporter.term(certificate['term'])
        parts.append(f'-- {name}\ndef proof{index} : {claim} :=\n{proof}\n#print axioms proof{index}\n')
    parts.append('end PortableArithmetic\n')
    return '\n'.join(parts)
