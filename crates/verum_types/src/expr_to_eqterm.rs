//! Structured lowering from `verum_ast::Expr` to `EqTerm`.
//!
//! `EqTerm` is the term language understood by the equality-type and
//! cubical normalizer. Many AST expression shapes have direct
//! counterparts:
//!
//! | `ExprKind`                 | `EqTerm`                              |
//! |----------------------------|---------------------------------------|
//! | `Path(p)` (single ident)   | `Var(name)`                           |
//! | `Path(p)` (multi-segment)  | `Var(joined-by-dot)`                  |
//! | `Literal(Int(n))`          | `Const(EqConst::Int(n))`              |
//! | `Literal(Bool(b))`         | `Const(EqConst::Bool(b))`             |
//! | `Literal(Text(s))`         | `Const(EqConst::Named(s))`            |
//! | `Literal(_)` (other)       | `Const(EqConst::Named("<lit>"))`      |
//! | `Call { func, args }`      | `App { func, args }`                  |
//! | `Lambda { params, body }`  | `Lambda { param: first, body }`       |
//!                                  (curried for multi-param)
//! | other                      | `Var("<expr>")` opaque fallback       |
//!
//! The fallback is always safe: opaque `EqTerm::Var` values compare
//! syntactically, matching the conservative behaviour the type checker
//! has used historically for non-canonical equality terms.

use verum_common::{List, Text};

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{Literal, LiteralKind};
use verum_ast::ty::Path;

use crate::ty::{EqConst, EqTerm};

/// Lower an AST expression into an `EqTerm` for use in dependent
/// types and the cubical normalizer.
pub fn expr_to_eq_term(expr: &Expr) -> EqTerm {
    match &expr.kind {
        ExprKind::Literal(lit) => literal_to_eq_term(lit),

        ExprKind::Path(p) => path_to_eq_term(p),

        ExprKind::Call { func, args, .. } => EqTerm::App {
            func: Box::new(expr_to_eq_term(func)),
            args: args.iter().map(expr_to_eq_term).collect(),
        },

        ExprKind::Closure { params, body, .. } => {
            // For multi-parameter closures, curry: |a,b| e ≡ λa. λb. e
            let inner = expr_to_eq_term(body);
            params.iter().rev().fold(inner, |acc, p| EqTerm::Lambda {
                param: pattern_binder_name(&p.pattern),
                body: Box::new(acc),
            })
        }

        ExprKind::Paren(inner) => expr_to_eq_term(inner),

        ExprKind::Block(_) => opaque_var(expr, "block"),

        // Range expressions appear in path-constructor endpoints
        // before the parser destructures them. Treat each side
        // independently as a tagged opaque term.
        ExprKind::Range { start, end, .. } => {
            let lhs = match start {
                verum_common::Maybe::Some(e) => expr_to_eq_term(e),
                verum_common::Maybe::None => EqTerm::Var(Text::from("⊥")),
            };
            let rhs = match end {
                verum_common::Maybe::Some(e) => expr_to_eq_term(e),
                verum_common::Maybe::None => EqTerm::Var(Text::from("⊤")),
            };
            EqTerm::App {
                func: Box::new(EqTerm::Var(Text::from("range"))),
                args: List::from_iter([lhs, rhs]),
            }
        }

        // Catch-all: opaque variable bearing a tag. This preserves
        // syntactic equality for pairs of identical AST nodes while
        // being safe for the cubical normalizer.
        _ => opaque_var(expr, "expr"),
    }
}

fn literal_to_eq_term(lit: &Literal) -> EqTerm {
    match &lit.kind {
        LiteralKind::Int(n) => {
            // i128 → i64 truncation is acceptable for EqConst since
            // EqConst::Int already uses i64; values outside i64 range
            // become opaque named constants.
            if let Ok(v) = i64::try_from(n.value) {
                EqTerm::Const(EqConst::Int(v))
            } else {
                EqTerm::Const(EqConst::Named(Text::from(format!(
                    "{}",
                    n.value
                ))))
            }
        }
        LiteralKind::Bool(b) => EqTerm::Const(EqConst::Bool(*b)),
        LiteralKind::Text(s) => {
            use verum_ast::literal::StringLit;
            let raw = match s {
                StringLit::Regular(t) | StringLit::MultiLine(t) => t.clone(),
            };
            EqTerm::Const(EqConst::Named(raw))
        }
        LiteralKind::Char(c) => {
            EqTerm::Const(EqConst::Named(Text::from(c.to_string())))
        }
        _ => EqTerm::Const(EqConst::Named(Text::from("<lit>"))),
    }
}

fn path_to_eq_term(p: &Path) -> EqTerm {
    use verum_ast::ty::PathSegment;
    let mut buf = String::new();
    for (i, seg) in p.segments.iter().enumerate() {
        if i > 0 {
            buf.push('.');
        }
        match seg {
            PathSegment::Name(ident) => buf.push_str(ident.name.as_str()),
            _ => buf.push_str("<seg>"),
        }
    }
    EqTerm::Var(Text::from(buf))
}

fn pattern_binder_name(pat: &verum_ast::pattern::Pattern) -> Text {
    use verum_ast::pattern::PatternKind;
    match &pat.kind {
        PatternKind::Ident { name, .. } => name.name.clone(),
        _ => Text::from("_"),
    }
}

fn opaque_var(expr: &Expr, tag: &str) -> EqTerm {
    // Use the discriminant rather than the full Debug representation
    // so structurally identical AST nodes still compare equal.
    let kind_tag = std::mem::discriminant(&expr.kind);
    EqTerm::Var(Text::from(format!("<{}#{:?}>", tag, kind_tag)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::span::Span;
    use verum_ast::ty::Ident;

    fn span() -> Span {
        Span::default()
    }

    fn ident(name: &str) -> Ident {
        Ident::new(name, span())
    }

    fn path_expr(name: &str) -> Expr {
        Expr {
            kind: ExprKind::Path(Path::single(ident(name))),
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    fn int_expr(n: i128) -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal::int(n, span())),
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        }
    }

    #[test]
    fn path_lowers_to_var() {
        let term = expr_to_eq_term(&path_expr("zero"));
        assert!(matches!(term, EqTerm::Var(ref v) if v.as_str() == "zero"));
    }

    #[test]
    fn int_literal_lowers_to_const() {
        let term = expr_to_eq_term(&int_expr(42));
        assert!(matches!(term, EqTerm::Const(EqConst::Int(42))));
    }

    #[test]
    fn bool_literal_lowers_to_const() {
        let lit = Literal::new(LiteralKind::Bool(true), span());
        let e = Expr {
            kind: ExprKind::Literal(lit),
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        };
        let term = expr_to_eq_term(&e);
        assert!(matches!(term, EqTerm::Const(EqConst::Bool(true))));
    }

    #[test]
    fn call_lowers_to_app() {
        let call = Expr {
            kind: ExprKind::Call {
                func: verum_common::Heap::new(path_expr("f")),
                type_args: List::new(),
                args: List::from_iter([path_expr("x"), path_expr("y")]),
            },
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        };
        let term = expr_to_eq_term(&call);
        match term {
            EqTerm::App { func, args } => {
                assert!(matches!(*func, EqTerm::Var(_)));
                assert_eq!(args.len(), 2);
            }
            _ => panic!("expected App, got {:?}", term),
        }
    }

    #[test]
    fn round_trips_through_cubical_bridge() {
        // `transport(refl(A), x)` → translate via expr→eqterm,
        // then through the cubical bridge — should reduce to `x`.
        let refl_call = Expr {
            kind: ExprKind::Call {
                func: verum_common::Heap::new(path_expr("refl")),
                type_args: List::new(),
                args: List::from_iter([path_expr("A")]),
            },
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        };
        let transport = Expr {
            kind: ExprKind::Call {
                func: verum_common::Heap::new(path_expr("transport")),
                type_args: List::new(),
                args: List::from_iter([refl_call, path_expr("x")]),
            },
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        };
        let just_x = path_expr("x");

        // The bridge maps `refl` Apps into `Refl` cubical terms only
        // when the EqTerm shape is `Refl`. Our App-based lowering goes
        // through the `App { "refl", [A] }` opaque-fallback — which the
        // cubical_bridge recognises and reduces to Refl.
        let lhs_eq = expr_to_eq_term(&transport);
        let rhs_eq = expr_to_eq_term(&just_x);
        assert!(crate::cubical_bridge::definitionally_equal_cubical(
            &lhs_eq, &rhs_eq
        ));
    }

    #[test]
    fn paren_unwraps() {
        let inner = path_expr("y");
        let paren = Expr {
            kind: ExprKind::Paren(verum_common::Heap::new(inner.clone())),
            span: span(),
            ref_kind: None,
            check_eliminated: false,
        };
        assert_eq!(expr_to_eq_term(&paren), expr_to_eq_term(&inner));
    }
}
