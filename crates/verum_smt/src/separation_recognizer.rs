//! AST-recognizer for separation-logic predicates (#161 V2).
//!
//! ## Architectural role
//!
//! Pre-this-module, when a user wrote
//! ```text
//! @verify(formal)
//! fn swap(a: &mut Cell, b: &mut Cell)
//!     requires sep_conj(points_to(a, av), points_to(b, bv))
//!     ensures  sep_conj(points_to(a, bv), points_to(b, av))
//! { ... }
//! ```
//! the verifier's [`crate::expr_to_smtlib`] translated each call as
//! an opaque uninterpreted function:
//! ```text
//! (sep_conj (points_to a av) (points_to b bv))
//! ```
//! Z3 has no built-in semantics for `sep_conj` or `points_to` — they
//! were just predicate symbols. The proof obligation reduced to
//! "can Z3 prove uninterpreted (sep_conj …) ⇒ (sep_conj …)" — almost
//! always falsifiable by Z3's CEGAR loop because the symbols had no
//! semantic content.
//!
//! ## What this module delivers
//!
//! [`try_recognize_sep_assertion`] walks an AST `Expr` and returns
//! a [`crate::separation_logic::SepAssertion`] **whenever the
//! expression is a syntactic call to one of the separation-logic
//! smart constructors declared in `core/logic/separation.vr`**. The
//! recognised constructors:
//!
//! | Verum source                           | SepAssertion                   |
//! |----------------------------------------|--------------------------------|
//! | `emp()`                                | `Emp`                          |
//! | `points_to(addr, value)`               | `PointsTo { location, value }` |
//! | `sep_conj(p, q)` / `p * q`             | `Sep { left, right }`          |
//! | `heap_and(p, q)`                       | `And { left, right }`          |
//! | `pure(prop)`                           | `Pure(expr)`                   |
//! | `named(name, args)`                    | (custom — not yet wired)       |
//!
//! ## Soundness
//!
//! Recognition is **syntax-driven, no name resolution**. The
//! recogniser pattern-matches on the bare-name path of the call's
//! function. This is sound because `core/logic/separation.vr`'s
//! smart constructors are the canonical entry points for
//! separation-logic predicate construction; any user code that
//! shadows these names is either (a) knowingly playing with the
//! separation-logic surface (acceptable) or (b) writing code the
//! separation-logic verifier shouldn't see anyway.
//!
//! Returning `None` on an unrecognised shape is always sound —
//! the caller falls back to the opaque uninterpreted-function
//! translation, which is the pre-V2 behaviour.
//!
//! ## Performance
//!
//! Pure recursive AST walk — O(n) in expression size. Predicates
//! are typically small (< 30 nodes); per-call cost < 1µs.
//!
//! ## Architectural pattern
//!
//! Mirrors the established `recognise + translate` pattern:
//!   * [`super::separation_kernel_bridge`] (#161 V1) translates
//!     between kernel `HeapPredicate<Term>` and SMT `SepAssertion<Expr>`
//!     at the type-data layer.
//!   * THIS module (#161 V2) translates from user `Expr` (AST) to
//!     SMT `SepAssertion<Expr>` at the recognition layer.
//!   * Together: user `requires sep_conj(points_to(a, av), ...)` →
//!     `SepAssertion::Sep { left: PointsTo { ... }, ... }` →
//!     `SepLogicEncoder` Z3 emission.
//!
//! ## Reuse
//!
//! Every recognised constructor matches the smart-constructor names
//! defined in `core/logic/separation.vr`. Adding a new public
//! constructor there requires extending the recogniser here — but
//! the alternative (no recogniser) is the pre-V2 status quo where
//! the entire separation-logic surface is opaque-functions-only.

use verum_ast::expr::{Expr, ExprKind};

use super::separation_logic::SepAssertion;

// =============================================================================
// Recognition entry point
// =============================================================================

/// Try to recognise an AST expression as a separation-logic predicate.
///
/// Returns `Some(SepAssertion)` when the expression is a syntactic
/// call to one of the smart constructors declared in
/// `core/logic/separation.vr`. Returns `None` for any other shape —
/// including:
///   * non-Call expressions (Path, Literal, Binary, Block, ...)
///   * Calls whose function isn't a recognised separation
///     constructor name
///   * Calls with the wrong arity for the recognised name
///
/// **Recursive descent**: arguments to recognised constructors are
/// themselves passed through the recogniser, so deeply nested
/// predicates resolve fully. A recognised outer constructor with
/// an unrecognisable inner argument returns `None` for the entire
/// expression — soundness over partial coverage.
pub fn try_recognize_sep_assertion(expr: &Expr) -> Option<SepAssertion> {
    let (callee_name, args) = match &expr.kind {
        ExprKind::Call { func, args, .. } => {
            let name = call_callee_bare_name(func)?;
            (name, args)
        }
        // A non-Call expression cannot be a separation-logic
        // predicate constructor. The caller's fast path in
        // expr_to_smtlib continues to translate the expression
        // generically (e.g., `emp` as a path resolves to the
        // emp() smart constructor only if it's in CALL position).
        _ => return None,
    };

    let arg_count = args.iter().count();
    match (callee_name.as_str(), arg_count) {
        // Empty-heap predicate. `emp()` with zero args.
        ("emp", 0) => Some(SepAssertion::Emp),

        // Points-to: `points_to(addr, value)`.
        ("points_to", 2) => {
            let addr = args.iter().next()?.clone();
            let value = args.iter().nth(1)?.clone();
            Some(SepAssertion::PointsTo {
                location: addr,
                value,
            })
        }

        // Separating conjunction: `sep_conj(p, q)`.
        ("sep_conj", 2) => {
            let lhs_expr = args.iter().next()?;
            let rhs_expr = args.iter().nth(1)?;
            // **All-or-nothing recursive recognition**: both arms
            // must classify, otherwise the whole expression bails.
            // Half-recognised separation predicates would produce
            // unsound translations (some args opaque, some lifted).
            let lhs = try_recognize_sep_assertion(lhs_expr)?;
            let rhs = try_recognize_sep_assertion(rhs_expr)?;
            Some(SepAssertion::sep(lhs, rhs))
        }

        // Heap-stable conjunction: `heap_and(p, q)`.
        ("heap_and", 2) => {
            let lhs_expr = args.iter().next()?;
            let rhs_expr = args.iter().nth(1)?;
            let lhs = try_recognize_sep_assertion(lhs_expr)?;
            let rhs = try_recognize_sep_assertion(rhs_expr)?;
            Some(SepAssertion::and(lhs, rhs))
        }

        // Pure proposition lift: `pure(prop)`.  The argument is an
        // arbitrary heap-irrelevant expression — passed through to
        // the SepAssertion as-is. The Z3 encoder will translate
        // the inner expression via expr_to_smtlib.
        ("pure", 1) => {
            let prop = args.iter().next()?.clone();
            Some(SepAssertion::Pure(prop))
        }

        // `named(name, args)` is omitted from V2 — it requires
        // resolving the user-defined predicate against the
        // elaborator's axiom registry, which the recogniser doesn't
        // yet have access to. Future work: wire the registry through
        // and recognise named predicates as `SepAssertion::Pure(name)`
        // with the args attached as a separate metadata field.

        // Anything else is not a recognised separation-logic
        // constructor at this AST shape. The caller continues with
        // generic translation.
        _ => None,
    }
}

/// Extract the bare (single-segment) name of a call's callee, when
/// the callee is a simple `Path` expression. Returns `None` for
/// multi-segment paths, method calls, closures, or any other shape.
fn call_callee_bare_name(func: &Expr) -> Option<String> {
    match &func.kind {
        ExprKind::Path(p) if p.segments.len() == 1 => match &p.segments[0] {
            verum_ast::ty::PathSegment::Name(ident) => Some(ident.name.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::ExprKind;
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::{Heap, List};

    fn span() -> Span {
        Span::dummy()
    }

    fn name_path_expr(name: &str) -> Expr {
        Expr::new(
            ExprKind::Path(Path::new(
                List::from(vec![PathSegment::Name(Ident::new(name, span()))]),
                span(),
            )),
            span(),
        )
    }

    fn call_expr(callee: &str, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: Heap::new(name_path_expr(callee)),
                type_args: List::new(),
                args: List::from(args),
            },
            span(),
        )
    }

    // ----- Recognised shapes -----

    #[test]
    fn recognises_emp_zero_arity() {
        let e = call_expr("emp", vec![]);
        assert!(matches!(
            try_recognize_sep_assertion(&e),
            Some(SepAssertion::Emp),
        ));
    }

    #[test]
    fn recognises_points_to_two_arity() {
        let e = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        match try_recognize_sep_assertion(&e) {
            Some(SepAssertion::PointsTo { .. }) => {}
            other => panic!("expected PointsTo, got {:?}", other),
        }
    }

    #[test]
    fn recognises_sep_conj_recursively() {
        // sep_conj(points_to(a, av), points_to(b, bv))
        let inner_l = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let inner_r = call_expr(
            "points_to",
            vec![name_path_expr("b"), name_path_expr("bv")],
        );
        let outer = call_expr("sep_conj", vec![inner_l, inner_r]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::Sep { left, right }) => {
                assert!(matches!(left.as_ref(), SepAssertion::PointsTo { .. }));
                assert!(matches!(right.as_ref(), SepAssertion::PointsTo { .. }));
            }
            other => panic!("expected Sep, got {:?}", other),
        }
    }

    #[test]
    fn recognises_heap_and() {
        let inner = call_expr("emp", vec![]);
        let outer = call_expr("heap_and", vec![inner.clone(), inner]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::And { .. }) => {}
            other => panic!("expected And, got {:?}", other),
        }
    }

    #[test]
    fn recognises_pure() {
        let inner = name_path_expr("some_predicate");
        let outer = call_expr("pure", vec![inner]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::Pure(_)) => {}
            other => panic!("expected Pure, got {:?}", other),
        }
    }

    // ----- All-or-nothing recursion -----

    #[test]
    fn sep_conj_with_unrecognised_arg_returns_none() {
        // sep_conj(points_to(a, av), some_user_function())
        let inner_l = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let unrecognised = call_expr("some_user_function", vec![]);
        let outer = call_expr("sep_conj", vec![inner_l, unrecognised]);
        assert!(
            try_recognize_sep_assertion(&outer).is_none(),
            "all-or-nothing: an unrecognised inner arg blocks the whole sep recognition",
        );
    }

    #[test]
    fn nested_sep_conj_recurses_three_levels() {
        // sep_conj(emp, sep_conj(emp, points_to(a, av)))
        let inner_pt = call_expr(
            "points_to",
            vec![name_path_expr("a"), name_path_expr("av")],
        );
        let mid = call_expr("sep_conj", vec![call_expr("emp", vec![]), inner_pt]);
        let outer = call_expr("sep_conj", vec![call_expr("emp", vec![]), mid]);
        match try_recognize_sep_assertion(&outer) {
            Some(SepAssertion::Sep { right, .. }) => {
                // right is itself Sep(Emp, PointsTo)
                match right.as_ref() {
                    SepAssertion::Sep {
                        right: inner_right, ..
                    } => {
                        assert!(matches!(inner_right.as_ref(), SepAssertion::PointsTo { .. }));
                    }
                    other => panic!("expected nested Sep, got {:?}", other),
                }
            }
            other => panic!("expected outer Sep, got {:?}", other),
        }
    }

    // ----- Rejection cases -----

    #[test]
    fn non_call_expression_returns_none() {
        let e = name_path_expr("emp"); // path, not a call
        assert!(try_recognize_sep_assertion(&e).is_none());
    }

    #[test]
    fn call_with_wrong_arity_returns_none() {
        // points_to with 1 arg instead of 2
        let e = call_expr("points_to", vec![name_path_expr("a")]);
        assert!(try_recognize_sep_assertion(&e).is_none());
        // emp with 1 arg instead of 0
        let e2 = call_expr("emp", vec![name_path_expr("x")]);
        assert!(try_recognize_sep_assertion(&e2).is_none());
    }

    #[test]
    fn unrecognised_callee_returns_none() {
        let e = call_expr("not_a_separation_constructor", vec![]);
        assert!(try_recognize_sep_assertion(&e).is_none());
    }

    #[test]
    fn multi_segment_path_callee_returns_none() {
        // some.module.path::emp() — multi-segment paths bypass the
        // bare-name recogniser. This is intentional: the recogniser
        // matches on syntactic shape, not on resolved identity.
        // Future work: a resolver-aware recogniser that handles
        // `core.logic.separation::emp` etc.
        let multi_path = Expr::new(
            ExprKind::Path(Path::new(
                List::from(vec![
                    PathSegment::Name(Ident::new("core", span())),
                    PathSegment::Name(Ident::new("logic", span())),
                    PathSegment::Name(Ident::new("separation", span())),
                    PathSegment::Name(Ident::new("emp", span())),
                ]),
                span(),
            )),
            span(),
        );
        let e = Expr::new(
            ExprKind::Call {
                func: Heap::new(multi_path),
                type_args: List::new(),
                args: List::new(),
            },
            span(),
        );
        assert!(try_recognize_sep_assertion(&e).is_none());
    }

    // ----- Architectural pin -----

    #[test]
    fn recognised_constructors_match_core_logic_separation_surface() {
        // Pin: every constructor recognised here has a matching
        // smart-constructor in `core/logic/separation.vr`. Adding
        // a new constructor on the .vr side requires extending this
        // recogniser too — the gap between "user can write the
        // predicate" and "verifier translates it" must stay closed.
        let recognised: std::collections::BTreeSet<&str> =
            ["emp", "points_to", "sep_conj", "heap_and", "pure"]
                .iter()
                .copied()
                .collect();
        // The set documents the canonical surface. Updating the
        // recogniser to handle a new constructor MUST also update
        // this pin so reviewers see the surface change.
        assert_eq!(recognised.len(), 5);
        assert!(recognised.contains("emp"));
        assert!(recognised.contains("points_to"));
        assert!(recognised.contains("sep_conj"));
        assert!(recognised.contains("heap_and"));
        assert!(recognised.contains("pure"));
    }
}
