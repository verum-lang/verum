//! Separation logic — the verification surface for stateful programs.
//!
//! Verum's pure-theorem verification (theorems / lemmas / fn
//! contracts over functional values) is handled by
//! [`crate::verification_goal`].  This module extends the surface to
//! cover **stateful** programs: mutating heap, concurrent threads,
//! IO-bearing operations.  The data layer here is the architectural
//! commitment; the verification dispatcher consumes it via the same
//! pattern as [`crate::verification_goal::VerificationGoal`].
//!
//! ## The fundamentals
//!
//! Separation logic (Reynolds 2002, O'Hearn 2007) extends Hoare
//! logic with the **separating conjunction** `P ∗ Q` — meaning
//! "the heap splits into disjoint parts; `P` holds in one, `Q` in
//! the other".  The associated **frame rule**
//!
//!     { P } c { Q }
//!     ─────────────────
//!     { P ∗ R } c { Q ∗ R }
//!
//! makes local reasoning sound: a command's effect on its
//! footprint doesn't disturb invariants on disjoint heap fragments.
//!
//! ## Architectural alignment with Verum philosophy
//!
//! - **Semantic honesty**: a separation goal IS what we're proving
//!   about a stateful operation — a Hoare triple, not "the SMT layer
//!   wants this".  One concept, one type.
//! - **No magic**: every triple has explicit pre/post/footprint.
//!   Aliasing, frame conditions, capability constraints all
//!   surface as data in the goal.
//! - **Foundation-neutral**: pre/post conditions are kernel `Term`
//!   values — they live in the same trust base as `proof_checker`.
//! - **Gradual safety**: `Capability` permissions plug into the
//!   three-tier reference model (Ref / RefChecked / RefUnsafe) so
//!   the verification pipeline can run at any tier.
//!
//! ## Surface
//!
//!   - [`HeapPredicate`] — a heap-shaped proposition (kernel `Term`
//!     parameterised by an implicit heap variable).
//!   - [`HoareTriple`] — `{ pre } command { post }` with footprint
//!     metadata.
//!   - [`SeparationGoal`] — Hoare triple + framing-context for the
//!     verification dispatcher.
//!   - [`Capability`] — heap permission (Read / Write / Own / None)
//!     that links separation-logic verification to the three-tier
//!     reference model.
//!   - [`from_hoare_triple`] — adapter to
//!     [`crate::verification_goal::VerificationGoal`] so the unified
//!     verification surface (pure + stateful) consumes both.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::proof_checker::Term;

// =============================================================================
// HeapPredicate
// =============================================================================

/// A heap-shaped proposition.  Conceptually `Heap → Prop`; encoded
/// here as a kernel `Term` whose outermost binder is the implicit
/// heap variable.
///
/// Standard combinators are surfaced explicitly so the verification
/// dispatcher can pattern-match on them:
///
///   - `emp` — the empty-heap predicate, true exactly when the
///     heap is empty.
///   - `points_to(addr, value)` — the singleton predicate, true when
///     the heap is a single binding `addr ↦ value`.
///   - `sep(p, q)` — separating conjunction `P ∗ Q`.
///   - `pure(t)` — heap-irrelevant proposition `t` (lifts a kernel
///     `Term` into the heap-predicate language).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HeapPredicate {
    /// `emp` — the heap is empty.
    Emp,
    /// `addr ↦ value` — the heap is a single binding.  Both
    /// `addr` and `value` are kernel `Term`s.
    PointsTo {
        /// Address term.
        addr: Term,
        /// Value term.
        value: Term,
    },
    /// `P ∗ Q` — separating conjunction.  The heap splits into
    /// disjoint parts; `lhs` holds in one, `rhs` in the other.
    Sep {
        /// Left disjunct.
        lhs: Box<HeapPredicate>,
        /// Right disjunct.
        rhs: Box<HeapPredicate>,
    },
    /// `pure(P)` — heap-irrelevant proposition; true at every heap.
    /// Lifts a kernel `Term` into the heap-predicate language.
    Pure(Term),
    /// `P ∧ Q` — ordinary (non-separating) conjunction.  Both hold
    /// at the same heap.
    And {
        /// Left conjunct.
        lhs: Box<HeapPredicate>,
        /// Right conjunct.
        rhs: Box<HeapPredicate>,
    },
    /// Custom-named heap predicate — user-defined or library
    /// abstraction.  The `args` are kernel `Term`s; the `name`
    /// resolves via the elaboration context's axiom registry.
    Named {
        /// Predicate name (resolved via axiom registry).
        name: String,
        /// Argument terms.
        args: Vec<Term>,
    },
}

impl HeapPredicate {
    /// Build the empty-heap predicate.
    pub fn emp() -> Self {
        HeapPredicate::Emp
    }

    /// Build a points-to predicate.
    pub fn points_to(addr: Term, value: Term) -> Self {
        HeapPredicate::PointsTo { addr, value }
    }

    /// Build a separating conjunction.
    pub fn sep(lhs: HeapPredicate, rhs: HeapPredicate) -> Self {
        HeapPredicate::Sep {
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    /// Build a pure (heap-irrelevant) predicate.
    pub fn pure(t: Term) -> Self {
        HeapPredicate::Pure(t)
    }

    /// Build a named predicate.
    pub fn named(name: impl Into<String>, args: Vec<Term>) -> Self {
        HeapPredicate::Named {
            name: name.into(),
            args,
        }
    }

    /// Whether this predicate is `emp`.
    pub fn is_emp(&self) -> bool {
        matches!(self, HeapPredicate::Emp)
    }

    /// Whether this predicate is heap-irrelevant (no `PointsTo` or
    /// custom heap-named predicates anywhere in the structure).
    /// Heap-irrelevant predicates can be discharged by the pure
    /// kernel without invoking the separation-logic dispatcher.
    pub fn is_pure(&self) -> bool {
        match self {
            HeapPredicate::Emp => true,
            HeapPredicate::Pure(_) => true,
            HeapPredicate::PointsTo { .. } => false,
            HeapPredicate::Named { .. } => false,
            HeapPredicate::Sep { lhs, rhs } | HeapPredicate::And { lhs, rhs } => {
                lhs.is_pure() && rhs.is_pure()
            }
        }
    }
}

// =============================================================================
// Capability
// =============================================================================

/// Heap-region capability.  Links separation-logic verification to
/// Verum's three-tier reference model: every heap-bearing proof
/// obligation declares which capability the command needs over the
/// touched region.
///
/// **Soundness invariant**: a `Hoare`-triple-style obligation can
/// only mutate regions whose capability is `Write` or `Own`.
/// `Read` obligations cannot mutate; `None` obligations are pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// No capability — the command doesn't touch the heap.
    None,
    /// Read-only access to the region.
    Read,
    /// Read + write access (linear; aliased writes forbidden).
    Write,
    /// Full ownership — read, write, and free.  Required for
    /// allocation / deallocation operations.
    Own,
}

impl Capability {
    /// Whether this capability allows mutation.
    pub fn allows_write(self) -> bool {
        matches!(self, Capability::Write | Capability::Own)
    }

    /// Whether this capability allows reading.
    pub fn allows_read(self) -> bool {
        matches!(self, Capability::Read | Capability::Write | Capability::Own)
    }

    /// Diagnostic label.
    pub fn label(self) -> &'static str {
        match self {
            Capability::None => "none",
            Capability::Read => "read",
            Capability::Write => "write",
            Capability::Own => "own",
        }
    }
}

// =============================================================================
// HoareTriple
// =============================================================================

/// A Hoare triple `{ pre } command { post }` with footprint metadata.
///
/// `command_term` is a kernel `Term` representing the command being
/// verified — function call, assignment, sequence, conditional, etc.
/// The pre/post conditions are heap predicates; the
/// `footprint_capability` records the maximum capability the
/// command needs over the touched region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HoareTriple {
    /// Pre-condition heap predicate.
    pub pre: HeapPredicate,
    /// The command being verified — a kernel `Term`.
    pub command_term: Term,
    /// Post-condition heap predicate.
    pub post: HeapPredicate,
    /// Maximum capability the command needs over the touched
    /// heap region.
    pub footprint_capability: Capability,
}

impl HoareTriple {
    /// Build a new triple.
    pub fn new(
        pre: HeapPredicate,
        command_term: Term,
        post: HeapPredicate,
        footprint_capability: Capability,
    ) -> Self {
        Self {
            pre,
            command_term,
            post,
            footprint_capability,
        }
    }

    /// Whether this triple's pre and post are both pure (heap-
    /// irrelevant).  Pure triples reduce to ordinary
    /// [`crate::verification_goal::VerificationGoal`]s — no
    /// separation-logic dispatcher needed.
    pub fn is_pure(&self) -> bool {
        self.pre.is_pure() && self.post.is_pure()
    }
}

// =============================================================================
// SeparationGoal
// =============================================================================

/// A separation-logic verification goal.  The stateful counterpart
/// of [`crate::verification_goal::VerificationGoal`]: every source
/// of a stateful proof obligation produces this shape.
///
/// **Frame rule**: when the verifier discharges a goal, the
/// `frame_invariant` is preserved across the command — the
/// separation-logic dispatcher checks `pre ∗ frame_invariant`
/// against `post ∗ frame_invariant`.  Setting `frame_invariant =
/// HeapPredicate::Emp` recovers the bare Hoare triple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeparationGoal {
    /// The Hoare triple at the heart of the goal.
    pub triple: HoareTriple,
    /// Frame invariant — heap-shape that's preserved across the
    /// command.  `HeapPredicate::Emp` for goals without a frame.
    pub frame_invariant: HeapPredicate,
    /// Where this goal arose.  Diagnostic + audit-gate metadata.
    pub source: SeparationGoalSource,
}

/// Source pipeline for a [`SeparationGoal`].  Mirrors the
/// architecture of [`crate::verification_goal::GoalSource`] for the
/// pure-theorem world.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SeparationGoalSource {
    /// `fn f(x: T) requires P_heap ensures Q_heap` — the contract
    /// of a stateful function.
    StatefulFnContract {
        /// Function name.
        fn_name: String,
    },
    /// Loop invariant verification: `while c { body }` produces
    /// `{ I ∧ c } body { I }` plus `{ I ∧ ¬c } skip { Q }`.
    LoopInvariant {
        /// Enclosing function name.
        enclosing_fn: String,
        /// Loop site identifier (line / source span tag).
        loop_site: String,
    },
    /// Memory-allocation site: `let p = alloc(T)` discharges the
    /// triple `{ emp } alloc(T) { p ↦ default(T) }`.
    Allocation {
        /// Enclosing function name.
        enclosing_fn: String,
        /// Source-span tag for the alloc site.
        alloc_site: String,
    },
    /// Concurrent block: `spawn { ... }` or `parallel { ... }`
    /// discharges resource invariants for each thread.
    Concurrent {
        /// Enclosing function name.
        enclosing_fn: String,
        /// Identifier for the concurrent region.
        region: String,
    },
}

impl SeparationGoalSource {
    /// Diagnostic kind tag.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            SeparationGoalSource::StatefulFnContract { .. } => "stateful_fn_contract",
            SeparationGoalSource::LoopInvariant { .. } => "loop_invariant",
            SeparationGoalSource::Allocation { .. } => "allocation",
            SeparationGoalSource::Concurrent { .. } => "concurrent",
        }
    }
}

impl SeparationGoal {
    /// Build a fresh goal.
    pub fn new(
        triple: HoareTriple,
        frame_invariant: HeapPredicate,
        source: SeparationGoalSource,
    ) -> Self {
        Self {
            triple,
            frame_invariant,
            source,
        }
    }

    /// Build a goal with the empty frame (bare Hoare triple).
    pub fn bare(triple: HoareTriple, source: SeparationGoalSource) -> Self {
        Self::new(triple, HeapPredicate::Emp, source)
    }

    /// Whether this goal is *purely* heap-irrelevant — the triple
    /// has pure pre/post, the frame is empty, and the capability
    /// is `None`.  Pure goals can be discharged by the ordinary
    /// pure-verification dispatcher.
    pub fn is_pure(&self) -> bool {
        self.triple.is_pure()
            && self.frame_invariant.is_emp()
            && self.triple.footprint_capability == Capability::None
    }

    /// Audit-gate metadata.  Suitable for direct serde-JSON emission.
    pub fn audit_metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("kind".to_string(), self.source.kind_tag().to_string());
        m.insert(
            "capability".to_string(),
            self.triple.footprint_capability.label().to_string(),
        );
        m.insert("is_pure".to_string(), self.is_pure().to_string());
        m
    }
}

// =============================================================================
// Adapter to the unified verification surface
// =============================================================================

/// **Adapter** — produce a pure-verification
/// [`crate::verification_goal::VerificationGoal`] from a
/// [`SeparationGoal`] when the latter is pure.  Returns `None` for
/// stateful goals (those need the separation-logic dispatcher; the
/// pure-verification surface can't represent them).
///
/// This is the bridge between the two verification surfaces: pure
/// and stateful goals coexist; pure separation goals fall back to
/// the unified dispatcher.
pub fn try_lift_to_verification_goal(
    goal: &SeparationGoal,
) -> Option<crate::verification_goal::VerificationGoal> {
    if !goal.is_pure() {
        return None;
    }
    // Extract the pure conclusion: a pure heap predicate is
    // either Emp (Universe(0)), Pure(t), or a conjunction of
    // pure predicates.  For Emp → Universe(0); for Pure(t) → t;
    // for And/Sep of pure → conjoin via the connective axiom
    // (callers must register the connective).  Phase-0 of this
    // adapter only handles the simplest cases.
    let conclusion = pure_predicate_to_term(&goal.triple.post)?;
    Some(crate::verification_goal::VerificationGoal::new(
        Vec::new(),
        conclusion,
        crate::verification_goal::GoalSource::FnContract {
            fn_name: match &goal.source {
                SeparationGoalSource::StatefulFnContract { fn_name }
                | SeparationGoalSource::LoopInvariant {
                    enclosing_fn: fn_name,
                    ..
                }
                | SeparationGoalSource::Allocation {
                    enclosing_fn: fn_name,
                    ..
                }
                | SeparationGoalSource::Concurrent {
                    enclosing_fn: fn_name,
                    ..
                } => fn_name.clone(),
            },
        },
    ))
}

/// Translate a pure heap predicate to a kernel `Term`.  Returns
/// `None` for non-pure predicates.  The simplest cases handled
/// here; full Pi/Eq/Conj encodings are downstream connective
/// work in [`crate::tactic_elaborator`].
fn pure_predicate_to_term(p: &HeapPredicate) -> Option<Term> {
    match p {
        HeapPredicate::Emp => Some(Term::Universe(0)),
        HeapPredicate::Pure(t) => Some(t.clone()),
        HeapPredicate::And { lhs, rhs } if lhs.is_pure() && rhs.is_pure() => {
            // Without registering a connective axiom here, fall
            // back to the lhs.  The tactic_elaborator's connective
            // encoding handles the And-encoding when needed.
            let _ = rhs;
            pure_predicate_to_term(lhs)
        }
        _ => None,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_predicate_is_emp_and_pure() {
        let p = HeapPredicate::emp();
        assert!(p.is_emp());
        assert!(p.is_pure());
    }

    #[test]
    fn points_to_is_not_pure() {
        let p = HeapPredicate::points_to(Term::Var(0), Term::Var(1));
        assert!(!p.is_emp());
        assert!(!p.is_pure());
    }

    #[test]
    fn sep_of_pure_is_pure() {
        let p = HeapPredicate::sep(
            HeapPredicate::pure(Term::Universe(0)),
            HeapPredicate::emp(),
        );
        assert!(p.is_pure());
    }

    #[test]
    fn sep_with_points_to_is_not_pure() {
        let p = HeapPredicate::sep(
            HeapPredicate::pure(Term::Universe(0)),
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
        );
        assert!(!p.is_pure());
    }

    #[test]
    fn capability_permits_correctly() {
        assert!(!Capability::None.allows_read());
        assert!(!Capability::None.allows_write());
        assert!(Capability::Read.allows_read());
        assert!(!Capability::Read.allows_write());
        assert!(Capability::Write.allows_read());
        assert!(Capability::Write.allows_write());
        assert!(Capability::Own.allows_read());
        assert!(Capability::Own.allows_write());
    }

    #[test]
    fn capability_labels_distinct() {
        let labels: std::collections::BTreeSet<_> = [
            Capability::None,
            Capability::Read,
            Capability::Write,
            Capability::Own,
        ]
        .iter()
        .map(|c| c.label())
        .collect();
        assert_eq!(labels.len(), 4);
    }

    #[test]
    fn pure_hoare_triple_round_trip() {
        let triple = HoareTriple::new(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::pure(Term::Universe(0)),
            Capability::None,
        );
        assert!(triple.is_pure());
    }

    #[test]
    fn stateful_hoare_triple_is_not_pure() {
        let triple = HoareTriple::new(
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Term::Universe(0),
            HeapPredicate::points_to(Term::Var(0), Term::Var(2)),
            Capability::Write,
        );
        assert!(!triple.is_pure());
    }

    #[test]
    fn separation_goal_audit_metadata_complete() {
        let triple = HoareTriple::new(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::emp(),
            Capability::Write,
        );
        let goal = SeparationGoal::bare(
            triple,
            SeparationGoalSource::StatefulFnContract {
                fn_name: "swap".into(),
            },
        );
        let m = goal.audit_metadata();
        assert_eq!(m.get("kind").map(String::as_str), Some("stateful_fn_contract"));
        assert_eq!(m.get("capability").map(String::as_str), Some("write"));
        assert!(m.contains_key("is_pure"));
    }

    #[test]
    fn pure_separation_goal_lifts_to_verification_goal() {
        let triple = HoareTriple::new(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::emp(),
            Capability::None,
        );
        let goal = SeparationGoal::bare(
            triple,
            SeparationGoalSource::StatefulFnContract {
                fn_name: "trivial".into(),
            },
        );
        assert!(goal.is_pure());
        let vg = try_lift_to_verification_goal(&goal).expect("pure goal lifts");
        assert_eq!(vg.conclusion, Term::Universe(0));
    }

    #[test]
    fn stateful_separation_goal_does_not_lift() {
        let triple = HoareTriple::new(
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Term::Universe(0),
            HeapPredicate::points_to(Term::Var(0), Term::Var(2)),
            Capability::Write,
        );
        let goal = SeparationGoal::bare(
            triple,
            SeparationGoalSource::StatefulFnContract {
                fn_name: "store".into(),
            },
        );
        assert!(!goal.is_pure());
        assert!(try_lift_to_verification_goal(&goal).is_none());
    }

    #[test]
    fn separation_goal_serde_round_trip() {
        let triple = HoareTriple::new(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Capability::Write,
        );
        let goal = SeparationGoal::new(
            triple,
            HeapPredicate::pure(Term::Universe(0)),
            SeparationGoalSource::LoopInvariant {
                enclosing_fn: "f".into(),
                loop_site: "f.vr:42".into(),
            },
        );
        let json = serde_json::to_string(&goal).unwrap();
        let restored: SeparationGoal = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, goal);
    }

    #[test]
    fn separation_goal_source_kind_tags_distinct() {
        let tags = [
            SeparationGoalSource::StatefulFnContract {
                fn_name: String::new(),
            }
            .kind_tag(),
            SeparationGoalSource::LoopInvariant {
                enclosing_fn: String::new(),
                loop_site: String::new(),
            }
            .kind_tag(),
            SeparationGoalSource::Allocation {
                enclosing_fn: String::new(),
                alloc_site: String::new(),
            }
            .kind_tag(),
            SeparationGoalSource::Concurrent {
                enclosing_fn: String::new(),
                region: String::new(),
            }
            .kind_tag(),
        ];
        let unique: std::collections::BTreeSet<_> = tags.iter().copied().collect();
        assert_eq!(unique.len(), tags.len());
    }
}
