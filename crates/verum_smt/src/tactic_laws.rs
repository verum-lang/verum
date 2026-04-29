//! Algebraic laws for tactic combinators.
//!
//! Closes task #86. A tactic combinator `TacticCombinator` is a
//! tree over `{Single, AndThen, OrElse, Repeat, TryFor, WithParams,
//! IfThenElse, ParOr}`. Treated as an algebra, the combinators
//! satisfy the standard monoid + distribution laws from the
//! Ltac / LCF / tactical-programming literature.
//!
//! This module exposes:
//!
//! * The laws themselves, as pure structural rewrite rules on
//!   `TacticCombinator`.
//! * `normalize()` — a fixed-point rewrite that applies every
//!   semantics-preserving simplification, shrinking user-written
//!   combinators to canonical form.
//! * `check_law_*()` — boolean predicates that verify a given
//!   combinator pair satisfies a named law on structural equality.
//!   Used by property tests and by
//!   `core.proof.tactics.laws`-style regression suites.
//!
//! # Laws
//!
//! Monoid of AndThen with identity `skip` (0-iteration Repeat of
//! the identity simplify):
//!
//!   skip ; t  ≡  t            (L1 left-identity)
//!   t ; skip  ≡  t            (L2 right-identity)
//!   (t ; u) ; v  ≡  t ; (u ; v)   (L3 associativity)
//!
//! Monoid of OrElse with identity `fail`:
//!
//!   fail | t  ≡  t            (L4 left-identity)
//!   t | fail  ≡  t            (L5 right-identity)
//!   (t | u) | v  ≡  t | (u | v)   (L6 associativity)
//!
//! Repeat:
//!
//!   Repeat(t, 0)  ≡  skip     (L7 zero-unfold)
//!   Repeat(t, 1)  ≡  t        (L8 one-unfold)
//!
//! Idempotence (conservative — only on Single-leaf tactics because
//! a compound combinator's effect sequence is not necessarily
//! idempotent):
//!
//!   Single(k) | Single(k)  ≡  Single(k)   (L9 OrElse-idempotent)
//!
//! Semantic note on idempotence: `simp ; simp` is *observationally*
//! idempotent on every known Z3 state, but the solver may have
//! trace side-effects (proof term size, statistics counters) — so
//! we decline to simplify `t ; t` to `t`. Only OrElse-idempotence
//! is applied; it never changes the solver's proof trace because
//! only the first alternative runs in practice.
//!
//! # Why this isn't "just a simplifier"
//!
//! The laws let `user_tactic::compile_tactic` produce smaller,
//! deterministic combinators that the executor can dispatch
//! faster. More fundamentally: they give the `core.proof.tactics`
//! stdlib (task #85) a canonical form to target when it exports
//! tactic definitions. Two stdlib tactics that are the same
//! algebraically become literally `==` after normalization, so
//! the `tactic_registry` (task #87) can dedup them across
//! imported cogs.

use crate::tactics::{TacticCombinator, TacticKind};

/// The identity element for AndThen: a 0-iteration repeat of the
/// identity simplify — matches the `skip_strategy()` emitted by
/// `user_tactic::compile_tactic` for `Quote` / `Unquote` /
/// `GoalIntro`.
pub fn skip() -> TacticCombinator {
    TacticCombinator::Repeat(
        Box::new(TacticCombinator::Single(TacticKind::Simplify)),
        0,
    )
}

/// The identity element for OrElse: a maximally-failing tactic.
/// Encoded as a `Single(Custom("fail"))` — a named tactic the
/// executor recognises as the always-failing step. Using a
/// `Single` rather than a zero-Repeat keeps `fail` distinct from
/// `skip` (both are zero-effect, but `skip` *succeeds* and
/// `fail` *fails* — different absorbing elements for AndThen vs
/// OrElse).
pub fn fail() -> TacticCombinator {
    TacticCombinator::Single(TacticKind::Custom(verum_common::Text::from("fail")))
}

/// Is this combinator structurally equivalent to `skip`?
pub fn is_skip(c: &TacticCombinator) -> bool {
    matches!(
        c,
        TacticCombinator::Repeat(inner, 0)
            if matches!(**inner, TacticCombinator::Single(TacticKind::Simplify))
    )
}

/// Is this combinator structurally equivalent to `fail`?
pub fn is_fail(c: &TacticCombinator) -> bool {
    matches!(
        c,
        TacticCombinator::Single(TacticKind::Custom(tag))
            if tag.as_str() == "fail"
    )
}

// =============================================================================
// LawId — single source of truth for the algebraic-law inventory
// =============================================================================
//
// Both this simplifier (`normalize_once` rewrites) and the
// `verum_verification::tactic_combinator` catalogue project off
// the `CANONICAL_LAW_TABLE` below.  Adding / renaming a law is a
// one-place edit.

/// Stable identifier for one canonical algebraic law.  The kebab-
/// case `name()` is what shows up in `verum tactic laws` output
/// and in the catalogue's JSON schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub enum LawId {
    SeqLeftIdentity,
    SeqRightIdentity,
    SeqAssociative,
    OrelseLeftIdentity,
    OrelseRightIdentity,
    OrelseAssociative,
    RepeatZeroIsSkip,
    RepeatOneIsBody,
    TryEqualsOrelseSkip,
    SolveOfSkipFailsWhenOpen,
    FirstOfSingletonCollapses,
    AllGoalsOfSkipIsSkip,
}

impl LawId {
    pub fn name(self) -> &'static str {
        match self {
            Self::SeqLeftIdentity => "seq-left-identity",
            Self::SeqRightIdentity => "seq-right-identity",
            Self::SeqAssociative => "seq-associative",
            Self::OrelseLeftIdentity => "orelse-left-identity",
            Self::OrelseRightIdentity => "orelse-right-identity",
            Self::OrelseAssociative => "orelse-associative",
            Self::RepeatZeroIsSkip => "repeat-zero-is-skip",
            Self::RepeatOneIsBody => "repeat-one-is-body",
            Self::TryEqualsOrelseSkip => "try-equals-orelse-skip",
            Self::SolveOfSkipFailsWhenOpen => "solve-of-skip-fails-when-open",
            Self::FirstOfSingletonCollapses => "first-of-singleton-collapses",
            Self::AllGoalsOfSkipIsSkip => "all-goals-of-skip-is-skip",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "seq-left-identity" => Some(Self::SeqLeftIdentity),
            "seq-right-identity" => Some(Self::SeqRightIdentity),
            "seq-associative" => Some(Self::SeqAssociative),
            "orelse-left-identity" => Some(Self::OrelseLeftIdentity),
            "orelse-right-identity" => Some(Self::OrelseRightIdentity),
            "orelse-associative" => Some(Self::OrelseAssociative),
            "repeat-zero-is-skip" => Some(Self::RepeatZeroIsSkip),
            "repeat-one-is-body" => Some(Self::RepeatOneIsBody),
            "try-equals-orelse-skip" => Some(Self::TryEqualsOrelseSkip),
            "solve-of-skip-fails-when-open" => Some(Self::SolveOfSkipFailsWhenOpen),
            "first-of-singleton-collapses" => Some(Self::FirstOfSingletonCollapses),
            "all-goals-of-skip-is-skip" => Some(Self::AllGoalsOfSkipIsSkip),
            _ => None,
        }
    }

    pub fn all() -> [LawId; 12] {
        [
            Self::SeqLeftIdentity,
            Self::SeqRightIdentity,
            Self::SeqAssociative,
            Self::OrelseLeftIdentity,
            Self::OrelseRightIdentity,
            Self::OrelseAssociative,
            Self::RepeatZeroIsSkip,
            Self::RepeatOneIsBody,
            Self::TryEqualsOrelseSkip,
            Self::SolveOfSkipFailsWhenOpen,
            Self::FirstOfSingletonCollapses,
            Self::AllGoalsOfSkipIsSkip,
        ]
    }
}

/// One canonical law's structured doc.
#[derive(Debug, Clone, Copy)]
pub struct LawSpec {
    pub id: LawId,
    /// Kebab-case name (must match `id.name()`).
    pub name: &'static str,
    pub lhs: &'static str,
    pub rhs: &'static str,
    pub rationale: &'static str,
}

/// **Single source of truth** for the canonical algebraic-law
/// inventory.  Both the simplifier (this module's `normalize_once`)
/// and the catalogue
/// (`verum_verification::tactic_combinator::canonical_laws`) read
/// from this table.  A law name appears here exactly once.
pub const CANONICAL_LAW_TABLE: &[LawSpec] = &[
    LawSpec {
        id: LawId::SeqLeftIdentity,
        name: "seq-left-identity",
        lhs: "skip ; t",
        rhs: "t",
        rationale: "skip is the left identity for sequential composition: prefixing any tactic with skip produces the original tactic.",
    },
    LawSpec {
        id: LawId::SeqRightIdentity,
        name: "seq-right-identity",
        lhs: "t ; skip",
        rhs: "t",
        rationale: "skip is the right identity for sequential composition: appending skip is a no-op.",
    },
    LawSpec {
        id: LawId::SeqAssociative,
        name: "seq-associative",
        lhs: "(t ; u) ; v",
        rhs: "t ; (u ; v)",
        rationale: "Sequential composition is associative — the simplifier canonicalises to right-association for dedup.",
    },
    LawSpec {
        id: LawId::OrelseLeftIdentity,
        name: "orelse-left-identity",
        lhs: "fail || t",
        rhs: "t",
        rationale: "fail is the left identity for choice: a never-succeeding alternative immediately yields to its fallback.",
    },
    LawSpec {
        id: LawId::OrelseRightIdentity,
        name: "orelse-right-identity",
        lhs: "t || fail",
        rhs: "t",
        rationale: "fail is the right identity for choice: a never-succeeding fallback can never override the primary's verdict.",
    },
    LawSpec {
        id: LawId::OrelseAssociative,
        name: "orelse-associative",
        lhs: "(t || u) || v",
        rhs: "t || (u || v)",
        rationale: "Choice is associative — the simplifier canonicalises to right-association.",
    },
    LawSpec {
        id: LawId::RepeatZeroIsSkip,
        name: "repeat-zero-is-skip",
        lhs: "repeat_n(0, t)",
        rhs: "skip",
        rationale: "Zero-iteration repetition cannot perform any work, so it collapses to skip.",
    },
    LawSpec {
        id: LawId::RepeatOneIsBody,
        name: "repeat-one-is-body",
        lhs: "repeat_n(1, t)",
        rhs: "t",
        rationale: "One-iteration repetition is just the body — the loop overhead is observable only at n ≥ 2.",
    },
    LawSpec {
        id: LawId::TryEqualsOrelseSkip,
        name: "try-equals-orelse-skip",
        lhs: "try { t }",
        rhs: "t || skip",
        rationale: "Soft-fail is desugared to a choice with skip: if t fails, the no-op alternative succeeds.",
    },
    LawSpec {
        id: LawId::SolveOfSkipFailsWhenOpen,
        name: "solve-of-skip-fails-when-open",
        lhs: "solve { skip }",
        rhs: "fail   (when goals are non-empty)",
        rationale: "solve enforces total discharge: a no-op cannot close any goal, so solve { skip } must fail whenever goals remain.",
    },
    LawSpec {
        id: LawId::FirstOfSingletonCollapses,
        name: "first-of-singleton-collapses",
        lhs: "first_of([t])",
        rhs: "t",
        rationale: "A first-of with a single alternative is operationally equivalent to that alternative.",
    },
    LawSpec {
        id: LawId::AllGoalsOfSkipIsSkip,
        name: "all-goals-of-skip-is-skip",
        lhs: "all_goals { skip }",
        rhs: "skip",
        rationale: "Applying skip to every goal is equivalent to skipping the focus operation altogether.",
    },
];

/// Lookup a law by its typed id.
pub fn law_by_id(id: LawId) -> &'static LawSpec {
    CANONICAL_LAW_TABLE
        .iter()
        .find(|s| s.id == id)
        .expect("CANONICAL_LAW_TABLE must cover every LawId variant")
}

/// Lookup a law by its kebab-case name.
pub fn law_by_name(name: &str) -> Option<&'static LawSpec> {
    CANONICAL_LAW_TABLE.iter().find(|s| s.name == name)
}

/// The subset of [`LawId`] this module's `normalize_once`
/// rewriter currently applies.  V0 covers the identity / repeat-
/// elision / OrElse-singleton subset; the remaining laws are
/// documented but not yet rewritten by the simplifier (e.g.
/// `solve-of-skip-fails-when-open` has no `Solve` constructor in
/// the Z3-side `TacticCombinator` enum yet).
///
/// Used by the catalogue's CI gate to verify that every law the
/// simplifier rewrites by is in the canonical inventory.
pub const SIMPLIFIER_APPLIES: &[LawId] = &[
    LawId::SeqLeftIdentity,
    LawId::SeqRightIdentity,
    LawId::SeqAssociative,
    LawId::OrelseLeftIdentity,
    LawId::OrelseRightIdentity,
    LawId::OrelseAssociative,
    LawId::RepeatZeroIsSkip,
    LawId::RepeatOneIsBody,
    LawId::TryEqualsOrelseSkip,
    LawId::SolveOfSkipFailsWhenOpen,
    LawId::FirstOfSingletonCollapses,
    LawId::AllGoalsOfSkipIsSkip,
];

/// Normalise a combinator to its canonical form by applying every
/// simplification law to fixpoint.
///
/// Complexity: O(n) passes where n is the depth of the tree; each
/// pass is O(tree size). Bounded by the tree size → overall
/// O(tree^2) worst-case. Callers with very deep user-written
/// combinators should bound depth at construction, not here.
pub fn normalize(c: TacticCombinator) -> TacticCombinator {
    let mut prev = format!("{:?}", c);
    let mut cur = normalize_once(c);
    loop {
        let next_repr = format!("{:?}", cur);
        if next_repr == prev {
            return cur;
        }
        prev = next_repr;
        cur = normalize_once(cur);
    }
}

/// Apply one pass of simplification laws.
fn normalize_once(c: TacticCombinator) -> TacticCombinator {
    match c {
        TacticCombinator::Single(k) => TacticCombinator::Single(k),

        TacticCombinator::AndThen(l, r) => {
            let l = normalize_once(*l);
            let r = normalize_once(*r);
            // LawId::SeqLeftIdentity — `skip ; t ≡ t`
            if is_skip(&l) {
                return r;
            }
            // LawId::SeqRightIdentity — `t ; skip ≡ t`
            if is_skip(&r) {
                return l;
            }
            // LawId::SeqAssociative — right-associate.
            // `(a ; b) ; c` becomes `a ; (b ; c)`.  Gives every
            // AndThen chain a canonical right-associated shape, so
            // two chains that differ only in bracketing compare
            // equal after normalize.
            if let TacticCombinator::AndThen(ll, lr) = l {
                return normalize_once(TacticCombinator::AndThen(
                    ll,
                    Box::new(TacticCombinator::AndThen(lr, Box::new(r))),
                ));
            }
            TacticCombinator::AndThen(Box::new(l), Box::new(r))
        }

        TacticCombinator::OrElse(l, r) => {
            let l = normalize_once(*l);
            let r = normalize_once(*r);
            // LawId::OrelseLeftIdentity — `fail | t ≡ t`
            if is_fail(&l) {
                return r;
            }
            // LawId::OrelseRightIdentity — `t | fail ≡ t`
            if is_fail(&r) {
                return l;
            }
            // LawId::OrelseAssociative — right-associate.
            // `(a || b) || c` becomes `a || (b || c)`.  Mirrors the
            // AndThen canonicalisation: two OrElse chains differing
            // only in bracketing compare equal after normalize.
            if let TacticCombinator::OrElse(ll, lr) = l {
                return normalize_once(TacticCombinator::OrElse(
                    ll,
                    Box::new(TacticCombinator::OrElse(lr, Box::new(r))),
                ));
            }
            // L9: Single(k) | Single(k) ≡ Single(k) (only for
            // identical single-leaf tactics — see module docs for
            // why this is sound but AndThen-idempotence isn't).
            // Not a member of the canonical catalogue: this is a
            // simplifier-internal optimisation that the catalogue
            // does not expose because it's a degenerate case of
            // OrElse rather than a primitive algebraic law.
            if let (
                TacticCombinator::Single(a),
                TacticCombinator::Single(b),
            ) = (&l, &r)
            {
                if a == b {
                    return TacticCombinator::Single(a.clone());
                }
            }
            TacticCombinator::OrElse(Box::new(l), Box::new(r))
        }

        TacticCombinator::Repeat(inner, 0) => {
            // LawId::RepeatZeroIsSkip — `repeat_n(0, t) ≡ skip`,
            // regardless of what t is.  This is how `Quote` /
            // `GoalIntro` compile-targets fall out during
            // normalization.
            let _ = inner;
            skip()
        }

        TacticCombinator::Repeat(inner, 1) => {
            // LawId::RepeatOneIsBody — `repeat_n(1, t) ≡ t`
            normalize_once(*inner)
        }

        TacticCombinator::Repeat(inner, n) => {
            TacticCombinator::Repeat(Box::new(normalize_once(*inner)), n)
        }

        TacticCombinator::TryFor(inner, dur) => {
            TacticCombinator::TryFor(Box::new(normalize_once(*inner)), dur)
        }

        TacticCombinator::WithParams(inner, params) => {
            TacticCombinator::WithParams(Box::new(normalize_once(*inner)), params)
        }

        TacticCombinator::IfThenElse {
            probe,
            then_tactic,
            else_tactic,
        } => TacticCombinator::IfThenElse {
            probe,
            then_tactic: Box::new(normalize_once(*then_tactic)),
            else_tactic: Box::new(normalize_once(*else_tactic)),
        },

        TacticCombinator::ParOr(branches) => {
            let mut norm_branches = verum_common::List::new();
            for b in branches {
                norm_branches.push(normalize_once(b));
            }
            TacticCombinator::ParOr(norm_branches)
        }

        TacticCombinator::Try(inner) => {
            // LawId::TryEqualsOrelseSkip — `try { t } ↪ t || skip`.
            // The catalogue rewrite desugars Try away into OrElse,
            // which is then subject to all OrElse simplifications.
            let inner_norm = normalize_once(*inner);
            normalize_once(TacticCombinator::OrElse(
                Box::new(inner_norm),
                Box::new(skip()),
            ))
        }

        TacticCombinator::FirstOf(branches) => {
            // LawId::FirstOfSingletonCollapses — `first_of([t]) ↪ t`.
            // Multi-element FirstOf reduces to a left-folded OrElse
            // chain (which the existing OrelseAssociative rule then
            // right-associates).  Empty FirstOf ↪ fail.
            let mut norm_branches: Vec<TacticCombinator> = Vec::new();
            for b in branches {
                norm_branches.push(normalize_once(b));
            }
            match norm_branches.len() {
                0 => fail(),
                1 => norm_branches.into_iter().next().unwrap(),
                _ => {
                    let mut iter = norm_branches.into_iter();
                    let first = iter.next().unwrap();
                    let rest = iter.collect::<Vec<_>>();
                    let mut acc = TacticCombinator::FirstOf({
                        let mut tail = verum_common::List::new();
                        for r in rest {
                            tail.push(r);
                        }
                        tail
                    });
                    acc = normalize_once(acc); // recursively reduce tail
                    normalize_once(TacticCombinator::OrElse(
                        Box::new(first),
                        Box::new(acc),
                    ))
                }
            }
        }

        TacticCombinator::Solve(inner) => {
            let inner_norm = normalize_once(*inner);
            // LawId catalogue: `solve-of-skip-fails` — Solve(skip)
            // can never close any goal so it's equivalent to fail.
            // `fail` is `Repeat(skip, 0)` per the canonical encoding.
            if is_skip(&inner_norm) {
                fail()
            } else {
                TacticCombinator::Solve(Box::new(inner_norm))
            }
        }

        TacticCombinator::AllGoals(inner) => {
            let inner_norm = normalize_once(*inner);
            // LawId catalogue: `all-goals-of-skip-is-skip` —
            // AllGoals(skip) preserves every goal unchanged, same
            // as skip on a single goal.
            if is_skip(&inner_norm) {
                skip()
            } else {
                TacticCombinator::AllGoals(Box::new(inner_norm))
            }
        }
    }
}

/// Check L1 (AndThen left-identity) on a specific pair.
/// Returns true if `normalize(skip ; t) == normalize(t)`.
pub fn check_andthen_left_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::AndThen(
        Box::new(skip()),
        Box::new(t.clone()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L2 (AndThen right-identity).
pub fn check_andthen_right_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::AndThen(
        Box::new(t.clone()),
        Box::new(skip()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L3 (AndThen associativity): `(a ; b) ; c ≡ a ; (b ; c)`.
pub fn check_andthen_associativity(
    a: &TacticCombinator,
    b: &TacticCombinator,
    c: &TacticCombinator,
) -> bool {
    let lhs = normalize(TacticCombinator::AndThen(
        Box::new(TacticCombinator::AndThen(
            Box::new(a.clone()),
            Box::new(b.clone()),
        )),
        Box::new(c.clone()),
    ));
    let rhs = normalize(TacticCombinator::AndThen(
        Box::new(a.clone()),
        Box::new(TacticCombinator::AndThen(
            Box::new(b.clone()),
            Box::new(c.clone()),
        )),
    ));
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L4 (OrElse left-identity).
pub fn check_orelse_left_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::OrElse(
        Box::new(fail()),
        Box::new(t.clone()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L5 (OrElse right-identity).
pub fn check_orelse_right_identity(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::OrElse(
        Box::new(t.clone()),
        Box::new(fail()),
    ));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L6 (OrElse associativity): `(a || b) || c ≡ a || (b || c)`.
pub fn check_orelse_associativity(
    a: &TacticCombinator,
    b: &TacticCombinator,
    c: &TacticCombinator,
) -> bool {
    let lhs = normalize(TacticCombinator::OrElse(
        Box::new(TacticCombinator::OrElse(
            Box::new(a.clone()),
            Box::new(b.clone()),
        )),
        Box::new(c.clone()),
    ));
    let rhs = normalize(TacticCombinator::OrElse(
        Box::new(a.clone()),
        Box::new(TacticCombinator::OrElse(
            Box::new(b.clone()),
            Box::new(c.clone()),
        )),
    ));
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

/// Check L7 (Repeat(t, 0) ≡ skip).
pub fn check_repeat_zero_is_skip(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::Repeat(Box::new(t.clone()), 0));
    format!("{:?}", lhs) == format!("{:?}", skip())
}

/// Check L8 (Repeat(t, 1) ≡ t).
pub fn check_repeat_one_is_inner(t: &TacticCombinator) -> bool {
    let lhs = normalize(TacticCombinator::Repeat(Box::new(t.clone()), 1));
    let rhs = normalize(t.clone());
    format!("{:?}", lhs) == format!("{:?}", rhs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simp() -> TacticCombinator {
        TacticCombinator::Single(TacticKind::Simplify)
    }
    fn smt() -> TacticCombinator {
        TacticCombinator::Single(TacticKind::SMT)
    }
    fn lia() -> TacticCombinator {
        TacticCombinator::Single(TacticKind::LIA)
    }

    // -- Identities ----------------------------------------------------

    #[test]
    fn l1_andthen_left_identity_simp() {
        assert!(check_andthen_left_identity(&simp()));
    }

    #[test]
    fn l2_andthen_right_identity_smt() {
        assert!(check_andthen_right_identity(&smt()));
    }

    #[test]
    fn l3_andthen_associativity_all_primitives() {
        assert!(check_andthen_associativity(&simp(), &smt(), &lia()));
    }

    #[test]
    fn l4_orelse_left_identity() {
        assert!(check_orelse_left_identity(&simp()));
    }

    #[test]
    fn l5_orelse_right_identity() {
        assert!(check_orelse_right_identity(&smt()));
    }

    #[test]
    fn l6_orelse_associativity_all_primitives() {
        // The catalogue's `LawId::OrelseAssociative` — wired into
        // the simplifier in this commit (#103).  `(a || b) || c ≡
        // a || (b || c)` must hold by structural equality after
        // normalization.
        assert!(check_orelse_associativity(&simp(), &smt(), &lia()));
    }

    #[test]
    fn normalize_right_associates_orelse_chains() {
        // The simplifier should produce a canonical right-associated
        // shape so two chains differing only in bracketing compare
        // equal as Rust Debug strings (the same canonicalisation
        // AndThen has shipped since V0).
        let left_assoc = TacticCombinator::OrElse(
            Box::new(TacticCombinator::OrElse(
                Box::new(simp()),
                Box::new(smt()),
            )),
            Box::new(lia()),
        );
        let right_assoc = TacticCombinator::OrElse(
            Box::new(simp()),
            Box::new(TacticCombinator::OrElse(
                Box::new(smt()),
                Box::new(lia()),
            )),
        );
        let n_left = format!("{:?}", normalize(left_assoc));
        let n_right = format!("{:?}", normalize(right_assoc));
        assert_eq!(n_left, n_right);
    }

    #[test]
    fn simplifier_applies_includes_orelse_associative() {
        // Pin: the SIMPLIFIER_APPLIES list now includes
        // OrelseAssociative — the catalogue ↔ simplifier
        // single-source-of-truth invariant must continue to hold.
        assert!(SIMPLIFIER_APPLIES.contains(&LawId::OrelseAssociative));
    }

    // -- Try / FirstOf desugaring (#107) --------------------------------

    #[test]
    fn try_desugars_to_orelse_with_skip() {
        // catalogue rule `try-equals-orelse-skip`:
        // `try { t } ↪ t || skip`
        let t = TacticCombinator::Try(Box::new(smt()));
        let n = normalize(t);
        let want = normalize(TacticCombinator::OrElse(
            Box::new(smt()),
            Box::new(skip()),
        ));
        assert_eq!(format!("{:?}", n), format!("{:?}", want));
    }

    #[test]
    fn try_followed_by_skip_simplification_collapses() {
        // try { t } where the catalogue desugar produces `t || skip`,
        // and `t || skip` is NOT a fail-eliminating shape — we
        // expect the post-desugar form to remain `t || skip`.
        // (skip is the right identity of AndThen, NOT OrElse — so
        // the OrElse(t, skip) form is canonical.)
        let t = TacticCombinator::Try(Box::new(simp()));
        let n = normalize(t);
        // After normalisation, expect OrElse(simp, skip).
        match n {
            TacticCombinator::OrElse(_, _) => {}
            other => panic!("expected OrElse(_, _), got {:?}", other),
        }
    }

    #[test]
    fn first_of_singleton_collapses_to_inner() {
        // catalogue rule `first-of-singleton-collapses`:
        // `first_of([t]) ↪ t`
        let mut single = verum_common::List::new();
        single.push(smt());
        let n = normalize(TacticCombinator::FirstOf(single));
        assert_eq!(format!("{:?}", n), format!("{:?}", smt()));
    }

    #[test]
    fn first_of_empty_reduces_to_fail() {
        let n = normalize(TacticCombinator::FirstOf(verum_common::List::new()));
        assert!(is_fail(&n));
    }

    #[test]
    fn first_of_two_arg_reduces_to_orelse_chain() {
        let mut branches = verum_common::List::new();
        branches.push(simp());
        branches.push(smt());
        let n = normalize(TacticCombinator::FirstOf(branches));
        let want = normalize(TacticCombinator::OrElse(
            Box::new(simp()),
            Box::new(smt()),
        ));
        assert_eq!(format!("{:?}", n), format!("{:?}", want));
    }

    #[test]
    fn first_of_three_arg_right_associates_via_orelse_assoc() {
        // FirstOf([a, b, c]) ↪ OrElse(a, OrElse(b, c)).
        let mut branches = verum_common::List::new();
        branches.push(simp());
        branches.push(smt());
        branches.push(lia());
        let n = normalize(TacticCombinator::FirstOf(branches));
        let want = normalize(TacticCombinator::OrElse(
            Box::new(simp()),
            Box::new(TacticCombinator::OrElse(
                Box::new(smt()),
                Box::new(lia()),
            )),
        ));
        assert_eq!(format!("{:?}", n), format!("{:?}", want));
    }

    #[test]
    fn simplifier_applies_includes_try_and_first_of() {
        assert!(SIMPLIFIER_APPLIES.contains(&LawId::TryEqualsOrelseSkip));
        assert!(SIMPLIFIER_APPLIES.contains(&LawId::FirstOfSingletonCollapses));
    }

    #[test]
    fn task_107_canonical_law_count_now_twelve_of_twelve() {
        // Pin: all 12 catalogue laws are now wired (V0 was 7;
        // #103 added OrelseAssociative; #107 added Try +
        // FirstOfSingleton; #108 added Solve + AllGoals).  Closes
        // the catalogue ↔ simplifier single-source-of-truth gap.
        assert_eq!(SIMPLIFIER_APPLIES.len(), 12);
        assert_eq!(SIMPLIFIER_APPLIES.len(), LawId::all().len());
    }

    // -- Solve / AllGoals desugaring (#108) -----------------------------

    #[test]
    fn solve_of_skip_reduces_to_fail() {
        // catalogue rule `solve-of-skip-fails-when-open`:
        // skip never closes any goal so Solve(skip) must fail
        // unconditionally.
        let n = normalize(TacticCombinator::Solve(Box::new(skip())));
        assert!(is_fail(&n));
    }

    #[test]
    fn solve_of_non_skip_preserves_constructor() {
        let n = normalize(TacticCombinator::Solve(Box::new(smt())));
        match n {
            TacticCombinator::Solve(_) => {}
            other => panic!("expected Solve(_), got {:?}", other),
        }
    }

    #[test]
    fn all_goals_of_skip_reduces_to_skip() {
        // catalogue rule `all-goals-of-skip-is-skip`:
        // skip on every goal is identical to skip on the focus.
        let n = normalize(TacticCombinator::AllGoals(Box::new(skip())));
        assert!(is_skip(&n));
    }

    #[test]
    fn all_goals_of_non_skip_preserves_constructor() {
        let n = normalize(TacticCombinator::AllGoals(Box::new(smt())));
        match n {
            TacticCombinator::AllGoals(_) => {}
            other => panic!("expected AllGoals(_), got {:?}", other),
        }
    }

    #[test]
    fn simplifier_applies_includes_solve_and_all_goals() {
        assert!(SIMPLIFIER_APPLIES.contains(&LawId::SolveOfSkipFailsWhenOpen));
        assert!(SIMPLIFIER_APPLIES.contains(&LawId::AllGoalsOfSkipIsSkip));
    }

    #[test]
    fn task_108_every_canonical_law_is_simplifier_applied() {
        // Pin: every entry in LawId::all() appears in
        // SIMPLIFIER_APPLIES.  No catalogue law is documentation-
        // only any more.
        for id in LawId::all() {
            assert!(
                SIMPLIFIER_APPLIES.contains(&id),
                "catalogue law `{}` is not in SIMPLIFIER_APPLIES",
                id.name()
            );
        }
    }

    #[test]
    fn l7_repeat_zero_is_skip() {
        assert!(check_repeat_zero_is_skip(&simp()));
        assert!(check_repeat_zero_is_skip(&smt()));
    }

    #[test]
    fn l8_repeat_one_is_inner() {
        assert!(check_repeat_one_is_inner(&simp()));
        // Also on a compound tactic.
        let seq = TacticCombinator::AndThen(Box::new(simp()), Box::new(smt()));
        assert!(check_repeat_one_is_inner(&seq));
    }

    // -- Normalize actually simplifies ---------------------------------

    #[test]
    fn normalize_strips_skip_on_the_left() {
        let t = TacticCombinator::AndThen(Box::new(skip()), Box::new(smt()));
        let got = format!("{:?}", normalize(t));
        let want = format!("{:?}", smt());
        assert_eq!(got, want);
    }

    #[test]
    fn normalize_strips_fail_on_the_right_of_orelse() {
        let t = TacticCombinator::OrElse(Box::new(simp()), Box::new(fail()));
        let got = format!("{:?}", normalize(t));
        let want = format!("{:?}", simp());
        assert_eq!(got, want);
    }

    #[test]
    fn normalize_collapses_single_or_single_same_kind() {
        let t = TacticCombinator::OrElse(Box::new(simp()), Box::new(simp()));
        let got = format!("{:?}", normalize(t));
        let want = format!("{:?}", simp());
        assert_eq!(got, want);
    }

    #[test]
    fn normalize_does_not_collapse_andthen_of_identical_singles() {
        // L9 only applies to OrElse — `t ; t` stays `t ; t`
        // because solver side-effects may differ per invocation.
        let t = TacticCombinator::AndThen(Box::new(simp()), Box::new(simp()));
        let normalized = normalize(t.clone());
        assert_eq!(format!("{:?}", t), format!("{:?}", normalized));
    }

    #[test]
    fn normalize_is_idempotent() {
        // Applying normalize twice yields the same tree.
        let t = TacticCombinator::AndThen(
            Box::new(skip()),
            Box::new(TacticCombinator::OrElse(
                Box::new(fail()),
                Box::new(simp()),
            )),
        );
        let once = normalize(t.clone());
        let twice = normalize(once.clone());
        assert_eq!(format!("{:?}", once), format!("{:?}", twice));
    }

    #[test]
    fn is_skip_identifies_zero_repeat_of_simplify() {
        assert!(is_skip(&skip()));
        assert!(!is_skip(&simp()));
        assert!(!is_skip(&smt()));
    }

    #[test]
    fn is_fail_identifies_zero_repeat_of_custom_fail() {
        assert!(is_fail(&fail()));
        assert!(!is_fail(&skip()));
    }

    #[test]
    fn deep_normalize_shrinks_nested_skip_andthen_chains() {
        // skip ; (skip ; (skip ; simp)) → simp
        let nested = TacticCombinator::AndThen(
            Box::new(skip()),
            Box::new(TacticCombinator::AndThen(
                Box::new(skip()),
                Box::new(TacticCombinator::AndThen(
                    Box::new(skip()),
                    Box::new(simp()),
                )),
            )),
        );
        let got = format!("{:?}", normalize(nested));
        let want = format!("{:?}", simp());
        assert_eq!(got, want);
    }
}
