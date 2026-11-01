"""Untrusted structural export of arithmetic certificates into Lean terms.

This checks the *proof term* with Lean's kernel, separately from executing the
reference checker. It supports the explicit arithmetic fragment and nonindexed algebraic
data with value and recursive fields. Unsupported syntax is refused, never replaced by an axiom.
"""
import hashlib
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

-- Structural implementation of the same list recursor. Lean's executable
-- code generator does not directly compile a Type-valued List.rec motive.
def listInduction {A : Type u} {motive : List A → Type v}
    (base : motive [])
    (step : (head : A) → (tail : List A) → motive tail → motive (head :: tail)) :
    (xs : List A) → motive xs
  | [] => base
  | head :: tail => step head tail (listInduction base step tail)

def pairValue {A : Type u} {B : Type v} (a : A) (b : B) : Sigma (fun _ : A => B) :=
  ⟨a, b⟩

'''


class Exporter:
    def __init__(self, declarations=None):
        self.counter = 0
        self.declarations = {} if declarations is None else declarations

    @staticmethod
    def data_type(kind):
        if not isinstance(kind, tuple):
            return kind
        return f'(List {kind[1]})' if kind[0] == 'List' else '(' + ' '.join([kind[1], *kind[3]]) + ')'

    def algebraic_data(self, decl, context):
        """Export a complete nonindexed data schema with explicit type parameters.

        Each nonrecursive field becomes a Type parameter instantiated by its
        actual field type. This preserves open types without capturing local
        Lean names in a global declaration. Native induction hypotheses are
        interleaved with recursive fields, unlike Lean's generated recursor.
        """
        if type(decl['name']) is not str or type(decl['constructors']) is not list:
            raise ValueError('malformed algebraic data')
        fields, arguments = [], []
        for constructor in decl['constructors']:
            if type(constructor) is not list:
                raise ValueError('malformed constructor fields')
            row = []
            for field in constructor:
                if field == 'Recursive':
                    row.append(None)
                elif isinstance(field, dict) and set(field) == {'Value'}:
                    row.append(f'A{len(arguments)}')
                    arguments.append(self.term(field['Value'], context))
                else:
                    raise ValueError('unsupported algebraic data field')
            fields.append(tuple(row))
        key = json.dumps([decl['name'], fields], separators=(',', ':'))
        name = 'WireData_' + hashlib.sha256(key.encode()).hexdigest()
        if name not in self.declarations:
            params = [f'A{i}' for i in range(len(arguments))]
            applied_type = '(' + ' '.join([name, *params]) + ')'
            constructors, parameters, branches = [], [], []
            for index, row in enumerate(fields):
                args = [f'a{i}' for i in range(len(row))]
                constructors.append(f'  | c{index} : ' + ' → '.join([ty or applied_type for ty in row] + [applied_type]))
                telescope, applied = [], []
                for a, ty in zip(args, row):
                    telescope.append(f'({a} : {ty or applied_type})'); applied.append(a)
                    if ty is None:
                        telescope.append(f'(_ih_{a} : motive {a})')
                        applied.append(f'({name}.fold ' + ' '.join(f'case{i}' for i in range(len(fields))) + f' {a})')
                target = f'motive ({name}.c{index} ' + ' '.join(args) + ')'
                parameters.append(f'(case{index} : ' + ('∀ ' + ' '.join(telescope) + ', ' if telescope else '') + target + ')')
                branches.append(f'  | .c{index} ' + ' '.join(args) + ' => ' + f'case{index} ' + ' '.join(applied))
            declaration = f'inductive {name} ' + ' '.join(f'({p} : Type)' for p in params)
            declaration += ' : Type where\n' + '\n'.join(constructors) + '\n'
            declaration += f'def {name}.fold ' + ' '.join(f'{{{p} : Type}}' for p in params)
            declaration += f' {{motive : {applied_type} → Type u}} ' + ' '.join(parameters)
            declaration += f' : (x : {applied_type}) → motive x'
            declaration += '\n' + '\n'.join(branches) + '\n' if branches else ' := fun x => nomatch x\n'
            self.declarations[name] = declaration
        return ('Wire', name, tuple(fields), tuple(arguments))

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
        if decl['name'] in ('Nat', 'Unit', 'Empty', 'List'):
            raise ValueError('changed builtin arithmetic constructors')
        return self.algebraic_data(decl, context)

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
            return self.data_type(data_kind)
        if kind == 'Construct':
            if set(value) != {'data', 'constructor', 'args'}:
                raise ValueError('malformed arithmetic constructor')
            data_kind = self.data(value['data'], context)
            choices = {'Nat': [('Nat.zero', 0), ('Nat.succ', 1)],
                       'Unit': [('Unit.unit', 0)], 'Empty': [],
                       'List': [('List.nil', 0), ('List.cons', 2)]}
            if isinstance(data_kind, tuple) and data_kind[0] == 'Wire':
                constructors = [(data_kind[1] + f'.c{i}', len(row)) for i, row in enumerate(data_kind[2])]
            else:
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
            name = data_kind[0] if isinstance(data_kind, tuple) else data_kind
            arity = len(data_kind[2]) if name == 'Wire' else {'Nat': 2, 'List': 2, 'Unit': 1, 'Empty': 0}[name]
            if len(value['cases']) != arity:
                raise ValueError('invalid arithmetic induction cases')
            cases = ' '.join(f'({emit(case)})' for case in value['cases'])
            recursor = data_kind[1] + '.fold' if name == 'Wire' else ('listInduction' if name == 'List' else name + '.rec')
            return f'({recursor} (motive := {emit(value["motive"])}) {cases} ({emit(value["scrutinee"])}))'
        if kind in ('App', 'Pair'):
            if not isinstance(value, list) or len(value) != 2:
                raise ValueError('malformed arithmetic pair/application')
            a, b = map(emit, value)
            return f'(({a}) ({b}))' if kind == 'App' else f'(pairValue ({a}) ({b}))'
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
    parts, declarations = [], {}
    for index, (name, certificate, _) in enumerate(cases):
        # JSON round-trip forces the exact portable wire representation.
        certificate = json.loads(json.dumps(certificate))
        exporter = Exporter(declarations)
        claim = exporter.term(certificate['claimed_type'])
        proof = exporter.term(certificate['term'])
        parts.append(f'-- {name}\ndef proof{index} : {claim} :=\n{proof}\n#print axioms proof{index}\n')
    parts.append('end PortableArithmetic\n')
    return '\n'.join([PRELUDE, *declarations.values(), *parts])
