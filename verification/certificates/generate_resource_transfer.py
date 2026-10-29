"""Untrusted proofs for exact partial-program embeddings and total-cost bounds.

Cost includes the claimed translation/search/library/execution overhead. None
is semantic absence, never operational suspension. No runtime or risk estimator
is certified here. Independent replay checks every definition and hypothesis.
"""
from portable_terms import (RECURSIVE, app, arrow, bind, congruence, construct,
    data, first, fun_ext, identity, induct, j, lam, local, lower, pair, pi, refl, second, sigma, symm,
    transitive, tt, unit, universe, value_field, var)

EMPTY = data('Empty', [])
nat = var('Nat')
zero = construct(nat, 0)
succ = lambda n: construct(nat, 1, n)
definitions = [('Nat', data('Nat', [[], [RECURSIVE]]))]
contracts, foundations = [], {}


def names(term):
    if not isinstance(term, (tuple, list)): return set()
    if term and term[0] == 'Var': return {term[1]}
    return set().union(*(names(child) for child in term))


def close(term):
    used = names(term)
    for name, definition in reversed(definitions):
        if name in used:
            term = local(name, definition, term); used.update(names(definition))
    return term


def certificate(term, claim):
    return dict(term=lower(close(term)), claimed_type=lower(close(claim)), metadata={})


def add(name, params, proof, claim, *, extensional=False):
    definitions.append((name, bind(lam, params, proof)))
    contracts.append((name, certificate(var(name), bind(pi, params, claim)), True))
    if extensional: foundations[name] = 'function-extensionality'


# Structural natural order. The transitivity theorem is proved, not supplied
# as an unchecked arithmetic fact or an opaque foreign primitive.
le = lambda a, b: app(var('le'), a, b)
a,b,c = map(var, ('a','b','c'))
definitions.append(('le', lam('a', nat, induct(a,
    lam('left', nat, arrow(nat, universe())), lam('b', nat, unit()),
    bind(lam, [('left_tail', nat), ('order_ih', arrow(nat, universe())), ('b', nat)],
        induct(b, lam('right', nat, universe()), EMPTY,
            bind(lam, [('right_tail', nat), ('unused', universe())], app(var('order_ih'), var('right_tail')))))))))
order_params = lambda left, right, end: [('ab', le(left,right)), ('bc', le(right,end))]
trans_goal = lambda left: bind(pi, [('b',nat), ('c',nat)] + order_params(left,b,c), le(left,c))
lt,rt,ct = map(var, ('left_tail','right_tail','end_tail'))
right_goal = lambda right: bind(pi, [('c',nat)] + order_params(succ(lt),right,c), le(succ(lt),c))
end_goal = lambda end: bind(pi, order_params(succ(lt),succ(rt),end), le(succ(lt),end))
right_zero = bind(lam, [('c',nat)] + order_params(succ(lt),zero,c),
    induct(var('ab'), lam('impossible',EMPTY,le(succ(lt),c))))
end_zero = bind(lam, order_params(succ(lt),succ(rt),zero),
    induct(var('bc'), lam('impossible',EMPTY,le(succ(lt),zero))))
end_succ = bind(lam, [('end_tail',nat), ('end_ih',end_goal(ct))] + order_params(succ(lt),succ(rt),succ(ct)),
    app(var('trans_ih'),rt,ct,var('ab'),var('bc')))
right_succ = bind(lam, [('right_tail',nat), ('right_ih',right_goal(rt)), ('c',nat)],
    induct(c,lam('end',nat,end_goal(var('end'))),end_zero,end_succ))
left_succ = bind(lam,[('left_tail',nat),('trans_ih',trans_goal(lt)),('b',nat)],
    induct(b,lam('right',nat,right_goal(var('right'))),right_zero,right_succ))
trans_proof = app(induct(a,lam('left',nat,trans_goal(var('left'))),
    bind(lam,[('b',nat),('c',nat)]+order_params(zero,b,c),tt()),left_succ),b,c,var('ab'),var('bc'))
add('natural_order_transitive',[('a',nat),('b',nat),('c',nat)]+order_params(a,b,c),trans_proof,le(a,c))


U = lambda name: universe({'Var': name})
P,Q,R,X,Z,W,A,B,C,L,K = map(var,('P','Q','R','X','Z','W','A','B','C','L','K'))
levels = {A:'a',B:'b',C:'c'}
option = lambda carrier: data('Option',[[],[value_field(carrier)]],level={'Var':levels[carrier]})
none = lambda carrier: construct(option(carrier),0)
some = lambda carrier,value: construct(option(carrier),1,value)
def mapped(source,target,function,value):
    return induct(value,lam('mapped_subject',option(source),option(target)),none(target),
        lam('mapped_value',source,some(target,app(function,var('mapped_value')))))
f,g,v = map(var,('f','g','v'))
compose_y = lam('result',A,app(g,app(f,var('result'))))
fusion_goal = lambda value: identity(option(C),mapped(B,C,g,mapped(A,B,f,value)),mapped(A,C,compose_y,value))
fusion = lambda value: induct(value,lam('subject',option(A),fusion_goal(var('subject'))),refl(none(C)),
    lam('value',A,refl(some(C,app(g,app(f,var('value')))))))
add('option_map_composition',[('A',U('a')),('B',U('b')),('C',U('c')),('f',arrow(A,B)),('g',arrow(B,C)),('v',option(A))],fusion(v),fusion_goal(v))

program,input_ = map(var,('program','input'))
types2 = [('P',U('p')),('Q',U('q')),('X',U('x')),('Z',U('y')),('A',U('a')),('B',U('b'))]
dS,dT,e,ix,iy = map(var,('denS','denT','translate','embedX','embedY'))
ops2 = [('denS',arrow(P,arrow(X,option(A)))),('denT',arrow(Q,arrow(Z,option(B)))),
        ('translate',arrow(P,Q)),('embedX',arrow(X,Z)),('embedY',arrow(A,B))]
source_result = lambda p,x: app(dS,p,x)
target_result = lambda p,x: app(dT,app(e,p),app(ix,x))
map_result = lambda value: mapped(A,B,iy,value)
exact = bind(pi,[('program',P),('input',X)],identity(option(B),target_result(program,input_),map_result(source_result(program,input_))))
sem_params = types2 + ops2 + [('exact',exact)]
point = [('program',P),('input',X)]
sr,tr = source_result(program,input_),target_result(program,input_)
mr = map_result(sr)
path = app(var('exact'),program,input_)

# Composition acts independently on programs, inputs and terminal results.
dU,et,xt,yt = map(var,('denU','translateTU','inputTU','outputTU'))
extra = [('R',U('r')),('W',U('z')),('C',U('c'))]
next_ops = [('denU',arrow(R,arrow(W,option(C)))),('translateTU',arrow(Q,R)),
            ('inputTU',arrow(Z,W)),('outputTU',arrow(B,C))]
next_exact = bind(pi,[('q',Q),('z',Z)],identity(option(C),app(dU,app(et,var('q')),app(xt,var('z'))),
    mapped(B,C,yt,app(dT,var('q'),var('z')))))
start = app(dU,app(et,app(e,program)),app(xt,app(ix,input_)))
map_t = lambda value: mapped(B,C,yt,value)
composed_output = lam('result',A,app(yt,app(iy,var('result'))))
end = mapped(A,C,composed_output,sr)
cong = congruence(option(B),option(C),lam('value',option(B),map_t(var('value'))),tr,mr,path)
compose_path = transitive(option(C),start,map_t(tr),end,app(var('exactTU'),app(e,program),app(ix,input_)),
    transitive(option(C),map_t(tr),map_t(mr),end,cong,
        app(var('option_map_composition'),A,B,C,iy,yt,sr)))
add('semantic_composition',types2+extra+ops2+next_ops+[('exact',exact),('exactTU',next_exact)]+point,
    compose_path,identity(option(C),start,end))

monotone = lambda fn: bind(pi,[('m',nat),('n',nat),('mn',le(var('m'),var('n')))],
    le(app(fn,var('m')),app(fn,var('n'))))
cS,cT,cU,phi,psi = map(var,('costS','costT','costU','phi','psi'))
cost_types = types2[:4]
cost_ops = [('costS',arrow(P,arrow(X,nat))),('costT',arrow(Q,arrow(Z,nat))),
            ('translate',arrow(P,Q)),('embedX',arrow(X,Z)),('phi',arrow(nat,nat))]
cs,ct,cu = app(cS,program,input_),app(cT,app(e,program),app(ix,input_)),app(cU,app(et,app(e,program)),app(xt,app(ix,input_)))
bounded = bind(pi,point,le(ct,app(phi,cs)))
bounded_tu = bind(pi,[('q',Q),('z',Z)],le(app(cU,app(et,var('q')),app(xt,var('z'))),app(psi,app(cT,var('q'),var('z')))))
order_trans = lambda a,b,c,ab,bc: app(var('natural_order_transitive'),a,b,c,ab,bc)
cost_proof = order_trans(cu,app(psi,ct),app(psi,app(phi,cs)),
    app(var('boundedTU'),app(e,program),app(ix,input_)),
    app(var('monotoneTU'),ct,app(phi,cs),app(var('bounded'),program,input_)))
add('cost_composition',cost_types+[('R',U('r')),('W',U('z'))]+cost_ops+
    [('costU',arrow(R,arrow(W,nat))),('translateTU',arrow(Q,R)),('inputTU',arrow(Z,W)),('psi',arrow(nat,nat)),
     ('bounded',bounded),('boundedTU',bounded_tu),('monotoneTU',monotone(psi))]+point,
    cost_proof,le(cu,app(psi,app(phi,cs))))
m,n = var('m'),var('n')
add('monotone_composition',[('f',arrow(nat,nat)),('g',arrow(nat,nat)),('hf',monotone(f)),('hg',monotone(g)),
    ('m',nat),('n',nat),('mn',le(m,n))],app(var('hg'),app(f,m),app(f,n),app(var('hf'),m,n,var('mn'))),
    le(app(g,app(f,m)),app(g,app(f,n))))


# Preserve absence and successful values without treating None as suspension.
none_hyp = identity(option(A),sr,none(A))
none_path = congruence(option(A),option(B),lam('value',option(A),map_result(var('value'))),sr,none(A),var('h'))
add('none_preserved',sem_params+point+[('h',none_hyp)],
    transitive(option(B),tr,mr,none(B),path,none_path),identity(option(B),tr,none(B)))

qnone = lambda value: identity(option(B),map_result(value),none(B))
none_goal = lambda value: pi('qnone',qnone(value),identity(option(A),value,none(A)))
value = var('value')
is_defined = lam('result',option(B),induct(var('result'),lam('subject',option(B),universe()),EMPTY,
    lam('value',B,unit())))
absurd = j(is_defined,tt(),var('qnone'))
none_branch = lam('qnone',qnone(none(A)),refl(none(A)))
some_branch = bind(lam,[('value',A),('qnone',qnone(some(A,value)))],
    induct(absurd,lam('impossible',EMPTY,identity(option(A),some(A,value),none(A)))))
map_none_path = transitive(option(B),mr,tr,none(B),symm(option(B),tr,mr,path),var('h'))
add('none_reflected',sem_params+point+[('h',identity(option(B),tr,none(B)))],
    app(induct(sr,lam('subject',option(A),none_goal(var('subject'))),none_branch,some_branch),map_none_path),
    identity(option(A),sr,none(A)))

success_path = congruence(option(A),option(B),lam('result',option(A),map_result(var('result'))),sr,some(A,value),var('h'))
add('some_preserved',sem_params+point+[('value',A),('h',identity(option(A),sr,some(A,value)))],
    transitive(option(B),tr,mr,some(B,app(iy,value)),path,success_path),identity(option(B),tr,some(B,app(iy,value))))

injective = bind(pi,[('left',A),('right',A),('eq',identity(B,app(iy,var('left')),app(iy,var('right'))))],
    identity(A,var('left'),var('right')))
qsome = lambda result: identity(option(B),map_result(result),some(B,app(iy,value)))
some_goal = lambda result: bind(pi,[('value',A),('qsome',qsome(result))],identity(option(A),result,some(A,value)))
is_absent = lam('result',option(B),induct(var('result'),lam('subject',option(B),universe()),unit(),lam('value',B,EMPTY)))
none_case = bind(lam,[('value',A),('qsome',qsome(none(A)))],
    induct(j(is_absent,tt(),var('qsome')),lam('impossible',EMPTY,identity(option(A),none(A),some(A,value)))))
current = var('current')
project = lam('result',option(B),induct(var('result'),lam('subject',option(B),B),app(iy,value),lam('result_value',B,var('result_value'))))
projected = congruence(option(B),B,project,some(B,app(iy,current)),some(B,app(iy,value)),var('qsome'))
some_case = bind(lam,[('current',A),('value',A),('qsome',qsome(some(A,current)))],
    congruence(A,option(A),lam('result',A,some(A,var('result'))),current,value,
        app(var('injective'),current,value,projected)))
map_some_path = transitive(option(B),mr,tr,some(B,app(iy,value)),symm(option(B),tr,mr,path),var('h'))
add('some_reflected',sem_params+point+[('injective',injective),('value',A),('h',identity(option(B),tr,some(B,app(iy,value))))],
    app(induct(sr,lam('subject',option(A),some_goal(var('subject'))),none_case,some_case),value,map_some_path),
    identity(option(A),sr,some(A,value)))

budget = var('budget')
add('budget_transport',cost_types+cost_ops+[('bounded',bounded),('monotone',monotone(phi))]+point+
    [('budget',nat),('h',le(cs,budget))],
    order_trans(ct,app(phi,cs),app(phi,budget),app(var('bounded'),program,input_),app(var('monotone'),cs,budget,var('h'))),
    le(ct,app(phi,budget)))

source_loss,target_loss = map(var,('sourceLoss','targetLoss'))
loss_params = [('L',U('l')),('sourceLoss',arrow(X,arrow(option(A),L))),('targetLoss',arrow(Z,arrow(option(B),L)))]
compatibility = bind(pi,[('x',X),('result',option(A))],identity(L,
    app(target_loss,app(ix,var('x')),map_result(var('result'))),app(source_loss,var('x'),var('result'))))
loss_params += [('compatible',compatibility)]
loss_left,loss_mid,loss_right = app(target_loss,app(ix,input_),tr),app(target_loss,app(ix,input_),mr),app(source_loss,input_,sr)
loss_proof = transitive(L,loss_left,loss_mid,loss_right,
    congruence(option(B),L,app(target_loss,app(ix,input_)),tr,mr,path),app(var('compatible'),input_,sr))
add('pointwise_loss_transport',sem_params+[loss_params[0]]+point+loss_params[1:],loss_proof,identity(L,loss_left,loss_right))

loss_function = arrow(X,L)
left_fn = lam('input',X,loss_left)
right_fn = lam('input',X,loss_right)
aggregate = var('aggregate')
add('fixed_aggregate_risk_transport',sem_params+[loss_params[0],('K',U('k')),('program',P)]+loss_params[1:]+[('aggregate',arrow(loss_function,K))],
    congruence(loss_function,K,aggregate,left_fn,right_fn,
        fun_ext(loss_function,left_fn,right_fn,lam('input',X,loss_proof))),
    identity(K,app(aggregate,left_fn),app(aggregate,right_fn)),extensional=True)


# Accumulated paid work is a separate component from the executable budget.
# A shrinking execution map cannot remove previously paid acquisition work.
add_cost = lambda left,right: app(var('addCost'),left,right)
definitions.append(('addCost',bind(lam,[('a',nat),('b',nat)],induct(a,
    lam('left',nat,nat),b,bind(lam,[('tail',nat),('sum',nat)],succ(var('sum')))))))
add_order_goal = lambda left: le(left,add_cost(left,b))
definitions.append(('paidMonotone',bind(lam,[('a',nat),('b',nat)],induct(a,
    lam('left',nat,add_order_goal(var('left'))),tt(),
    bind(lam,[('tail',nat),('ih',add_order_goal(var('tail')))],var('ih'))))))
prefix,m,n = map(var,('prefix','m','n'))
add_monotone_goal = lambda left: le(add_cost(left,m),add_cost(left,n))
definitions.append(('addMonotone',bind(lam,[('prefix',nat),('m',nat),('n',nat),('h',le(m,n))],
    induct(prefix,lam('prefix',nat,add_monotone_goal(prefix)),var('h'),
        bind(lam,[('tail',nat),('ih',add_monotone_goal(var('tail')))],var('ih'))))))
paid_state = sigma('paid',nat,nat)
state = var('state')
advance = lambda overhead,fn,state: pair(add_cost(first(state),overhead),app(fn,second(state)))
resource_params = [('a',nat),('b',nat),('f',arrow(nat,nat)),('g',arrow(nat,nat)),('state',paid_state)]
advanced = advance(b,g,advance(a,f,state))
paid = add_cost(add_cost(first(state),a),b)
execution_bound = app(g,app(f,second(state)))
add('paid_budgets_compose',resource_params,refl(pair(paid,execution_bound)),
    identity(paid_state,advanced,pair(paid,execution_bound)))
add('paid_work_monotone',[('a',nat),('f',arrow(nat,nat)),('state',paid_state)],
    app(var('paidMonotone'),first(state),a),le(first(state),first(advance(a,f,state))))
execution = var('execution')
add('lifetime_budget_transport',resource_params+[('execution',nat),('within',le(execution,execution_bound))],
    app(var('addMonotone'),paid,execution,execution_bound,var('within')),
    le(add_cost(first(advanced),execution),add_cost(paid,execution_bound)))


def cases():
    result = list(contracts)
    one = succ(zero)
    result.append(('translation_overhead_erased',certificate(tt(),le(add_cost(one,one),one)),False))
    decreasing = lam('n',nat,induct(var('n'),lam('n',nat,nat),one,
        bind(lam,[('tail',nat),('ih',nat)],zero)))
    result.append(('nonmonotone_budget_assumed',certificate(tt(),le(app(decreasing,zero),app(decreasing,one))),False))
    constant_one,constant_zero = lam('n',nat,one),lam('n',nat,zero)
    actual_paid = first(advance(zero,constant_zero,advance(one,constant_one,pair(zero,zero))))
    scalar_composition = app(constant_zero,app(constant_one,zero))
    result.append(('incurred_overhead_discounted',certificate(tt(),le(actual_paid,scalar_composition)),False))
    bits = data('Bool',[[],[]]); off,on = construct(bits,0),construct(bits,1)
    optional_bits = data('Option',[[],[value_field(bits)]])
    absent,present = construct(optional_bits,0),construct(optional_bits,1,on)
    mapped_absence = induct(absent,lam('optional',optional_bits,optional_bits),absent,
        lam('bit',bits,present))
    result.append(('semantic_none_replaced',certificate(refl(absent),identity(optional_bits,mapped_absence,present)),False))
    before,after = construct(optional_bits,1,off),construct(optional_bits,1,on)
    optional_unit = data('Option',[[],[value_field(unit())]])
    erase = lambda optional: induct(optional,lam('optional',optional_bits,optional_unit),construct(optional_unit,0),
        lam('bit',bits,construct(optional_unit,1,tt())))
    erased_equal = identity(optional_unit,erase(before),erase(after))
    result.append(('noninjective_value_reflection',certificate(lam('equal',erased_equal,refl(before)),
        pi('equal',erased_equal,identity(optional_bits,before,after))),False))
    actual = advance(zero,constant_zero,advance(one,constant_one,pair(zero,zero)))
    result.append(('paid_component_erased',certificate(refl(actual),identity(paid_state,actual,pair(zero,zero))),False))
    return result
