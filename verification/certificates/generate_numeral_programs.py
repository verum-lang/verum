"""Partial iteration laws shared with the typed-program numeral bridge.

Only the structural core has portable terms here. Indexed program composition
and bounded machine arithmetic are separate Lean theorems in NumeralPrograms.
"""
from portable_terms import (RECURSIVE, app, arrow, bind, congruence, construct,
    data, identity, induct, lam, local, lower, pi, refl, universe, value_field, var)

N = data('Nat', [[], [RECURSIVE]])
zero = construct(N,0)
succ = lambda n: construct(N,1,n)
option = lambda a: data('Option',[[],[value_field(a)]])
none = lambda a: construct(option(a),0)
some = lambda a,x: construct(option(a),1,x)
A,step,initial,n = map(var,('A','step','initial','n'))
option_bind = lambda a,value,action: induct(value,lam('optional',option(a),option(a)),
    none(a),lam('value',a,app(action,var('value'))))
iteration = bind(lam,[('A',universe()),('step',arrow(A,option(A))),('initial',option(A)),('n',N)],
    induct(n,lam('count',N,option(A)),initial,
        bind(lam,[('tail',N),('result',option(A))],option_bind(A,var('result'),step))))
iterate = lambda a,f,x,count: app(var('iterate'),a,f,x,count)

def certificate(term,claim):
    return dict(term=lower(local('iterate',iteration,term)),
        claimed_type=lower(local('iterate',iteration,claim)),metadata={})


def cases():
    result=[]
    def add(name,params,proof,claim):
        result.append((name,certificate(bind(lam,params,proof),bind(pi,params,claim)),True))
    params=[('A',universe()),('step',arrow(A,option(A))),('initial',option(A))]
    add('iterate_zero',params,refl(initial),identity(option(A),iterate(A,step,initial,zero),initial))
    at=iterate(A,step,initial,n)
    add('iterate_successor',params+[('n',N)],refl(option_bind(A,at,step)),
        identity(option(A),iterate(A,step,initial,succ(n)),option_bind(A,at,step)))
    absent=lambda count:iterate(A,step,none(A),count)
    goal=lambda count:identity(option(A),absent(count),none(A))
    absent_proof=induct(n,lam('n',N,goal(n)),refl(none(A)),
        bind(lam,[('tail',N),('ih',goal(var('tail')))],
            congruence(option(A),option(A),lam('x',option(A),option_bind(A,var('x'),step)),
                absent(var('tail')),none(A),var('ih'))))
    add('undefined_stays_undefined',params[:2]+[('n',N)],absent_proof,goal(n))
    successor=lam('x',N,some(N,succ(var('x'))))
    numeral=lambda count:iterate(N,successor,some(N,zero),count)
    goal=lambda count:identity(option(N),numeral(count),some(N,count))
    proof=induct(n,lam('n',N,goal(n)),refl(some(N,zero)),
        bind(lam,[('tail',N),('ih',goal(var('tail')))],
            congruence(option(N),option(N),lam('x',option(N),option_bind(N,var('x'),successor)),
                numeral(var('tail')),some(N,var('tail')),var('ih'))))
    add('successor_expresses_every_natural',[('n',N)],proof,goal(n))
    one=succ(zero)
    false_claims=[
        ('initial_value_erased',iterate(N,successor,some(N,one),zero),some(N,zero)),
        ('zero_without_successor_names_one',iterate(N,lam('x',N,some(N,zero)),some(N,zero),one),some(N,one)),
        ('undefined_recovers_as_zero',iterate(N,successor,none(N),one),some(N,zero)),
        ('refused_successor_wraps',iterate(N,lam('x',N,none(N)),some(N,zero),one),some(N,zero)),
    ]
    for name,actual,wrong in false_claims:
        result.append((name,certificate(refl(actual),identity(option(N),actual,wrong)),False))
    return result
