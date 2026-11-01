"""Universal fixed-alphabet KT subprobability bounds, with no ML premise.

The separate empirical maximum-likelihood lower bound is not established here.
All fractions reuse the actual rising-product definitions of generate_fixed_kt.
"""
import generate_fixed_kt as kt
from generate_fixed_kt import (N, Z, ONE, S, Path, add, mul, le, eq, call,
    change_order, ring, induction, L, Rows, Q, fraction, num, den, times,
    one, order, positive, rise, pure, pure_product, cell, product,
    list_induction, rows_induction)
from peano_arithmetic import Library
from portable_terms import var, tt, refl, lower

library=Library()
library.definitions=kt.library.definitions.copy()
library.claims=kt.library.claims.copy()
FOUNDATIONS=set(library.claims)
a,b,n,k,xs,rows=map(var,('a','b','n','k','xs','rows'))

library.prove('rising_base_monotone',[('a',N),('b',N),('n',N),('ab',le(a,b))],
    induction('n',lambda count:le(rise(a,count),rise(b,count)),tt(),
        lambda count,ih:call('mul_le',rise(a,count),rise(b,count),
            add(a,add(count,count)),add(b,add(count,count)),ih,
            call('add_le_right',add(count,count),a,b,var('ab')))),
    le(rise(a,n),rise(b,n)))

library.prove('pure_kt_at_most_one',[('k',N),('n',N)],
    change_order(rise(ONE,n),rise(S(k),n),
        call('rising_base_monotone',ONE,S(k),n,tt()),
        ring(rise(ONE,n),mul(rise(ONE,n),ONE)),
        ring(rise(S(k),n),mul(ONE,rise(S(k),n)))),order(pure(k,n),one))


def product_step(head,tail,ih):
    return call('fraction_trans',times(pure(k,head),pure_product(k,tail)),times(one,one),one,
        call('fraction_mul_positive',one,one,tt(),tt()),
        call('fraction_mul_le',pure(k,head),one,pure_product(k,tail),one,
            call('pure_kt_at_most_one',k,head),ih),
        call('fraction_equal_le',times(one,one),one,call('fraction_one_left',one)))

library.prove('pure_histogram_at_most_one',[('k',N),('xs',L)],
    list_induction('xs',lambda counts:order(pure_product(k,counts),one),tt(),product_step),
    order(pure_product(k,xs),one))
library.prove('categorical_kt_at_most_one',[('k',N),('xs',L)],
    call('fraction_trans',cell(k,xs),pure_product(k,xs),one,
        call('pure_product_positive',k,xs),call('cell_split',k,xs),
        call('pure_histogram_at_most_one',k,xs)),order(cell(k,xs),one))


def cells_step(head,tail,ih):
    return call('fraction_trans',times(cell(k,head),product(k,tail)),times(one,one),one,
        call('fraction_mul_positive',one,one,tt(),tt()),
        call('fraction_mul_le',cell(k,head),one,product(k,tail),one,
            call('categorical_kt_at_most_one',k,head),ih),
        call('fraction_equal_le',times(one,one),one,call('fraction_one_left',one)))

library.prove('partition_kt_at_most_one',[('k',N),('rows',Rows)],
    rows_induction('rows',lambda partition:order(product(k,partition),one),tt(),cells_step),
    order(product(k,rows),one))


def cases():
    result=[(name,library.certificate(name),True) for name in library.claims if name not in FOUNDATIONS]
    # Two outcomes, one observation: mass is exactly 1/2, not normalized to one.
    half=pure(ONE,ONE)
    result.append(('nontrivial_kt_cannot_be_renormalized',{'term':lower(library.close(refl(mul(num(half),den(one))))),
        'claimed_type':lower(library.close(eq(mul(num(half),den(one)),mul(num(one),den(half))))),'metadata':{}},False))
    return result
