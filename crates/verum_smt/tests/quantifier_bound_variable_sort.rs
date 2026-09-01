//! Regression: the quantifier must bind the variable it declared.
//!
//! `translate_single_{forall,exists}[_with_body]` created the bound
//! variable with `create_typed_const` — carrying its DECLARED sort,
//! `Verum!Color` for `c: Color` — translated the body against it, and
//! then quantified over a DIFFERENT constant chosen by
//!
//!     match (bound_var.as_int(), bound_var.as_bool(), bound_var.as_real()) {
//!         …
//!         _ => { /* Default to Int for unknown types */ Int::new_const(..) }
//!     }
//!
//! An opaque sort answers None to all three, so every non-primitive
//! type took the default: `forall c: Color. P(c)` emitted
//! `(forall ((c Int)) …)` around a body that still mentions a FREE
//! `c : Verum!Color`.  The variant-exhaustiveness constraint was
//! injected correctly and landed on the wrong variable.
//!
//! Int/Bool/Real were unaffected only by coincidence: the re-created
//! constant was identical to the declared one.
//!
//! Measured through `VERUM_DUMP_SMT_DIR` on
//! `exists c: Color. c == Color.Red`, which produced both
//! `(declare-fun c () Verum!Color)` and `(exists ((c Int)) …)` in one
//! benchmark.  Six of the seven theorems in
//! `vcs/specs/L1-core/verification_phase/variant_quantifier_exhaustiveness.vr`
//! failed; the one that passed — `c == Red || c != Red` — is a
//! tautology at any sorts, which is exactly what survives a broken
//! encoding (T1050).

use verum_ast::expr::QuantifierBinding;
use verum_ast::pattern::{Pattern, PatternKind};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, Type};
use verum_ast::{BinOp, Expr, ExprKind};
use verum_common::{Heap, List, Maybe};
use verum_smt::context::Context;
use verum_smt::translate::Translator;

fn ident_expr(name: &str) -> Expr {
    Expr::path(Path::single(Ident::new(name, Span::dummy())))
}

fn path_expr(segments: &[&str]) -> Expr {
    let mut segs: List<verum_ast::PathSegment> = List::new();
    for seg in segments {
        segs.push(verum_ast::PathSegment::Name(Ident::new(*seg, Span::dummy())));
    }
    Expr::path(Path::new(segs, Span::dummy()))
}

fn named_type(name: &str) -> Type {
    Type::new(
        verum_ast::ty::TypeKind::Path(Path::single(Ident::new(name, Span::dummy()))),
        Span::dummy(),
    )
}

fn binding(var: &str, ty: &str) -> QuantifierBinding {
    QuantifierBinding::typed(
        Pattern::new(
            PatternKind::Ident {
                name: Ident::new(var, Span::dummy()),
                by_ref: false,
                mutable: false,
                subpattern: Maybe::None,
            },
            Span::dummy(),
        ),
        named_type(ty),
        Span::dummy(),
    )
}

fn eq(lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op: BinOp::Eq,
            left: Heap::new(lhs),
            right: Heap::new(rhs),
        },
        Span::dummy(),
    )
}

fn one(b: QuantifierBinding) -> List<QuantifierBinding> {
    let mut l = List::new();
    l.push(b);
    l
}

/// The registered variant type's opaque sort must reach the binder.
#[test]
fn exists_over_a_variant_binds_the_variant_sort() {
    let ctx = Context::new();
    let translator = Translator::new(&ctx);
    translator.register_variant_type(
        "Color",
        vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
    );

    let body = eq(ident_expr("c"), path_expr(&["Color", "Red"]));
    let expr = Expr::new(
        ExprKind::Exists {
            bindings: one(binding("c", "Color")),
            body: Heap::new(body),
        },
        Span::dummy(),
    );

    let z3 = translator
        .translate_expr(&expr)
        .expect("`exists c: Color. c == Color.Red` must translate");

    // The rendering here is the SMT-LIB the solver actually receives,
    // not a debug view — asserting on it asserts on the wire form.
    let rendered = format!("{}", z3);
    assert!(
        !rendered.contains("(c Int)"),
        "the binder fell back to Int for an opaque sort:\n{}",
        rendered
    );
    assert!(
        rendered.contains("Verum!Color"),
        "the binder must carry the declared variant sort:\n{}",
        rendered
    );
}

#[test]
fn forall_over_a_variant_binds_the_variant_sort() {
    let ctx = Context::new();
    let translator = Translator::new(&ctx);
    translator.register_variant_type(
        "Color",
        vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
    );

    let body = eq(ident_expr("c"), path_expr(&["Color", "Red"]));
    let expr = Expr::new(
        ExprKind::Forall {
            bindings: one(binding("c", "Color")),
            body: Heap::new(body),
        },
        Span::dummy(),
    );

    let z3 = translator
        .translate_expr(&expr)
        .expect("`forall c: Color. c == Color.Red` must translate");

    let rendered = format!("{}", z3);
    assert!(
        !rendered.contains("(c Int)"),
        "the binder fell back to Int for an opaque sort:\n{}",
        rendered
    );
}

/// Control: a primitive binder is unchanged.  The old code produced
/// the right sort for `Int` by coincidence; the new code must produce
/// it by rule, and nothing about the Int path may have moved.
#[test]
fn forall_over_an_int_still_binds_int() {
    let ctx = Context::new();
    let translator = Translator::new(&ctx);

    let body = eq(ident_expr("n"), ident_expr("n"));
    let expr = Expr::new(
        ExprKind::Forall {
            bindings: one(binding("n", "Int")),
            body: Heap::new(body),
        },
        Span::dummy(),
    );

    let z3 = translator
        .translate_expr(&expr)
        .expect("`forall n: Int. n == n` must translate");

    let rendered = format!("{}", z3);
    assert!(
        rendered.contains("(n Int)"),
        "an Int binder must still be Int:\n{}",
        rendered
    );
}
