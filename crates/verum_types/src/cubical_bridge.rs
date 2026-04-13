//! Bridge between `EqTerm` (propositional-equality term language used
//! throughout the type system) and `CubicalTerm` (the internal cubical
//! normalizer representation).
//!
//! The type checker stores `Type::Eq { lhs, rhs }` where `lhs`/`rhs`
//! are `EqTerm` values. Syntactic equality on `EqTerm` is fast but
//! incomplete — it misses `transport Refl x = x`, path-lambda
//! β-reduction, hcomp collapse, etc.
//!
//! This module provides a translation `eq_to_cubical` that lowers
//! `EqTerm` into `CubicalTerm`, plus a one-shot
//! `definitionally_equal_cubical` predicate that the unifier uses as
//! a more powerful fallback when syntactic comparison fails.
//!
//! ## Mapping
//!
//! | `EqTerm`                             | `CubicalTerm`                         |
//! |--------------------------------------|---------------------------------------|
//! | `Var(v)`                             | `Value(v)`                            |
//! | `Const(c)`                           | `Value(<const-name>)`                 |
//! | `Refl(x)`                            | `Refl(cubical(x))`                    |
//! | `App { "transport", [line, val] }`   | `Transport { line, value }`           |
//! | `App { "hcomp",     [base, sides] }` | `Hcomp { base, sides }`               |
//! | `App { "sym",       [p] }`           | `Sym(p)`                              |
//! | `App { "trans",     [p, q] }`        | `Trans(p, q)`                         |
//! | `App { "path",      [dim, body] }`   | `PathLambda { dim, body }`            |
//! | `App { "at",        [path, pt] }`    | `PathApp { path, at }`                |
//! | `App { func, args }`                 | opaque `Value(func(a1, ..., an))`     |
//! | `Lambda { param, body }`             | `PathLambda { param, body }`          |
//! | `Proj { pair, component }`           | opaque `Value("proj_<c>(<p>)")`       |
//! | `J { proof, motive, base }`          | opaque `Value("J(...)")`              |
//!
//! The opaque fallback is always safe: two opaque values compare
//! syntactically, matching the conservative behaviour of the pre-bridge
//! unifier.

use verum_common::Text;

use crate::cubical::{CubicalTerm, DimVar, IntervalEndpoint};
use crate::ty::{EqConst, EqTerm, ProjComponent};

/// Translate an `EqTerm` into a `CubicalTerm`.
///
/// Terms the cubical core does not model (arbitrary function
/// application, projections, J) become opaque `Value` strings built
/// from a canonical textual representation so that syntactic
/// comparison on the cubical side still succeeds when the originals
/// were structurally identical.
pub fn eq_to_cubical(term: &EqTerm) -> CubicalTerm {
    match term {
        EqTerm::Var(v) => CubicalTerm::Value(v.clone()),

        EqTerm::Const(c) => CubicalTerm::Value(const_to_text(c)),

        EqTerm::Refl(inner) => CubicalTerm::Refl(Box::new(eq_to_cubical(inner))),

        EqTerm::App { func, args } => translate_app(func, args),

        EqTerm::Lambda { param, body } => CubicalTerm::PathLambda {
            dim: DimVar::new(param.clone()),
            body: Box::new(eq_to_cubical(body)),
        },

        EqTerm::Proj { pair, component } => {
            let label = match component {
                ProjComponent::Fst => "fst",
                ProjComponent::Snd => "snd",
            };
            CubicalTerm::Value(Text::from(format!(
                "{}({})",
                label,
                format_eq_term(pair)
            )))
        }

        EqTerm::J {
            proof,
            motive,
            base,
        } => CubicalTerm::Value(Text::from(format!(
            "J({}, {}, {})",
            format_eq_term(proof),
            format_eq_term(motive),
            format_eq_term(base)
        ))),
    }
}

/// Translate an application whose head is a constant with a
/// recognised cubical name. Falls back to an opaque `Value`.
fn translate_app(func: &EqTerm, args: &verum_common::List<EqTerm>) -> CubicalTerm {
    let head = head_name(func);

    match (head.as_deref(), args.len()) {
        (Some("transport"), 2) => CubicalTerm::Transport {
            line: Box::new(eq_to_cubical(&args[0])),
            value: Box::new(eq_to_cubical(&args[1])),
        },

        (Some("hcomp"), 2) => CubicalTerm::Hcomp {
            base: Box::new(eq_to_cubical(&args[0])),
            sides: Box::new(eq_to_cubical(&args[1])),
        },

        (Some("sym"), 1) => CubicalTerm::Sym(Box::new(eq_to_cubical(&args[0]))),

        (Some("trans"), 2) => CubicalTerm::Trans(
            Box::new(eq_to_cubical(&args[0])),
            Box::new(eq_to_cubical(&args[1])),
        ),

        (Some("refl"), 1) => CubicalTerm::Refl(Box::new(eq_to_cubical(&args[0]))),

        (Some("path"), 2) => {
            // `path(dim, body)` — dim is a variable, body is the path body.
            if let EqTerm::Var(dim_name) = &args[0] {
                return CubicalTerm::PathLambda {
                    dim: DimVar::new(dim_name.clone()),
                    body: Box::new(eq_to_cubical(&args[1])),
                };
            }
            opaque_app(func, args)
        }

        (Some("at"), 2) => CubicalTerm::PathApp {
            path: Box::new(eq_to_cubical(&args[0])),
            at: Box::new(eq_to_cubical(&args[1])),
        },

        (Some("i0"), 0) => CubicalTerm::Endpoint(IntervalEndpoint::I0),
        (Some("i1"), 0) => CubicalTerm::Endpoint(IntervalEndpoint::I1),

        _ => opaque_app(func, args),
    }
}

fn opaque_app(func: &EqTerm, args: &verum_common::List<EqTerm>) -> CubicalTerm {
    let head = format_eq_term(func);
    let mut rendered = String::with_capacity(head.len() + 8);
    rendered.push_str(&head);
    rendered.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(&format_eq_term(a));
    }
    rendered.push(')');
    CubicalTerm::Value(Text::from(rendered))
}

fn head_name(term: &EqTerm) -> Option<String> {
    match term {
        EqTerm::Var(v) => Some(v.as_str().to_string()),
        EqTerm::Const(EqConst::Named(n)) => Some(n.as_str().to_string()),
        _ => None,
    }
}

fn const_to_text(c: &EqConst) -> Text {
    match c {
        EqConst::Int(n) => Text::from(format!("{}", n)),
        EqConst::Bool(b) => Text::from(if *b { "true" } else { "false" }),
        EqConst::Nat(n) => Text::from(format!("{}", n)),
        EqConst::Unit => Text::from("()"),
        EqConst::Named(n) => n.clone(),
    }
}

fn format_eq_term(term: &EqTerm) -> String {
    match term {
        EqTerm::Var(v) => v.as_str().to_string(),
        EqTerm::Const(c) => const_to_text(c).as_str().to_string(),
        EqTerm::Refl(x) => format!("refl({})", format_eq_term(x)),
        EqTerm::App { func, args } => {
            let mut s = format_eq_term(func);
            s.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&format_eq_term(a));
            }
            s.push(')');
            s
        }
        EqTerm::Lambda { param, body } => {
            format!("λ({}). {}", param.as_str(), format_eq_term(body))
        }
        EqTerm::Proj { pair, component } => {
            let label = match component {
                ProjComponent::Fst => "fst",
                ProjComponent::Snd => "snd",
            };
            format!("{}({})", label, format_eq_term(pair))
        }
        EqTerm::J {
            proof,
            motive,
            base,
        } => format!(
            "J({}, {}, {})",
            format_eq_term(proof),
            format_eq_term(motive),
            format_eq_term(base)
        ),
    }
}

/// Definitional equality of two `EqTerm`s by way of the cubical
/// normalizer. Both terms are translated to `CubicalTerm`, reduced to
/// WHNF, and compared structurally.
///
/// This is strictly more permissive than syntactic equality on
/// `EqTerm` — every pair that is syntactically equal is also cubically
/// equal, and additional identities like `transport Refl x ≡ x`,
/// `(λi. e) @ j ≡ e[i := j]`, and `sym(refl(x)) ≡ refl(x)` are also
/// accepted.
pub fn definitionally_equal_cubical(lhs: &EqTerm, rhs: &EqTerm) -> bool {
    let c1 = eq_to_cubical(lhs);
    let c2 = eq_to_cubical(rhs);
    c1.definitionally_equal(&c2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::List;

    #[test]
    fn refl_on_var_is_equal_to_itself() {
        let t = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("x"))));
        assert!(definitionally_equal_cubical(&t, &t));
    }

    #[test]
    fn distinct_vars_not_equal() {
        let a = EqTerm::Var(Text::from("a"));
        let b = EqTerm::Var(Text::from("b"));
        assert!(!definitionally_equal_cubical(&a, &b));
    }

    #[test]
    fn transport_refl_reduces_to_value() {
        // transport(refl(A), x)  ≡  x
        let refl_a = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("A"))));
        let x = EqTerm::Var(Text::from("x"));

        let transport = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("transport"))),
            args: List::from_iter([refl_a, x.clone()]),
        };

        assert!(definitionally_equal_cubical(&transport, &x));
    }

    #[test]
    fn sym_refl_reduces_to_refl() {
        // sym(refl(x))  ≡  refl(x)
        let refl_x = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("x"))));
        let sym_refl = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("sym"))),
            args: List::from_iter([refl_x.clone()]),
        };

        assert!(definitionally_equal_cubical(&sym_refl, &refl_x));
    }

    #[test]
    fn hcomp_on_refl_sides_reduces_to_base() {
        // hcomp(base, refl(sides)) ≡ base
        let base = EqTerm::Var(Text::from("base"));
        let refl_sides = EqTerm::Refl(Box::new(EqTerm::Var(Text::from("sides"))));

        let hcomp = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("hcomp"))),
            args: List::from_iter([base.clone(), refl_sides]),
        };

        assert!(definitionally_equal_cubical(&hcomp, &base));
    }

    #[test]
    fn opaque_apps_compare_syntactically() {
        // Unknown function applications still unify when identical.
        let lhs = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("f"))),
            args: List::from_iter([
                EqTerm::Var(Text::from("x")),
                EqTerm::Var(Text::from("y")),
            ]),
        };
        let rhs = lhs.clone();
        assert!(definitionally_equal_cubical(&lhs, &rhs));
    }

    #[test]
    fn opaque_apps_with_different_args_differ() {
        let lhs = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("f"))),
            args: List::from_iter([EqTerm::Var(Text::from("x"))]),
        };
        let rhs = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("f"))),
            args: List::from_iter([EqTerm::Var(Text::from("y"))]),
        };
        assert!(!definitionally_equal_cubical(&lhs, &rhs));
    }

    #[test]
    fn path_lambda_app_beta_reduces() {
        // at(path(i, body), i0) ≡ body[i := i0]
        let body = EqTerm::Var(Text::from("b"));
        let path = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("path"))),
            args: List::from_iter([EqTerm::Var(Text::from("i")), body.clone()]),
        };
        let i0 = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("i0"))),
            args: List::new(),
        };
        let at = EqTerm::App {
            func: Box::new(EqTerm::Var(Text::from("at"))),
            args: List::from_iter([path, i0]),
        };

        // Since `b` has no free occurrence of `i`, subst is a no-op;
        // the reduced form is just `b`.
        assert!(definitionally_equal_cubical(&at, &body));
    }

    #[test]
    fn const_nat_roundtrips() {
        let lhs = EqTerm::Const(EqConst::Nat(42));
        let rhs = EqTerm::Const(EqConst::Nat(42));
        assert!(definitionally_equal_cubical(&lhs, &rhs));

        let other = EqTerm::Const(EqConst::Nat(41));
        assert!(!definitionally_equal_cubical(&lhs, &other));
    }
}
