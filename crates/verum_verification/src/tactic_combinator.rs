//! Industrial-grade tactic combinator catalogue — the single source
//! of truth for Verum's tactical proof-construction surface.
//!

//! ## What this module is
//!

//! Verum has multiple tactic surfaces (parser-level `TacticExpr`, Z3
//! `verum_smt::tactics::TacticCombinator`, .vr stdlib `tactic` decls
//! in `core/proof/tactics/`). Each surface evolved independently —
//! there was no single typed catalogue describing the canonical
//! combinator set, its algebraic laws, or its semantics in a form
//! consumable by IDE / CLI / documentation tooling.
//!

//! This module provides that **single trait boundary**:
//!

//!  * [`TacticCombinator`] — typed enum of the 15 canonical
//!  combinator forms (sequencing / choice / iteration / focus /
//!  forward-style / explicit instantiation / decision-procedure
//!  ergonomics).
//!  * [`TacticEntry`] — a structured-doc record (name + signature
//!  + semantics + laws + example).
//!  * [`TacticCatalog`] — single-trait dispatch interface; LSP /
//!  CLI / docs-generator consume the same catalogue.
//!  * [`DefaultTacticCatalog`] — V0 reference catalogue covering
//!  every combinator listed in the #76 acceptance criteria.
//!  * [`AlgebraicLaw`] — typed inventory of the algebraic laws
//!  (`skip ; t ≡ t`, `(t ; u) ; v ≡ t ; (u ; v)`, etc.) the
//!  `verum_smt::tactic_laws` simplifier exploits.
//!

//! ## Why this is a fundamental refactor
//!

//! The pre-this-module situation:
//!

//!  * `core/proof/tactics/combinators.vr` carried prose-comment
//!  algebraic laws but they were not machine-readable; the
//!  `verum_smt::tactic_laws` Rust simplifier had its own copy.
//!  * `verum_smt::tactics::TacticCombinator` covered Z3-side
//!  primitives (Single / AndThen / OrElse / Repeat / TryFor /
//!  IfThenElse / WithParams / ParOr) but said nothing about
//!  surface-level combinators (Solve / NamedFocus / Have /
//!  ApplyWith / PerGoalSplit) that compile down to those.
//!  * No CLI / IDE entry point existed to ask "what are the
//!  combinators? what are their laws? what's the example for
//!  `solve`?".
//!

//! After this module:
//!

//!  * Every Verum surface (LSP completion, docs generator,
//!  `verum tactic` CLI) consumes [`DefaultTacticCatalog`] for
//!  authoritative metadata.
//!  * Adapter chaining via [`CompositeTacticCatalog`] lets domain-
//!  specific catalogues (cubical, stochastic, MSFS) extend the
//!  base set without forking.
//!

//! ## Foundation-neutral
//!

//! The catalogue carries semantics in human-readable text (not as
//! executable code) — execution lives in
//! `verum_smt::tactics::apply_combinator`. The catalogue is the
//! *naming + documentation + laws* layer; the executor is the
//! *operational* layer. Single-responsibility on both sides.

use serde::{Deserialize, Serialize};
use verum_common::Text;
use verum_smt::tactic_laws::{CANONICAL_LAW_TABLE, LawId};

// =============================================================================
// TacticCombinator — the 15 canonical forms
// =============================================================================

/// A canonical combinator form. Mirrors the `tactic_expr` rule in
/// `grammar/verum.ebnf` plus the four forward-style operators
/// (`have` / `case` / `apply X with` / per-goal split) added in #76.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TacticCombinator {
    /// `skip` — identity for `Seq`. Always succeeds, leaves state
    /// unchanged.
    Skip,
    /// `fail` — identity for `OrElse`. Never succeeds.
    Fail,
    /// `t1 ; t2` — sequential composition. Runs `t1`, then `t2` on
    /// every resulting subgoal.
    Seq,
    /// `t1 || t2` — choice. Try `t1`; on failure, try `t2`.
    OrElse,
    /// `repeat t` — unbounded repetition. Stops at fixpoint or when
    /// `t` fails / makes no progress.
    Repeat,
    /// `repeat n t` — bounded repetition. At most `n` iterations.
    RepeatN,
    /// `try t` — soft-fail. Run `t`; if it fails, succeed silently.
    /// Equivalent to `t || skip`.
    Try,
    /// `solve t` — total-discharge guard. Run `t`; if any goal
    /// remains open, FAIL the whole tactic.
    Solve,
    /// `first { t1; t2; …; tn }` — first-success choice.
    FirstOf,
    /// `all_goals { t }` — apply `t` to every open goal.
    AllGoals,
    /// `i: t` — focus on the `i`-th goal (1-based).
    IndexFocus,
    /// `case foo => t` — focus on the goal labelled `foo`.
    NamedFocus,
    /// `[t1; t2; …; tn]` — per-goal split: apply `ti` to the `i`-th
    /// goal. Fails if the goal count differs from `n`.
    PerGoalSplit,
    /// `have h : T := pt` — forward-style hypothesis introduction.
    Have,
    /// `apply X with [a, b, …]` — explicit-instantiation lemma
    /// application.
    ApplyWith,
}

/// Per-variant projection for [`TacticCombinator`].
///
/// Four parallel match blocks (legacy `name` / `from_name` / `all` /
/// `category`) collapse to per-variant fields. Adding a new
/// combinator forces the docs-side TOC ordering, the canonical
/// keyword, and the category bucket to land together in one place.
#[derive(Debug, Clone, Copy)]
pub struct TacticCombinatorMeta {
    pub name: &'static str,
    pub category: CombinatorCategory,
}

impl TacticCombinator {
    /// All 15 canonical combinators, in canonical reading order
    /// (matches the docs-generator's TOC).
    pub const ALL: &'static [Self] = &[
        Self::Skip,
        Self::Fail,
        Self::Seq,
        Self::OrElse,
        Self::Repeat,
        Self::RepeatN,
        Self::Try,
        Self::Solve,
        Self::FirstOf,
        Self::AllGoals,
        Self::IndexFocus,
        Self::NamedFocus,
        Self::PerGoalSplit,
        Self::Have,
        Self::ApplyWith,
    ];

    pub const fn meta(self) -> TacticCombinatorMeta {
        match self {
            Self::Skip => TacticCombinatorMeta {
                name: "skip",
                category: CombinatorCategory::Identity,
            },
            Self::Fail => TacticCombinatorMeta {
                name: "fail",
                category: CombinatorCategory::Identity,
            },
            Self::Seq => TacticCombinatorMeta {
                name: "seq",
                category: CombinatorCategory::Composition,
            },
            Self::OrElse => TacticCombinatorMeta {
                name: "orelse",
                category: CombinatorCategory::Composition,
            },
            Self::Repeat => TacticCombinatorMeta {
                name: "repeat",
                category: CombinatorCategory::Control,
            },
            Self::RepeatN => TacticCombinatorMeta {
                name: "repeat_n",
                category: CombinatorCategory::Control,
            },
            Self::Try => TacticCombinatorMeta {
                name: "try",
                category: CombinatorCategory::Control,
            },
            Self::Solve => TacticCombinatorMeta {
                name: "solve",
                category: CombinatorCategory::Control,
            },
            Self::FirstOf => TacticCombinatorMeta {
                name: "first_of",
                category: CombinatorCategory::Composition,
            },
            Self::AllGoals => TacticCombinatorMeta {
                name: "all_goals",
                category: CombinatorCategory::Focus,
            },
            Self::IndexFocus => TacticCombinatorMeta {
                name: "index_focus",
                category: CombinatorCategory::Focus,
            },
            Self::NamedFocus => TacticCombinatorMeta {
                name: "named_focus",
                category: CombinatorCategory::Focus,
            },
            Self::PerGoalSplit => TacticCombinatorMeta {
                name: "per_goal_split",
                category: CombinatorCategory::Focus,
            },
            Self::Have => TacticCombinatorMeta {
                name: "have",
                category: CombinatorCategory::Forward,
            },
            Self::ApplyWith => TacticCombinatorMeta {
                name: "apply_with",
                category: CombinatorCategory::Forward,
            },
        }
    }

    /// Stable diagnostic name (matches the surface keyword).
    #[inline]
    pub const fn name(self) -> &'static str {
        self.meta().name
    }

    /// Convenience synonym for `name` matching the meta() series
    /// idiom across the codebase.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.meta().name
    }

    /// Parse a combinator from its diagnostic name.
    pub fn from_name(name: &str) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            let v = Self::ALL[i];
            if v.meta().name.as_bytes() == name.as_bytes() {
                return Some(v);
            }
            i += 1;
        }
        None
    }

    /// Convenience synonym for `from_name`.
    #[inline]
    pub fn from_str(name: &str) -> Option<Self> {
        Self::from_name(name)
    }

    /// All 15 canonical combinators (preserved as a fixed-size
    /// array for source compat; new callers should prefer `ALL`).
    pub fn all() -> [TacticCombinator; 15] {
        let mut out = [Self::Skip; 15];
        let mut i = 0;
        while i < 15 {
            out[i] = Self::ALL[i];
            i += 1;
        }
        out
    }

    /// Conceptual category (used by docs / `verum tactic list`
    /// grouping headers).
    #[inline]
    pub const fn category(self) -> CombinatorCategory {
        self.meta().category
    }
}

/// Conceptual category for a combinator — used purely for grouping
/// in human-readable output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CombinatorCategory {
    /// `skip` / `fail` — identity elements.
    Identity,
    /// `seq` / `orelse` / `first_of` — combine multiple tactics.
    Composition,
    /// `repeat` / `try` / `solve` — control evaluation flow.
    Control,
    /// `all_goals` / `*_focus` / `per_goal_split` — direct attention
    /// across the open-goal stack.
    Focus,
    /// `have` / `apply_with` — Lean-style forward reasoning.
    Forward,
}

/// Per-variant projection for [`CombinatorCategory`].
#[derive(Debug, Clone, Copy)]
pub struct CombinatorCategoryMeta {
    pub name: &'static str,
}

impl CombinatorCategory {
    pub const ALL: &'static [Self] = &[
        Self::Identity,
        Self::Composition,
        Self::Control,
        Self::Focus,
        Self::Forward,
    ];

    pub const fn meta(self) -> CombinatorCategoryMeta {
        match self {
            Self::Identity => CombinatorCategoryMeta { name: "identity" },
            Self::Composition => CombinatorCategoryMeta {
                name: "composition",
            },
            Self::Control => CombinatorCategoryMeta { name: "control" },
            Self::Focus => CombinatorCategoryMeta { name: "focus" },
            Self::Forward => CombinatorCategoryMeta { name: "forward" },
        }
    }

    #[inline]
    pub const fn name(self) -> &'static str {
        self.meta().name
    }

    #[inline]
    pub const fn as_str(self) -> &'static str {
        self.meta().name
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }
}

// =============================================================================
// AlgebraicLaw — typed inventory of the simplifier's normalisation rules
// =============================================================================

/// One algebraic identity satisfied by the combinators. These laws
/// are the simplifier's normalisation rule-set; the catalogue surfaces
/// them as machine-readable data so the docs generator and the
/// simplifier share a single source of truth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlgebraicLaw {
    /// Short human-readable name (`"seq-left-identity"`, etc.).
    pub name: Text,
    /// Combinators participating in the law.
    pub participants: Vec<TacticCombinator>,
    /// Left-hand side as a symbolic expression.
    pub lhs: Text,
    /// Right-hand side.
    pub rhs: Text,
    /// One-sentence rationale describing the underlying algebra.
    pub rationale: Text,
}

// =============================================================================
// TacticEntry — a structured-doc record per combinator
// =============================================================================

/// One catalogue entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticEntry {
    /// The combinator this entry describes.
    pub combinator: TacticCombinator,
    /// The full surface signature (e.g. `repeat_n(n: Int, t: Tactic)`).
    pub signature: Text,
    /// Operational semantics in one sentence.
    pub semantics: Text,
    /// A canonical example expression.
    pub example: Text,
    /// Algebraic laws this combinator participates in (subset of
    /// [`TacticCatalog::laws`]).
    pub laws: Vec<Text>,
    /// Stable doc-link target (the docs-generator emits an anchor
    /// whose name matches this field).
    pub doc_anchor: Text,
}

// =============================================================================
// TacticCatalog — the trait boundary
// =============================================================================

/// Single dispatch interface for the canonical combinator catalogue.
///

/// Contract:
///

///  * `entries()` returns one [`TacticEntry`] per combinator covered.
///  * `lookup(name)` returns `Some` for every name the catalogue
///  ships; `None` for unknown names.
///  * `laws()` returns the algebraic laws relevant to the catalogue's
///  combinators — used by the docs generator and the runtime
///  simplifier alike.
///

/// Implementations MAY restrict their entry set (e.g. a cubical-only
/// catalogue could ship only the path-induction-style combinators).
/// The reference [`DefaultTacticCatalog`] covers all 15 canonical
/// forms.
pub trait TacticCatalog {
    /// Every entry the catalogue ships.
    fn entries(&self) -> Vec<TacticEntry>;
    /// Lookup by stable diagnostic name.
    fn lookup(&self, name: &str) -> Option<TacticEntry>;
    /// Algebraic laws relevant to the entries the catalogue ships.
    fn laws(&self) -> Vec<AlgebraicLaw>;
}

// =============================================================================
// DefaultTacticCatalog — V0 reference (all 15 combinators)
// =============================================================================

/// V0 reference catalogue. Every combinator listed in the #76
/// acceptance criteria is shipped with full structured doc.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTacticCatalog;

impl DefaultTacticCatalog {
    pub fn new() -> Self {
        Self
    }
}

impl TacticCatalog for DefaultTacticCatalog {
    fn entries(&self) -> Vec<TacticEntry> {
        TacticCombinator::all()
            .iter()
            .map(|&c| entry_for(c))
            .collect()
    }

    fn lookup(&self, name: &str) -> Option<TacticEntry> {
        TacticCombinator::from_name(name).map(entry_for)
    }

    fn laws(&self) -> Vec<AlgebraicLaw> {
        canonical_laws()
    }
}

/// Per-combinator entry constructor. Single source of truth for
/// `signature` / `semantics` / `example` / participating laws.
fn entry_for(c: TacticCombinator) -> TacticEntry {
    let (signature, semantics, example, laws): (&str, &str, &str, &[&str]) = match c {
        TacticCombinator::Skip => (
            "skip()",
            "Identity tactic. Always succeeds, leaves the proof state unchanged.",
            "if has_hypothesis(h) { intro } else { skip }",
            &["seq-left-identity", "seq-right-identity"],
        ),
        TacticCombinator::Fail => (
            "fail()",
            "Always-failing tactic. Identity element for OrElse — `fail || t ≡ t`.",
            "first { specialised_tactic | fail }   // forces user to provide a working alternative",
            &["orelse-left-identity"],
        ),
        TacticCombinator::Seq => (
            "seq(first: Tactic, then: Tactic)",
            "Sequential composition: run `first`, then run `then` on every resulting subgoal.",
            "intro ; split ; auto",
            &["seq-left-identity", "seq-right-identity", "seq-associative"],
        ),
        TacticCombinator::OrElse => (
            "orelse(primary: Tactic, fallback: Tactic)",
            "Choice: try `primary`; if it fails, try `fallback`. The first success wins.",
            "ring || nlinarith",
            &[
                "orelse-left-identity",
                "orelse-right-identity",
                "orelse-associative",
            ],
        ),
        TacticCombinator::Repeat => (
            "repeat(body: Tactic)",
            "Unbounded repetition. Runs `body` until it fails or makes no progress (fixpoint).",
            "repeat { simp ; rewrite_with(assoc) }",
            &[],
        ),
        TacticCombinator::RepeatN => (
            "repeat_n(count: Int, body: Tactic)",
            "Bounded repetition. Runs `body` at most `count` times.",
            "repeat_n(3, simp)",
            &["repeat-zero-is-skip", "repeat-one-is-body"],
        ),
        TacticCombinator::Try => (
            "try(body: Tactic)",
            "Soft-fail. Runs `body`; if it fails, the proof state is unchanged and `try` still succeeds.",
            "try { norm_num } ; auto",
            &["try-equals-orelse-skip"],
        ),
        TacticCombinator::Solve => (
            "solve(body: Tactic)",
            "Total-discharge guard. Runs `body`; if any open goal remains, the whole tactic FAILS.",
            "solve { intro ; auto }   // commits to fully closing the goal",
            &["solve-of-skip-fails-when-open"],
        ),
        TacticCombinator::FirstOf => (
            "first_of(alternatives: List<Tactic>)",
            "First-success choice. Tries each alternative in order; the first success wins.",
            "first { refl | assumption | auto | smt }",
            &["first-of-singleton-collapses"],
        ),
        TacticCombinator::AllGoals => (
            "all_goals(body: Tactic)",
            "Apply `body` to every open goal independently. Fails if `body` fails on any goal.",
            "split ; all_goals { auto }",
            &["all-goals-of-skip-is-skip"],
        ),
        TacticCombinator::IndexFocus => (
            "index_focus(index: Int, body: Tactic)",
            "Focus on the i-th goal (1-based). Runs `body` on that goal alone; other goals are preserved.",
            "split ; 1: { auto } ; 2: { ring }",
            &[],
        ),
        TacticCombinator::NamedFocus => (
            "named_focus(label: Text, body: Tactic)",
            "Focus on the goal labelled `label`. Goal labels come from `intro_as` / `case` introductions.",
            "destruct h ; case left => { auto } ; case right => { contradiction }",
            &[],
        ),
        TacticCombinator::PerGoalSplit => (
            "per_goal_split(branches: List<Tactic>)",
            "Distribute `branches` across the open goals one-to-one. Fails if the goal count differs from the branch count.",
            "split ; [ auto ; ring ]",
            &[],
        ),
        TacticCombinator::Have => (
            "have(name: Text, ty: Type, proof: Tactic)",
            "Forward-style hypothesis introduction. Proves `ty` via `proof`, binds it as `name`, and continues.",
            "have h : x > 0 := { norm_num } ; rewrite_with(h)",
            &[],
        ),
        TacticCombinator::ApplyWith => (
            "apply_with(lemma: Text, args: List<Expr>)",
            "Explicit-instantiation lemma application. Useful when type inference can't pick the right witness.",
            "apply add_comm with [a, b]",
            &[],
        ),
    };
    TacticEntry {
        combinator: c,
        signature: Text::from(signature),
        semantics: Text::from(semantics),
        example: Text::from(example),
        laws: laws.iter().map(|s| Text::from(*s)).collect(),
        doc_anchor: Text::from(format!("tactic-{}", c.name().replace('_', "-"))),
    }
}

/// Map a [`LawId`] to the surface combinators that participate in it.
///

/// The catalogue's `participants` field is the *only* per-law metadata
/// the verification crate adds on top of the `verum_smt::tactic_laws`
/// canonical inventory — name / lhs / rhs / rationale all come from
/// the single source of truth (`CANONICAL_LAW_TABLE`).
fn participants_for(id: LawId) -> Vec<TacticCombinator> {
    use TacticCombinator as TC;
    match id {
        LawId::SeqLeftIdentity => vec![TC::Skip, TC::Seq],
        LawId::SeqRightIdentity => vec![TC::Skip, TC::Seq],
        LawId::SeqAssociative => vec![TC::Seq],
        LawId::OrelseLeftIdentity => vec![TC::Fail, TC::OrElse],
        LawId::OrelseRightIdentity => vec![TC::Fail, TC::OrElse],
        LawId::OrelseAssociative => vec![TC::OrElse],
        LawId::RepeatZeroIsSkip => vec![TC::RepeatN, TC::Skip],
        LawId::RepeatOneIsBody => vec![TC::RepeatN],
        LawId::TryEqualsOrelseSkip => vec![TC::Try, TC::OrElse, TC::Skip],
        LawId::SolveOfSkipFailsWhenOpen => vec![TC::Solve, TC::Skip],
        LawId::FirstOfSingletonCollapses => vec![TC::FirstOf],
        LawId::AllGoalsOfSkipIsSkip => vec![TC::AllGoals, TC::Skip],
    }
}

/// Canonical algebraic-laws inventory.
///

/// **Single source of truth** — the prose (`name` / `lhs` / `rhs` /
/// `rationale`) is projected from
/// [`verum_smt::tactic_laws::CANONICAL_LAW_TABLE`]. Adding or
/// renaming a law happens in one place (the `verum_smt` table) and
/// propagates here automatically; this crate only contributes the
/// per-law `participants` mapping over its own `TacticCombinator`
/// enum.
fn canonical_laws() -> Vec<AlgebraicLaw> {
    CANONICAL_LAW_TABLE
        .iter()
        .map(|spec| AlgebraicLaw {
            name: Text::from(spec.name),
            participants: participants_for(spec.id),
            lhs: Text::from(spec.lhs),
            rhs: Text::from(spec.rhs),
            rationale: Text::from(spec.rationale),
        })
        .collect()
}

// =============================================================================
// CompositeTacticCatalog — adapter chaining
// =============================================================================

/// Combine multiple catalogues — the base [`DefaultTacticCatalog`] +
/// domain-specific extensions (cubical, stochastic, MSFS). Lookup
/// queries each in order; entries from earlier catalogues shadow
/// later ones with the same name. Laws are unioned (deduplicated by
/// name).
pub struct CompositeTacticCatalog {
    pub catalogs: Vec<Box<dyn TacticCatalog + Send + Sync>>,
}

impl CompositeTacticCatalog {
    pub fn new(catalogs: Vec<Box<dyn TacticCatalog + Send + Sync>>) -> Self {
        Self { catalogs }
    }
}

impl std::fmt::Debug for CompositeTacticCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CompositeTacticCatalog {{ catalogs: <{}> }}",
            self.catalogs.len()
        )
    }
}

impl TacticCatalog for CompositeTacticCatalog {
    fn entries(&self) -> Vec<TacticEntry> {
        let mut seen: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
        let mut out: Vec<TacticEntry> = Vec::new();
        for c in &self.catalogs {
            for e in c.entries() {
                if seen.insert(e.combinator.name()) {
                    out.push(e);
                }
            }
        }
        out
    }

    fn lookup(&self, name: &str) -> Option<TacticEntry> {
        for c in &self.catalogs {
            if let Some(e) = c.lookup(name) {
                return Some(e);
            }
        }
        None
    }

    fn laws(&self) -> Vec<AlgebraicLaw> {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut out: Vec<AlgebraicLaw> = Vec::new();
        for c in &self.catalogs {
            for l in c.laws() {
                if seen.insert(l.name.as_str().to_string()) {
                    out.push(l);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- TacticCombinator basics -----

    #[test]
    fn all_returns_fifteen() {
        assert_eq!(TacticCombinator::all().len(), 15);
    }

    #[test]
    fn from_name_round_trip() {
        for &c in &TacticCombinator::all() {
            assert_eq!(TacticCombinator::from_name(c.name()), Some(c));
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(TacticCombinator::from_name(""), None);
        assert_eq!(TacticCombinator::from_name("garbage"), None);
        assert_eq!(TacticCombinator::from_name("SKIP"), None); // case-sensitive
    }

    #[test]
    fn category_partitions_combinators() {
        use std::collections::BTreeMap;
        let mut by_cat: BTreeMap<&'static str, usize> = BTreeMap::new();
        for &c in &TacticCombinator::all() {
            *by_cat.entry(c.category().name()).or_insert(0) += 1;
        }
        // Every category receives at least one member.
        for cat in ["identity", "composition", "control", "focus", "forward"] {
            assert!(
                by_cat.get(cat).copied().unwrap_or(0) > 0,
                "category {} has no members",
                cat
            );
        }
        // Total adds up to 15.
        assert_eq!(by_cat.values().sum::<usize>(), 15);
    }

    // ----- DefaultTacticCatalog -----

    #[test]
    fn default_catalog_ships_all_entries() {
        let cat = DefaultTacticCatalog::new();
        assert_eq!(cat.entries().len(), 15);
    }

    #[test]
    fn lookup_finds_every_combinator_by_name() {
        let cat = DefaultTacticCatalog::new();
        for &c in &TacticCombinator::all() {
            let entry = cat.lookup(c.name()).expect(c.name());
            assert_eq!(entry.combinator, c);
        }
    }

    #[test]
    fn lookup_rejects_unknown() {
        let cat = DefaultTacticCatalog::new();
        assert!(cat.lookup("nonsense").is_none());
        assert!(cat.lookup("").is_none());
    }

    #[test]
    fn every_entry_has_non_empty_signature_semantics_example() {
        let cat = DefaultTacticCatalog::new();
        for e in cat.entries() {
            assert!(!e.signature.as_str().is_empty(), "{:?}", e.combinator);
            assert!(!e.semantics.as_str().is_empty(), "{:?}", e.combinator);
            assert!(!e.example.as_str().is_empty(), "{:?}", e.combinator);
            assert!(!e.doc_anchor.as_str().is_empty(), "{:?}", e.combinator);
        }
    }

    #[test]
    fn doc_anchors_are_unique() {
        use std::collections::HashSet;
        let cat = DefaultTacticCatalog::new();
        let anchors: HashSet<String> = cat
            .entries()
            .iter()
            .map(|e| e.doc_anchor.as_str().to_string())
            .collect();
        assert_eq!(anchors.len(), 15);
    }

    // ----- AlgebraicLaw -----

    #[test]
    fn laws_inventory_non_empty() {
        let cat = DefaultTacticCatalog::new();
        let laws = cat.laws();
        // The catalogue projects the canonical law-table from
        // `verum_smt::tactic_laws` — count must match `LawId::all()`.
        assert_eq!(laws.len(), LawId::all().len());
    }

    #[test]
    fn laws_match_verum_smt_canonical_table_one_to_one() {
        // Single-source-of-truth invariant: the catalogue's law set
        // equals `LawId::all()` (kebab-case names). Drift here means
        // `verum_smt::tactic_laws::CANONICAL_LAW_TABLE` and the
        // verification-crate catalogue have diverged — a regression
        // the V0→V1 hardening pass exists to prevent.
        use std::collections::BTreeSet;
        let catalogue: BTreeSet<String> = canonical_laws()
            .iter()
            .map(|l| l.name.as_str().to_string())
            .collect();
        let canonical: BTreeSet<String> = LawId::all()
            .iter()
            .map(|id| id.name().to_string())
            .collect();
        assert_eq!(catalogue, canonical);
    }

    #[test]
    fn participants_cover_every_law_id() {
        // `participants_for` is exhaustive over `LawId` (the match
        // arm count enforces this at compile time), but pin the
        // post-condition so a future refactor that switches to
        // `_ => vec![]` would surface here too.
        for id in LawId::all() {
            let parts = participants_for(id);
            assert!(
                !parts.is_empty(),
                "law `{}` has no participating combinators",
                id.name()
            );
        }
    }

    #[test]
    fn law_names_unique() {
        use std::collections::HashSet;
        let names: HashSet<String> = canonical_laws()
            .iter()
            .map(|l| l.name.as_str().to_string())
            .collect();
        assert_eq!(names.len(), canonical_laws().len());
    }

    #[test]
    fn every_law_carries_lhs_rhs_rationale() {
        for l in canonical_laws() {
            assert!(!l.lhs.as_str().is_empty(), "{}", l.name.as_str());
            assert!(!l.rhs.as_str().is_empty(), "{}", l.name.as_str());
            assert!(!l.rationale.as_str().is_empty(), "{}", l.name.as_str());
            assert!(!l.participants.is_empty(), "{}", l.name.as_str());
        }
    }

    #[test]
    fn entry_law_references_resolve_to_laws_inventory() {
        // Every law name listed in an entry MUST exist in canonical_laws()
        // (single source of truth — the docs generator relies on this).
        use std::collections::HashSet;
        let known: HashSet<String> = canonical_laws()
            .iter()
            .map(|l| l.name.as_str().to_string())
            .collect();
        let cat = DefaultTacticCatalog::new();
        for e in cat.entries() {
            for law_ref in &e.laws {
                assert!(
                    known.contains(law_ref.as_str()),
                    "{:?} references unknown law `{}`",
                    e.combinator,
                    law_ref.as_str()
                );
            }
        }
    }

    // ----- CompositeTacticCatalog -----

    #[test]
    fn composite_dedups_entries_by_name() {
        let composite = CompositeTacticCatalog::new(vec![
            Box::new(DefaultTacticCatalog::new()),
            Box::new(DefaultTacticCatalog::new()),
        ]);
        // Two copies of the default catalogue → still 15 unique entries.
        assert_eq!(composite.entries().len(), 15);
    }

    #[test]
    fn composite_dedups_laws_by_name() {
        let composite = CompositeTacticCatalog::new(vec![
            Box::new(DefaultTacticCatalog::new()),
            Box::new(DefaultTacticCatalog::new()),
        ]);
        assert_eq!(composite.laws().len(), LawId::all().len());
    }

    #[test]
    fn composite_lookup_falls_through_in_order() {
        // Empty + Default → lookup must find every combinator (falls through to Default).
        struct Empty;
        impl TacticCatalog for Empty {
            fn entries(&self) -> Vec<TacticEntry> {
                Vec::new()
            }
            fn lookup(&self, _: &str) -> Option<TacticEntry> {
                None
            }
            fn laws(&self) -> Vec<AlgebraicLaw> {
                Vec::new()
            }
        }
        let composite = CompositeTacticCatalog::new(vec![
            Box::new(Empty),
            Box::new(DefaultTacticCatalog::new()),
        ]);
        assert!(composite.lookup("solve").is_some());
        assert!(composite.lookup("apply_with").is_some());
    }

    // ----- Coverage of the 15 combinators required by #76 -----

    #[test]
    fn all_combinators_required_by_task_76_are_present() {
        // Pin the contract that the catalogue covers every combinator
        // listed in the #76 acceptance criteria. If a future refactor
        // adds or removes a combinator without updating the spec,
        // this test surfaces the divergence.
        let required: &[&str] = &[
            "skip",
            "fail",
            "seq",            // 1. sequencing
            "orelse",         // 2. choice
            "repeat",         // 3. iteration
            "repeat_n",       // 3. iteration (bounded)
            "try",            // 4. soft-fail
            "solve",          // 5. solve
            "first_of",       // (general choice)
            "all_goals",      // 7. all goals
            "index_focus",    // 6. focus by index
            "named_focus",    // 8. case foo => t
            "per_goal_split", // 6. [t1; t2; t3]
            "have",           // 9. forward-style
            "apply_with",     // 10. explicit instantiation
        ];
        let cat = DefaultTacticCatalog::new();
        for name in required {
            assert!(
                cat.lookup(name).is_some(),
                "task #76 requires `{}` — catalogue gap",
                name
            );
        }
    }

    #[test]
    fn meta_pin_tactic_combinator_round_trip_unique_and_category_partition() {
        assert_eq!(TacticCombinator::ALL.len(), 15);
        let mut seen = Vec::new();
        for v in TacticCombinator::ALL {
            let s = v.name();
            assert_eq!(
                TacticCombinator::from_name(s),
                Some(*v),
                "TacticCombinator::{:?}: name '{}' round-trip",
                v,
                s
            );
            assert_eq!(TacticCombinator::from_str(s), Some(*v));
            assert_eq!(v.as_str(), v.name());
            assert!(!seen.contains(&s), "duplicate name '{}'", s);
            seen.push(s);
        }
        assert!(TacticCombinator::from_name("__bogus__").is_none());

        // Legacy `all()` array stays in lockstep with `ALL` slice.
        let arr = TacticCombinator::all();
        assert_eq!(arr.len(), TacticCombinator::ALL.len());
        for (i, v) in TacticCombinator::ALL.iter().enumerate() {
            assert_eq!(arr[i], *v);
        }

        // Category partition: 2 Identity + 3 Composition + 4 Control
        // + 4 Focus + 2 Forward = 15.
        let mut counts = [0usize; 5];
        for v in TacticCombinator::ALL {
            let i = match v.category() {
                CombinatorCategory::Identity => 0,
                CombinatorCategory::Composition => 1,
                CombinatorCategory::Control => 2,
                CombinatorCategory::Focus => 3,
                CombinatorCategory::Forward => 4,
            };
            counts[i] += 1;
        }
        assert_eq!(counts, [2, 3, 4, 4, 2]);
        assert_eq!(counts.iter().sum::<usize>(), 15);

        // Spot pins on critical category assignments.
        assert_eq!(
            TacticCombinator::Skip.category(),
            CombinatorCategory::Identity
        );
        assert_eq!(
            TacticCombinator::FirstOf.category(),
            CombinatorCategory::Composition
        );
        assert_eq!(
            TacticCombinator::Solve.category(),
            CombinatorCategory::Control
        );
        assert_eq!(
            TacticCombinator::Have.category(),
            CombinatorCategory::Forward
        );
    }

    #[test]
    fn meta_pin_combinator_category_round_trip_unique() {
        assert_eq!(CombinatorCategory::ALL.len(), 5);
        for v in CombinatorCategory::ALL {
            let s = v.name();
            assert_eq!(CombinatorCategory::from_str(s), Some(*v));
            assert_eq!(v.as_str(), v.name());
        }
        assert!(CombinatorCategory::from_str("__bogus__").is_none());
    }
}
