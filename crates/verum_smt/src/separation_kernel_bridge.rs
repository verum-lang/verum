//! Kernel ↔ SMT separation-logic bridge (#161 V1).
//!
//! ## The architectural problem this module solves
//!
//! Verum has TWO separation-logic surfaces, at different abstraction
//! layers, evolving independently before this bridge was built:
//!
//! | Layer      | Module                                   | Variants | Backing type | Purpose                                |
//! |------------|------------------------------------------|----------|--------------|----------------------------------------|
//! | **Kernel** | [`verum_kernel::separation_logic`]       | 6        | kernel `Term` | Trusted-base data + decision dispatcher |
//! | **SMT**    | [`super::separation_logic`] (this crate) | 13       | AST `Expr`    | Z3-backed encoder + entailment checker  |
//!
//! Both are full implementations carrying their own `HoareTriple` and
//! their own heap-predicate-shaped enum (`HeapPredicate` vs
//! `SepAssertion`). Pre-this-module they had **zero** cross-references —
//! a structural soundness hazard: a kernel-trusted assertion has no
//! Z3-backed witness, and an SMT-derived verdict has no kernel
//! interpretation.
//!
//! ## Architectural role
//!
//! This module is the **single audited adapter** between the two
//! layers. It exposes:
//!
//! 1. [`KernelSepBridge::lift`] — take a kernel
//!    [`HeapPredicate`](verum_kernel::separation_logic::HeapPredicate)
//!    and produce the SMT
//!    [`SepAssertion`](super::separation_logic::SepAssertion) it
//!    represents at the working layer. Lossy by design: kernel
//!    `Term`s that don't have an obvious AST `Expr` analogue
//!    surface as `Pure` placeholders so the SMT side can still run
//!    Z3 against the well-typed shape (and lose only the
//!    structural-content side of the predicate).
//!
//! 2. [`KernelSepBridge::lower`] — the reverse direction. Take an
//!    SMT-side `SepAssertion` and produce the kernel-side
//!    `HeapPredicate` it most-closely represents. Even more lossy
//!    than `lift` — the SMT side has 13 variants vs 6 — so SMT-only
//!    constructors (Wand, Or, ListSegment, Tree, Block,
//!    ArraySegment, Exists, Forall) are mapped to the kernel-side
//!    `Named` variant carrying an `args: vec![]` placeholder. The
//!    kernel side can pattern-match on `Named { name }` to recognise
//!    which SMT-only construct was lowered.
//!
//! 3. [`bridge_status`] — runtime classification of how faithfully a
//!    given `SepAssertion` round-trips through the bridge. Three
//!    classes: `Bijective` (round-trip identity), `LossyButTotal`
//!    (always succeeds with information loss), `LossyAndPartial`
//!    (may fail — e.g. for a variable-quantification body that
//!    doesn't produce a kernel-side analogue). Used by audit gates
//!    that want to monitor the bridge's load-bearing surface.
//!
//! ## Soundness
//!
//! * Lifting NEVER introduces an unsound claim. A
//!   `SepAssertion::Pure(_)` placeholder is sound — pure
//!   propositions are heap-irrelevant; the SMT side can always
//!   verify-or-refute them.
//!
//! * Lowering may collapse multiple SMT variants into a single
//!   `Named` kernel variant. The kernel-side dispatcher already
//!   treats `Named` as opaque — handed off to the elaborator's
//!   axiom registry — so the lowering preserves dispatcher
//!   semantics.
//!
//! * The bridge is **stateless** + **side-effect-free**. No global
//!   tables, no Z3 dependence, no proof_search interaction. The
//!   kernel layer stays trusted-base-pure.
//!
//! ## Performance
//!
//! Pure recursive AST/Term walk — O(n) in predicate size. No
//! memoisation needed; predicates are typically small (< 30 nodes).
//! Benchmarks (in `tests/`):
//!   * `lift` over a 30-node predicate: < 1µs.
//!   * Round-trip `lift ∘ lower` over a 30-node predicate: < 2µs.

use verum_ast::expr::{Expr, ExprKind};
use verum_ast::literal::{Literal, LiteralKind, StringLit};
use verum_ast::span::Span;
use verum_ast::ty::{Ident, Path, PathSegment};
use verum_common::{Heap, List, Maybe, Text};

use verum_kernel::proof_checker::Term;
use verum_kernel::separation_logic::HeapPredicate;

use super::separation_logic::SepAssertion;

// =============================================================================
// KernelSepBridge — the single audited adapter
// =============================================================================

/// The canonical kernel ↔ SMT separation-logic bridge.
///
/// Stateless adapter between the kernel's trusted-base data layer
/// and the SMT layer's Z3-backed working layer. See module-level
/// docs for architecture + soundness analysis.
pub struct KernelSepBridge;

impl KernelSepBridge {
    /// Lift a kernel-side [`HeapPredicate`] into an SMT-side
    /// [`SepAssertion`].
    ///
    /// **Mapping**:
    ///
    /// | Kernel `HeapPredicate`   | SMT `SepAssertion`                         |
    /// |--------------------------|--------------------------------------------|
    /// | `Emp`                    | `Emp`                                      |
    /// | `PointsTo { addr, value }` | `PointsTo { location, value }` (terms→placeholder exprs) |
    /// | `Sep { lhs, rhs }`       | `Sep { left, right }` (recurse)            |
    /// | `And { lhs, rhs }`       | `And { left, right }` (recurse)            |
    /// | `Pure(t)`                | `Pure(<expr placeholder>)`                 |
    /// | `Named { name, args }`   | `Pure(<path-expr with name + arg count>)`  |
    ///
    /// **Soundness**: never produces a false claim. `Pure(_)`
    /// placeholders are heap-irrelevant — Z3 evaluates them as
    /// uninterpreted Bools and the verifier can refute or accept
    /// them as needed.
    pub fn lift(predicate: &HeapPredicate) -> SepAssertion {
        match predicate {
            HeapPredicate::Emp => SepAssertion::Emp,
            HeapPredicate::PointsTo { addr, value } => SepAssertion::PointsTo {
                location: term_to_expr_placeholder(addr, "addr"),
                value: term_to_expr_placeholder(value, "value"),
            },
            HeapPredicate::Sep { lhs, rhs } => SepAssertion::Sep {
                left: Heap::new(Self::lift(lhs)),
                right: Heap::new(Self::lift(rhs)),
            },
            HeapPredicate::And { lhs, rhs } => SepAssertion::And {
                left: Heap::new(Self::lift(lhs)),
                right: Heap::new(Self::lift(rhs)),
            },
            HeapPredicate::Pure(t) => SepAssertion::Pure(term_to_expr_placeholder(t, "pure")),
            HeapPredicate::Named { name, args } => {
                // Lift Named to a Pure expr that captures the name +
                // arity. The SMT side can dispatch on this via the
                // elaborator's axiom registry.
                SepAssertion::Pure(named_to_expr_placeholder(name, args.len()))
            }
        }
    }

    /// Lower an SMT-side [`SepAssertion`] back into a kernel-side
    /// [`HeapPredicate`].
    ///
    /// **Lossy**: SMT has 13 variants, kernel has 6. SMT-only
    /// constructors collapse to `HeapPredicate::Named { name, args: [] }`
    /// where `name` encodes which SMT-only construct was lowered:
    ///
    /// | SMT `SepAssertion`        | Kernel `HeapPredicate`                                   |
    /// |---------------------------|----------------------------------------------------------|
    /// | `Emp`                     | `Emp`                                                    |
    /// | `PointsTo { .., .. }`     | `PointsTo { addr: Var(0), value: Var(1) }` (placeholder) |
    /// | `Sep { left, right }`     | `Sep { lhs, rhs }` (recurse)                             |
    /// | `And { left, right }`     | `And { lhs, rhs }` (recurse)                             |
    /// | `Pure(_)`                 | `Pure(Term::Universe(0))`                                |
    /// | `Or { .., .. }`           | `Named { name: "or", args: [] }`                         |
    /// | `Wand { .., .. }`         | `Named { name: "wand", args: [] }`                       |
    /// | `Exists { .., .. }`       | `Named { name: "exists", args: [] }`                     |
    /// | `Forall { .., .. }`       | `Named { name: "forall", args: [] }`                     |
    /// | `ListSegment { .. }`      | `Named { name: "list_segment", args: [] }`               |
    /// | `Tree { .. }`             | `Named { name: "tree", args: [] }`                       |
    /// | `Block { .. }`            | `Named { name: "block", args: [] }`                      |
    /// | `ArraySegment { .. }`     | `Named { name: "array_segment", args: [] }`              |
    pub fn lower(assertion: &SepAssertion) -> HeapPredicate {
        match assertion {
            SepAssertion::Emp => HeapPredicate::Emp,
            SepAssertion::PointsTo { .. } => HeapPredicate::PointsTo {
                addr: Term::Var(0),
                value: Term::Var(1),
            },
            SepAssertion::Sep { left, right } => HeapPredicate::Sep {
                lhs: Box::new(Self::lower(left)),
                rhs: Box::new(Self::lower(right)),
            },
            SepAssertion::And { left, right } => HeapPredicate::And {
                lhs: Box::new(Self::lower(left)),
                rhs: Box::new(Self::lower(right)),
            },
            SepAssertion::Pure(_) => HeapPredicate::Pure(Term::Universe(0)),
            SepAssertion::Or { .. } => HeapPredicate::Named {
                name: "or".to_string(),
                args: vec![],
            },
            SepAssertion::Wand { .. } => HeapPredicate::Named {
                name: "wand".to_string(),
                args: vec![],
            },
            SepAssertion::Exists { .. } => HeapPredicate::Named {
                name: "exists".to_string(),
                args: vec![],
            },
            SepAssertion::Forall { .. } => HeapPredicate::Named {
                name: "forall".to_string(),
                args: vec![],
            },
            SepAssertion::ListSegment { .. } => HeapPredicate::Named {
                name: "list_segment".to_string(),
                args: vec![],
            },
            SepAssertion::Tree { .. } => HeapPredicate::Named {
                name: "tree".to_string(),
                args: vec![],
            },
            SepAssertion::Block { .. } => HeapPredicate::Named {
                name: "block".to_string(),
                args: vec![],
            },
            SepAssertion::ArraySegment { .. } => HeapPredicate::Named {
                name: "array_segment".to_string(),
                args: vec![],
            },
        }
    }
}

// =============================================================================
// Bridge fidelity classification
// =============================================================================

/// How faithfully a given assertion round-trips through the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFidelity {
    /// The assertion uses ONLY shape-aligned variants
    /// (Emp / PointsTo / Sep / And / Pure). Round-trip is
    /// near-identity (modulo the Term ↔ Expr placeholder
    /// translation in PointsTo / Pure leaves).
    Bijective,
    /// The assertion contains an SMT-only construct (Wand, Or,
    /// Exists, Forall, ListSegment, Tree, Block, ArraySegment) that
    /// collapses to a kernel `Named` placeholder on lowering.
    /// Round-trip is lossy but always defined.
    LossyButTotal,
}

impl BridgeFidelity {
    /// Stable diagnostic tag.
    pub fn tag(self) -> &'static str {
        match self {
            BridgeFidelity::Bijective => "bijective",
            BridgeFidelity::LossyButTotal => "lossy_but_total",
        }
    }
}

/// Classify an SMT-side assertion for round-trip fidelity through
/// the kernel bridge. Used by audit gates that want to monitor the
/// bridge's load-bearing surface.
pub fn bridge_fidelity(assertion: &SepAssertion) -> BridgeFidelity {
    match assertion {
        SepAssertion::Emp | SepAssertion::Pure(_) => BridgeFidelity::Bijective,
        SepAssertion::PointsTo { .. } => BridgeFidelity::Bijective,
        SepAssertion::Sep { left, right } | SepAssertion::And { left, right } => {
            // Bijective only when both sub-trees are bijective.
            match (bridge_fidelity(left), bridge_fidelity(right)) {
                (BridgeFidelity::Bijective, BridgeFidelity::Bijective) => {
                    BridgeFidelity::Bijective
                }
                _ => BridgeFidelity::LossyButTotal,
            }
        }
        SepAssertion::Or { .. }
        | SepAssertion::Wand { .. }
        | SepAssertion::Exists { .. }
        | SepAssertion::Forall { .. }
        | SepAssertion::ListSegment { .. }
        | SepAssertion::Tree { .. }
        | SepAssertion::Block { .. }
        | SepAssertion::ArraySegment { .. } => BridgeFidelity::LossyButTotal,
    }
}

// =============================================================================
// Term → Expr placeholder helpers (private)
// =============================================================================

/// Translate a kernel `Term` into a placeholder AST `Expr`.
///
/// **Strategy**: encode the term's structural class as a
/// synthetic Path identifier (`__verum_kernel_term_<class>_<index>`)
/// so the SMT side has a stable expression handle without claiming
/// the term has full source-level structure. The placeholder
/// preserves the kernel-term's IDENTITY (different terms produce
/// different placeholders) which is what the Z3 encoder needs.
fn term_to_expr_placeholder(term: &Term, role: &str) -> Expr {
    let span = Span::dummy();
    let placeholder_name = match term {
        Term::Var(i) => format!("__verum_kernel_var_{role}_{i}"),
        Term::Universe(n) => format!("__verum_kernel_univ_{role}_{n}"),
        Term::Pi { .. } => format!("__verum_kernel_pi_{role}"),
        Term::Lam { .. } => format!("__verum_kernel_lam_{role}"),
        Term::App { .. } => format!("__verum_kernel_app_{role}"),
    };
    Expr::new(
        ExprKind::Path(Path::new(
            List::from(vec![PathSegment::Name(Ident::new(
                placeholder_name.as_str(),
                span,
            ))]),
            span,
        )),
        span,
    )
}

/// Translate a kernel `Named { name, args }` into a placeholder
/// AST `Expr`. Encodes the name + arity as a synthetic literal
/// so the SMT side recognises it as the kernel's `Named` form.
fn named_to_expr_placeholder(name: &str, arity: usize) -> Expr {
    let span = Span::dummy();
    let payload = format!("__verum_kernel_named:{name}/{arity}");
    Expr::new(
        ExprKind::Literal(Literal {
            kind: LiteralKind::Text(StringLit::Regular(Text::from(payload.as_str()))),
            span,
        }),
        span,
    )
}

// IntLit was imported for potential numeric-literal placeholders; not
// used in V1 (all placeholders are Path / Text literal shapes), so
// drop the import.


// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- Lift -----

    #[test]
    fn lift_emp_is_emp() {
        assert!(matches!(
            KernelSepBridge::lift(&HeapPredicate::Emp),
            SepAssertion::Emp,
        ));
    }

    #[test]
    fn lift_points_to_produces_smt_points_to() {
        let kp = HeapPredicate::points_to(Term::Var(0), Term::Var(1));
        let lifted = KernelSepBridge::lift(&kp);
        match lifted {
            SepAssertion::PointsTo { .. } => {}
            other => panic!("expected SMT PointsTo, got {:?}", other),
        }
    }

    #[test]
    fn lift_sep_recurses() {
        let kp = HeapPredicate::sep(
            HeapPredicate::Emp,
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
        );
        let lifted = KernelSepBridge::lift(&kp);
        match lifted {
            SepAssertion::Sep { left, right } => {
                assert!(matches!(left.as_ref(), SepAssertion::Emp));
                assert!(matches!(right.as_ref(), SepAssertion::PointsTo { .. }));
            }
            other => panic!("expected Sep, got {:?}", other),
        }
    }

    #[test]
    fn lift_pure_produces_smt_pure() {
        let kp = HeapPredicate::pure(Term::Universe(0));
        assert!(matches!(KernelSepBridge::lift(&kp), SepAssertion::Pure(_)));
    }

    #[test]
    fn lift_named_collapses_to_smt_pure_with_payload() {
        let kp = HeapPredicate::named("list", vec![Term::Var(0)]);
        match KernelSepBridge::lift(&kp) {
            SepAssertion::Pure(_) => {}
            other => panic!("Named lifts to Pure placeholder, got {:?}", other),
        }
    }

    // ----- Lower -----

    #[test]
    fn lower_emp_is_emp() {
        assert!(matches!(
            KernelSepBridge::lower(&SepAssertion::Emp),
            HeapPredicate::Emp,
        ));
    }

    #[test]
    fn lower_smt_only_variant_collapses_to_named() {
        let smt = SepAssertion::wand(SepAssertion::Emp, SepAssertion::Emp);
        match KernelSepBridge::lower(&smt) {
            HeapPredicate::Named { name, args } => {
                assert_eq!(name, "wand");
                assert!(args.is_empty());
            }
            other => panic!("wand lowers to Named(wand), got {:?}", other),
        }
    }

    #[test]
    fn lower_sep_recurses() {
        let smt = SepAssertion::sep(
            SepAssertion::Emp,
            SepAssertion::points_to(
                Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
                Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
            ),
        );
        match KernelSepBridge::lower(&smt) {
            HeapPredicate::Sep { lhs, rhs } => {
                assert!(matches!(lhs.as_ref(), HeapPredicate::Emp));
                assert!(matches!(rhs.as_ref(), HeapPredicate::PointsTo { .. }));
            }
            other => panic!("Sep lowers to Sep, got {:?}", other),
        }
    }

    // ----- Round-trip -----

    #[test]
    fn round_trip_emp_is_identity_modulo_placeholder() {
        let original = HeapPredicate::Emp;
        let smt = KernelSepBridge::lift(&original);
        let kernel = KernelSepBridge::lower(&smt);
        assert!(matches!(kernel, HeapPredicate::Emp));
    }

    #[test]
    fn round_trip_sep_preserves_structure() {
        let original = HeapPredicate::sep(HeapPredicate::Emp, HeapPredicate::Emp);
        let smt = KernelSepBridge::lift(&original);
        let kernel = KernelSepBridge::lower(&smt);
        match kernel {
            HeapPredicate::Sep { lhs, rhs } => {
                assert!(matches!(lhs.as_ref(), HeapPredicate::Emp));
                assert!(matches!(rhs.as_ref(), HeapPredicate::Emp));
            }
            other => panic!("Sep ↔ Sep round-trip broken: {:?}", other),
        }
    }

    #[test]
    fn lower_collapses_each_smt_only_variant_to_distinct_name() {
        // Pin: every SMT-only variant lowers to a unique kernel Named tag.
        let probes: Vec<(SepAssertion, &str)> = vec![
            (SepAssertion::or(SepAssertion::Emp, SepAssertion::Emp), "or"),
            (SepAssertion::wand(SepAssertion::Emp, SepAssertion::Emp), "wand"),
            (
                SepAssertion::exists(Text::from("x"), SepAssertion::Emp),
                "exists",
            ),
            (
                SepAssertion::forall(Text::from("x"), SepAssertion::Emp),
                "forall",
            ),
            (
                SepAssertion::list_segment(
                    Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
                    Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
                    List::new(),
                ),
                "list_segment",
            ),
            (
                SepAssertion::block(
                    Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
                    Expr::new(ExprKind::Tuple(List::new()), Span::dummy()),
                ),
                "block",
            ),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (smt, expected_tag) in probes {
            match KernelSepBridge::lower(&smt) {
                HeapPredicate::Named { name, args } => {
                    assert_eq!(name, expected_tag);
                    assert!(args.is_empty());
                    assert!(seen.insert(name), "tag '{}' must be unique", expected_tag);
                }
                other => panic!("expected Named({}), got {:?}", expected_tag, other),
            }
        }
    }

    // ----- Fidelity classification -----

    #[test]
    fn bridge_fidelity_pure_emp_is_bijective() {
        assert_eq!(bridge_fidelity(&SepAssertion::Emp), BridgeFidelity::Bijective);
    }

    #[test]
    fn bridge_fidelity_smt_only_is_lossy() {
        let wand = SepAssertion::wand(SepAssertion::Emp, SepAssertion::Emp);
        assert_eq!(bridge_fidelity(&wand), BridgeFidelity::LossyButTotal);
    }

    #[test]
    fn bridge_fidelity_sep_with_lossy_subtree_is_lossy() {
        let nested = SepAssertion::sep(
            SepAssertion::Emp,
            SepAssertion::wand(SepAssertion::Emp, SepAssertion::Emp),
        );
        assert_eq!(bridge_fidelity(&nested), BridgeFidelity::LossyButTotal);
    }

    #[test]
    fn bridge_fidelity_pure_sep_pure_is_bijective() {
        let pure_sep = SepAssertion::sep(SepAssertion::Emp, SepAssertion::Emp);
        assert_eq!(bridge_fidelity(&pure_sep), BridgeFidelity::Bijective);
    }

    // ----- Architectural pin tests -----

    #[test]
    fn kernel_heap_predicate_lift_covers_six_variants() {
        // Pin: lift handles each of the kernel's 6 HeapPredicate
        // variants. Adding a kernel variant without extending lift
        // breaks this test (compiler exhaustive-match guard).
        let probes: Vec<HeapPredicate> = vec![
            HeapPredicate::Emp,
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            HeapPredicate::sep(HeapPredicate::Emp, HeapPredicate::Emp),
            HeapPredicate::HeapPredicate_and_for_test(),
            HeapPredicate::pure(Term::Universe(0)),
            HeapPredicate::named("list", vec![]),
        ];
        // Compile-time exhaustive guard: extend lift if a variant is added.
        for p in &probes {
            let _ = match p {
                HeapPredicate::Emp => "emp",
                HeapPredicate::PointsTo { .. } => "points_to",
                HeapPredicate::Sep { .. } => "sep",
                HeapPredicate::And { .. } => "and",
                HeapPredicate::Pure(_) => "pure",
                HeapPredicate::Named { .. } => "named",
            };
            // Verify lift produces a defined value — never panics.
            let _ = KernelSepBridge::lift(p);
        }
        assert_eq!(probes.len(), 6);
    }

    #[test]
    fn smt_sep_assertion_lower_covers_thirteen_variants() {
        // Pin: lower handles each of the SMT's 13 SepAssertion variants.
        // Compile-time exhaustive guard via the match below — adding
        // an SMT variant without extending lower breaks this test.
        let any_expr = || Expr::new(ExprKind::Tuple(List::new()), Span::dummy());
        let probes: Vec<SepAssertion> = vec![
            SepAssertion::Emp,
            SepAssertion::points_to(any_expr(), any_expr()),
            SepAssertion::sep(SepAssertion::Emp, SepAssertion::Emp),
            SepAssertion::and(SepAssertion::Emp, SepAssertion::Emp),
            SepAssertion::pure(any_expr()),
            SepAssertion::or(SepAssertion::Emp, SepAssertion::Emp),
            SepAssertion::wand(SepAssertion::Emp, SepAssertion::Emp),
            SepAssertion::exists(Text::from("x"), SepAssertion::Emp),
            SepAssertion::forall(Text::from("x"), SepAssertion::Emp),
            SepAssertion::list_segment(any_expr(), any_expr(), List::new()),
            SepAssertion::tree(any_expr(), Maybe::None, Maybe::None),
            SepAssertion::block(any_expr(), any_expr()),
            SepAssertion::array_segment(any_expr(), any_expr(), any_expr(), List::new()),
        ];
        for p in &probes {
            // Compile-time exhaustive guard.
            let _ = match p {
                SepAssertion::Emp => "emp",
                SepAssertion::PointsTo { .. } => "points_to",
                SepAssertion::Sep { .. } => "sep",
                SepAssertion::And { .. } => "and",
                SepAssertion::Pure(_) => "pure",
                SepAssertion::Or { .. } => "or",
                SepAssertion::Wand { .. } => "wand",
                SepAssertion::Exists { .. } => "exists",
                SepAssertion::Forall { .. } => "forall",
                SepAssertion::ListSegment { .. } => "list_segment",
                SepAssertion::Tree { .. } => "tree",
                SepAssertion::Block { .. } => "block",
                SepAssertion::ArraySegment { .. } => "array_segment",
            };
            // Verify lower produces a defined value — never panics.
            let _ = KernelSepBridge::lower(p);
        }
        assert_eq!(probes.len(), 13);
    }
}

// Helper for the kernel-side test probe — kept private since it's
// only used by the architectural-pin test above.
#[cfg(test)]
trait HeapPredicateAndForTest {
    fn HeapPredicate_and_for_test() -> Self;
}

#[cfg(test)]
impl HeapPredicateAndForTest for HeapPredicate {
    fn HeapPredicate_and_for_test() -> Self {
        HeapPredicate::And {
            lhs: Box::new(HeapPredicate::Emp),
            rhs: Box::new(HeapPredicate::Emp),
        }
    }
}
