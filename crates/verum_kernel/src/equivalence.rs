//! Alpha-equivalence of the named core syntax. Free names retain their identity;
//! bound names are compared by binder position, including lexical shadowing.
use crate::CoreTerm;
use verum_common::{List, Text};

pub(crate) fn alpha_equivalent(left: &CoreTerm, right: &CoreTerm) -> bool {
    compare(left, right, &mut List::new())
}

fn compare<'a>(
    left: &'a CoreTerm,
    right: &'a CoreTerm,
    bound: &mut List<(&'a Text, &'a Text)>,
) -> bool {
    use CoreTerm::*;
    match (left, right) {
        (Var(a), Var(b)) => {
            match (
                bound.iter().rev().position(|(x, _)| *x == a),
                bound.iter().rev().position(|(_, y)| *y == b),
            ) {
                (Some(x), Some(y)) => x == y,
                (None, None) => a == b,
                _ => false,
            }
        }
        (Universe(a), Universe(b)) => a == b,
        (
            Pi {
                binder: a,
                domain: ad,
                codomain: ab,
            },
            Pi {
                binder: b,
                domain: bd,
                codomain: bb,
            },
        )
        | (
            Lam {
                binder: a,
                domain: ad,
                body: ab,
            },
            Lam {
                binder: b,
                domain: bd,
                body: bb,
            },
        )
        | (
            Sigma {
                binder: a,
                fst_ty: ad,
                snd_ty: ab,
            },
            Sigma {
                binder: b,
                fst_ty: bd,
                snd_ty: bb,
            },
        )
        | (
            Refine {
                binder: a,
                base: ad,
                predicate: ab,
            },
            Refine {
                binder: b,
                base: bd,
                predicate: bb,
            },
        ) => {
            if !compare(ad, bd, bound) {
                return false;
            }
            bound.push((a, b));
            let equal = compare(ab, bb, bound);
            bound.pop();
            equal
        }
        (App(a, b), App(x, y))
        | (Pair(a, b), Pair(x, y))
        | (Quotient { base: a, equiv: b }, Quotient { base: x, equiv: y }) => {
            compare(a, x, bound) && compare(b, y, bound)
        }
        (Fst(a), Fst(b))
        | (Snd(a), Snd(b))
        | (Refl(a), Refl(b))
        | (EpsilonOf(a), EpsilonOf(b))
        | (AlphaOf(a), AlphaOf(b))
        | (ModalBox(a), ModalBox(b))
        | (ModalDiamond(a), ModalDiamond(b))
        | (Shape(a), Shape(b))
        | (Flat(a), Flat(b))
        | (Sharp(a), Sharp(b)) => compare(a, b, bound),
        (
            PathTy {
                carrier: a,
                lhs: b,
                rhs: c,
            },
            PathTy {
                carrier: x,
                lhs: y,
                rhs: z,
            },
        )
        | (
            HComp {
                phi: a,
                walls: b,
                base: c,
            },
            HComp {
                phi: x,
                walls: y,
                base: z,
            },
        )
        | (
            Transp {
                path: a,
                regular: b,
                value: c,
            },
            Transp {
                path: x,
                regular: y,
                value: z,
            },
        )
        | (
            QuotIntro {
                value: a,
                base: b,
                equiv: c,
            },
            QuotIntro {
                value: x,
                base: y,
                equiv: z,
            },
        )
        | (
            QuotElim {
                scrutinee: a,
                motive: b,
                case: c,
            },
            QuotElim {
                scrutinee: x,
                motive: y,
                case: z,
            },
        ) => compare(a, x, bound) && compare(b, y, bound) && compare(c, z, bound),
        (
            PathOver {
                motive: a,
                path: b,
                lhs: c,
                rhs: d,
            },
            PathOver {
                motive: x,
                path: y,
                lhs: z,
                rhs: w,
            },
        )
        | (
            Glue {
                carrier: a,
                phi: b,
                fiber: c,
                equiv: d,
            },
            Glue {
                carrier: x,
                phi: y,
                fiber: z,
                equiv: w,
            },
        ) => {
            compare(a, x, bound)
                && compare(b, y, bound)
                && compare(c, z, bound)
                && compare(d, w, bound)
        }
        (Inductive { path: a, args: aa }, Inductive { path: b, args: ba }) => {
            a == b
                && aa.len() == ba.len()
                && aa.iter().zip(ba.iter()).all(|(a, b)| compare(a, b, bound))
        }
        (
            Elim {
                scrutinee: a,
                motive: am,
                cases: ac,
            },
            Elim {
                scrutinee: b,
                motive: bm,
                cases: bc,
            },
        ) => {
            compare(a, b, bound)
                && compare(am, bm, bound)
                && ac.len() == bc.len()
                && ac.iter().zip(bc.iter()).all(|(a, b)| compare(a, b, bound))
        }
        (ModalBigAnd(a), ModalBigAnd(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| compare(a, b, bound))
        }
        (
            Axiom {
                name: a,
                ty: at,
                framework: af,
            },
            Axiom {
                name: b,
                ty: bt,
                framework: bf,
            },
        ) => a == b && af == bf && compare(at, bt, bound),
        (SmtProof(a), SmtProof(b)) => a == b,
        _ => false,
    }
}
