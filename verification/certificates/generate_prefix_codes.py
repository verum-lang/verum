"""Constructive prefix decoding and Kraft bounds for concrete binary parsers.

Trees are first-order inductive data. Rejection consumes no probability mass;
acceptance stops before any suffix. No codec or Kraft law is a premise.
"""
import positive_fractions as fractions
from peano_arithmetic import (Library, N, Z, ONE, S, EMPTY, Path, add, mul, le,
    eq, call, change_order, ring, law, induction)
from positive_fractions import Q, fraction, num, den, order, equal, positive, one, times
from portable_terms import (RECURSIVE, data, value_field, construct, app, bind,
    lam, pi, var, induct, tt, refl, pair, list_type, nil, cons, j, identity,
    sigma, first, second, universe, arrow, lower)

library = Library()
library.definitions = fractions.library.definitions.copy()
library.claims = fractions.library.claims.copy()
FOUNDATIONS = set(library.claims)
B = library.define('Bit', data('Bit', [[], []]))
F, T = construct(B, 0), construct(B, 1)
W = list_type(B)
Tree = library.define('PrefixTree', data('PrefixTree', [[], [value_field(N)], [RECURSIVE, RECURSIVE]]))
reject = construct(Tree, 0)
leaf = lambda n: construct(Tree, 1, n)
branch = lambda l, r: construct(Tree, 2, l, r)
Result = sigma('_ok', N, sigma('_value', N, W))
success = lambda n, rest: pair(ONE, pair(n, rest))
failure = pair(Z, pair(Z, nil(B)))
_uid = 0

def fresh(label):
    global _uid
    _uid += 1
    return f'_{label}_{_uid}'

def word_case(word, goal, empty, step):
    h, t, ih, x = [fresh('word') for _ in range(4)]
    return induct(word, lam(x, W, goal(var(x))), empty,
        bind(lam, [(h, B), (t, W), (ih, goal(var(t)))], step(var(h), var(t))))

def bit_case(bit, goal, false, true):
    x = fresh('bit')
    return induct(bit, lam(x, B, goal(var(x))), false, true)

def tree_induct(tree, goal, empty, stop, node):
    x, n, l, il, r, ir = [fresh('tree') for _ in range(6)]
    return induct(tree, lam(x, Tree, goal(var(x))), empty,
        lam(n, N, stop(var(n))),
        bind(lam, [(l, Tree), (il, goal(var(l))), (r, Tree), (ir, goal(var(r)))],
             node(var(l), var(il), var(r), var(ir))))

w, tail, t, a, n = map(var, ('word', 'tail', 'tree', 'a', 'n'))
append = lambda w, tail: call('word_append', w, tail)
consume = lambda t, w: call('consume', t, w)
accept = lambda t, w, a: call('accept', t, w, a)
mass = lambda t: call('tree_mass', t)
blend = lambda l, r: fraction(add(mul(num(l), den(r)), mul(num(r), den(l))),
                              mul(S(ONE), mul(den(l), den(r))))
library.define('word_append', bind(lam, [('word', W), ('tail', W)],
    induct(w, lam('_w', W, W), tail,
        bind(lam, [('_head', B), ('_tail', W), ('_ih', W)], cons(B, var('_head'), var('_ih'))))))
library.define('consume', lam('tree', Tree, tree_induct(t, lambda _: arrow(W, Result),
    lam('word', W, failure), lambda a: lam('word', W, success(a, w)),
    lambda l, il, r, ir: lam('word', W,
        word_case(w, lambda _: Result, failure,
            lambda b, rest: bit_case(b, lambda _: Result, app(il, rest), app(ir, rest)))))))
library.define('accept', lam('tree', Tree, tree_induct(t, lambda _: arrow(W, arrow(N, universe())),
    bind(lam, [('word', W), ('a', N)], EMPTY),
    lambda label: bind(lam, [('word', W), ('a', N)],
        word_case(w, lambda _: universe(), eq(label, a), lambda _, __: EMPTY)),
    lambda l, il, r, ir: bind(lam, [('word', W), ('a', N)],
        word_case(w, lambda _: universe(), EMPTY,
            lambda b, rest: bit_case(b, lambda _: universe(), app(il, rest, a), app(ir, rest, a)))))))
library.define('tree_mass', lam('tree', Tree, tree_induct(t, lambda _: Q,
    fraction(Z, ONE), lambda _: one, lambda l, il, r, ir: blend(il, ir))))

# The accepted language is the tree's actual decoder relation, not an external
# PrefixCodec.exact assumption. Arbitrary suffixes are preserved.
def decoding_goal(tree):
    return bind(pi, [('word', W), ('a', N), ('tail', W), ('accepted', accept(tree, w, a))],
        identity(Result, consume(tree, append(w, tail)), success(a, tail)))
def impossible(ty, proof):
    return induct(proof, lam('_impossible', EMPTY, ty))
def stop_proof(label):
    def goal(word):
        return bind(pi, [('a', N), ('tail', W), ('accepted', accept(leaf(label), word, a))],
                    identity(Result, consume(leaf(label), append(word, tail)), success(a, tail)))
    base = bind(lam, [('a', N), ('tail', W), ('accepted', eq(label, a))],
        Path(label, a, var('accepted')).under(lambda z: success(z, tail), carrier=Result).proof)
    return lam('word', W, word_case(w, goal, base,
        lambda b, rest: bind(lam, [('a', N), ('tail', W), ('accepted', EMPTY)],
            impossible(identity(Result, consume(leaf(label), append(cons(B,b,rest), tail)), success(a,tail)), var('accepted')))))
def branch_proof(left, il, right, ir):
    tree = branch(left,right)
    def goal(word):
        return bind(pi, [('a', N), ('tail', W), ('accepted', accept(tree,word,a))],
            identity(Result, consume(tree,append(word,tail)),success(a,tail)))
    base = bind(lam, [('a', N), ('tail', W), ('accepted', EMPTY)],
        impossible(identity(Result, consume(tree,tail),success(a,tail)),var('accepted')))
    def step(b, rest):
        def choice(bit):
            return goal(cons(B,bit,rest))
        return bit_case(b,choice,
            bind(lam,[('a',N),('tail',W),('accepted',accept(left,rest,a))],app(il,rest,a,tail,var('accepted'))),
            bind(lam,[('a',N),('tail',W),('accepted',accept(right,rest,a))],app(ir,rest,a,tail,var('accepted'))))
    return lam('word',W,word_case(w,goal,base,step))
library.prove('decode_preserves_suffix', [('tree',Tree)],
    tree_induct(t,decoding_goal,
        bind(lam,[('word',W),('a',N),('tail',W),('accepted',EMPTY)],
            impossible(identity(Result,failure,success(a,tail)),var('accepted'))),
        stop_proof,branch_proof),decoding_goal(t))

# A positive denominator is proved independently from the order bound.
library.prove('tree_mass_positive', [('tree',Tree)],
    tree_induct(t,lambda x:positive(mass(x)),tt(),lambda _:tt(),
        lambda l,il,r,ir:call('mul_positive',S(ONE),mul(den(mass(l)),den(mass(r))),tt(),
            call('mul_positive',den(mass(l)),den(mass(r)),il,ir))),positive(mass(t)))
def bound_step(l,il,r,ir):
    ml,mr=mass(l),mass(r)
    dl,dr=den(ml),den(mr)
    total=add(mul(num(ml),dr),mul(num(mr),dl))
    bound=add(mul(dl,dr),mul(dr,dl))
    return change_order(total,bound,
        call('add_le',mul(num(ml),dr),mul(dl,dr),mul(num(mr),dl),mul(dr,dl),
             call('mul_le_right',dr,num(ml),dl,il),call('mul_le_right',dl,num(mr),dr,ir)),
        right_path=ring(bound,den(blend(ml,mr))))
library.prove('kraft_bound', [('tree',Tree)],
    tree_induct(t,lambda x:le(num(mass(x)),den(mass(x))),tt(),lambda _:tt(),bound_step),
    le(num(mass(t)),den(mass(t))))

# Concrete fixed-width MSB-first fields. The leaf label is the native accumulator.
width, seed = var('width'),var('seed')
fixed = lambda width,seed:call('fixed_width',width,seed)
library.define('fixed_width', lam('width',N,induction('width',lambda _:arrow(N,Tree),
    lam('seed',N,leaf(seed)),lambda _,ih:lam('seed',N,
        branch(app(ih,add(seed,seed)),app(ih,S(add(seed,seed))))))))
library.prove('fixed_width_kraft', [('width',N),('seed',N)],
    call('kraft_bound',fixed(width,seed)),le(num(mass(fixed(width,seed))),den(mass(fixed(width,seed)))))

# Gamma cutoffs are by the number of leading zeros, not sample outcomes.
# At z zeros the stop bit is followed by exactly z payload bits, starting at 1.
levels, zeros = var('levels'),var('zeros')
gamma = lambda levels,zeros:call('gamma_prefix',levels,zeros)
library.define('gamma_prefix',lam('levels',N,induction('levels',lambda _:arrow(N,Tree),
    lam('zeros',N,reject),lambda _,ih:lam('zeros',N,branch(app(ih,S(zeros)),fixed(zeros,ONE))))))
library.prove('gamma_kraft', [('levels',N),('zeros',N)],
    call('kraft_bound',gamma(levels,zeros)),le(num(mass(gamma(levels,zeros))),den(mass(gamma(levels,zeros)))))

# State-dependent continuation: fixed fields select their own remaining grammar.
# It also composes lexical children and I/U/R words; rejection stays rejection.
f=var('continuation')
follow=lambda tree,f:call('follow',tree,f)
library.define('follow',bind(lam,[('tree',Tree),('continuation',arrow(N,Tree))],
    tree_induct(t,lambda _:Tree,reject,lambda label:app(f,label),lambda l,il,r,ir:branch(il,ir))))
library.prove('dependent_sequence_kraft',[('tree',Tree),('continuation',arrow(N,Tree))],
    call('kraft_bound',follow(t,f)),le(num(mass(follow(t,f))),den(mass(follow(t,f)))))


# Sequencing is derived from the decoder relation, including dependent choices.
x, y, v = map(var, ('middle', 'value', 'next_word'))
def sequence_goal(tree):
    return bind(pi, [('continuation',arrow(N,Tree)),('word',W),('middle',N),('next_word',W),('value',N),
        ('first',accept(tree,w,x)),('next',accept(app(f,x),v,y))],accept(follow(tree,f),append(w,v),y))
def sequence_stop(label):
    def goal(word):
        return bind(pi,[('middle',N),('next_word',W),('value',N),
            ('first',accept(leaf(label),word,x)),('next',accept(app(f,x),v,y))],
            accept(follow(leaf(label),f),append(word,v),y))
    base=bind(lam,[('middle',N),('next_word',W),('value',N),('first',eq(label,x)),('next',accept(app(f,x),v,y))],
        j(lam('_endpoint',N,accept(app(f,var('_endpoint')),v,y)),var('next'),Path(label,x,var('first')).reverse().proof))
    step=lambda b,rest:bind(lam,[('middle',N),('next_word',W),('value',N),('first',EMPTY),('next',accept(app(f,x),v,y))],
        impossible(accept(app(f,label),append(cons(B,b,rest),v),y),var('first')))
    return bind(lam,[('continuation',arrow(N,Tree)),('word',W)],word_case(w,goal,base,step))
def sequence_branch(l,il,r,ir):
    tree=branch(l,r)
    def goal(word):
        return bind(pi,[('middle',N),('next_word',W),('value',N),('first',accept(tree,word,x)),
            ('next',accept(app(f,x),v,y))],accept(follow(tree,f),append(word,v),y))
    base=bind(lam,[('middle',N),('next_word',W),('value',N),('first',EMPTY),('next',accept(app(f,x),v,y))],
        impossible(accept(follow(tree,f),v,y),var('first')))
    def step(b,rest):
        def side(child,ih):
            return bind(lam,[('middle',N),('next_word',W),('value',N),('first',accept(child,rest,x)),
                ('next',accept(app(f,x),v,y))],app(ih,f,rest,x,v,y,var('first'),var('next')))
        return bit_case(b,lambda bit:goal(cons(B,bit,rest)),side(l,il),side(r,ir))
    return bind(lam,[('continuation',arrow(N,Tree)),('word',W)],word_case(w,goal,base,step))
library.prove('accepted_dependent_sequence',[('tree',Tree)],
    tree_induct(t,sequence_goal,
        bind(lam,[('continuation',arrow(N,Tree)),('word',W),('middle',N),('next_word',W),('value',N),
            ('first',EMPTY),('next',accept(app(f,x),v,y))],impossible(EMPTY,var('first'))),
        sequence_stop,sequence_branch),sequence_goal(t))

library.prove('append_empty',[('word',W)],
    induct(w,lam('_w',W,identity(W,append(var('_w'),nil(B)),var('_w'))),refl(nil(B)),
        bind(lam,[('_b',B),('_ws',W),('_ih',identity(W,append(var('_ws'),nil(B)),var('_ws')))],
            Path(append(var('_ws'),nil(B)),var('_ws'),var('_ih'),W).under(lambda z:cons(B,var('_b'),z)).proof)),
    identity(W,append(w,nil(B)),w))
# A word and a proper extension cannot both be accepted, even with other labels.
other, extra, b = var('other'),var('extra'),var('b')
p1=Path(consume(t,append(w,extra)),success(a,extra),call('decode_preserves_suffix',t,w,a,extra,var('first')),Result)
p2=Path(consume(t,append(other,nil(B))),success(b,nil(B)),call('decode_preserves_suffix',t,other,b,nil(B),var('next')),Result)
joined=Path(append(w,extra),other,var('joined'),W).then(
    Path(append(other,nil(B)),other,call('append_empty',other),W).reverse()).under(lambda z:consume(t,z),carrier=Result)
unique=p1.reverse().then(joined).then(p2)
library.prove('accepted_prefix_free',[('tree',Tree),('word',W),('a',N),('other',W),('b',N),('extra',W),
    ('first',accept(t,w,a)),('next',accept(t,other,b)),('joined',identity(W,append(w,extra),other))],
    pair(unique.under(lambda z:first(second(z)),carrier=N).proof,
         unique.under(lambda z:second(second(z)),carrier=W).proof),
    sigma('_same_value',eq(a,b),identity(W,extra,nil(B))))

length=lambda w:call('word_length',w)
pow2=lambda n:call('pow_two',n)
library.define('word_length',lam('word',W,induct(w,lam('_w',W,N),Z,
    bind(lam,[('_b',B),('_tail',W),('_ih',N)],S(var('_ih'))))))
library.define('pow_two',lam('n',N,induction('n',lambda _:N,ONE,lambda _,ih:mul(S(ONE),ih))))
library.prove('word_append_length',[('word',W),('tail',W)],
    induct(w,lam('_w',W,eq(length(append(var('_w'),tail)),add(length(var('_w')),length(tail)))),refl(length(tail)),
        bind(lam,[('_b',B),('_ws',W),('_ih',eq(length(append(var('_ws'),tail)),add(length(var('_ws')),length(tail))))],
            Path(length(append(var('_ws'),tail)),add(length(var('_ws')),length(tail)),var('_ih')).under(S).proof)),
    eq(length(append(w,tail)),add(length(w),length(tail))))
library.prove('pow_two_positive',[('n',N)],
    induction('n',lambda x:le(ONE,pow2(x)),tt(),lambda z,ih:call('mul_positive',S(ONE),pow2(z),tt(),ih)),le(ONE,pow2(n)))
library.prove('pow_two_sum',[('a',N),('n',N)],
    induction('a',lambda x:eq(pow2(add(x,n)),mul(pow2(x),pow2(n))),
        ring(pow2(n),mul(ONE,pow2(n))).proof,
        lambda z,ih:Path(pow2(add(z,n)),mul(pow2(z),pow2(n)),ih).under(lambda q:mul(S(ONE),q)).then(
            law('mul_assoc',mul(mul(S(ONE),pow2(z)),pow2(n)),mul(S(ONE),mul(pow2(z),pow2(n))),S(ONE),pow2(z),pow2(n)).reverse()).proof),
    eq(pow2(add(a,n)),mul(pow2(a),pow2(n))))
weight=lambda w:fraction(ONE,pow2(length(w)))
library.prove('encoded_word_mass_positive',[('word',W)],call('pow_two_positive',length(w)),positive(weight(w)))
library.prove('encoded_word_mass_at_most_one',[('word',W)],
    change_order(ONE,pow2(length(w)),call('pow_two_positive',length(w)),
        ring(ONE,mul(ONE,ONE)),ring(pow2(length(w)),mul(ONE,pow2(length(w))))),order(weight(w),one))
left,right=weight(append(w,tail)),times(weight(w),weight(tail))
denpath=Path(length(append(w,tail)),add(length(w),length(tail)),call('word_append_length',w,tail)).under(pow2).then(
    Path(pow2(add(length(w),length(tail))),mul(pow2(length(w)),pow2(length(tail))),call('pow_two_sum',length(w),length(tail))))
library.prove('encoded_sequence_mass',[('word',W),('tail',W)],
    ring(mul(num(left),den(right)),den(right)).then(denpath.reverse()).then(
        ring(den(left),mul(num(right),den(left)))).proof,equal(left,right))


# Full fixed-width fields exhaust their bit-space. Rejecting out-of-range
# alternatives later does not redistribute the discarded mass.
def fixed_total_goal(width):
    return pi('seed',N,eq(num(mass(fixed(width,seed))),den(mass(fixed(width,seed)))))
def fixed_total_step(width,ih):
    l,r=mass(fixed(width,add(seed,seed))),mass(fixed(width,S(add(seed,seed))))
    path=Path(num(l),den(l),app(ih,add(seed,seed))).under(lambda z:add(mul(z,den(r)),mul(num(r),den(l)))).then(
        Path(num(r),den(r),app(ih,S(add(seed,seed)))).under(lambda z:add(mul(den(l),den(r)),mul(z,den(l))))).then(
        ring(add(mul(den(l),den(r)),mul(den(r),den(l))),den(blend(l,r))))
    return lam('seed',N,path.proof)
library.prove('fixed_width_total',[('width',N),('seed',N)],
    app(induction('width',fixed_total_goal,lam('seed',N,refl(ONE)),fixed_total_step),seed),
    eq(num(mass(fixed(width,seed))),den(mass(fixed(width,seed)))))

def gamma_balance(levels,zeros):
    q=mass(gamma(levels,zeros)); scale=pow2(levels)
    return eq(add(mul(num(q),scale),den(q)),mul(den(q),scale))
def gamma_balance_step(levels,ih):
    l,r=mass(gamma(levels,S(zeros))),mass(fixed(zeros,ONE))
    A,Bd,C,D,P=num(l),den(l),num(r),den(r),pow2(levels)
    two=S(ONE)
    left=add(mul(num(blend(l,r)),mul(two,P)),den(blend(l,r)))
    middle=mul(mul(two,D),add(add(mul(A,P),Bd),mul(Bd,P)))
    changed=add(mul(add(mul(A,D),mul(D,Bd)),mul(two,P)),den(blend(l,r)))
    path=Path(C,D,call('fixed_width_total',zeros,ONE)).under(
        lambda z:add(mul(add(mul(A,D),mul(z,Bd)),mul(two,P)),den(blend(l,r)))).then(
        ring(changed,middle)).then(
        Path(add(mul(A,P),Bd),mul(Bd,P),app(ih,S(zeros))).under(
            lambda z:mul(mul(two,D),add(z,mul(Bd,P))))).then(
        ring(mul(mul(two,D),add(mul(Bd,P),mul(Bd,P))),mul(den(blend(l,r)),mul(two,P))))
    return lam('zeros',N,path.proof)
library.prove('gamma_exact_unspent_mass',[('levels',N),('zeros',N)],
    app(induction('levels',lambda d:pi('zeros',N,gamma_balance(d,zeros)),
        lam('zeros',N,refl(ONE)),gamma_balance_step),zeros),gamma_balance(levels,zeros))

# A field may reject unused ordinals, without changing any accepted word price.
below=lambda x,bound:call('below',x,bound)
bound=var('bound')
library.define('below',lam('a',N,induction('a',lambda _:arrow(N,B),
    lam('bound',N,induction('bound',lambda _:B,F,lambda _,__:T)),
    lambda prior,ih:lam('bound',N,induction('bound',lambda _:B,F,lambda b,_:app(ih,b))))))
limited=lambda width,bound:call('ordinal_field',width,bound)
library.define('ordinal_field',bind(lam,[('width',N),('bound',N)],
    follow(fixed(width,Z),lam('a',N,bit_case(below(a,bound),lambda _:Tree,reject,leaf(a))))))
library.prove('ordinal_field_kraft',[('width',N),('bound',N)],
    call('kraft_bound',limited(width,bound)),le(num(mass(limited(width,bound))),den(mass(limited(width,bound)))))


# The optimistic opening budget bounds, but never replaces, the actual price.
budget=var('budget')
scaled=times(budget,weight(w))
lhs,rhs=mul(num(scaled),den(budget)),mul(num(budget),den(scaled))
base=mul(num(budget),den(budget))
price_bound=change_order(mul(base,ONE),mul(base,pow2(length(w))),
    call('mul_le_left',base,ONE,pow2(length(w)),call('pow_two_positive',length(w))),
    ring(mul(base,ONE),lhs),ring(mul(base,pow2(length(w))),rhs))
library.prove('candidate_mass_bounded_by_opening',[('budget',Q),('defined',positive(budget)),('word',W)],
    pair(call('fraction_mul_positive',budget,weight(w),var('defined'),call('encoded_word_mass_positive',w)),price_bound),
    sigma('_defined_price',positive(scaled),order(scaled,budget)))


# The same exact paid-look schedule used by the native source, over arbitrary
# naturals rather than a truncated machine ordinal. The unspent tail is retained.
plus=lambda l,r:fraction(add(mul(num(l),den(r)),mul(num(r),den(l))),mul(den(l),den(r)))
look_weight=lambda n:fraction(ONE,mul(S(n),S(S(n))))
look_sum=lambda n:call('look_sum',n)
library.define('look_sum',lam('n',N,induction('n',lambda _:Q,fraction(Z,ONE),
    lambda n,ih:plus(ih,look_weight(n)))))
def look_exact(n):
    return equal(look_sum(n),fraction(n,S(n)))
# Factor the arithmetic into checked scalar lemmas before inserting the
# recursive sum. This keeps certificates below the unchanged admission limits.
u,v,z=map(var,('u','v','z'))
for name,params,left,right in [
    ('successor_square',[('n',N)],add(mul(n,S(S(n))),ONE),mul(S(n),S(n))),
    ('factor_mul_plus_one',[('u',N),('v',N),('z',N)],add(mul(mul(u,v),z),v),mul(v,add(mul(u,z),ONE))),
    ('price_product_reorder',[('u',N),('v',N),('z',N)],mul(mul(u,mul(v,v)),z),mul(v,mul(u,mul(v,z)))),
]:
    library.prove(name,params,ring(left,right).proof,eq(left,right))
def look_step_path(A,Bd,n,ih):
    x,y=S(n),S(S(n))
    path=Path(add(Bd,Z),Bd,call('add_zero',Bd)).under(lambda q:add(mul(A,mul(x,y)),q)).then(
        law('mul_assoc',mul(mul(A,x),y),mul(A,mul(x,y)),A,x,y).reverse().under(lambda q:add(q,Bd))).then(
        Path(mul(A,x),mul(n,Bd),ih).under(lambda q:add(mul(q,y),Bd))).then(
        law('factor_mul_plus_one',add(mul(mul(n,Bd),y),Bd),mul(Bd,add(mul(n,y),ONE)),n,Bd,y)).then(
        Path(add(mul(n,y),ONE),mul(x,x),call('successor_square',n)).under(lambda q:mul(Bd,q))).under(
        lambda q:mul(q,y)).then(
        law('price_product_reorder',mul(mul(Bd,mul(x,x)),y),mul(x,mul(Bd,mul(x,y))),Bd,x,y))
    return path.proof
A,Bd=var('old_num'),var('old_den')
updated=plus(fraction(A,Bd),look_weight(n))
library.prove('look_step_algebra',[('old_num',N),('old_den',N),('n',N),
    ('previous',eq(mul(A,S(n)),mul(n,Bd)))],look_step_path(A,Bd,n,var('previous')),
    eq(mul(num(updated),S(S(n))),mul(S(n),den(updated))))
library.prove('paid_look_partial_sum',[('n',N)],
    induction('n',look_exact,refl(Z),lambda n,ih:
        call('look_step_algebra',num(look_sum(n)),den(look_sum(n)),n,ih)),look_exact(n))
library.prove('paid_look_positive',[('n',N)],call('mul_positive',S(n),S(S(n)),tt(),tt()),positive(look_weight(n)))
library.prove('paid_look_sum_positive_denominator',[('n',N)],
    induction('n',lambda n:positive(look_sum(n)),tt(),
        lambda n,ih:call('mul_positive',den(look_sum(n)),den(look_weight(n)),ih,call('paid_look_positive',n))),
    positive(look_sum(n)))
remainder=fraction(ONE,S(n));q=look_sum(n)
conserved=plus(q,remainder)
library.prove('paid_look_exact_remainder',[('n',N)],
    ring(mul(num(conserved),ONE),add(mul(num(q),S(n)),den(q))).then(
        Path(mul(num(q),S(n)),mul(n,den(q)),call('paid_look_partial_sum',n)).under(lambda z:add(z,den(q)))).then(
        ring(add(mul(n,den(q)),den(q)),mul(ONE,den(conserved)))).proof,equal(conserved,one))
ratio=fraction(n,S(n))
ratio_bound=change_order(n,S(n),call('le_step',n,n,call('le_refl',n)),ring(n,mul(n,ONE)),ring(S(n),mul(ONE,S(n))))
library.prove('paid_look_sum_bounded',[('n',N)],
    call('fraction_trans',look_sum(n),ratio,one,tt(),
        call('fraction_equal_le',look_sum(n),ratio,call('paid_look_partial_sum',n)),ratio_bound),order(look_sum(n),one))
tree_bound=change_order(num(mass(t)),den(mass(t)),call('kraft_bound',t),
    ring(num(mass(t)),mul(num(mass(t)),ONE)),ring(den(mass(t)),mul(ONE,den(mass(t)))))
library.prove('prefix_name_look_rectangle_bounded',[('tree',Tree),('n',N)],
    call('fraction_mul_le',mass(t),one,look_sum(n),one,tree_bound,call('paid_look_sum_bounded',n)),
    order(times(mass(t),look_sum(n)),one))


def number(n):
    result=Z
    for _ in range(n):result=S(result)
    return result

def word(bits):
    result=nil(B)
    for b in reversed(bits):result=cons(B,T if b else F,result)
    return result

def cases():
    result=[(name,library.certificate(name),True) for name in library.claims if name not in FOUNDATIONS]
    def case(name,term,ty,ok):
        result.append((name,{'term':lower(library.close(term)),'claimed_type':lower(library.close(ty)),'metadata':{}},ok))
    concrete=gamma(number(3),Z)
    # gamma(5)=00101, retaining the exact payload and arbitrary following bits.
    actual=consume(concrete,word([0,0,1,0,1,1,0]))
    expected=success(number(5),word([1,0]))
    case('gamma_five_with_suffix',refl(actual),identity(Result,actual,expected),True)
    for name,wrong in [('discarded_suffix',success(number(5),nil(B))),
                       ('changed_gamma_value',success(number(4),word([1,0]))),
                       ('truncated_gamma_accepted',success(ONE,nil(B)))]:
        value=consume(concrete,word([0,0,1])) if name.startswith('truncated') else actual
        case(name,refl(value),identity(Result,value,wrong),False)
    invalid_ordinal=consume(limited(number(2),number(3)),word([1,1]))
    case('unused_ordinal_is_rejected',refl(invalid_ordinal),identity(Result,invalid_ordinal,failure),True)
    case('unused_ordinal_is_not_accepted',refl(invalid_ordinal),identity(Result,invalid_ordinal,success(number(3),nil(B))),False)
    case('empty_ordinal_field_rejects_empty_word',refl(consume(limited(Z,Z),nil(B))),identity(Result,consume(limited(Z,Z),nil(B)),failure),True)
    case('rejection_is_not_normalized',refl(Z),eq(num(mass(reject)),den(mass(reject))),False)
    case('gamma_cutoff_is_not_normalized',refl(num(mass(gamma(ONE,Z)))),
         eq(num(mass(gamma(ONE,Z))),den(mass(gamma(ONE,Z)))),False)
    A,item=var('A'),var('item')
    Box=data('ValueBox',[[value_field(A)]])
    box=construct(Box,0,item)
    case('algebraic_type_parameter_preserved',bind(lam,[('A',universe()),('item',A)],refl(box)),
         bind(pi,[('A',universe()),('item',A)],identity(Box,box,box)),True)
    Void=data('Uninhabited',[])
    case('algebraic_empty_elimination',lam('void',Void,induct(var('void'),lam('_v',Void,N))),arrow(Void,N),True)
    first_schema=data('SameLabel',[[]]); second_schema=data('SameLabel',[[],[]])
    case('display_label_cannot_replace_schema',construct(first_schema,0),second_schema,False)
    nominal=fraction(ONE,number(5)); priced=times(nominal,weight(word([0,1])))
    case('opening_bound_is_not_candidate_price',refl(mul(num(priced),den(nominal))),equal(priced,nominal),False)
    case('paid_look_zero_cannot_be_reused',refl(mul(num(look_weight(ONE)),den(look_weight(Z)))),equal(look_weight(ONE),look_weight(Z)),False)
    case('finite_looks_do_not_spend_tail',refl(mul(num(look_sum(ONE)),den(one))),equal(look_sum(ONE),one),False)
    return result
