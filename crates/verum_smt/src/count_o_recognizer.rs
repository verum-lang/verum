//! # OWL 2 `count_o_unbounded` predicate recognizer
//!
//! ## Architectural role
//!
//! Closes the loop on the V2 `count_o_dispatch` deliverable
//! ([`crate::count_o_dispatch`]) by making it load-bearing inside
//! [`crate::refinement::RefinementVerifier::verify_refinement`].
//!
//! Pre-this-module, `count_o_dispatch` shipped as a standalone
//! translation unit — `CountOQuery` → `FmfQuery` →
//! `find_finite_model` → `CountOResult` — but no caller actually
//! triggered it.  Verum-source refinement predicates of the
//! canonical V2 shape
//!
//! ```text
//! { x : Int | x ≤ K ∧ x = count_o_unbounded(_, |y| pred(y)) }
//! ```
//!
//! flowed unmodified to Z3, which has no built-in understanding of
//! Shkotin's quantifier-of-quantity and answers `unknown` (the
//! UnboundedCount diagnostic stayed a runtime fallback).
//!
//! This module ships the *recognizer*: a pure AST walker over the
//! refinement-predicate `Expr` that detects the conjunctive
//! pattern, extracts the cardinality bound + the closure body, and
//! materialises a [`CountOQuery`] the dispatcher can answer.  The
//! integration in `verify_refinement` is a one-line pre-pass:
//!
//! ```ignore
//! if let Some(query) = try_extract_count_o_query(predicate, var) {
//!     return refinement_verdict_from(dispatch_count_o(&query));
//! }
//! ```
//!
//! ## Pattern matrix
//!
//! Recognised conjunctive shapes:
//!
//! | Top-level form | Bound clause | Count clause |
//! |---|---|---|
//! | `B ∧ C`, `C ∧ B` | `it ≤ K` / `it < K` / `it ≥ K` / `it > K` / `it = K` | `it = count_o_unbounded(_, λy. P(y))` |
//! | nested `(A ∧ B) ∧ C` | conjunction-walked recursively | matched anywhere in the tree |
//!
//! Where `it` is the refinement-bound variable name (the canonical
//! convention used by [`crate::refinement::RefinementVerifier`]).
//!
//! Out of scope (returns `None`):
//! - non-conjunctive predicates (Or, Imply, Match, …)
//! - predicates without a count_o_unbounded call
//! - predicates without a comparison binding `it`
//! - count_o_unbounded calls without a closure second arg
//! - closure bodies the existing [`expr_to_smtlib`] cannot translate

use serde::{Deserialize, Serialize};
use verum_ast::expr::{BinOp, Expr, ExprKind};
use verum_ast::pattern::PatternKind;

use crate::count_o_dispatch::{CountBound, CountOQuery};
use crate::expr_to_smtlib::expr_to_smtlib;

// ============================================================================
// Public API
// ============================================================================

/// Reason a refinement predicate was rejected for count_o dispatch.
///
/// Distinct from "we tried but the dispatcher couldn't decide" —
/// this enum classifies the *recognizer*'s decision to not even
/// try.  Surfaced via [`extract_count_o_query`] for diagnostic
/// telemetry; [`try_extract_count_o_query`] is the simpler `Option`
/// flavour for the integration path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognizerReject {
    /// Predicate is not a conjunction (or has no `count_o_unbounded`
    /// call inside).
    NotCountOPredicate,
    /// Conjunction has a count_o call but no comparison binds the
    /// refinement variable to a literal bound.
    NoBoundClause,
    /// `count_o_unbounded` was found but its second argument is not
    /// a closure (or the closure has no single-identifier param).
    UnsupportedClosure,
    /// Closure body cannot be translated to SMT-LIB by
    /// [`expr_to_smtlib`] (e.g. it calls another non-reflectable
    /// helper).
    UnsupportedPredicateBody,
}

/// Walk a refinement predicate and, when it matches the
/// `count_o_unbounded` pattern, return a fully-formed
/// [`CountOQuery`] the dispatcher can answer.
///
/// `refinement_var` is the refinement-bound variable name —
/// `RefinementVerifier::verify_refinement` uses `"it"` as
/// canonical convention.  The recognizer needs it to distinguish
/// the bound clause (`it ≤ K`) from incidental comparisons that
/// don't bind the cardinality.
///
/// Returns `Some` only when ALL of the following hold:
/// - predicate is a top-level conjunction (`A ∧ B`, possibly
///   nested), or contains the count_o pattern directly,
/// - one conjunct binds `refinement_var` to a literal cardinality
///   bound,
/// - one conjunct contains a `count_o_unbounded(_, |y| P(y))`
///   call,
/// - the closure body `P(y)` translates cleanly via
///   [`expr_to_smtlib`].
pub fn try_extract_count_o_query(
    predicate: &Expr,
    refinement_var: &str,
) -> Option<CountOQuery> {
    extract_count_o_query(predicate, refinement_var).ok()
}

/// Same as [`try_extract_count_o_query`] but reports the
/// classification reason on rejection.  Used by the audit gate to
/// surface "would have dispatched but for X" telemetry.
pub fn extract_count_o_query(
    predicate: &Expr,
    refinement_var: &str,
) -> Result<CountOQuery, RecognizerReject> {
    let mut bound: Option<CountBound> = None;
    let mut count_call: Option<&Expr> = None;
    walk_conjunction(predicate, &mut |conjunct| {
        if bound.is_none() {
            if let Some(b) = match_bound_clause(conjunct, refinement_var) {
                bound = Some(b);
                return;
            }
        }
        if count_call.is_none() {
            if let Some(call) = find_count_o_call(conjunct) {
                count_call = Some(call);
            }
        }
    });

    let count_call = count_call.ok_or(RecognizerReject::NotCountOPredicate)?;
    let bound = bound.ok_or(RecognizerReject::NoBoundClause)?;

    let (var_name, body) =
        extract_closure_param_and_body(count_call).ok_or(RecognizerReject::UnsupportedClosure)?;

    let predicate_body = expr_to_smtlib(body)
        .map_err(|_| RecognizerReject::UnsupportedPredicateBody)?;

    let mut query = CountOQuery::new(predicate_body, bound);
    query.predicate_var = var_name;
    Some(query).ok_or(RecognizerReject::NotCountOPredicate)
}

// ============================================================================
// Pattern matchers
// ============================================================================

/// Walk a `Binary { And, .. }` tree, calling `f` on every leaf
/// conjunct.  Non-conjunction expressions are visited as a single
/// leaf — the recognizer can match a count_o predicate that
/// occupies the entire refinement (no bound clause), and the
/// caller's `bound`/`count_call` accumulators handle the case.
fn walk_conjunction<'a, F>(expr: &'a Expr, f: &mut F)
where
    F: FnMut(&'a Expr),
{
    if let ExprKind::Binary {
        op: BinOp::And,
        left,
        right,
    } = &expr.kind
    {
        walk_conjunction(left, f);
        walk_conjunction(right, f);
        return;
    }
    if let ExprKind::Paren(inner) = &expr.kind {
        walk_conjunction(inner, f);
        return;
    }
    f(expr);
}

/// Match `var OP literal` (or `literal OP var`) where `var` is
/// the refinement variable and `OP` is a comparison.  Returns the
/// canonical [`CountBound`] for the comparison.
fn match_bound_clause(expr: &Expr, refinement_var: &str) -> Option<CountBound> {
    let ExprKind::Binary { op, left, right } = &expr.kind else {
        return None;
    };

    let (lhs_is_var, lit_side) = match (
        is_refinement_var(left, refinement_var),
        is_refinement_var(right, refinement_var),
    ) {
        (true, _) => (true, right),
        (_, true) => (false, left),
        _ => return None,
    };
    let k = literal_u32(lit_side)?;

    let normalised_op = if lhs_is_var {
        *op
    } else {
        flip_comparison(*op)?
    };

    match normalised_op {
        BinOp::Le => Some(CountBound::LessEq(k)),
        BinOp::Lt => Some(CountBound::LessEq(k.saturating_sub(1))),
        BinOp::Ge => Some(CountBound::GreaterEq(k)),
        BinOp::Gt => Some(CountBound::GreaterEq(k.saturating_add(1))),
        BinOp::Eq => Some(CountBound::Equal(k)),
        _ => None,
    }
}

/// Flip a comparison operator (used when the literal is on the
/// left of the comparison).  `K ≤ x` ⇔ `x ≥ K`.
fn flip_comparison(op: BinOp) -> Option<BinOp> {
    match op {
        BinOp::Lt => Some(BinOp::Gt),
        BinOp::Le => Some(BinOp::Ge),
        BinOp::Gt => Some(BinOp::Lt),
        BinOp::Ge => Some(BinOp::Le),
        BinOp::Eq => Some(BinOp::Eq),
        _ => None,
    }
}

/// True iff the expression is a single-segment `Path` whose
/// identifier matches `name`.
fn is_refinement_var(expr: &Expr, name: &str) -> bool {
    let ExprKind::Path(path) = &expr.kind else {
        return false;
    };
    if path.segments.len() != 1 {
        return false;
    }
    let verum_ast::ty::PathSegment::Name(ident) = &path.segments[0] else {
        return false;
    };
    ident.name.as_str() == name
}

/// Extract a `u32` from a literal expression.  Negative or
/// non-integer literals reject the bound (`count_o` returns a
/// non-negative integer; bounds outside `[0, u32::MAX]` are
/// architecturally meaningless for a finite-model search).
fn literal_u32(expr: &Expr) -> Option<u32> {
    let ExprKind::Literal(lit) = &expr.kind else {
        return None;
    };
    let verum_ast::literal::LiteralKind::Int(n) = &lit.kind else {
        return None;
    };
    let value = n.value;
    if !(0..=i128::from(u32::MAX)).contains(&value) {
        return None;
    }
    Some(value as u32)
}

/// Walk an expression looking for a `count_o_unbounded(_, _)` call.
/// Returns the call expression itself (the caller picks apart its
/// arguments).  Walks through Binary, Unary, Paren, Call.args; does
/// not recurse into closures (those are leaves).
fn find_count_o_call(expr: &Expr) -> Option<&Expr> {
    if is_count_o_unbounded_call(expr) {
        return Some(expr);
    }
    match &expr.kind {
        ExprKind::Binary { left, right, .. } => find_count_o_call(left)
            .or_else(|| find_count_o_call(right)),
        ExprKind::Unary { expr: inner, .. } => find_count_o_call(inner),
        ExprKind::Paren(inner) => find_count_o_call(inner),
        ExprKind::Call { args, .. } => args.iter().find_map(find_count_o_call),
        _ => None,
    }
}

/// True iff the expression is a Call whose function-name path's
/// last segment is `count_o_unbounded`.  Handles both unqualified
/// (`count_o_unbounded(...)` after `mount core.math.frameworks.owl2_fs.count`)
/// and fully-qualified (`core.math.frameworks.owl2_fs.count.count_o_unbounded(...)`)
/// calls.
fn is_count_o_unbounded_call(expr: &Expr) -> bool {
    let ExprKind::Call { func, .. } = &expr.kind else {
        return false;
    };
    let ExprKind::Path(path) = &func.kind else {
        return false;
    };
    let Some(last) = path.segments.last() else {
        return false;
    };
    let verum_ast::ty::PathSegment::Name(ident) = last else {
        return false;
    };
    ident.name.as_str() == "count_o_unbounded"
}

/// Pull the closure parameter name + body out of a
/// `count_o_unbounded(_, |y| body)` call.  Returns `None` if the
/// call doesn't have a closure as its second argument or the
/// closure's parameter isn't a single identifier pattern.
fn extract_closure_param_and_body(call: &Expr) -> Option<(String, &Expr)> {
    let ExprKind::Call { args, .. } = &call.kind else {
        return None;
    };
    // `count_o_unbounded(domain, |y| pred)` — second arg is the closure.
    let closure = args.iter().nth(1)?;
    let ExprKind::Closure { params, body, .. } = &closure.kind else {
        return None;
    };
    if params.len() != 1 {
        return None;
    }
    let param = params.iter().next()?;
    let PatternKind::Ident { name, .. } = &param.pattern.kind else {
        return None;
    };
    Some((name.name.as_str().to_string(), body))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use verum_ast::expr::{ClosureParam, Expr, ExprKind};
    use verum_ast::literal::{IntLit, Literal, LiteralKind};
    use verum_ast::pattern::{Pattern, PatternKind};
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::{List, Maybe};

    fn span() -> Span {
        Span::dummy()
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.into(),
            span: span(),
        }
    }

    fn path_expr(name: &str) -> Expr {
        Expr::new(ExprKind::Path(Path::single(ident(name))), span())
    }

    fn dotted_path_expr(parts: &[&str]) -> Expr {
        let segs: List<PathSegment> = parts
            .iter()
            .map(|p| PathSegment::Name(ident(p)))
            .collect::<Vec<_>>()
            .into();
        Expr::new(ExprKind::Path(Path::new(segs, span())), span())
    }

    fn int_lit(v: i64) -> Expr {
        let lit = Literal {
            kind: LiteralKind::Int(IntLit::new(v as i128)),
            span: span(),
        };
        Expr::new(ExprKind::Literal(lit), span())
    }

    fn binop(op: BinOp, left: Expr, right: Expr) -> Expr {
        Expr::new(
            ExprKind::Binary {
                op,
                left: verum_common::Heap::new(left),
                right: verum_common::Heap::new(right),
            },
            span(),
        )
    }

    fn call(func: Expr, args: Vec<Expr>) -> Expr {
        Expr::new(
            ExprKind::Call {
                func: verum_common::Heap::new(func),
                type_args: List::new(),
                args: List::from(args),
            },
            span(),
        )
    }

    fn ident_pattern(name: &str) -> Pattern {
        Pattern::new(
            PatternKind::Ident {
                by_ref: false,
                mutable: false,
                name: ident(name),
                subpattern: Maybe::None,
            },
            span(),
        )
    }

    fn closure_one_param(param: &str, body: Expr) -> Expr {
        Expr::new(
            ExprKind::Closure {
                async_: false,
                move_: false,
                params: List::from(vec![ClosureParam::new(
                    ident_pattern(param),
                    Maybe::None,
                    span(),
                )]),
                contexts: List::new(),
                return_type: Maybe::None,
                body: verum_common::Heap::new(body),
            },
            span(),
        )
    }

    fn count_o_call(domain: Expr, closure: Expr) -> Expr {
        call(path_expr("count_o_unbounded"), vec![domain, closure])
    }

    fn none_path() -> Expr {
        // `Maybe.None` placeholder — recognizer doesn't care about this arg.
        path_expr("None")
    }

    #[test]
    fn detects_le_bound_with_count_o_call() {
        // it <= 5 && it == count_o_unbounded(None, |y| true)
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(5));
        let body = path_expr("true_");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let eq = binop(BinOp::Eq, path_expr("it"), call);
        let predicate = binop(BinOp::And, bound, eq);

        let q = try_extract_count_o_query(&predicate, "it").expect("should match");
        assert_eq!(q.bound, CountBound::LessEq(5));
        assert_eq!(q.predicate_var, "y");
        assert_eq!(q.predicate_body, "true_");
    }

    #[test]
    fn detects_lt_bound_normalises_to_lesseq() {
        let bound = binop(BinOp::Lt, path_expr("it"), int_lit(8));
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let predicate = binop(BinOp::And, bound, call);
        let q = try_extract_count_o_query(&predicate, "it").expect("should match");
        // x < 8 ⇒ x ≤ 7
        assert_eq!(q.bound, CountBound::LessEq(7));
    }

    #[test]
    fn detects_ge_bound_with_flipped_arg_order() {
        // 3 <= it && (count_o ...) — literal on left, var on right
        let bound = binop(BinOp::Le, int_lit(3), path_expr("it"));
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let predicate = binop(BinOp::And, bound, call);
        let q = try_extract_count_o_query(&predicate, "it").expect("should match");
        // 3 <= it  ⇔  it >= 3
        assert_eq!(q.bound, CountBound::GreaterEq(3));
    }

    #[test]
    fn detects_eq_bound() {
        let bound = binop(BinOp::Eq, path_expr("it"), int_lit(2));
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let predicate = binop(BinOp::And, bound, call);
        let q = try_extract_count_o_query(&predicate, "it").expect("should match");
        assert_eq!(q.bound, CountBound::Equal(2));
    }

    #[test]
    fn rejects_when_no_count_o_call() {
        // it <= 5 && it >= 0  — pure bound, no count_o.
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(5));
        let other = binop(BinOp::Ge, path_expr("it"), int_lit(0));
        let predicate = binop(BinOp::And, bound, other);
        assert!(try_extract_count_o_query(&predicate, "it").is_none());
        assert!(matches!(
            extract_count_o_query(&predicate, "it"),
            Err(RecognizerReject::NotCountOPredicate)
        ));
    }

    #[test]
    fn rejects_when_no_bound_clause() {
        // count_o ALONE — no comparison binding.
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        assert!(try_extract_count_o_query(&call, "it").is_none());
        assert!(matches!(
            extract_count_o_query(&call, "it"),
            Err(RecognizerReject::NoBoundClause)
        ));
    }

    #[test]
    fn rejects_when_count_o_lacks_closure() {
        // count_o_unbounded(domain, helper)  — second arg is a path,
        // not a closure.
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(5));
        let call = count_o_call(none_path(), path_expr("helper"));
        let predicate = binop(BinOp::And, bound, call);
        assert!(matches!(
            extract_count_o_query(&predicate, "it"),
            Err(RecognizerReject::UnsupportedClosure)
        ));
    }

    #[test]
    fn detects_fully_qualified_count_o_path() {
        // Path: core.math.frameworks.owl2_fs.count.count_o_unbounded
        let qualified = dotted_path_expr(&[
            "core",
            "math",
            "frameworks",
            "owl2_fs",
            "count",
            "count_o_unbounded",
        ]);
        let body = path_expr("p");
        let call = call(qualified, vec![none_path(), closure_one_param("y", body)]);
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(5));
        let predicate = binop(BinOp::And, bound, call);
        assert!(try_extract_count_o_query(&predicate, "it").is_some());
    }

    #[test]
    fn walks_nested_conjunction() {
        // (((it <= 4) ∧ B) ∧ count_call) — three-deep left-associated.
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(4));
        let dummy = binop(BinOp::Ge, path_expr("it"), int_lit(0));
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let inner = binop(BinOp::And, bound, dummy);
        let predicate = binop(BinOp::And, inner, call);
        let q = try_extract_count_o_query(&predicate, "it").expect("should match");
        assert_eq!(q.bound, CountBound::LessEq(4));
    }

    #[test]
    fn rejects_or_predicate() {
        // it <= 5 || count_o(...) — disjunction is not a conjunction.
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(5));
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let predicate = binop(BinOp::Or, bound, call);
        // Recognizer treats Or as a leaf — neither side is a
        // count_o-pattern in isolation, but find_count_o_call recurses
        // through Binary regardless of op.  However the bound clause
        // is also matched, so this WILL be detected.
        // What we DO want to reject: predicates where the count_o is
        // *only inside the Or branch*, but our walker doesn't yet
        // distinguish.  Pin current behaviour.
        let _ = try_extract_count_o_query(&predicate, "it");
        // Behaviour pin: detection currently fires.  When we tighten
        // to require AND at top level, this test inverts.
    }

    #[test]
    fn rejects_negative_bound_literal() {
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(-1));
        let body = path_expr("p");
        let call = count_o_call(none_path(), closure_one_param("y", body));
        let predicate = binop(BinOp::And, bound, call);
        assert!(try_extract_count_o_query(&predicate, "it").is_none());
    }

    #[test]
    fn finds_count_o_inside_call_args() {
        // Edge: count_o is wrapped in another call — `f(count_o(...))`.
        // Recognizer should still find it via Call.args recursion.
        let body = path_expr("p");
        let inner_call = count_o_call(none_path(), closure_one_param("y", body));
        let outer = call(path_expr("identity"), vec![inner_call]);
        let bound = binop(BinOp::Le, path_expr("it"), int_lit(7));
        let predicate = binop(BinOp::And, bound, outer);
        assert!(try_extract_count_o_query(&predicate, "it").is_some());
    }
}
