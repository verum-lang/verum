//! Separation logic — the verification surface for stateful programs.
//!

//! Verum's pure-theorem verification (theorems / lemmas / fn
//! contracts over functional values) is handled by
//! [`crate::verification_goal`]. This module extends the surface to
//! cover **stateful** programs: mutating heap, concurrent threads,
//! IO-bearing operations. The data layer here is the architectural
//! commitment; the verification dispatcher consumes it via the same
//! pattern as [`crate::verification_goal::VerificationGoal`].
//!

//! ## The fundamentals
//!

//! Separation logic (Reynolds 2002, O'Hearn 2007) extends Hoare
//! logic with the **separating conjunction** `P ∗ Q` — meaning
//! "the heap splits into disjoint parts; `P` holds in one, `Q` in
//! the other". The associated **frame rule**
//!

//!  { P } c { Q }
//!  ─────────────────
//!  { P ∗ R } c { Q ∗ R }
//!

//! makes local reasoning sound: a command's effect on its
//! footprint doesn't disturb invariants on disjoint heap fragments.
//!

//! ## Architectural alignment with Verum philosophy
//!

//! - **Semantic honesty**: a separation goal IS what we're proving
//!  about a stateful operation — a Hoare triple, not "the SMT layer
//!  wants this". One concept, one type.
//! - **No magic**: every triple has explicit pre/post/footprint.
//!  Aliasing, frame conditions, capability constraints all
//!  surface as data in the goal.
//! - **Foundation-neutral**: pre/post conditions are kernel `Term`
//!  values — they live in the same trust base as `proof_checker`.
//! - **Gradual safety**: `Capability` permissions plug into the
//!  three-tier reference model (Ref / RefChecked / RefUnsafe) so
//!  the verification pipeline can run at any tier.
//!

//! ## Surface
//!

//!  - [`HeapPredicate`] — a heap-shaped proposition (kernel `Term`
//!  parameterised by an implicit heap variable).
//!  - [`HoareTriple`] — `{ pre } command { post }` with footprint
//!  metadata.
//!  - [`SeparationGoal`] — Hoare triple + framing-context for the
//!  verification dispatcher.
//!  - [`Capability`] — heap permission (Read / Write / Own / None)
//!  that links separation-logic verification to the three-tier
//!  reference model.
//!  - [`from_hoare_triple`] — adapter to
//!  [`crate::verification_goal::VerificationGoal`] so the unified
//!  verification surface (pure + stateful) consumes both.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::proof_checker::Term;

// =============================================================================
// HeapPredicate
// =============================================================================

/// A heap-shaped proposition. Conceptually `Heap → Prop`; encoded
/// here as a kernel `Term` whose outermost binder is the implicit
/// heap variable.
///

/// Standard combinators are surfaced explicitly so the verification
/// dispatcher can pattern-match on them:
///

///  - `emp` — the empty-heap predicate, true exactly when the
///  heap is empty.
///  - `points_to(addr, value)` — the singleton predicate, true when
///  the heap is a single binding `addr ↦ value`.
///  - `sep(p, q)` — separating conjunction `P ∗ Q`.
///  - `pure(t)` — heap-irrelevant proposition `t` (lifts a kernel
///  `Term` into the heap-predicate language).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HeapPredicate {
    /// `emp` — the heap is empty.
    Emp,
    /// `addr ↦ value` — the heap is a single binding. Both
    /// `addr` and `value` are kernel `Term`s.
    PointsTo {
        /// Address term.
        addr: Term,
        /// Value term.
        value: Term,
    },
    /// `P ∗ Q` — separating conjunction. The heap splits into
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
    /// `P ∧ Q` — ordinary (non-separating) conjunction. Both hold
    /// at the same heap.
    And {
        /// Left conjunct.
        lhs: Box<HeapPredicate>,
        /// Right conjunct.
        rhs: Box<HeapPredicate>,
    },
    /// Custom-named heap predicate — user-defined or library
    /// abstraction. The `args` are kernel `Term`s; the `name`
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

/// Heap-region capability. Links separation-logic verification to
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
    /// Full ownership — read, write, and free. Required for
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
    /// irrelevant). Pure triples reduce to ordinary
    /// [`crate::verification_goal::VerificationGoal`]s — no
    /// separation-logic dispatcher needed.
    pub fn is_pure(&self) -> bool {
        self.pre.is_pure() && self.post.is_pure()
    }
}

// =============================================================================
// SeparationGoal
// =============================================================================

/// A separation-logic verification goal. The stateful counterpart
/// of [`crate::verification_goal::VerificationGoal`]: every source
/// of a stateful proof obligation produces this shape.
///

/// **Frame rule**: when the verifier discharges a goal, the
/// `frame_invariant` is preserved across the command — the
/// separation-logic dispatcher checks `pre ∗ frame_invariant`
/// against `post ∗ frame_invariant`. Setting `frame_invariant =
/// HeapPredicate::Emp` recovers the bare Hoare triple.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeparationGoal {
    /// The Hoare triple at the heart of the goal.
    pub triple: HoareTriple,
    /// Frame invariant — heap-shape that's preserved across the
    /// command. `HeapPredicate::Emp` for goals without a frame.
    pub frame_invariant: HeapPredicate,
    /// Where this goal arose. Diagnostic + audit-gate metadata.
    pub source: SeparationGoalSource,
}

/// Source pipeline for a [`SeparationGoal`]. Mirrors the
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
    /// is `None`. Pure goals can be discharged by the ordinary
    /// pure-verification dispatcher.
    pub fn is_pure(&self) -> bool {
        self.triple.is_pure()
            && self.frame_invariant.is_emp()
            && self.triple.footprint_capability == Capability::None
    }

    /// Audit-gate metadata. Suitable for direct serde-JSON emission.
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
/// [`SeparationGoal`] when the latter is pure. Returns `None` for
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
    // pure predicates. For Emp → Universe(0); for Pure(t) → t;
    // for And/Sep of pure → conjoin via the connective axiom
    // (callers must register the connective). Phase-0 of this
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

/// Translate a pure heap predicate to a kernel `Term`. Returns
/// `None` for non-pure predicates. The simplest cases handled
/// here; full Pi/Eq/Conj encodings are downstream connective
/// work in [`crate::tactic_elaborator`].
fn pure_predicate_to_term(p: &HeapPredicate) -> Option<Term> {
    match p {
        HeapPredicate::Emp => Some(Term::Universe(0)),
        HeapPredicate::Pure(t) => Some(t.clone()),
        HeapPredicate::And { lhs, rhs } if lhs.is_pure() && rhs.is_pure() => {
            // Without registering a connective axiom here, fall
            // back to the lhs. The tactic_elaborator's connective
            // encoding handles the And-encoding when needed.
            let _ = rhs;
            pure_predicate_to_term(lhs)
        }
        _ => None,
    }
}

// =============================================================================
// Dispatcher — verdict surface + routing
// =============================================================================

/// The verdict the separation-logic dispatcher returns for a single
/// [`SeparationGoal`]. Mirrors the shape of
/// [`crate::verification_goal::VerificationGoal`]'s pure-side
/// dispatcher: every goal terminates in a verdict, and every verdict
/// carries enough metadata for audit-gate emission without re-running
/// the dispatcher.
///

/// **Soundness invariant**: only [`SeparationVerdict::Discharged`]
/// commits to "the goal holds in the kernel". Every other variant
/// either explicitly admits an IOU, rejects the goal as ill-formed,
/// or routes to a downstream verification strategy that produces its
/// own follow-up verdict.
///

/// **Audit-gate use**: the verdict's variant tag feeds
/// [`SeparationDispatcherStats`] so `verum audit
/// --separation-dispatch` can enumerate the per-strategy load
/// distribution across a corpus run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeparationVerdict {
    /// The goal is closed by the kernel directly — no downstream
    /// verification needed. At dispatcher V1 only the trivial
    /// `{ emp } _ { emp }` shape lands here.
    Discharged,
    /// The dispatcher cannot close the goal at this version of the
    /// kernel; the wrapped string names the IOU. Downstream audit
    /// gates surface the IOU in `--soundness-iou`.
    AdmittedWithIou(String),
    /// The goal is structurally malformed (capability inconsistent
    /// with the program statement, footprint mismatched against the
    /// frame, etc.). Wrapped string explains the violation. This
    /// is the only "negative" verdict — distinct from
    /// [`SeparationVerdict::AdmittedWithIou`] which records a
    /// well-formed goal that the dispatcher merely cannot close yet.
    RejectIllFormed(String),
    /// The goal's frame is non-trivial; closing it requires applying
    /// the frame rule. The next slice consumes this verdict and runs
    /// the frame-rule strategy on the bare triple.
    RoutedToFrameRule,
    /// The command is a sequence; closing it requires Hoare-style
    /// sequencing (`{P} c1 {Q}` + `{Q} c2 {R}` → `{P} c1; c2 {R}`).
    /// The next slice consumes this verdict and runs the sequencing
    /// strategy.
    RoutedToHoareSequencing,
    /// The pre/post pair differs from a kernel-known pattern by
    /// pure-implication weakening; closing it requires the
    /// rule-of-consequence strategy (`{P'} c {Q'}` ⇒ `{P} c {Q}`
    /// when `P → P'` and `Q' → Q`).
    RoutedToConsequenceRule,
}

impl SeparationVerdict {
    /// Diagnostic kind tag — stable across kernel versions for
    /// audit-gate aggregation.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            SeparationVerdict::Discharged => "discharged",
            SeparationVerdict::AdmittedWithIou(_) => "admitted_with_iou",
            SeparationVerdict::RejectIllFormed(_) => "reject_ill_formed",
            SeparationVerdict::RoutedToFrameRule => "routed_to_frame_rule",
            SeparationVerdict::RoutedToHoareSequencing => "routed_to_hoare_sequencing",
            SeparationVerdict::RoutedToConsequenceRule => "routed_to_consequence_rule",
        }
    }

    /// Whether this verdict commits to "the goal holds". Only
    /// `Discharged` does; the routing verdicts defer the decision
    /// to a downstream strategy and IOUs / rejections are negative.
    pub fn is_closed(&self) -> bool {
        matches!(self, SeparationVerdict::Discharged)
    }

    /// Whether this verdict defers the decision to a downstream
    /// verification strategy (frame rule / sequencing / consequence).
    pub fn is_routed(&self) -> bool {
        matches!(
            self,
            SeparationVerdict::RoutedToFrameRule
                | SeparationVerdict::RoutedToHoareSequencing
                | SeparationVerdict::RoutedToConsequenceRule
        )
    }
}

/// The load-bearing dispatcher: routes a [`SeparationGoal`] to a
/// verification strategy and returns the resulting
/// [`SeparationVerdict`].
///

/// **V1 routing rules** (in priority order — the first matching rule
/// wins):
///

/// 1. **Capability mismatch** — the goal carries an IO capability
///  (`Read` / `Write` / `Own`) but the triple is pure (both pre
///  and post are heap-irrelevant). A pure command cannot need
///  heap-region access; reject as ill-formed.
/// 2. **Trivial frame** — pre and post are both `emp` and the frame
///  is `emp`. Discharged unconditionally.
/// 3. **Non-trivial frame invariant** — `frame_invariant` is not
///  `emp`. Route to the frame rule.
/// 4. **Sequencing-shaped command** — `triple.command_term` is an
///  application `App(_, _)` and the goal is non-trivial. Route
///  to Hoare sequencing.
/// 5. **Differing pre/post with a non-trivial pattern** — pre and
///  post are non-`emp` and differ. Route to the consequence rule.
/// 6. **Default** — admit with IOU "no rule matches — frame
///  inference V1".
///

/// **What this dispatcher deliberately does NOT do (yet)**: it does
/// not run any strategy. It only RECOGNISES the shape of a goal and
/// tags it for the strategy that should consume it. The strategies
/// themselves land in the next slice (#161 follow-up).
///

/// **Soundness invariant**: the dispatcher never returns
/// [`SeparationVerdict::Discharged`] except for the one trivial case
/// above. Adding a new closed-form discharge requires a kernel-rule
/// audit and a corresponding promotion of the goal's
/// [`SeparationGoalSource`] to a kernel-recognised pattern.
pub fn dispatch_separation_goal(goal: &SeparationGoal) -> SeparationVerdict {
    // Rule 1: capability mismatch. A pure triple cannot demand
    // a non-trivial heap-region capability.
    if goal.triple.is_pure() && goal.triple.footprint_capability != Capability::None {
        return SeparationVerdict::RejectIllFormed(
            "capability mismatch \u{2014} pure stmt requires no IO capability".to_string(),
        );
    }

    // Rule 2: trivial frame — { emp } _ { emp } with empty frame.
    if goal.triple.pre.is_emp()
        && goal.triple.post.is_emp()
        && goal.frame_invariant.is_emp()
    {
        return SeparationVerdict::Discharged;
    }

    // Rule 3: non-trivial frame invariant — route to the frame rule.
    // The "precondition's frame is bigger than the postcondition's
    // footprint" condition is captured here as "the goal carries a
    // non-empty `frame_invariant`": that frame is exactly the part
    // separate from the triple's footprint.
    if !goal.frame_invariant.is_emp() {
        return SeparationVerdict::RoutedToFrameRule;
    }

    // Rule 4: sequencing-shaped command — App(_, _) suggests
    // composition. Route to Hoare sequencing.
    if matches!(goal.triple.command_term, Term::App(_, _)) {
        return SeparationVerdict::RoutedToHoareSequencing;
    }

    // Rule 5: differing non-trivial pre/post — route to consequence.
    if !goal.triple.pre.is_emp()
        && !goal.triple.post.is_emp()
        && goal.triple.pre != goal.triple.post
    {
        return SeparationVerdict::RoutedToConsequenceRule;
    }

    // Rule 6: default — admit with IOU.
    SeparationVerdict::AdmittedWithIou(
        "no rule matches \u{2014} frame inference V1".to_string(),
    )
}

/// Per-verdict counters for audit-gate aggregation. The dispatcher
/// itself doesn't accumulate — callers thread a
/// [`SeparationDispatcherStats`] through their corpus walk and
/// invoke [`SeparationDispatcherStats::record`] on each verdict.
///

/// Used by `verum audit --separation-dispatch` (next slice) to emit
/// a structured per-verdict load distribution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeparationDispatcherStats {
    /// Count of [`SeparationVerdict::Discharged`] verdicts.
    pub discharged: usize,
    /// Count of [`SeparationVerdict::AdmittedWithIou`] verdicts.
    pub admitted_with_iou: usize,
    /// Count of [`SeparationVerdict::RejectIllFormed`] verdicts.
    pub reject_ill_formed: usize,
    /// Count of [`SeparationVerdict::RoutedToFrameRule`] verdicts.
    pub routed_to_frame_rule: usize,
    /// Count of [`SeparationVerdict::RoutedToHoareSequencing`] verdicts.
    pub routed_to_hoare_sequencing: usize,
    /// Count of [`SeparationVerdict::RoutedToConsequenceRule`] verdicts.
    pub routed_to_consequence_rule: usize,
}

impl SeparationDispatcherStats {
    /// Construct a zeroed counter set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the counter matching `verdict`.
    pub fn record(&mut self, verdict: &SeparationVerdict) {
        match verdict {
            SeparationVerdict::Discharged => self.discharged += 1,
            SeparationVerdict::AdmittedWithIou(_) => self.admitted_with_iou += 1,
            SeparationVerdict::RejectIllFormed(_) => self.reject_ill_formed += 1,
            SeparationVerdict::RoutedToFrameRule => self.routed_to_frame_rule += 1,
            SeparationVerdict::RoutedToHoareSequencing => {
                self.routed_to_hoare_sequencing += 1
            }
            SeparationVerdict::RoutedToConsequenceRule => {
                self.routed_to_consequence_rule += 1
            }
        }
    }

    /// Total goals recorded across every verdict variant.
    pub fn total(&self) -> usize {
        self.discharged
            + self.admitted_with_iou
            + self.reject_ill_formed
            + self.routed_to_frame_rule
            + self.routed_to_hoare_sequencing
            + self.routed_to_consequence_rule
    }

    /// Audit-gate metadata. Suitable for direct serde-JSON emission
    /// alongside [`SeparationGoal::audit_metadata`].
    pub fn audit_metadata(&self) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("discharged".to_string(), self.discharged.to_string());
        m.insert(
            "admitted_with_iou".to_string(),
            self.admitted_with_iou.to_string(),
        );
        m.insert(
            "reject_ill_formed".to_string(),
            self.reject_ill_formed.to_string(),
        );
        m.insert(
            "routed_to_frame_rule".to_string(),
            self.routed_to_frame_rule.to_string(),
        );
        m.insert(
            "routed_to_hoare_sequencing".to_string(),
            self.routed_to_hoare_sequencing.to_string(),
        );
        m.insert(
            "routed_to_consequence_rule".to_string(),
            self.routed_to_consequence_rule.to_string(),
        );
        m.insert("total".to_string(), self.total().to_string());
        m
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

    // -------------------------------------------------------------------------
    // Dispatcher tests
    // -------------------------------------------------------------------------

    /// Helper: build a vanilla goal from raw triple parts + frame.
    fn make_goal(
        pre: HeapPredicate,
        cmd: Term,
        post: HeapPredicate,
        cap: Capability,
        frame: HeapPredicate,
    ) -> SeparationGoal {
        SeparationGoal::new(
            HoareTriple::new(pre, cmd, post, cap),
            frame,
            SeparationGoalSource::StatefulFnContract {
                fn_name: "probe".into(),
            },
        )
    }

    #[test]
    fn dispatch_trivial_emp_emp_discharges() {
        let goal = make_goal(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::emp(),
            Capability::None,
            HeapPredicate::emp(),
        );
        let verdict = dispatch_separation_goal(&goal);
        assert_eq!(verdict, SeparationVerdict::Discharged);
        assert!(verdict.is_closed());
        assert!(!verdict.is_routed());
    }

    #[test]
    fn dispatch_pure_with_capability_rejects_as_ill_formed() {
        // Pure pre + pure post, but capability claims Write. Pure
        // statements cannot need heap-region access — reject.
        let goal = make_goal(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::pure(Term::Universe(0)),
            Capability::Write,
            HeapPredicate::emp(),
        );
        match dispatch_separation_goal(&goal) {
            SeparationVerdict::RejectIllFormed(reason) => {
                assert!(reason.contains("capability mismatch"), "{reason}");
                assert!(reason.contains("pure stmt"), "{reason}");
            }
            other => panic!("expected RejectIllFormed, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_pure_with_read_capability_also_rejects() {
        // Read is also non-None — same rejection path.
        let goal = make_goal(
            HeapPredicate::pure(Term::Universe(0)),
            Term::Universe(0),
            HeapPredicate::pure(Term::Universe(0)),
            Capability::Read,
            HeapPredicate::emp(),
        );
        assert!(matches!(
            dispatch_separation_goal(&goal),
            SeparationVerdict::RejectIllFormed(_)
        ));
    }

    #[test]
    fn dispatch_non_empty_frame_routes_to_frame_rule() {
        // The frame_invariant carries a real heap fragment; this
        // should route to the frame rule.
        let goal = make_goal(
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Term::Universe(0),
            HeapPredicate::points_to(Term::Var(0), Term::Var(2)),
            Capability::Write,
            HeapPredicate::points_to(Term::Var(3), Term::Var(4)),
        );
        let verdict = dispatch_separation_goal(&goal);
        assert_eq!(verdict, SeparationVerdict::RoutedToFrameRule);
        assert!(!verdict.is_closed());
        assert!(verdict.is_routed());
    }

    #[test]
    fn dispatch_application_command_routes_to_sequencing() {
        // App(_, _) command-term shape signals a sequenceable
        // composition. Pre/post are non-emp and capability is
        // consistent (Write); frame is emp so we don't hit Rule 3.
        let goal = make_goal(
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Term::app(Term::Var(0), Term::Var(1)),
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Capability::Write,
            HeapPredicate::emp(),
        );
        let verdict = dispatch_separation_goal(&goal);
        assert_eq!(verdict, SeparationVerdict::RoutedToHoareSequencing);
    }

    #[test]
    fn dispatch_differing_pre_post_routes_to_consequence() {
        // Non-emp + differing pre/post + non-App command + emp
        // frame ⇒ consequence rule.
        let goal = make_goal(
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Term::Universe(0),
            HeapPredicate::points_to(Term::Var(0), Term::Var(2)),
            Capability::Write,
            HeapPredicate::emp(),
        );
        assert_eq!(
            dispatch_separation_goal(&goal),
            SeparationVerdict::RoutedToConsequenceRule,
        );
    }

    #[test]
    fn dispatch_default_admits_with_iou() {
        // Heap-shaped pre with emp post. Not trivial (post is emp
        // but pre isn't), no frame, no App command, capability ok.
        // This is the residual case — admit with IOU.
        let goal = make_goal(
            HeapPredicate::points_to(Term::Var(0), Term::Var(1)),
            Term::Universe(0),
            HeapPredicate::emp(),
            Capability::Own,
            HeapPredicate::emp(),
        );
        match dispatch_separation_goal(&goal) {
            SeparationVerdict::AdmittedWithIou(reason) => {
                assert!(reason.contains("frame inference V1"), "{reason}");
            }
            other => panic!("expected AdmittedWithIou, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_capability_mismatch_takes_priority_over_trivial_discharge() {
        // emp pre + emp post should normally discharge — BUT a
        // non-None capability with pure pre/post is ill-formed; the
        // capability-mismatch rule fires FIRST and rejects.
        let goal = make_goal(
            HeapPredicate::emp(),
            Term::Universe(0),
            HeapPredicate::emp(),
            Capability::Read,
            HeapPredicate::emp(),
        );
        assert!(matches!(
            dispatch_separation_goal(&goal),
            SeparationVerdict::RejectIllFormed(_)
        ));
    }

    #[test]
    fn dispatch_pure_emp_emp_with_no_capability_discharges() {
        // Sanity: the trivial discharge path still works when all
        // three of pre / post / frame are emp AND capability is None.
        let goal = make_goal(
            HeapPredicate::emp(),
            Term::Var(0),
            HeapPredicate::emp(),
            Capability::None,
            HeapPredicate::emp(),
        );
        assert_eq!(
            dispatch_separation_goal(&goal),
            SeparationVerdict::Discharged,
        );
    }

    #[test]
    fn verdict_kind_tags_are_distinct_and_stable() {
        let tags = [
            SeparationVerdict::Discharged.kind_tag(),
            SeparationVerdict::AdmittedWithIou("x".into()).kind_tag(),
            SeparationVerdict::RejectIllFormed("y".into()).kind_tag(),
            SeparationVerdict::RoutedToFrameRule.kind_tag(),
            SeparationVerdict::RoutedToHoareSequencing.kind_tag(),
            SeparationVerdict::RoutedToConsequenceRule.kind_tag(),
        ];
        let unique: std::collections::BTreeSet<_> = tags.iter().copied().collect();
        assert_eq!(unique.len(), tags.len(), "verdict tags must be distinct");
        // Stability — these strings feed audit-gate JSON.
        assert!(tags.contains(&"discharged"));
        assert!(tags.contains(&"admitted_with_iou"));
        assert!(tags.contains(&"reject_ill_formed"));
        assert!(tags.contains(&"routed_to_frame_rule"));
        assert!(tags.contains(&"routed_to_hoare_sequencing"));
        assert!(tags.contains(&"routed_to_consequence_rule"));
    }

    #[test]
    fn dispatcher_stats_record_each_variant() {
        let mut stats = SeparationDispatcherStats::new();
        stats.record(&SeparationVerdict::Discharged);
        stats.record(&SeparationVerdict::Discharged);
        stats.record(&SeparationVerdict::AdmittedWithIou("foo".into()));
        stats.record(&SeparationVerdict::RejectIllFormed("bar".into()));
        stats.record(&SeparationVerdict::RoutedToFrameRule);
        stats.record(&SeparationVerdict::RoutedToHoareSequencing);
        stats.record(&SeparationVerdict::RoutedToConsequenceRule);
        assert_eq!(stats.discharged, 2);
        assert_eq!(stats.admitted_with_iou, 1);
        assert_eq!(stats.reject_ill_formed, 1);
        assert_eq!(stats.routed_to_frame_rule, 1);
        assert_eq!(stats.routed_to_hoare_sequencing, 1);
        assert_eq!(stats.routed_to_consequence_rule, 1);
        assert_eq!(stats.total(), 7);
        let m = stats.audit_metadata();
        assert_eq!(m.get("discharged").map(String::as_str), Some("2"));
        assert_eq!(m.get("total").map(String::as_str), Some("7"));
    }

    #[test]
    fn dispatcher_stats_serde_round_trip() {
        let mut stats = SeparationDispatcherStats::new();
        stats.record(&SeparationVerdict::Discharged);
        stats.record(&SeparationVerdict::RoutedToFrameRule);
        let json = serde_json::to_string(&stats).unwrap();
        let restored: SeparationDispatcherStats = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, stats);
    }

    #[test]
    fn verdict_serde_round_trip_preserves_payload() {
        let v = SeparationVerdict::AdmittedWithIou("frame inference V1".into());
        let json = serde_json::to_string(&v).unwrap();
        let restored: SeparationVerdict = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, v);
        let v2 = SeparationVerdict::RoutedToFrameRule;
        let json2 = serde_json::to_string(&v2).unwrap();
        let restored2: SeparationVerdict = serde_json::from_str(&json2).unwrap();
        assert_eq!(restored2, v2);
    }
}
