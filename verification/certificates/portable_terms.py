"""Shared untrusted producer syntax for portable proof terms.

Named construction is lowered to explicit de Bruijn indices. Generated terms
remain subject to native admission and independent replay; these helpers confer
no typing authority.
"""

def var(name): return ('Var', name)

def universe(level=0): return ('Universe', level)

def pi(name, domain, body): return ('Pi', name, domain, body)

def lam(name, domain, body): return ('Lam', name, domain, body)

def local(name, value, body): return ('Let', name, value, body)

def app(function, *arguments):
    for argument in arguments: function = ('App', function, argument)
    return function

def bind(kind, parameters, body):
    for name, domain in reversed(parameters): body = kind(name, domain, body)
    return body

def identity(carrier, left, right): return ('Id', carrier, left, right)

def refl(value): return ('Refl', value)

def j(motive, base, path): return ('J', motive, base, path)

def pair(left, right): return ('Pair', left, right)

def first(value): return ('Fst', value)

def second(value): return ('Snd', value)

def sigma(name, domain, body): return ('Sigma', name, domain, body)

def data(name, constructors, indices=(), level=0): return ('Data', name, constructors, indices, level)

def indexed_constructor(fields, indices): return ('Constructor', fields, indices)

def recursive_field(name, *indices): return ('RecursiveAt', name, indices)

def value_field(term): return ('Value', term)

def dependent_field(name, term): return ('Dependent', name, term)

RECURSIVE = ('Recursive',)

def construct(declaration, constructor, *arguments): return ('Construct', declaration, constructor, arguments)

def induct(scrutinee, motive, *cases): return ('Induct', scrutinee, motive, cases)

def list_type(element): return data('List', [[], [value_field(element), RECURSIVE]])

def nil(element): return construct(list_type(element), 0)

def cons(element, head, tail): return construct(list_type(element), 1, head, tail)

def singleton(element, item): return cons(element, item, nil(element))

def unit(): return data('Unit', [[]])

def tt(): return construct(unit(), 0)

def arrow(domain, codomain): return pi('_', domain, codomain)

def symm(carrier, left, right, path):
    return j(lam('sym_endpoint', carrier, identity(carrier, var('sym_endpoint'), left)), refl(left), path)

def congruence(source, target, function, left, right, path):
    return j(lam('cong_endpoint', source, identity(target, app(function, left), app(function, var('cong_endpoint')))), refl(app(function, left)), path)

def transitive(carrier, left, middle, right, first_path, second_path):
    return j(lam('trans_endpoint', carrier, identity(carrier, left, var('trans_endpoint'))), first_path, second_path)

def list_map(source, target, function, items):
    return induct(items, lam('map_subject', list_type(source), list_type(target)), nil(target),
        bind(lam, [('map_head', source), ('map_tail', list_type(source)), ('map_ih', list_type(target))],
             cons(target, app(function, var('map_head')), var('map_ih'))))

def lower(term, context=()):
    kind = term[0]
    if kind == 'Var':
        if term[1] not in context: raise ValueError(f'unbound generator variable: {term[1]}')
        return {'Var': context.index(term[1])}
    if kind == 'Universe': return {'Universe': {'Concrete': term[1]}}
    if kind == 'Let':
        return {'Let': {'value': lower(term[2], context), 'body': lower(term[3], (term[1],) + context)}}
    if kind in ('Pi', 'Lam', 'Sigma'):
        _, name, domain, body = term
        return {kind: [lower(domain, context), lower(body, (name,) + context)]}
    if kind in ('App', 'Pair'): return {kind: [lower(term[1], context), lower(term[2], context)]}
    if kind in ('Refl', 'Fst', 'Snd'): return {kind: lower(term[1], context)}
    if kind == 'Id': return {'Id': dict(zip(('ty', 'lhs', 'rhs'), (lower(x, context) for x in term[1:])))}
    if kind == 'J': return {'J': dict(zip(('motive', 'base', 'scrutinee'), (lower(x, context) for x in term[1:])))}
    if kind == 'Data':
        indices, index_context = [], context
        for name, domain in term[3]:
            indices.append(lower(domain, index_context))
            index_context = (name,) + index_context
        constructors = []
        for constructor in term[2]:
            if isinstance(constructor, tuple) and constructor[0] == 'Constructor':
                fields, result_indices = constructor[1:]
            else:
                fields, result_indices = constructor, ()
            lowered, field_context = [], context
            for field in fields:
                if field[0] == 'Dependent':
                    lowered.append({'Dependent': lower(field[2], field_context)})
                    binder = field[1]
                elif field[0] == 'RecursiveAt':
                    lowered.append({'RecursiveAt': [lower(index, field_context) for index in field[2]]})
                    binder = field[1]
                elif field[0] == 'Value':
                    lowered.append({'Value': lower(field[1], context)})
                    binder = object()  # An unnamed field still occupies one binder.
                elif field[0] == 'Recursive':
                    lowered.append('Recursive')
                    binder = object()
                else:
                    raise ValueError(f'unknown constructor field: {field[0]}')
                field_context = (binder,) + field_context
            result = [lower(index, field_context) for index in result_indices]
            constructors.append({'fields': lowered, 'indices': result} if result else lowered)
        declaration = {'name': term[1], 'level': {'Concrete': term[4]}, 'constructors': constructors}
        if indices:
            declaration['indices'] = indices
        return {'Data': declaration}
    if kind == 'Construct': return {'Construct': {'data': lower(term[1], context), 'constructor': term[2], 'args': [lower(x, context) for x in term[3]]}}
    if kind == 'Induct': return {'Induct': {'scrutinee': lower(term[1], context), 'motive': lower(term[2], context), 'cases': [lower(x, context) for x in term[3]]}}
    raise ValueError(kind)

def factor(value, available):
    for name, expression in available:
        if value == expression: return var(name)
    if isinstance(value, tuple): return tuple(factor(item, available) for item in value)
    if isinstance(value, list): return [factor(item, available) for item in value]
    return value
