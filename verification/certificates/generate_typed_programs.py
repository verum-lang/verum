"""Untrusted native certificates for intrinsically typed finite register programs.

Contexts are lists of arbitrary sorts; references carry both context and result
sort. Heterogeneous environments are derived dependent products. Programs use
explicit register arguments, arbitrary pure partial primitives and structural
continuations. Substitution and splicing preserve their entire optional result.

Every helper below builds ordinary portable terms, not checker rules. Definitions
are checked transparent lets. Neither metadata nor the names of these proofs
confer authority. This module does not yet prove the full macro-translation
contract or any compiler/runtime correspondence.
"""
from portable_terms import RECURSIVE, app, arrow, bind, congruence, construct, data, dependent_field, first, identity, indexed_constructor, induct, lam, list_type, local, lower, pair, pi, recursive_field, refl, second, sigma, symm, transitive, tt, unit, universe, value_field, var


# Typed contexts, references, heterogeneous environments and execution
v=var; ap=app
S,T,F,G,rho,jt,g,t,s,r,xs,h,tail,ih=map(v,('S','T','F','G','rho','j','ctx','t','s','r','xs','head','tail','ih'))
L=lambda a:ap(v('List'),a)
N=lambda a:ap(v('nil'),a)
C=lambda a,h,t:ap(v('cons'),a,h,t)
HL=lambda a,f,c:ap(v('HList'),a,f,c)
RF=lambda a,c,t:ap(v('Ref'),a,c,t)
MAP=lambda c:ap(v('mapList'),S,T,rho,c)
HM=lambda c,e:ap(v('mapH'),S,T,rho,F,G,jt,c,e)
GET=lambda a,f,c,t,e,r:ap(v('get'),a,f,c,t,r,e)
RR=lambda c,t,r:ap(v('relabel'),S,T,rho,c,t,r)
params=[('S',universe()),('T',universe()),('rho',arrow(S,T)),('F',arrow(S,universe())),('G',arrow(T,universe())),('j',pi('t',S,arrow(ap(F,t),ap(G,ap(rho,t)))))]
ref_fields=[dependent_field('ctx',L(S)),dependent_field('t',S)]
definitions=[
('List',lam('S',universe(),list_type(S))),
('nil',lam('S',universe(),construct(L(S),0))),
('cons',bind(lam,[('S',universe()),('head',S),('tail',L(S))],construct(L(S),1,h,tail))),
('mapList',bind(lam,[('S',universe()),('T',universe()),('rho',arrow(S,T)),('ctx',L(S))],induct(g,lam('_ctx',L(S),L(T)),N(T),bind(lam,[('head',S),('tail',L(S)),('ih',L(T))],C(T,ap(rho,h),ih))))),
('HList',bind(lam,[('S',universe()),('F',arrow(S,universe())),('ctx',L(S))],induct(g,lam('_ctx',L(S),universe()),unit(),bind(lam,[('head',S),('tail',L(S)),('ih',universe())],sigma('_head',ap(F,h),ih))))),
('Ref',lam('S',universe(),data('Ref',[
 indexed_constructor(ref_fields,[C(S,t,g),t]),
 indexed_constructor(ref_fields+[dependent_field('s',S),recursive_field('r',g,t)],[C(S,s,g),t]),
],indices=[('ctx',L(S)),('t',S)]))),
]
get_motive=lambda c,t:arrow(HL(S,F,c),ap(F,t))
definitions.append(('get',bind(lam,[('S',universe()),('F',arrow(S,universe())),('ctx',L(S)),('t',S),('r',RF(S,g,t))],induct(r,
 bind(lam,[('ctx',L(S)),('t',S),('r',RF(S,g,t))],get_motive(g,t)),
 bind(lam,[('ctx',L(S)),('t',S),('xs',HL(S,F,C(S,t,g)))],first(xs)),
 bind(lam,[('ctx',L(S)),('t',S),('s',S),('r',RF(S,g,t)),('ih',get_motive(g,t)),('xs',HL(S,F,C(S,s,g)))],ap(ih,second(xs)))))))
definitions.append(('relabel',bind(lam,params[:3]+[('ctx',L(S)),('t',S),('r',RF(S,g,t))],induct(r,
 bind(lam,[('ctx',L(S)),('t',S),('r',RF(S,g,t))],RF(T,MAP(g),ap(rho,t))),
 bind(lam,[('ctx',L(S)),('t',S)],construct(ap(v('Ref'),T),0,MAP(g),ap(rho,t))),
 bind(lam,[('ctx',L(S)),('t',S),('s',S),('r',RF(S,g,t)),('ih',RF(T,MAP(g),ap(rho,t)))],construct(ap(v('Ref'),T),1,MAP(g),ap(rho,t),ap(rho,s),ih))))))
maptype=lambda c:arrow(HL(S,F,c),HL(T,G,MAP(c)))
definitions.append(('mapH',bind(lam,params+[('ctx',L(S))],induct(g,lam('ctx',L(S),maptype(g)),
 lam('xs',HL(S,F,N(S)),tt()),
 bind(lam,[('head',S),('tail',L(S)),('ih',maptype(tail)),('xs',HL(S,F,C(S,h,tail)))],pair(ap(jt,h,first(xs)),ap(ih,second(xs))))))))

def free(term,bound=()):
    # Conservatively collect names; closing may retain an unused definition,
    # but lowering and both checkers still validate every selected binding.
    if not isinstance(term,(list,tuple)): return set()
    if term and term[0]=='Var': return {term[1]}
    return set().union(*(free(x) for x in term))

def close(term):
    used=free(term)
    for name,value in reversed(definitions):
        if name in used:
            term=local(name,value,term);used.update(free(value))
    return term

def judgment(name,term,claim):
 return dict(id=name,term=lower(close(term)),claimed_type=lower(close(claim)))

rows=[]
rows.append(judgment('reference-formation',v('Ref'),pi('S',universe(),bind(pi,[('ctx',L(S)),('t',S)],universe()))))
rows.append(judgment('heterogeneous-lookup',v('get'),bind(pi,[('S',universe()),('F',arrow(S,universe())),('ctx',L(S)),('t',S),('r',RF(S,g,t))],get_motive(g,t))))
rows.append(judgment('reference-relabel',v('relabel'),bind(pi,params[:3]+[('ctx',L(S)),('t',S),('r',RF(S,g,t))],RF(T,MAP(g),ap(rho,t)))))
rows.append(judgment('heterogeneous-map',v('mapH'),bind(pi,params+[('ctx',L(S))],maptype(g))))
P, Lit, V, lit, prim, p, paramsv, op, args, rest, x = map(v, ('Prim','Lit','V','literal','primitive','p','params','op','args','rest','x'))
ARG=lambda a,l,c,t:ap(v('Arg'),a,l,c,t)
PR=lambda a,o,l,c,t:ap(v('Program'),a,o,l,c,t)
OPT=lambda a:ap(v('Option'),a)
AF=lambda a,l,c:lam('arg_sort',a,ARG(a,l,c,v('arg_sort')))
prog_params=[('S',universe()),('Prim',arrow(L(S),arrow(S,universe()))),('Lit',arrow(S,universe()))]
sem_params=prog_params+[('V',arrow(S,universe())),('literal',pi('t',S,arrow(ap(Lit,t),ap(V,t)))),('primitive',bind(pi,[('params',L(S)),('s',S),('op',ap(P,paramsv,s)),('args',HL(S,V,paramsv))],OPT(ap(V,s))))]
definitions += [
 ('Option',lam('A',universe(),data('Option',[[],[value_field(v('A'))]]))),
 ('Arg',bind(lam,[('S',universe()),('Lit',arrow(S,universe())),('ctx',L(S)),('t',S)],data('Arg',[[value_field(RF(S,g,t))],[value_field(ap(Lit,t))]]))),
 ('Program',bind(lam,prog_params,data('Program',[
   indexed_constructor(ref_fields+[dependent_field('arg',ARG(S,Lit,g,t))],[g,t]),
   indexed_constructor(ref_fields+[dependent_field('params',L(S)),dependent_field('s',S),dependent_field('op',ap(P,paramsv,s)),dependent_field('args',HL(S,AF(S,Lit,g),paramsv)),recursive_field('rest',C(S,s,g),t)],[g,t])
 ],indices=[('ctx',L(S)),('t',S)]))),
 ('evalArg',bind(lam,[('S',universe()),('Lit',arrow(S,universe())),('V',arrow(S,universe())),('literal',pi('t',S,arrow(ap(Lit,t),ap(V,t)))),('ctx',L(S)),('t',S),('arg',ARG(S,Lit,g,t)),('xs',HL(S,V,g))],
   induct(v('arg'),lam('_arg',ARG(S,Lit,g,t),ap(V,t)),lam('r',RF(S,g,t),GET(S,V,g,t,xs,r)),lam('x',ap(Lit,t),ap(lit,t,x))))),
]
# Map heterogeneous lists without relabeling their index (no propositional map-id cast).
ht=lambda c:arrow(HL(S,F,c),HL(S,G,c))
definitions.append(('hmap',bind(lam,[('S',universe()),('F',arrow(S,universe())),('G',arrow(S,universe())),('j',pi('t',S,arrow(ap(F,t),ap(G,t)))),('ctx',L(S))],
 induct(g,lam('ctx',L(S),ht(g)),lam('xs',HL(S,F,N(S)),tt()),
 bind(lam,[('head',S),('tail',L(S)),('ih',ht(tail)),('xs',HL(S,F,C(S,h,tail)))],pair(ap(jt,h,first(xs)),ap(ih,second(xs))))))))
EA=lambda c,t,a,e:ap(v('evalArg'),S,Lit,V,lit,c,t,a,e)
EA_ARGS=lambda c,ps,aa,e:ap(v('hmap'),S,AF(S,Lit,c),V,bind(lam,[('t',S),('arg',ARG(S,Lit,c,t))],EA(c,t,v('arg'),e)),ps,aa)
run_type=lambda c,t:arrow(HL(S,V,c),OPT(ap(V,t)))
run_motive=bind(lam,[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],run_type(g,t))
run_ret=bind(lam,[('ctx',L(S)),('t',S),('arg',ARG(S,Lit,g,t)),('xs',HL(S,V,g))],construct(OPT(ap(V,t)),1,EA(g,t,v('arg'),xs)))
run_call=bind(lam,[('ctx',L(S)),('t',S),('params',L(S)),('s',S),('op',ap(P,paramsv,s)),('args',HL(S,AF(S,Lit,g),paramsv)),('rest',PR(S,P,Lit,C(S,s,g),t)),('ih',run_type(C(S,s,g),t)),('xs',HL(S,V,g))],
 induct(ap(prim,paramsv,s,op,EA_ARGS(g,paramsv,args,xs)),lam('_result',OPT(ap(V,s)),OPT(ap(V,t))),construct(OPT(ap(V,t)),0),lam('x',ap(V,s),ap(ih,pair(x,xs)))))
definitions.append(('run',bind(lam,sem_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],induct(p,run_motive,run_ret,run_call))))
rows.append(judgment('typed-program-formation',v('Program'),bind(pi,prog_params+[('ctx',L(S)),('t',S)],universe())))
rows.append(judgment('typed-program-execution',v('run'),bind(pi,sem_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],run_type(g,t))))
lookup_law=lambda c,t,r,e:identity(ap(G,ap(rho,t)),
 GET(T,G,MAP(c),ap(rho,t),HM(c,e),RR(c,t,r)),
 ap(jt,t,GET(S,F,c,t,e,r)))
lookup_motive=lambda c,t,r:pi('xs',HL(S,F,c),lookup_law(c,t,r,xs))
lookup_proof=bind(lam,params+[('ctx',L(S)),('t',S),('r',RF(S,g,t))],induct(r,
 bind(lam,[('ctx',L(S)),('t',S),('r',RF(S,g,t))],lookup_motive(g,t,r)),
 bind(lam,[('ctx',L(S)),('t',S),('xs',HL(S,F,C(S,t,g)))],refl(ap(jt,t,first(xs)))),
 bind(lam,[('ctx',L(S)),('t',S),('s',S),('r',RF(S,g,t)),('ih',lookup_motive(g,t,r)),('xs',HL(S,F,C(S,s,g)))],ap(ih,second(xs)))))
lookup_claim=bind(pi,params+[('ctx',L(S)),('t',S),('r',RF(S,g,t))],lookup_motive(g,t,r))
rows.append(judgment('general-typed-register-lookup',lookup_proof,lookup_claim))


# Capture-safe substitution beneath new registers
D,sub,alpha,aa = map(v,('dest','sub','a','arg'))
SUB=lambda c,d:HL(S,AF(S,Lit,d),c)
AS=lambda c,d,sb,t,a:ap(v('argSubst'),S,Lit,c,d,sb,t,a)
AW=lambda c,s,t,a:ap(v('argWeaken'),S,Lit,c,s,t,a)
HC=lambda c,f,g,fn,items:ap(v('hmap'),S,f,g,fn,c,items)
weak_params=[('S',universe()),('Lit',arrow(S,universe())),('ctx',L(S)),('s',S),('t',S),('arg',ARG(S,Lit,g,t))]
weak_body=induct(aa,lam('_a',ARG(S,Lit,g,t),ARG(S,Lit,C(S,s,g),t)),
 lam('r',RF(S,g,t),construct(ARG(S,Lit,C(S,s,g),t),0,construct(ap(v('Ref'),S),1,g,t,s,r))),
 lam('x',ap(Lit,t),construct(ARG(S,Lit,C(S,s,g),t),1,x)))
definitions.append(('argWeaken',bind(lam,weak_params,weak_body)))
subst_params=[('S',universe()),('Lit',arrow(S,universe())),('ctx',L(S)),('dest',L(S)),('sub',SUB(g,D)),('t',S),('arg',ARG(S,Lit,g,t))]
subst_body=induct(aa,lam('_a',ARG(S,Lit,g,t),ARG(S,Lit,D,t)),
 lam('r',RF(S,g,t),GET(S,AF(S,Lit,D),g,t,sub,r)),
 lam('x',ap(Lit,t),construct(ARG(S,Lit,D,t),1,x)))
definitions.append(('argSubst',bind(lam,subst_params,subst_body)))
weaken_all=lambda c,d,s,sb:HC(c,AF(S,Lit,d),AF(S,Lit,C(S,s,d)),bind(lam,[('t',S),('arg',ARG(S,Lit,d,t))],AW(d,s,t,aa)),sb)
lift_sub=lambda c,d,s,sb:pair(construct(ARG(S,Lit,C(S,s,d),s),0,construct(ap(v('Ref'),S),0,d,s)),weaken_all(c,d,s,sb))
subst_type=lambda c,t:bind(pi,[('dest',L(S)),('sub',SUB(c,D))],PR(S,P,Lit,D,t))
subst_ret=bind(lam,[('ctx',L(S)),('t',S),('arg',ARG(S,Lit,g,t)),('dest',L(S)),('sub',SUB(g,D))],
 construct(ap(v('Program'),S,P,Lit),0,D,t,AS(g,D,sub,t,aa)))
subst_call=bind(lam,[('ctx',L(S)),('t',S),('params',L(S)),('s',S),('op',ap(P,paramsv,s)),('args',HL(S,AF(S,Lit,g),paramsv)),('rest',PR(S,P,Lit,C(S,s,g),t)),('ih',subst_type(C(S,s,g),t)),('dest',L(S)),('sub',SUB(g,D))],
 construct(ap(v('Program'),S,P,Lit),1,D,t,paramsv,s,op,
 HC(paramsv,AF(S,Lit,g),AF(S,Lit,D),bind(lam,[('t',S),('arg',ARG(S,Lit,g,t))],AS(g,D,sub,t,aa)),args),
 ap(ih,C(S,s,D),lift_sub(g,D,s,sub))))
definitions.append(('subst',bind(lam,prog_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],
 induct(p,bind(lam,[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],subst_type(g,t)),subst_ret,subst_call))))
rows.append(judgment('typed-program-substitution',v('subst'),bind(pi,prog_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],subst_type(g,t))))



# Lookup, weakening and argument substitution laws
F2,G2,H2,f2,g2,k2=map(v,('F2','G2','H2','f2','g2','k2'))
base_h=[('S',universe()),('F',arrow(S,universe())),('G',arrow(S,universe())),('j',pi('t',S,arrow(ap(F,t),ap(G,t))))]
hget_law=lambda c,t,r,e:identity(ap(G,t),GET(S,G,c,t,HC(c,F,G,jt,e),r),ap(jt,t,GET(S,F,c,t,e,r)))
hget_motive=lambda c,t,r:pi('xs',HL(S,F,c),hget_law(c,t,r,xs))
hget_proof=bind(lam,base_h+[('ctx',L(S)),('t',S),('r',RF(S,g,t))],induct(r,
 bind(lam,[('ctx',L(S)),('t',S),('r',RF(S,g,t))],hget_motive(g,t,r)),
 bind(lam,[('ctx',L(S)),('t',S),('xs',HL(S,F,C(S,t,g)))],refl(ap(jt,t,first(xs)))),
 bind(lam,[('ctx',L(S)),('t',S),('s',S),('r',RF(S,g,t)),('ih',hget_motive(g,t,r)),('xs',HL(S,F,C(S,s,g)))],ap(ih,second(xs)))))
definitions.append(('getMap',hget_proof))
rows.append(judgment('general-heterogeneous-map-lookup',v('getMap'),bind(pi,base_h+[('ctx',L(S)),('t',S),('r',RF(S,g,t))],hget_motive(g,t,r))))

base_eval=[('S',universe()),('Lit',arrow(S,universe())),('V',arrow(S,universe())),('literal',pi('t',S,arrow(ap(Lit,t),ap(V,t))))]
ew_law=lambda c,s,t,a,e,x:identity(ap(V,t),EA(C(S,s,c),t,AW(c,s,t,a),pair(x,e)),EA(c,t,a,e))
ew_motive=lambda c,s,t,a:bind(pi,[('xs',HL(S,V,c)),('x',ap(V,s))],ew_law(c,s,t,a,xs,x))
ew_proof=bind(lam,base_eval+[('ctx',L(S)),('s',S),('t',S),('arg',ARG(S,Lit,g,t))],induct(aa,
 lam('arg',ARG(S,Lit,g,t),ew_motive(g,s,t,aa)),
 bind(lam,[('r',RF(S,g,t)),('xs',HL(S,V,g)),('x',ap(V,s))],refl(GET(S,V,g,t,xs,r))),
 bind(lam,[('l',ap(Lit,t)),('xs',HL(S,V,g)),('x',ap(V,s))],refl(ap(lit,t,v('l'))))))
definitions.append(('evalWeaken',ew_proof))
rows.append(judgment('general-argument-weakening',v('evalWeaken'),bind(pi,base_eval+[('ctx',L(S)),('s',S),('t',S),('arg',ARG(S,Lit,g,t))],ew_motive(g,s,t,aa))))
ES=lambda c,d,sb,e:EA_ARGS(d,c,sb,e)
es_law=lambda c,d,sb,t,a,e:identity(ap(V,t),EA(d,t,AS(c,d,sb,t,a),e),EA(c,t,a,ES(c,d,sb,e)))
es_motive=lambda c,d,sb,t,a:pi('xs',HL(S,V,d),es_law(c,d,sb,t,a,xs))
es_proof=bind(lam,base_eval+[('ctx',L(S)),('dest',L(S)),('sub',SUB(g,D)),('t',S),('arg',ARG(S,Lit,g,t))],induct(aa,
 lam('arg',ARG(S,Lit,g,t),es_motive(g,D,sub,t,aa)),
 bind(lam,[('r',RF(S,g,t)),('xs',HL(S,V,D))],symm(ap(V,t),
 GET(S,V,g,t,ES(g,D,sub,xs),r),EA(D,t,GET(S,AF(S,Lit,D),g,t,sub,r),xs),
 ap(v('getMap'),S,AF(S,Lit,D),V,bind(lam,[('t',S),('arg',ARG(S,Lit,D,t))],EA(D,t,aa,xs)),g,t,r,sub))),
 bind(lam,[('l',ap(Lit,t)),('xs',HL(S,V,D))],refl(ap(lit,t,v('l'))))))
definitions.append(('evalSubstArg',es_proof))
rows.append(judgment('general-argument-substitution',v('evalSubstArg'),bind(pi,base_eval+[('ctx',L(S)),('dest',L(S)),('sub',SUB(g,D)),('t',S),('arg',ARG(S,Lit,g,t))],es_motive(g,D,sub,t,aa))))



# Heterogeneous composition and lifted environments
H,fm,gm,km,exact=map(v,('H','fm','gm','km','exact'))
compose_params=[('S',universe()),('F',arrow(S,universe())),('G',arrow(S,universe())),('H',arrow(S,universe())),
 ('fm',pi('t',S,arrow(ap(F,t),ap(G,t)))),('gm',pi('t',S,arrow(ap(G,t),ap(H,t)))),('km',pi('t',S,arrow(ap(F,t),ap(H,t)))),
 ('exact',bind(pi,[('t',S),('a',ap(F,t))],identity(ap(H,t),ap(gm,t,ap(fm,t,alpha)),ap(km,t,alpha))))]
compose_law=lambda c,e:identity(HL(S,H,c),HC(c,G,H,gm,HC(c,F,G,fm,e)),HC(c,F,H,km,e))
compose_motive=lambda c:pi('xs',HL(S,F,c),compose_law(c,xs))
# HList cons is a dependent pair whose tail carrier does not depend on the head value.
def pair_paths(A,B,C,x1,x2,y1,y2,px,py):
    return transitive(C,pair(x1,y1),pair(x2,y1),pair(x2,y2),
        congruence(A,C,lam('new_head',A,pair(v('new_head'),y1)),x1,x2,px),
        congruence(B,C,lam('new_tail',B,pair(x2,v('new_tail'))),y1,y2,py))
left_head=ap(gm,h,ap(fm,h,first(xs)));right_head=ap(km,h,first(xs))
left_tail=HC(tail,G,H,gm,HC(tail,F,G,fm,second(xs)));right_tail=HC(tail,F,H,km,second(xs))
compose_proof=bind(lam,compose_params+[('ctx',L(S))],induct(g,lam('ctx',L(S),compose_motive(g)),
 lam('xs',HL(S,F,N(S)),refl(tt())),
 bind(lam,[('head',S),('tail',L(S)),('ih',compose_motive(tail)),('xs',HL(S,F,C(S,h,tail)))],
 pair_paths(ap(H,h),HL(S,H,tail),HL(S,H,C(S,h,tail)),left_head,right_head,left_tail,right_tail,
 ap(exact,h,first(xs)),ap(ih,second(xs))))))
definitions.append(('mapCompose',compose_proof))
rows.append(judgment('general-heterogeneous-composition',v('mapCompose'),bind(pi,compose_params+[('ctx',L(S))],compose_motive(g))))

def evaluator(c,e):
 return bind(lam,[('t',S),('arg',ARG(S,Lit,c,t))],EA(c,t,aa,e))
weaken_fn=lambda d,s:bind(lam,[('t',S),('arg',ARG(S,Lit,d,t))],AW(d,s,t,aa))
subst_fn=lambda c,d,sb:bind(lam,[('t',S),('arg',ARG(S,Lit,c,t))],AS(c,d,sb,t,aa))
# Evaluation commutes with lifting a substitution beneath one new register.
lift_law=identity(HL(S,V,C(S,s,g)),ES(C(S,s,g),C(S,s,D),lift_sub(g,D,s,sub),pair(x,xs)),pair(x,ES(g,D,sub,xs)))
lift_tail_proof=ap(v('mapCompose'),S,AF(S,Lit,D),AF(S,Lit,C(S,s,D)),V,
 weaken_fn(D,s),evaluator(C(S,s,D),pair(x,xs)),evaluator(D,xs),
 bind(lam,[('t',S),('arg',ARG(S,Lit,D,t))],ap(v('evalWeaken'),S,Lit,V,lit,D,s,t,aa,xs,x)),g,sub)
lift_proof=bind(lam,base_eval+[('ctx',L(S)),('dest',L(S)),('sub',SUB(g,D)),('s',S),('xs',HL(S,V,D)),('x',ap(V,s))],
 congruence(HL(S,V,g),HL(S,V,C(S,s,g)),lam('new_tail',HL(S,V,g),pair(x,v('new_tail'))),
 ES(g,C(S,s,D),weaken_all(g,D,s,sub),pair(x,xs)),ES(g,D,sub,xs),lift_tail_proof))
definitions.append(('evalLift',lift_proof))
rows.append(judgment('general-lifted-substitution',v('evalLift'),bind(pi,base_eval+[('ctx',L(S)),('dest',L(S)),('sub',SUB(g,D)),('s',S),('xs',HL(S,V,D)),('x',ap(V,s))],lift_law)))



# General execution preservation under substitution
RUN=lambda c,t,p,e:ap(v('run'),S,P,Lit,V,lit,prim,c,t,p,e)
PSUB=lambda c,t,p,d,sb:ap(v('subst'),S,P,Lit,c,t,p,d,sb)
rs_law=lambda c,t,p,d,sb,e:identity(OPT(ap(V,t)),RUN(d,t,PSUB(c,t,p,d,sb),e),RUN(c,t,p,ES(c,d,sb,e)))
rs_motive=lambda c,t,p:bind(pi,[('dest',L(S)),('sub',SUB(c,D)),('xs',HL(S,V,D))],rs_law(c,t,p,D,sub,xs))
rs_ret=bind(lam,[('ctx',L(S)),('t',S),('arg',ARG(S,Lit,g,t)),('dest',L(S)),('sub',SUB(g,D)),('xs',HL(S,V,D))],
 congruence(ap(V,t),OPT(ap(V,t)),lam('result',ap(V,t),construct(OPT(ap(V,t)),1,v('result'))),
 EA(D,t,AS(g,D,sub,t,aa),xs),EA(g,t,aa,ES(g,D,sub,xs)),ap(v('evalSubstArg'),S,Lit,V,lit,g,D,sub,t,aa,xs)))
args_sub=HC(paramsv,AF(S,Lit,g),AF(S,Lit,D),subst_fn(g,D,sub),args)
args_left=EA_ARGS(D,paramsv,args_sub,xs)
args_right=EA_ARGS(g,paramsv,args,ES(g,D,sub,xs))
args_path=ap(v('mapCompose'),S,AF(S,Lit,g),AF(S,Lit,D),V,subst_fn(g,D,sub),evaluator(D,xs),evaluator(g,ES(g,D,sub,xs)),
 bind(lam,[('t',S),('arg',ARG(S,Lit,g,t))],ap(v('evalSubstArg'),S,Lit,V,lit,g,D,sub,t,aa,xs)),paramsv,args)
step_left=ap(prim,paramsv,s,op,args_left);step_right=ap(prim,paramsv,s,op,args_right)
step_path=congruence(HL(S,V,paramsv),OPT(ap(V,s)),lam('actual_args',HL(S,V,paramsv),ap(prim,paramsv,s,op,v('actual_args'))),args_left,args_right,args_path)
left_cont=lambda z:RUN(C(S,s,D),t,PSUB(C(S,s,g),t,rest,C(S,s,D),lift_sub(g,D,s,sub)),pair(z,xs))
right_cont=lambda z:RUN(C(S,s,g),t,rest,pair(z,ES(g,D,sub,xs)))
opt_bind=lambda o,fn:induct(o,lam('_option',OPT(ap(V,s)),OPT(ap(V,t))),construct(OPT(ap(V,t)),0),lam('result',ap(V,s),fn(v('result'))))
left_bind=lambda o:opt_bind(o,left_cont)
right_bind=lambda o:opt_bind(o,right_cont)
# The recursive proof is applied to the actual extended environment.
left_result=left_cont(x)
mid_env=ES(C(S,s,g),C(S,s,D),lift_sub(g,D,s,sub),pair(x,xs))
right_env=pair(x,ES(g,D,sub,xs))
mid_result=RUN(C(S,s,g),t,rest,mid_env)
result_path=transitive(OPT(ap(V,t)),left_result,mid_result,right_cont(x),
 ap(ih,C(S,s,D),lift_sub(g,D,s,sub),pair(x,xs)),
 congruence(HL(S,V,C(S,s,g)),OPT(ap(V,t)),lam('env',HL(S,V,C(S,s,g)),RUN(C(S,s,g),t,rest,v('env'))),mid_env,right_env,
 ap(v('evalLift'),S,Lit,V,lit,g,D,sub,s,xs,x)))
bind_path=induct(step_right,lam('outcome',OPT(ap(V,s)),identity(OPT(ap(V,t)),left_bind(v('outcome')),right_bind(v('outcome')))),refl(construct(OPT(ap(V,t)),0)),lam('x',ap(V,s),result_path))
rs_call=bind(lam,[('ctx',L(S)),('t',S),('params',L(S)),('s',S),('op',ap(P,paramsv,s)),('args',HL(S,AF(S,Lit,g),paramsv)),('rest',PR(S,P,Lit,C(S,s,g),t)),('ih',rs_motive(C(S,s,g),t,rest)),('dest',L(S)),('sub',SUB(g,D)),('xs',HL(S,V,D))],
 transitive(OPT(ap(V,t)),left_bind(step_left),left_bind(step_right),right_bind(step_right),
 congruence(OPT(ap(V,s)),OPT(ap(V,t)),lam('outcome',OPT(ap(V,s)),left_bind(v('outcome'))),step_left,step_right,step_path),bind_path))
rs_proof=bind(lam,sem_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],
 induct(p,bind(lam,[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],rs_motive(g,t,p)),rs_ret,rs_call))
definitions.append(('runSubst',rs_proof))
rows.append(judgment('general-program-substitution-preservation',v('runSubst'),bind(pi,sem_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],rs_motive(g,t,p))))



# Continuation splicing and its general execution law
z,k=map(v,('z','k'))
sp_type=lambda c,t:bind(pi,[('dest',L(S)),('z',S),('sub',SUB(D,c)),('k',PR(S,P,Lit,C(S,t,D),z))],PR(S,P,Lit,c,z))
sp_ret=bind(lam,[('ctx',L(S)),('t',S),('arg',ARG(S,Lit,g,t)),('dest',L(S)),('z',S),('sub',SUB(D,g)),('k',PR(S,P,Lit,C(S,t,D),z))],PSUB(C(S,t,D),z,k,g,pair(aa,sub)))
sp_call=bind(lam,[('ctx',L(S)),('t',S),('params',L(S)),('s',S),('op',ap(P,paramsv,s)),('args',HL(S,AF(S,Lit,g),paramsv)),('rest',PR(S,P,Lit,C(S,s,g),t)),('ih',sp_type(C(S,s,g),t)),('dest',L(S)),('z',S),('sub',SUB(D,g)),('k',PR(S,P,Lit,C(S,t,D),z))],
 construct(ap(v('Program'),S,P,Lit),1,g,z,paramsv,s,op,args,ap(ih,D,z,weaken_all(D,g,s,sub),k)))
sp_def=bind(lam,prog_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],induct(p,
 bind(lam,[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],sp_type(g,t)),sp_ret,sp_call))
definitions.append(('spliceInto',sp_def))
rows.append(judgment('typed-program-splicing',v('spliceInto'),bind(pi,prog_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],sp_type(g,t))))
SP=lambda c,t,p,d,z,sb,k:ap(v('spliceInto'),S,P,Lit,c,t,p,d,z,sb,k)
def obind(A,B,o,fn):
 return induct(o,lam('_option',OPT(A),OPT(B)),construct(OPT(B),0),lam('_bind_value',A,fn(v('_bind_value'))))
sp_right=lambda c,t,p,d,z,sb,k,e:obind(ap(V,t),ap(V,z),RUN(c,t,p,e),lambda a:RUN(C(S,t,d),z,k,pair(a,ES(d,c,sb,e))))
sp_law=lambda c,t,p,d,z,sb,k,e:identity(OPT(ap(V,z)),RUN(c,z,SP(c,t,p,d,z,sb,k),e),sp_right(c,t,p,d,z,sb,k,e))
sp_motive=lambda c,t,p:bind(pi,[('dest',L(S)),('z',S),('sub',SUB(D,c)),('k',PR(S,P,Lit,C(S,t,D),z)),('xs',HL(S,V,c))],sp_law(c,t,p,D,z,sub,k,xs))
sp_pres_ret=bind(lam,[('ctx',L(S)),('t',S),('arg',ARG(S,Lit,g,t)),('dest',L(S)),('z',S),('sub',SUB(D,g)),('k',PR(S,P,Lit,C(S,t,D),z)),('xs',HL(S,V,g))],
 ap(v('runSubst'),S,P,Lit,V,lit,prim,C(S,t,D),z,k,g,pair(aa,sub),xs))
new_ctx=C(S,s,g);new_env=pair(x,xs);new_sub=weaken_all(D,g,s,sub)
new_mapped=ES(D,new_ctx,new_sub,new_env);old_mapped=ES(D,g,sub,xs)
mapped_path=ap(v('mapCompose'),S,AF(S,Lit,g),AF(S,Lit,new_ctx),V,
 weaken_fn(g,s),evaluator(new_ctx,new_env),evaluator(g,xs),
 bind(lam,[('t',S),('arg',ARG(S,Lit,g,t))],ap(v('evalWeaken'),S,Lit,V,lit,g,s,t,aa,xs,x)),D,sub)
cont_result=lambda mapped:obind(ap(V,t),ap(V,z),RUN(new_ctx,t,rest,new_env),lambda a:RUN(C(S,t,D),z,k,pair(a,mapped)))
rec_left=RUN(new_ctx,z,SP(new_ctx,t,rest,D,z,new_sub,k),new_env)
rec_path=transitive(OPT(ap(V,z)),rec_left,cont_result(new_mapped),cont_result(old_mapped),
 ap(ih,D,z,new_sub,k,new_env),
 congruence(HL(S,V,D),OPT(ap(V,z)),lam('mapped',HL(S,V,D),cont_result(v('mapped'))),new_mapped,old_mapped,mapped_path))
source_step=ap(prim,paramsv,s,op,EA_ARGS(g,paramsv,args,xs))
left_eval=lambda outcome:obind(ap(V,s),ap(V,z),outcome,lambda a:RUN(new_ctx,z,SP(new_ctx,t,rest,D,z,new_sub,k),pair(a,xs)))
right_eval=lambda outcome:obind(ap(V,t),ap(V,z),obind(ap(V,s),ap(V,t),outcome,lambda a:RUN(new_ctx,t,rest,pair(a,xs))),lambda a:RUN(C(S,t,D),z,k,pair(a,old_mapped)))
sp_pres_call=bind(lam,[('ctx',L(S)),('t',S),('params',L(S)),('s',S),('op',ap(P,paramsv,s)),('args',HL(S,AF(S,Lit,g),paramsv)),('rest',PR(S,P,Lit,new_ctx,t)),('ih',sp_motive(new_ctx,t,rest)),('dest',L(S)),('z',S),('sub',SUB(D,g)),('k',PR(S,P,Lit,C(S,t,D),z)),('xs',HL(S,V,g))],
 induct(source_step,lam('outcome',OPT(ap(V,s)),identity(OPT(ap(V,z)),left_eval(v('outcome')),right_eval(v('outcome')))),refl(construct(OPT(ap(V,z)),0)),lam('x',ap(V,s),rec_path)))
sp_proof=bind(lam,sem_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],induct(p,
 bind(lam,[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],sp_motive(g,t,p)),sp_pres_ret,sp_pres_call))
definitions.append(('runSplice',sp_proof))
rows.append(judgment('general-program-splicing-preservation',v('runSplice'),bind(pi,sem_params+[('ctx',L(S)),('t',S),('p',PR(S,P,Lit,g,t))],sp_motive(g,t,p))))



def cases():
    result = [(row['id'], {'term': row['term'], 'claimed_type': row['claimed_type'], 'metadata': {}}, True) for row in rows]
    altered = [('discard-register-lookup-induction', bind(lam, params + [('ctx', L(S)), ('t', S), ('r', RF(S, g, t)), ('xs', HL(S, F, g))], refl(ap(jt, t, GET(S, F, g, t, xs, r)))), lookup_claim), ('discard-program-substitution-proof', bind(lam, sem_params + [('ctx', L(S)), ('t', S), ('p', PR(S, P, Lit, g, t)), ('dest', L(S)), ('sub', SUB(g, D)), ('xs', HL(S, V, D))], refl(RUN(g, t, p, ES(g, D, sub, xs)))), bind(pi, sem_params + [('ctx', L(S)), ('t', S), ('p', PR(S, P, Lit, g, t))], rs_motive(g, t, p))), ('discard-program-splicing-proof', bind(lam, sem_params + [('ctx', L(S)), ('t', S), ('p', PR(S, P, Lit, g, t)), ('dest', L(S)), ('z', S), ('sub', SUB(D, g)), ('k', PR(S, P, Lit, C(S, t, D), z)), ('xs', HL(S, V, g))], refl(sp_right(g, t, p, D, z, sub, k, xs))), bind(pi, sem_params + [('ctx', L(S)), ('t', S), ('p', PR(S, P, Lit, g, t))], sp_motive(g, t, p)))]
    for name, proof, goal in altered:
        row = judgment(name, proof, goal)
        result.append((name, {'term': row['term'], 'claimed_type': row['claimed_type'], 'metadata': {}}, False))
    # A failed eager instruction must remain observable even when its result
    # is unused. A successful instruction must not capture an older register.
    sorts, sort = unit(), tt()
    booleans = data('Bool', [[], []])
    false, true = construct(booleans, 0), construct(booleans, 1)
    values = lam('_sort', sorts, booleans)
    operations = bind(lam, [('_params', L(sorts)), ('_sort', sorts)], unit())
    literals = bind(lam, [('_sort', sorts), ('_literal', booleans)], v('_literal'))
    context = C(sorts, sort, N(sorts))
    extended = C(sorts, sort, context)
    owner = ap(v('Program'), sorts, operations, values)
    old_reference = construct(ap(v('Ref'), sorts), 1, context, sort, sort,
                              construct(ap(v('Ref'), sorts), 0, N(sorts), sort))
    returned = construct(ARG(sorts, values, extended, sort), 0, old_reference)
    continuation = construct(owner, 0, extended, sort, returned)
    program = construct(owner, 1, context, sort, N(sorts), sort, tt(), tt(), continuation)
    none = construct(OPT(booleans), 0)
    some_false = construct(OPT(booleans), 1, false)
    some_true = construct(OPT(booleans), 1, true)
    for label, instruction, expected, accepted in [
        ('unused-failed-instruction-remains-failure', none, none, True),
        ('new-register-does-not-capture-input', some_true, some_false, True),
        ('reject-skipped-failed-instruction', none, some_false, False),
        ('reject-captured-input-register', some_true, some_true, False),
    ]:
        semantics = bind(lam, [('_params', L(sorts)), ('_sort', sorts),
                              ('_operation', unit()), ('_args', HL(sorts, values, v('_params')))], instruction)
        actual = ap(v('run'), sorts, operations, values, values, literals, semantics,
                    context, sort, program, pair(false, tt()))
        row = judgment(label, refl(expected), identity(OPT(booleans), actual, expected))
        result.append((label, {'term': row['term'], 'claimed_type': row['claimed_type'], 'metadata': {}}, accepted))
    return result
