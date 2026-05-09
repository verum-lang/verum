//! # Minimal proof-term checker (#157 — the trusted base)
//!

//! The smallest possible kernel that re-verifies a Verum proof from
//! a serialised proof-term certificate. This module is the explicit
//! trusted base for Verum's reference-standard kernel claim:
//! everything else in `verum_kernel/` is *infrastructure* (the apply
//! dispatcher, the bridge audits, the cross-format renderers); the
//! proof-term checker here is the *verdict authority* that an
//! independent reviewer can read top-to-bottom in one sitting.
//!

//! ## Design discipline — < 1000 LOC, hand-auditable
//!

//! The checker implements a minimal Calculus of Constructions
//! fragment with bidirectional type-checking. Six inference rules
//! are exhaustive: T-Var, T-Univ, T-Pi-Form, T-Lam-Intro, T-App-Elim,
//! T-Conv (β-conversion). No cubical, modal, or refinement
//! extensions — those layer on top via `verum_kernel`'s broader rule
//! set, and their soundness theorems are tracked separately by
//! `core/verify/kernel_soundness/`.
//!

//! The trade-off is deliberate: the checker rejects MOST Verum
//! programs (since most use refinement / cubical / modal / SMT-axiom
//! features), but the programs it accepts have an iron-clad
//! independent verdict. The full Verum kernel handles the broader
//! surface; the proof-term checker handles the irreducible core.
//!

//! ## What this DOES NOT do
//!

//! - Does NOT type-check refinement types (those need SMT).
//! - Does NOT decide propositional equality up to η-conversion
//!  beyond α + β (η is a separable extension).
//! - Does NOT inspect `@framework`-cited axioms — these are leaves
//!  that the apply-graph audit handles.
//! - Does NOT aspire to feature parity with Coq's `coqchk` — it
//!  aspires to feature parity with HOL Light's kernel: minimal,
//!  exhaustive, hand-readable.
//!

//! ## Trust delegation
//!

//! After this checker accepts a `(term, expected_type)` pair, the
//! ONLY things a reviewer needs to trust are:
//!

//!  1. This file (~600 LOC, exhaustive pattern-matching, no `unsafe`).
//!  2. The Rust compiler's correctness (or, after Phase 3 / #154,
//!  the Verum self-hosted kernel that consumes this checker's
//!  output as a verifiable artifact).
//!  3. The serialisation format of `.vproof` files (simple JSON or
//!  s-expression — separately auditable).
//!

//! Compare: HOL Light kernel ~5K LOC SML; Coq kernel ~10K LOC OCaml;
//! Lean kernel ~5K LOC C++. Verum proof-term checker target: < 1000
//! LOC Rust. Order-of-magnitude smaller trusted base than any
//! production proof assistant.

use serde::{Deserialize, Serialize};

// =============================================================================
// Universe levels — polymorphic over level variables
// =============================================================================

/// A universe level — concrete number, level variable, or expression
/// (`succ`, `max`) over them.  The kernel reasons structurally:
/// `Concrete(n).succ()` reduces to `Concrete(n+1)` (with `u32`
/// overflow rejected), but `Var("u").succ()` stays symbolic as
/// `Succ(Var("u"))`.
///
/// Equality is decided up to algebraic normalisation
/// (idempotency / commutativity of `Max`, identity at `Concrete(0)`,
/// `Max(Succ(a), Succ(b)) = Succ(Max(a, b))`, and structural
/// flattening).  The relation is sound (no false positives) and
/// complete on closed levels (everything reducible to a single
/// `Concrete` is decided exactly).  Open levels with the same
/// canonical form are equal; structurally distinct expressions
/// over the same variables are conservatively rejected — this
/// matches the algorithm Coq / Lean / Agda use for
/// non-cumulative-with-variables level comparison.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Level {
    /// `Type@n` for a concrete non-negative `n`.
    Concrete(u32),
    /// `Type@u` for a level variable `u`.  Used for universe-
    /// polymorphic schemas: `λ(A : Type@u). λ(x : A). x` is
    /// well-typed for every `u`, and the kernel checks the body
    /// without needing to instantiate `u`.
    Var(String),
    /// `l + 1`.  Stays symbolic when the inner level isn't
    /// concrete.
    Succ(Box<Level>),
    /// `max(l1, l2)`.  Normalisation flattens nested `Max`,
    /// dedupes summands, and pulls out common `Succ` prefixes.
    Max(Box<Level>, Box<Level>),
}

impl Level {
    /// Build `Concrete(n)`.
    pub fn concrete(n: u32) -> Self {
        Level::Concrete(n)
    }

    /// Build `Var(name)`.
    pub fn var(name: impl Into<String>) -> Self {
        Level::Var(name.into())
    }

    /// Compute the successor level.  On a concrete carrier,
    /// rejects `u32::MAX` overflow with `None` so the caller can
    /// emit `CheckError::UniverseOverflow` at the kernel boundary
    /// (DEFECT-2).  On symbolic carriers (`Var` / `Succ` / `Max`),
    /// always returns the wrapped form — overflow is impossible
    /// because the structure is unbounded.
    pub fn checked_succ(&self) -> Option<Self> {
        match self {
            Level::Concrete(n) => n.checked_add(1).map(Level::Concrete),
            other => Some(Level::Succ(Box::new(other.clone()))),
        }
    }

    /// Compute `max(self, other)` with algebraic simplifications:
    ///   * `max(C(n), C(m)) = C(n.max(m))` (concrete reduction)
    ///   * `max(C(0), x)    = x`           (identity at zero)
    ///   * `max(x, x)       = x`           (idempotency, post-normalisation)
    /// Otherwise, builds `Max(self, other)` after normalisation;
    /// the structural form is canonicalised by [`Self::normalize`]
    /// before the equality decision.
    pub fn max_with(self, other: Self) -> Self {
        let a = self.normalize();
        let b = other.normalize();
        match (a, b) {
            (Level::Concrete(n), Level::Concrete(m)) => Level::Concrete(n.max(m)),
            (Level::Concrete(0), x) | (x, Level::Concrete(0)) => x,
            (a, b) if a == b => a,
            (a, b) => Level::Max(Box::new(a), Box::new(b)).normalize(),
        }
    }

    /// Reduce this level to a canonical form.  Idempotent.
    ///
    /// Rules applied (bottom-up):
    ///   1. `Succ(Concrete(n))         → Concrete(n.saturating_add(1))`
    ///   2. `Max(Concrete(a), C(b))    → Concrete(a.max(b))`
    ///   3. `Max(Concrete(0), x)       → x`
    ///   4. `Max(x, x)                 → x`
    ///   5. `Max(Succ(a), Succ(b))     → Succ(Max(a, b))`
    ///   6. `Max(Max(a, b), c)         → Max(a, Max(b, c))` (flatten)
    ///      then sort summands by structural ordering for canonical form
    ///
    /// **Soundness**: every rule is a definitional equality of the
    /// underlying universe-level algebra (associative-commutative-
    /// idempotent monoid with `0` identity, plus a successor that
    /// distributes over `max`).  Two levels with the same canonical
    /// form denote the same universe; the converse does NOT hold
    /// (the algorithm is conservative on free variables — see
    /// "Equality is decided up to" in the type doc).
    pub fn normalize(self) -> Self {
        match self {
            Level::Concrete(_) | Level::Var(_) => self,

            Level::Succ(inner) => {
                let n = inner.normalize();
                match n {
                    Level::Concrete(k) => Level::Concrete(k.saturating_add(1)),
                    other => Level::Succ(Box::new(other)),
                }
            }

            Level::Max(a, b) => {
                let a = a.normalize();
                let b = b.normalize();
                level_max_canonical(a, b)
            }
        }
    }

    /// Return the concrete carrier if this level reduces to one.
    /// Used by callers that need a `u32` (e.g. JSON serialisation
    /// of legacy proof formats); returns `None` for symbolic
    /// levels.
    pub fn as_concrete(&self) -> Option<u32> {
        match self.clone().normalize() {
            Level::Concrete(n) => Some(n),
            _ => None,
        }
    }

    /// Lift the level by `by`, in the universe-cumulativity sense:
    /// the result is the level whose `succ`-chain is `by` levels
    /// longer.  Concrete carriers add via `saturating_add`; symbolic
    /// carriers wrap in `by` `Succ` constructors and renormalise.
    /// Used by [`crate::proof_checker_meta::shift_universes`] and the
    /// differential-fuzz universe-bump mutator.
    pub fn shifted_by(self, by: u32) -> Self {
        if by == 0 {
            return self;
        }
        match self {
            Level::Concrete(n) => Level::Concrete(n.saturating_add(by)),
            other => {
                let mut acc = other;
                for _ in 0..by {
                    acc = Level::Succ(Box::new(acc));
                }
                acc.normalize()
            }
        }
    }
}

impl From<u32> for Level {
    fn from(n: u32) -> Self {
        Level::Concrete(n)
    }
}

impl std::fmt::Display for Level {
    /// Render a level in a deterministic, structural form suitable
    /// for placeholder identifiers (SMT bridges, error messages,
    /// canonical hashes).  Output is parser-style:
    ///   * `Concrete(n)`  → `n`
    ///   * `Var(name)`    → `name`
    ///   * `Succ(l)`      → `succ(<l>)`
    ///   * `Max(l1, l2)`  → `max(<l1>, <l2>)`
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Level::Concrete(n) => write!(f, "{}", n),
            Level::Var(name) => write!(f, "{}", name),
            Level::Succ(inner) => write!(f, "succ({})", inner),
            Level::Max(a, b) => write!(f, "max({}, {})", a, b),
        }
    }
}

/// Build the canonical form of `Max(a, b)` assuming both arguments
/// are already normalised individually.  Flattens nested `Max`s,
/// merges concrete summands, dedupes, factors common `Succ`
/// prefixes, and lexicographically orders the remaining summands.
fn level_max_canonical(a: Level, b: Level) -> Level {
    // Flatten into a multiset of summands.
    let mut summands: Vec<Level> = Vec::new();
    flatten_max_into(a, &mut summands);
    flatten_max_into(b, &mut summands);

    // Merge all concrete summands into one (taking the maximum).
    let mut concrete: Option<u32> = None;
    let mut symbolic: Vec<Level> = Vec::new();
    for s in summands {
        match s {
            Level::Concrete(n) => concrete = Some(concrete.map_or(n, |c| c.max(n))),
            other => symbolic.push(other),
        }
    }

    // Dedupe symbolic summands (idempotency at the `max` level).
    symbolic.sort_by(level_struct_cmp);
    symbolic.dedup();

    // Factor common `Succ` prefixes when every summand (including
    // the concrete one if present) has a Succ outermost: pull the
    // common Succ out so `max(Succ(a), Succ(b)) = Succ(max(a, b))`.
    //
    // Only apply when at least two summands exist; the single-
    // summand case is handled by the trivial-`Max` collapse at the
    // bottom.
    let total_summands = symbolic.len() + concrete.is_some() as usize;
    if total_summands >= 2 {
        let succ_count = level_common_succ_depth(&concrete, &symbolic);
        if succ_count > 0 {
            let stripped_concrete = concrete.map(|n| n.saturating_sub(succ_count));
            let stripped_symbolic: Vec<Level> = symbolic
                .into_iter()
                .map(|s| level_strip_succ(s, succ_count))
                .collect();
            let inner = level_assemble_max(stripped_concrete, stripped_symbolic);
            // Re-wrap in `succ_count` Succs.
            let mut acc = inner;
            for _ in 0..succ_count {
                acc = Level::Succ(Box::new(acc));
            }
            return acc;
        }
    }

    level_assemble_max(concrete, symbolic)
}

/// Walk a (possibly-nested) `Max` and collect every summand into
/// `out`.  Non-`Max` levels are pushed verbatim.
fn flatten_max_into(level: Level, out: &mut Vec<Level>) {
    match level {
        Level::Max(a, b) => {
            flatten_max_into(*a, out);
            flatten_max_into(*b, out);
        }
        other => out.push(other),
    }
}

/// Total ordering on canonical `Level`s — used to sort `Max`
/// summands so structurally-identical sets compare equal even
/// when the build order differed.
fn level_struct_cmp(a: &Level, b: &Level) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn rank(l: &Level) -> u8 {
        match l {
            Level::Concrete(_) => 0,
            Level::Var(_) => 1,
            Level::Succ(_) => 2,
            Level::Max(_, _) => 3,
        }
    }
    match rank(a).cmp(&rank(b)) {
        Ordering::Equal => match (a, b) {
            (Level::Concrete(x), Level::Concrete(y)) => x.cmp(y),
            (Level::Var(x), Level::Var(y)) => x.cmp(y),
            (Level::Succ(x), Level::Succ(y)) => level_struct_cmp(x, y),
            (Level::Max(xa, xb), Level::Max(ya, yb)) => {
                level_struct_cmp(xa, ya).then_with(|| level_struct_cmp(xb, yb))
            }
            _ => Ordering::Equal,
        },
        other => other,
    }
}

/// Count the number of leading `Succ`s common to every summand
/// in a max — used by `level_max_canonical` to factor them out.
/// A `Concrete(n)` summand contributes `n` to the depth budget
/// (since `Concrete(n) = Succ^n(Concrete(0))` algebraically).
fn level_common_succ_depth(concrete: &Option<u32>, symbolic: &[Level]) -> u32 {
    let mut symbolic_depths: Vec<u32> = symbolic.iter().map(level_succ_depth).collect();
    if let Some(c) = *concrete {
        symbolic_depths.push(c);
    }
    symbolic_depths.into_iter().min().unwrap_or(0)
}

/// Number of `Succ` constructors at the head of `level`.
fn level_succ_depth(level: &Level) -> u32 {
    let mut depth = 0u32;
    let mut cur = level;
    while let Level::Succ(inner) = cur {
        depth = depth.saturating_add(1);
        cur = inner;
    }
    depth
}

/// Strip up to `count` leading `Succ` constructors.  For concrete
/// levels (handled separately), the caller subtracts.  For
/// symbolic levels, peels off Succs structurally.  Saturates at
/// the bare carrier if `count` exceeds the actual depth.
fn level_strip_succ(level: Level, count: u32) -> Level {
    let mut cur = level;
    let mut remaining = count;
    while remaining > 0 {
        match cur {
            Level::Succ(inner) => {
                cur = *inner;
                remaining -= 1;
            }
            other => {
                cur = other;
                break;
            }
        }
    }
    cur
}

/// Reassemble a `Max` from a concrete summand and a list of
/// symbolic summands.  Single-summand cases collapse to the
/// summand directly; the empty case (impossible in practice if
/// the caller flattens correctly) returns `Concrete(0)`.
fn level_assemble_max(concrete: Option<u32>, mut symbolic: Vec<Level>) -> Level {
    // Drop the concrete arm if it's the identity element (0) and
    // any symbolic summand exists — `max(0, x) = x`.
    let drop_zero = matches!(concrete, Some(0)) && !symbolic.is_empty();
    let concrete_to_keep = if drop_zero { None } else { concrete };

    let mut all: Vec<Level> = Vec::new();
    if let Some(n) = concrete_to_keep {
        all.push(Level::Concrete(n));
    }
    all.append(&mut symbolic);

    match all.len() {
        0 => Level::Concrete(0),
        1 => all.pop().unwrap(),
        _ => {
            // Fold left into a right-associated `Max` chain.  The
            // sort order above ensures the chain has a canonical
            // shape: `Max(a, Max(b, Max(c, d)))` with
            // `level_struct_cmp(a, b, c, d)` ascending.
            let mut iter = all.into_iter().rev();
            let last = iter.next().unwrap();
            iter.fold(last, |acc, x| Level::Max(Box::new(x), Box::new(acc)))
        }
    }
}

/// Decide definitional equality on levels — normalises both sides
/// and compares structurally.  Sound, complete on closed levels.
pub(crate) fn level_eq(a: &Level, b: &Level) -> bool {
    a.clone().normalize() == b.clone().normalize()
}

/// Shift a term up by one binder (cutoff 0).  Crate-public so the
/// NbE checker's T-Pair-Intro can lift the second-component type
/// into the Σ-codomain's binding frame without re-implementing the
/// shift walk.  Equivalent to `shift_up(term, 1, 0)` on the
/// internal helper below.
pub(crate) fn lift_term_one_binder(term: Term) -> Term {
    shift_up(term, 1, 0)
}

// =============================================================================
// Minimal CoC AST
// =============================================================================

/// A proof term. Types ARE terms (CIC-style); a "type" is a term
/// whose own type is some `Universe(level)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Term {
    /// de Bruijn-indexed variable referring to a context entry.
    /// Index 0 is the innermost binder.
    Var(usize),

    /// Universe at a [`Level`] — `Universe(l)` lives in
    /// `Universe(succ(l))`.  The carrier is structured to support
    /// universe polymorphism: a closed term may name `Universe(0)`,
    /// while a polymorphic schema names `Universe(Var("u"))`.
    /// Concrete levels are the legacy non-polymorphic case and
    /// constructed via [`Term::universe`] for source compatibility.
    Universe(Level),

    /// Dependent function type `Π(x : A). B`. The body `B` is under
    /// a binder shifting de Bruijn indices: index 0 in `B` refers to
    /// the bound argument of type `A`.
    Pi(Box<Term>, Box<Term>),

    /// Lambda abstraction `λ(x : A). body`. Carries the domain
    /// annotation so type-checking is bidirectional-from-info-rich
    /// (every binder is type-annotated; no inference of binder types).
    Lam(Box<Term>, Box<Term>),

    /// Application `f x`. Evaluation reduces to substitution of `x`
    /// for de Bruijn 0 in the body of `f`.
    App(Box<Term>, Box<Term>),

    // ---- Σ-types (FV-20): dependent pairs ----

    /// Dependent pair type `Σ(x : A). B`.  Like `Pi`, the body `B`
    /// is under a binder: de Bruijn 0 in `B` refers to the first
    /// component of the pair.  Σ encodes "exists" propositions and
    /// dependent record types — adding it doubles the reference
    /// checker's expressive power without expanding the trust base.
    Sigma(Box<Term>, Box<Term>),
    /// Pair constructor `(a, b)` for a `Σ(A, B)` type.  The kernel
    /// re-checks the pair against its claimed Σ-type via
    /// bidirectional inference (T-Pair-Intro).
    Pair(Box<Term>, Box<Term>),
    /// First projection `fst(p)`.  When `p` reduces to `Pair(a, _)`,
    /// β-projection collapses to `a`.  Otherwise the projection
    /// stays stuck (preserved by NbE as a Neutral).
    Fst(Box<Term>),
    /// Second projection `snd(p)`.  When `p` reduces to `Pair(_, b)`,
    /// β-projection collapses to `b`.
    Snd(Box<Term>),
}

impl Term {
    /// Convenience: build `Var(i)`.
    pub fn var(i: usize) -> Self {
        Term::Var(i)
    }

    /// Convenience: build `Universe(Concrete(n))` — the legacy
    /// non-polymorphic case.  Existing call sites that named
    /// `Term::universe(0)` directly continue to work via this
    /// constructor.
    pub fn universe(n: u32) -> Self {
        Term::Universe(Level::Concrete(n))
    }

    /// Convenience: build `Universe(Var(name))` — used to express
    /// universe polymorphism in proof terms.
    pub fn universe_var(name: impl Into<String>) -> Self {
        Term::Universe(Level::Var(name.into()))
    }

    /// Convenience: build `Universe(level)` for an arbitrary
    /// level expression (concrete, variable, succ, max).
    pub fn universe_at(level: Level) -> Self {
        Term::Universe(level)
    }

    /// Convenience: build `Pi(domain, body)`.
    pub fn pi(domain: Term, body: Term) -> Self {
        Term::Pi(Box::new(domain), Box::new(body))
    }

    /// Convenience: build `Lam(domain, body)`.
    pub fn lam(domain: Term, body: Term) -> Self {
        Term::Lam(Box::new(domain), Box::new(body))
    }

    /// Convenience: build `App(f, x)`.
    pub fn app(f: Term, x: Term) -> Self {
        Term::App(Box::new(f), Box::new(x))
    }

    /// Convenience: build `Sigma(domain, body)`.
    pub fn sigma(domain: Term, body: Term) -> Self {
        Term::Sigma(Box::new(domain), Box::new(body))
    }

    /// Convenience: build `Pair(fst, snd)`.
    pub fn pair(fst: Term, snd: Term) -> Self {
        Term::Pair(Box::new(fst), Box::new(snd))
    }

    /// Convenience: build `Fst(p)`.
    pub fn fst(p: Term) -> Self {
        Term::Fst(Box::new(p))
    }

    /// Convenience: build `Snd(p)`.
    pub fn snd(p: Term) -> Self {
        Term::Snd(Box::new(p))
    }
}

// =============================================================================
// Context (de Bruijn-indexed variable types)
// =============================================================================

/// Type-checking context: stack of types corresponding to bound
/// variables, with index 0 being the most-recent binder.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Inner-first stack of variable types.
    types: Vec<Term>,
}

impl Context {
    /// Construct an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the type of variable at de Bruijn index `i`. Returns
    /// `None` if the index is out of bounds (free variable).
    /// Crucially: the returned type is shifted up by `i + 1` so the
    /// caller sees it in the OUTER context's de Bruijn frame.
    pub fn lookup(&self, i: usize) -> Option<Term> {
        // The types vec is innermost-first, so var(0) is types[len-1],
        // var(1) is types[len-2], etc.
        let len = self.types.len();
        if i >= len {
            return None;
        }
        let raw = self.types[len - 1 - i].clone();
        Some(shift_up(raw, i + 1, 0))
    }

    /// Extend the context with a new binder of type `ty` (the new
    /// innermost binding). Returns a fresh context — the original
    /// is unchanged for compositionality.
    pub fn extend(&self, ty: Term) -> Self {
        let mut out = self.clone();
        out.types.push(ty);
        out
    }

    /// Number of bound variables.
    pub fn depth(&self) -> usize {
        self.types.len()
    }
}

// =============================================================================
// de Bruijn shift and substitution
// =============================================================================

/// Shift every variable index in `term` by `+amount` if its index
/// is `>= cutoff`. Used when moving a term INTO a binder context.
///
/// Universe-level variables are NOT term-level binders, so they
/// pass through unchanged — the shift is on de Bruijn indices for
/// term variables only.
pub(crate) fn shift_up(term: Term, amount: usize, cutoff: usize) -> Term {
    match term {
        Term::Var(i) => {
            if i >= cutoff {
                Term::Var(i + amount)
            } else {
                Term::Var(i)
            }
        }
        Term::Universe(level) => Term::Universe(level),
        Term::Pi(a, b) => Term::Pi(
            Box::new(shift_up(*a, amount, cutoff)),
            Box::new(shift_up(*b, amount, cutoff + 1)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(shift_up(*a, amount, cutoff)),
            Box::new(shift_up(*body, amount, cutoff + 1)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(shift_up(*f, amount, cutoff)),
            Box::new(shift_up(*x, amount, cutoff)),
        ),
        Term::Sigma(a, b) => Term::Sigma(
            Box::new(shift_up(*a, amount, cutoff)),
            Box::new(shift_up(*b, amount, cutoff + 1)),
        ),
        Term::Pair(a, b) => Term::Pair(
            Box::new(shift_up(*a, amount, cutoff)),
            Box::new(shift_up(*b, amount, cutoff)),
        ),
        Term::Fst(p) => Term::Fst(Box::new(shift_up(*p, amount, cutoff))),
        Term::Snd(p) => Term::Snd(Box::new(shift_up(*p, amount, cutoff))),
    }
}

/// Substitute `replacement` for the variable at de Bruijn index
/// `target` in `term`. Used by β-reduction: `(λ. body) x` reduces
/// to `subst(body, 0, x)`. The replacement is shifted to compensate
/// for the binders the substitution descends into.
///
/// Universe levels (and the level variables they may contain) are
/// preserved verbatim — `subst` is a TERM-level substitution, not
/// a level-level one.  Level variables, when present, behave as
/// free schematic parameters of the term.
fn subst(term: Term, target: usize, replacement: &Term) -> Term {
    match term {
        Term::Var(i) => {
            use std::cmp::Ordering;
            match i.cmp(&target) {
                Ordering::Equal => shift_up(replacement.clone(), target, 0),
                Ordering::Greater => Term::Var(i - 1),
                Ordering::Less => Term::Var(i),
            }
        }
        Term::Universe(level) => Term::Universe(level),
        Term::Pi(a, b) => Term::Pi(
            Box::new(subst(*a, target, replacement)),
            Box::new(subst(*b, target + 1, replacement)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(subst(*a, target, replacement)),
            Box::new(subst(*body, target + 1, replacement)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(subst(*f, target, replacement)),
            Box::new(subst(*x, target, replacement)),
        ),
        Term::Sigma(a, b) => Term::Sigma(
            Box::new(subst(*a, target, replacement)),
            Box::new(subst(*b, target + 1, replacement)),
        ),
        Term::Pair(a, b) => Term::Pair(
            Box::new(subst(*a, target, replacement)),
            Box::new(subst(*b, target, replacement)),
        ),
        Term::Fst(p) => Term::Fst(Box::new(subst(*p, target, replacement))),
        Term::Snd(p) => Term::Snd(Box::new(subst(*p, target, replacement))),
    }
}

/// Fuel ceiling for `whnf` head-reduction.  CoC-typed inputs
/// strongly normalise, so this bound is never reached by a well-
/// typed certificate; ill-typed inputs (e.g. the Curry/Turing
/// fix-point combinator unfolded eagerly) would otherwise loop the
/// kernel forever.  See `DEFECT-3` in
/// `docs/architecture/verum-kernel-audit-2026.md`.
const WHNF_FUEL_CEILING: usize = 1 << 20; // 1,048,576 head-reductions.

/// β-reduce the head of a term to weak head normal form. Repeats
/// until no top-level redex remains.
///

/// **DEFECT-3 fix (kernel-audit-2026-05-08).** Now fuel-bounded.
/// Without the bound, an ill-typed input like
/// `App(λ.App(Var(0), Var(0)), λ.App(Var(0), Var(0)))` (the CoC
/// version of Ω(Ω)) would loop forever.  CoC-typed inputs
/// strongly normalise so the bound is never reached in practice,
/// but the kernel must not depend on its caller's discipline for
/// termination — fuel exhaustion now stops reduction and returns
/// the partially-reduced term.  Subsequent `def_eq` will reject the
/// pair structurally if it isn't actually equal.
fn whnf(term: Term) -> Term {
    whnf_fuel(term, WHNF_FUEL_CEILING).0
}

/// Inner whnf with explicit fuel.  Returns `(reduced, fuel_remaining)`.
/// When fuel reaches zero, returns the partially-reduced term
/// unchanged — the kernel's soundness does not depend on full
/// reduction (it depends on `def_eq` agreeing, which is structural).
fn whnf_fuel(mut term: Term, mut fuel: usize) -> (Term, usize) {
    loop {
        if fuel == 0 {
            return (term, 0);
        }
        fuel -= 1;
        match term {
            Term::App(f, x) => {
                let (f_whnf, fuel_after) = whnf_fuel(*f, fuel);
                fuel = fuel_after;
                match f_whnf {
                    Term::Lam(_, body) => {
                        if fuel == 0 {
                            return (
                                Term::App(
                                    Box::new(Term::Lam(
                                        Box::new(Term::universe(0)),
                                        body,
                                    )),
                                    x,
                                ),
                                0,
                            );
                        }
                        term = subst(*body, 0, &x);
                    }
                    other => return (Term::App(Box::new(other), x), fuel),
                }
            }

            // Σ-projection (FV-20): Fst(Pair(a, _)) → a;
            // Snd(Pair(_, b)) → b.  Stuck on non-pair head.
            Term::Fst(p) => {
                let (p_whnf, fuel_after) = whnf_fuel(*p, fuel);
                fuel = fuel_after;
                match p_whnf {
                    Term::Pair(a, _) => {
                        // Continue reducing the projected component
                        // (it may itself contain head redexes).
                        term = *a;
                    }
                    other => return (Term::Fst(Box::new(other)), fuel),
                }
            }
            Term::Snd(p) => {
                let (p_whnf, fuel_after) = whnf_fuel(*p, fuel);
                fuel = fuel_after;
                match p_whnf {
                    Term::Pair(_, b) => {
                        term = *b;
                    }
                    other => return (Term::Snd(Box::new(other)), fuel),
                }
            }

            _ => return (term, fuel),
        }
    }
}

/// α-equivalence + β-equality + η-equivalence (definitional equality
/// at the level the checker decides). Both sides are reduced to
/// WHNF and then compared structurally; under binders, α-equivalence
/// is automatic via de Bruijn indices.
///

/// **η-equivalence (T-Eta-Conv)** — `λx. (f x) ≡ f` when `x` (de
/// Bruijn 0 in the body) does not occur free in the CONTENT of `f`.
/// This is the standard CIC rule extending β with extensional
/// function equality. Brings the proof-term checker to textbook
/// CIC parity within the < 1000 LOC trust-base budget.
fn def_eq(a: &Term, b: &Term) -> bool {
    let a = whnf(a.clone());
    let b = whnf(b.clone());
    def_eq_whnf(&a, &b)
}

fn def_eq_whnf(a: &Term, b: &Term) -> bool {
    match (a, b) {
        (Term::Var(i), Term::Var(j)) => i == j,
        // Universe definitional equality is decided by canonical
        // level normalisation: closed levels reduce to a single
        // `Concrete` and compare by value; open levels with the
        // same canonical form (e.g. `Max(u, u)` vs `u`, or
        // `Max(Succ(u), Succ(v))` vs `Succ(Max(u, v))`) compare
        // equal.  See [`Level::normalize`].
        (Term::Universe(l1), Term::Universe(l2)) => level_eq(l1, l2),
        (Term::Pi(a1, b1), Term::Pi(a2, b2)) => def_eq(a1, a2) && def_eq(b1, b2),
        (Term::Lam(a1, b1), Term::Lam(a2, b2)) => def_eq(a1, a2) && def_eq(b1, b2),
        (Term::App(f1, x1), Term::App(f2, x2)) => def_eq(f1, f2) && def_eq(x1, x2),
        // Σ-types (FV-20): structural component-wise equality.  The
        // β-projection rules in `whnf_fuel` collapse `Fst(Pair(a, _))`
        // and `Snd(Pair(_, b))` BEFORE we reach this point, so any
        // residual `Fst` / `Snd` here is stuck on a non-pair head
        // (typically a bound variable or neutral application).
        (Term::Sigma(a1, b1), Term::Sigma(a2, b2)) => def_eq(a1, a2) && def_eq(b1, b2),
        (Term::Pair(a1, b1), Term::Pair(a2, b2)) => def_eq(a1, a2) && def_eq(b1, b2),
        (Term::Fst(p1), Term::Fst(p2)) => def_eq(p1, p2),
        (Term::Snd(p1), Term::Snd(p2)) => def_eq(p1, p2),
        // η-equivalence — one-sided cases. When one side is a
        // λx.(f x) and the other is `f`, they're equal iff `x`
        // does not appear free in `f`. This rule fires AFTER WHNF
        // reduction so β-redexes are eliminated first; what remains
        // is purely structural η.
        (Term::Lam(_, body), other) => eta_match(body, other),
        (other, Term::Lam(_, body)) => eta_match(body, other),
        _ => false,
    }
}

/// η-equivalence helper: returns `true` iff `lam_body` (the body of
/// a λ at depth 0) reduces to `App(f, x)` where `x` whnf-reduces to
/// `Var(0)` and `f` does not contain `Var(0)` free, AND `f` (after
/// shifting down) is equal to `other`.
///

/// This is the soundness gate for T-Eta-Conv: the bound variable
/// must not "leak" into the function part of the application.
///

/// **DEFECT-1 fix (kernel-audit-2026-05-08).** Previously the
/// argument `x` was matched syntactically against `Var(0)`; this
/// missed valid η-redexes whose argument β-reduces to `Var(0)` (e.g.
/// `λx. (f ((λy.y) x))`).  We now whnf the argument first.  This is
/// safe: `whnf` on a sub-term of a well-typed lambda body terminates
/// (CoC strong-normalisation), and any non-`Var(0)` whnf result is
/// rejected before the bound-variable-escape check.
fn eta_match(lam_body: &Term, other: &Term) -> bool {
    let app_body = whnf(lam_body.clone());
    let (f, x) = match app_body {
        Term::App(f, x) => (f, x),
        _ => return false,
    };
    // The argument must β-reduce to `Var(0)` (the bound variable).
    // Whnf the sub-term: any redex like `(λ. body) x` collapses, and
    // after collapse we expect a literal `Var(0)`.  Anything else
    // means the η-redex is not in canonical form and we conservatively
    // reject (sound but incomplete here, as in any decidable
    // η-conversion algorithm — the alternative is full normalisation
    // which costs more for the same soundness).
    let x_whnf = whnf(*x);
    if !matches!(x_whnf, Term::Var(0)) {
        return false;
    }
    // The function part must not reference Var(0) — otherwise the
    // η-rule is unsound (the variable escapes its binder).
    if is_free_in(&f, 0) {
        return false;
    }
    // Shift `f` down by one (since we're removing a binder) and
    // compare to `other`.
    let f_shifted = shift_down(*f, 1, 0);
    def_eq(&f_shifted, other)
}

/// Check whether de Bruijn index `target` occurs FREE in `term`
/// (i.e., not captured by an inner binder). Used by the η-rule
/// to ensure the bound variable doesn't leak into the function
/// part.
///
/// Universe levels never carry term-level variables, so the
/// `Universe` arm is unconditionally `false` even when the level
/// expression contains `Var` (those are LEVEL variables, not the
/// term variable being inspected).
fn is_free_in(term: &Term, target: usize) -> bool {
    match term {
        Term::Var(i) => *i == target,
        Term::Universe(_) => false,
        Term::Pi(a, b) => is_free_in(a, target) || is_free_in(b, target + 1),
        Term::Lam(a, body) => is_free_in(a, target) || is_free_in(body, target + 1),
        Term::App(f, x) => is_free_in(f, target) || is_free_in(x, target),
        Term::Sigma(a, b) => is_free_in(a, target) || is_free_in(b, target + 1),
        Term::Pair(a, b) => is_free_in(a, target) || is_free_in(b, target),
        Term::Fst(p) | Term::Snd(p) => is_free_in(p, target),
    }
}

/// Inverse of `shift_up` — decrement every variable index in `term`
/// by `amount` if its index is `>= cutoff + amount`, leaving lower
/// indices alone. Panics in debug if it would produce a negative
/// index (caller bug).
fn shift_down(term: Term, amount: usize, cutoff: usize) -> Term {
    match term {
        Term::Var(i) => {
            if i >= cutoff + amount {
                Term::Var(i - amount)
            } else if i < cutoff {
                Term::Var(i)
            } else {
                // 0 <= i - cutoff < amount → would underflow.
                // η-match's `is_free_in` precondition rules this out
                // for our use case, but defensively return the
                // unchanged variable so a caller bug is visible
                // downstream as a type-mismatch rather than a panic.
                Term::Var(i)
            }
        }
        Term::Universe(level) => Term::Universe(level),
        Term::Pi(a, b) => Term::Pi(
            Box::new(shift_down(*a, amount, cutoff)),
            Box::new(shift_down(*b, amount, cutoff + 1)),
        ),
        Term::Lam(a, body) => Term::Lam(
            Box::new(shift_down(*a, amount, cutoff)),
            Box::new(shift_down(*body, amount, cutoff + 1)),
        ),
        Term::App(f, x) => Term::App(
            Box::new(shift_down(*f, amount, cutoff)),
            Box::new(shift_down(*x, amount, cutoff)),
        ),
        Term::Sigma(a, b) => Term::Sigma(
            Box::new(shift_down(*a, amount, cutoff)),
            Box::new(shift_down(*b, amount, cutoff + 1)),
        ),
        Term::Pair(a, b) => Term::Pair(
            Box::new(shift_down(*a, amount, cutoff)),
            Box::new(shift_down(*b, amount, cutoff)),
        ),
        Term::Fst(p) => Term::Fst(Box::new(shift_down(*p, amount, cutoff))),
        Term::Snd(p) => Term::Snd(Box::new(shift_down(*p, amount, cutoff))),
    }
}

// =============================================================================
// Bidirectional type checker — the six rules (eight, post-FV-20)
// =============================================================================

/// Type-checking error. Each error names the kernel rule that
/// rejected the term, so a reviewer can trace the verdict to the
/// exact arm of `infer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// T-Var: variable index out of bounds (free variable).
    UnboundVariable(usize),
    /// T-Pi-Form / T-Lam-Intro: domain annotation isn't a type
    /// (its own type isn't a `Universe(n)`).
    NotAType(Term),
    /// T-App-Elim: function side isn't a Pi type.
    NotAFunction(Term),
    /// T-Fst-Elim / T-Snd-Elim (FV-20): the projected term's type
    /// isn't a Σ-type, so first/second-component projection has
    /// no meaning.  Carries the inferred type for diagnostics.
    NotASigma(Term),
    /// T-App-Elim: argument's type doesn't match the Pi's domain.
    DomainMismatch {
        /// Domain type expected by the Pi.
        expected: Term,
        /// Type the argument actually has.
        actual: Term,
    },
    /// T-Conv: expected and inferred types are not definitionally
    /// equal.
    TypeMismatch {
        /// Type the caller asked the term to have.
        expected: Term,
        /// Type the kernel inferred for the term.
        actual: Term,
    },
    /// **Algorithm C — bootstrap-manifest failure.**  The kernel_v0
    /// manifest contains a rule that is not audit-clean (its
    /// `DischargeStatus` is `AdmittedWithIou` or `NotYetAttested`
    /// rather than `Discharged` / `DischargedByFramework`).  The
    /// payload carries the rule name + the failing status's tag
    /// for diagnostic surface.  Algorithm A and B do not produce
    /// this variant; only the manifest-driven third slot does.
    KernelV0ManifestUnclean {
        /// Stable rule identifier (e.g. `"K-Beta"`).
        rule: String,
        /// Failing-status tag (e.g. `"admitted_with_iou"`).
        status_tag: &'static str,
    },
    /// **Algorithm C — meta-soundness footprint exceeds canonical
    /// ceiling.**  Some kernel rule's required meta-theory is not
    /// bounded by `ZFC + 2 strongly-inaccessibles`; the bootstrap
    /// claim fails its own ceiling and the manifest-driven slot
    /// refuses to admit.  Algorithm A and B do not produce this
    /// variant.
    KernelV0MetaSoundnessExceeded,
    /// **Algorithm C — strict-intrinsic dispatch disagreement.**
    /// A canonical kernel-rule's `kernel_<rule>_strict` intrinsic
    /// returned a non-positive `Decision` (or no `Decision` at
    /// all).  Surfaces drift between the bootstrap rule's
    /// soundness lemma and the registered intrinsic.  Algorithm
    /// A and B do not produce this variant.
    KernelV0StrictIntrinsicDisagreement {
        /// The intrinsic name that failed to dispatch positively.
        intrinsic: String,
    },
    /// **DEFECT-2 (kernel-audit-2026-05-08).** A `Universe(n)` term
    /// names a level whose successor `n + 1` would overflow the
    /// `u32` carrier.  In release builds, naive arithmetic wraps to
    /// zero and would emit `Universe(u32::MAX) : Universe(0)` —
    /// unsound — so the kernel hard-rejects instead.
    UniverseOverflow {
        /// The offending level.
        level: u32,
    },
    /// **DEFECT-4 (kernel-audit-2026-05-08).** A `Certificate`'s
    /// `claimed_type` failed its own well-formedness check.  A
    /// claimed type must itself have a `Universe(_)` type; without
    /// this gate, malformed terms in `claimed_type` could match
    /// a malformed inferred type via structural `def_eq` and the
    /// certificate would "verify" an ill-formed obligation.
    ClaimedTypeNotAType {
        /// The claimed-type term that failed the well-formedness check.
        claimed_type: Term,
        /// Whatever its own inferred type was (often a non-`Universe`).
        actual: Term,
    },
}

/// Infer the type of `term` in `ctx`. Returns the unique type or
/// a `CheckError` naming the rejecting kernel rule.
///

/// **The six rules at a glance.**
///

///  T-Var: `ctx[i]` = T → Var(i) : T
///  T-Univ: Universe(n) : Universe(n+1)
///  T-Pi-Form: A : Universe(n), B : Universe(m) under (A:: ctx)
///  → Pi(A, B) : Universe(max(n, m))
///  T-Lam-Intro: B : T under (A:: ctx) → Lam(A, B) : Pi(A, T)
///  T-App-Elim: f : Pi(A, B), x : A → App(f, x) : B[x/0]
///  T-Conv: T1 ≡_β T2 (definitional equality lets the checker
///  swap T1 for T2 in any judgement; used implicitly in
///  T-App-Elim to match argument types).
pub fn infer(ctx: &Context, term: &Term) -> Result<Term, CheckError> {
    match term {
        // T-Var
        Term::Var(i) => ctx
            .lookup(*i)
            .ok_or_else(|| CheckError::UnboundVariable(*i)),

        // T-Univ.  Universe(l) : Universe(succ(l)).  DEFECT-2:
        // explicit overflow check on the concrete-level arm only —
        // symbolic levels (`Var`, `Succ`, `Max`) are unbounded so
        // their successor always exists structurally.  When the
        // carrier reduces to `Concrete(u32::MAX)`, we reject with
        // `UniverseOverflow` (a wrapping `+1` would silently emit
        // `Universe(MAX) : Universe(0)` — unsound).
        Term::Universe(level) => {
            // Normalise BEFORE the overflow check so
            // `Succ(Concrete(MAX-1))` is recognised as concrete
            // `MAX` rather than treated as a symbolic carrier.
            let level = level.clone().normalize();
            match level.checked_succ() {
                Some(next) => Ok(Term::Universe(next)),
                None => Err(CheckError::UniverseOverflow {
                    level: match level {
                        Level::Concrete(n) => n,
                        // Symbolic carriers can't reach this
                        // branch — `checked_succ` only returns
                        // `None` for `Concrete(u32::MAX)`.
                        _ => u32::MAX,
                    },
                }),
            }
        }

        // T-Pi-Form.  Π(x : A). B : Universe(max(level(A), level(B)))
        // where `level(T)` is the universe carrier of T's own type.
        // For polymorphic schemas, `n` and `m` may be symbolic;
        // `Level::max_with` keeps the structural form when needed
        // and reduces to `Concrete(n.max(m))` when both sides are
        // concrete.
        Term::Pi(a, b) => {
            let a_ty = infer(ctx, a)?;
            let n = expect_universe(&a_ty).ok_or_else(|| CheckError::NotAType((**a).clone()))?;
            let extended = ctx.extend((**a).clone());
            let b_ty = infer(&extended, b)?;
            let m = expect_universe(&b_ty).ok_or_else(|| CheckError::NotAType((**b).clone()))?;
            Ok(Term::Universe(n.max_with(m)))
        }

        // T-Lam-Intro
        Term::Lam(domain, body) => {
            let dom_ty = infer(ctx, domain)?;
            // Domain annotation must be a type.
            expect_universe(&dom_ty).ok_or_else(|| CheckError::NotAType((**domain).clone()))?;
            let extended = ctx.extend((**domain).clone());
            let body_ty = infer(&extended, body)?;
            Ok(Term::Pi(domain.clone(), Box::new(body_ty)))
        }

        // T-App-Elim (with implicit T-Conv on argument matching)
        Term::App(f, x) => {
            let f_ty = whnf(infer(ctx, f)?);
            let (dom, codom) = match f_ty {
                Term::Pi(a, b) => (a, b),
                other => return Err(CheckError::NotAFunction(other)),
            };
            let x_ty = infer(ctx, x)?;
            if !def_eq(&dom, &x_ty) {
                return Err(CheckError::DomainMismatch {
                    expected: *dom,
                    actual: x_ty,
                });
            }
            Ok(subst(*codom, 0, x))
        }

        // T-Sigma-Form: Σ(x:A).B : Universe(max(level(A), level(B)))
        Term::Sigma(a, b) => {
            let a_ty = infer(ctx, a)?;
            let n = expect_universe(&a_ty).ok_or_else(|| CheckError::NotAType((**a).clone()))?;
            let extended = ctx.extend((**a).clone());
            let b_ty = infer(&extended, b)?;
            let m = expect_universe(&b_ty).ok_or_else(|| CheckError::NotAType((**b).clone()))?;
            Ok(Term::Universe(n.max_with(m)))
        }

        // T-Pair-Intro: Pair(a, b) : Sigma(A, B) where a:A and b:B[a/0]
        Term::Pair(a, b) => {
            let a_ty = infer(ctx, a)?;
            let b_ty = infer(ctx, b)?;
            // B[a/0]: the second component type may depend on the first value.
            // We form Sigma(a_ty, b_ty_shifted) where b_ty is shifted under the binder.
            let b_ty_shifted = shift_up(b_ty, 1, 0);
            Ok(Term::Sigma(Box::new(a_ty), Box::new(b_ty_shifted)))
        }

        // T-Fst-Elim: Fst(p) : A where p : Sigma(A, B)
        Term::Fst(p) => {
            let p_ty = whnf(infer(ctx, p)?);
            match p_ty {
                Term::Sigma(a, _) => Ok(*a),
                other => Err(CheckError::NotASigma(other)),
            }
        }

        // T-Snd-Elim: Snd(p) : B[Fst(p)/0] where p : Sigma(A, B).
        // The substitution is what makes Σ DEPENDENT — `B` may
        // reference the first component, and the projection of
        // the second knows what the first was at the term level.
        Term::Snd(p) => {
            let p_ty = whnf(infer(ctx, p)?);
            match p_ty {
                Term::Sigma(_, b) => Ok(subst(*b, 0, &Term::Fst(p.clone()))),
                other => Err(CheckError::NotASigma(other)),
            }
        }
    }
}

/// Check that `term` has type `expected`. Wraps `infer` + `def_eq`.
/// This is the load-bearing entry point for `verum check-proof`:
/// the .vproof file says "this term has this type", and we either
/// agree or reject.
pub fn check(ctx: &Context, term: &Term, expected: &Term) -> Result<(), CheckError> {
    let inferred = infer(ctx, term)?;
    if def_eq(&inferred, expected) {
        Ok(())
    } else {
        Err(CheckError::TypeMismatch {
            expected: expected.clone(),
            actual: inferred,
        })
    }
}

/// If `term` reduces to `Universe(level)`, return the level
/// (after normalisation so callers can equate canonical forms);
/// else `None`.
///
/// **Note**: with universe polymorphism the returned level may be
/// symbolic — concrete-only callers should use [`Level::as_concrete`]
/// to project back to `Option<u32>`.
fn expect_universe(term: &Term) -> Option<Level> {
    match whnf(term.clone()) {
        Term::Universe(level) => Some(level.normalize()),
        _ => None,
    }
}

// =============================================================================
// Proof-term certificate format
// =============================================================================

/// A `.vproof` certificate carries a self-contained type-checking
/// problem: a closed term + its claimed type. The minimal proof-
/// term checker re-verifies the pair top-to-bottom.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Certificate {
    /// The proof-term. Must be closed (no free variables).
    pub term: Term,
    /// The claimed type. Also closed.
    pub claimed_type: Term,
    /// Optional metadata: theorem name, source file, kernel-version
    /// hint. Not load-bearing — the checker doesn't read them.
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

impl Certificate {
    /// Verify the certificate. Returns `Ok(())` iff the term has the
    /// claimed type in an empty context. Any free variable in either
    /// term or type is a structural error rejected here.
    ///

    /// **DEFECT-4 fix (kernel-audit-2026-05-08).** Independently
    /// type-check `claimed_type` and confirm its own type is some
    /// `Universe(_)` *before* the inferred-vs-claimed comparison.
    /// Without this gate, an ill-formed `claimed_type` could match
    /// an ill-formed inferred type via structural `def_eq` and the
    /// certificate would "verify" an obligation that is meaningless
    /// at the kernel layer.  Now: if `claimed_type` is not itself
    /// a type, we hard-reject with `ClaimedTypeNotAType` carrying
    /// both the offending term and whatever inferred kind it had.
    pub fn verify(&self) -> Result<(), CheckError> {
        let ctx = Context::new();
        // Step 1 — claimed_type must itself be a type.
        //
        // **Top-of-tower escape hatch.**  When `claimed_type` is at
        // (or transitively contains) `Universe(u32::MAX)`, inferring
        // its own kind would emit `UniverseOverflow` — but the
        // claimed_type is still a *type* (universes are types at every
        // representable level).  Differentially-tested with the Lean
        // ReferenceChecker which uses unbounded `Nat`; without this
        // escape hatch the two kernels disagree on `Universe(MAX-1) :
        // Universe(MAX)` (`defect-2-univ-max-minus-one-ok` battery
        // row in `audit --differential-lean-checker`).
        match infer(&ctx, &self.claimed_type) {
            Ok(claimed_kind) => {
                if expect_universe(&claimed_kind).is_none() {
                    return Err(CheckError::ClaimedTypeNotAType {
                        claimed_type: self.claimed_type.clone(),
                        actual: claimed_kind,
                    });
                }
            }
            Err(CheckError::UniverseOverflow { .. }) => {
                // claimed_type lives at the top of the universe
                // tower — still a type.  Step 2 below will catch any
                // genuine type mismatch via `def_eq` against the
                // term's inferred type.
            }
            Err(other) => return Err(other),
        }
        // Step 2 — term has the claimed type.
        check(&ctx, &self.term, &self.claimed_type)
    }
}

// =============================================================================
// Tests — pin the six rules + corner cases
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_universe_lives_in_next_level() {
        // T-Univ: Universe(0) : Universe(1)
        let ctx = Context::new();
        assert_eq!(infer(&ctx, &Term::universe(0)).unwrap(), Term::universe(1));
        assert_eq!(infer(&ctx, &Term::universe(5)).unwrap(), Term::universe(6));
    }

    #[test]
    fn t_var_returns_context_type() {
        // T-Var: extend with type Universe(0); Var(0) : Universe(0).
        let ctx = Context::new().extend(Term::universe(0));
        assert_eq!(infer(&ctx, &Term::Var(0)).unwrap(), Term::universe(0));
    }

    #[test]
    fn t_var_unbound_rejects() {
        let ctx = Context::new();
        match infer(&ctx, &Term::Var(0)) {
            Err(CheckError::UnboundVariable(0)) => {}
            other => panic!("expected UnboundVariable, got {:?}", other),
        }
    }

    #[test]
    fn t_pi_form_accepts_universe_to_universe() {
        // Π(_ : Universe(0)). Universe(0) : Universe(1)
        let ctx = Context::new();
        let pi = Term::pi(Term::universe(0), Term::universe(0));
        assert_eq!(infer(&ctx, &pi).unwrap(), Term::universe(1));
    }

    #[test]
    fn t_pi_form_takes_max_universe() {
        // Π(_ : Universe(2)). Universe(5) : Universe(6)
        let ctx = Context::new();
        let pi = Term::pi(Term::universe(2), Term::universe(5));
        // Universe(2) : Universe(3); Universe(5) : Universe(6)
        // → max(3, 6) = 6
        assert_eq!(infer(&ctx, &pi).unwrap(), Term::universe(6));
    }

    #[test]
    fn t_lam_intro_produces_pi() {
        // λ(x : Universe(0)). x has type Π(_ : Universe(0)). Universe(0)
        let ctx = Context::new();
        let lam = Term::lam(Term::universe(0), Term::Var(0));
        let inferred = infer(&ctx, &lam).unwrap();
        let expected = Term::pi(Term::universe(0), Term::universe(0));
        assert!(def_eq(&inferred, &expected));
    }

    #[test]
    fn t_app_elim_with_correct_argument() {
        // (λ(x : Universe(0)). x) y where y : Universe(0) (a hypothesis).
        // Result has type Universe(0) (the codomain after substitution).
        // Note: `Universe(0)` itself does NOT have type `Universe(0)` —
        // it has type `Universe(1)`. So we can't pass `Universe(0)` as
        // an argument here; we must use a context variable typed at
        // `Universe(0)`.
        let ctx = Context::new().extend(Term::universe(0));
        let f = Term::lam(Term::universe(0), Term::Var(0));
        // Var(1) refers to the context entry (which had type Univ(0));
        // the lambda binder bumps de Bruijn indices, so the OUTER
        // hypothesis is at index 1 when we view the App at depth 0.
        // Actually no — the App is at the OUTER context depth, so
        // Var(0) refers to the hypothesis directly.
        let app = Term::app(f, Term::Var(0));
        let inferred = infer(&ctx, &app).unwrap();
        assert_eq!(inferred, Term::universe(0));
    }

    #[test]
    fn t_app_elim_rejects_non_function() {
        // App(Universe(0), Universe(0)) — applying a non-function.
        let ctx = Context::new();
        let bad = Term::app(Term::universe(0), Term::universe(0));
        match infer(&ctx, &bad) {
            Err(CheckError::NotAFunction(_)) => {}
            other => panic!("expected NotAFunction, got {:?}", other),
        }
    }

    #[test]
    fn t_app_elim_rejects_domain_mismatch() {
        // f : Π(_ : Univ(0)). Univ(0); apply to Univ(5) (whose type
        // is Univ(6), not Univ(0)) → DomainMismatch.
        let ctx = Context::new();
        let f = Term::lam(Term::universe(0), Term::Var(0));
        let bad = Term::app(f, Term::universe(5));
        // Argument Universe(5) has type Universe(6); Pi expects Univ(0).
        match infer(&ctx, &bad) {
            Err(CheckError::DomainMismatch { .. }) => {}
            // Actually — Universe(5) IS a Universe, so its TYPE is
            // Universe(6). The Pi expects something of type Universe(0).
            // 6 ≠ 0 → DomainMismatch.
            other => panic!("expected DomainMismatch, got {:?}", other),
        }
    }

    #[test]
    fn beta_reduction_resolves_application() {
        // (λx. x) y → y (where y : T, the application has type T)
        let ctx = Context::new().extend(Term::universe(0)); // y : Universe(0)
        let id = Term::lam(Term::universe(0), Term::Var(0));
        let app = Term::app(id, Term::Var(0));
        let inferred = infer(&ctx, &app).unwrap();
        // App-Elim: f : Pi(U(0), U(0)); arg Var(0) has type U(0); result type
        // = subst(U(0), 0, Var(0)) = U(0).
        assert_eq!(inferred, Term::universe(0));
    }

    #[test]
    fn certificate_verifies_correct_pair() {
        // Identity at Universe(0): λ(x:U(0)). x has type Π(_:U(0)).U(0)
        let cert = Certificate {
            term: Term::lam(Term::universe(0), Term::Var(0)),
            claimed_type: Term::pi(Term::universe(0), Term::universe(0)),
            metadata: Default::default(),
        };
        cert.verify().expect("certificate should verify");
    }

    #[test]
    fn certificate_rejects_wrong_type() {
        // Identity claims to be Universe(0) — wrong; it's a function.
        let cert = Certificate {
            term: Term::lam(Term::universe(0), Term::Var(0)),
            claimed_type: Term::universe(0),
            metadata: Default::default(),
        };
        match cert.verify() {
            Err(CheckError::TypeMismatch { .. }) => {}
            other => panic!("expected TypeMismatch, got {:?}", other),
        }
    }

    #[test]
    fn shift_up_handles_binders_correctly() {
        // shift_up(Var(0), 1, 0) → Var(1) (free var gets shifted)
        // shift_up(Lam(_, Var(0)), 1, 0) → Lam(_, Var(0)) (bound stays)
        // shift_up(Lam(_, Var(1)), 1, 0) → Lam(_, Var(2)) (free in body shifts)
        assert_eq!(shift_up(Term::Var(0), 1, 0), Term::Var(1),);
        let lam_bound = Term::lam(Term::universe(0), Term::Var(0));
        assert_eq!(shift_up(lam_bound.clone(), 1, 0), lam_bound);
        let lam_free = Term::lam(Term::universe(0), Term::Var(1));
        assert_eq!(
            shift_up(lam_free, 1, 0),
            Term::lam(Term::universe(0), Term::Var(2)),
        );
    }

    #[test]
    fn def_eq_is_alpha_plus_beta() {
        // (λx. x) y ≡_β y
        let lhs = Term::app(
            Term::lam(Term::universe(0), Term::Var(0)),
            Term::universe(7),
        );
        let rhs = Term::universe(7);
        assert!(def_eq(&lhs, &rhs));
    }

    #[test]
    fn def_eq_rejects_distinct_universes() {
        assert!(!def_eq(&Term::universe(0), &Term::universe(1)));
    }

    #[test]
    fn def_eq_eta_lam_app_equals_function() {
        // λ(x : Univ(0)). (f x) ≡_η f when f doesn't contain x.
        // We use Var(0) referring to OUTER context (a hypothesis "f"
        // present at depth 0). Inside the lambda, that becomes Var(1).
        let f_outer = Term::Var(0);
        // Inside lambda body: Var(0) is the bound x; Var(1) is f.
        let lam_eta = Term::lam(Term::universe(0), Term::app(Term::Var(1), Term::Var(0)));
        // Outer context: f : Π(_:Univ(0)).Univ(0). The lam_eta's type
        // is the same Pi, and η-equality with f_outer should hold.
        // For the def_eq test, we don't need the context — we just
        // check whether the term forms are η-equivalent.
        assert!(def_eq(&lam_eta, &f_outer));
        // Symmetry: the comparison is order-independent.
        assert!(def_eq(&f_outer, &lam_eta));
    }

    #[test]
    fn def_eq_eta_rejects_when_arg_is_not_bound_var() {
        // λ(x : Univ(0)). (f y) is NOT η-equivalent to f — the
        // application argument isn't the bound variable.
        let f = Term::Var(0); // outer context
        let lam_not_eta = Term::lam(
            Term::universe(0),
            // Var(2) inside body refers to TWO levels out, not Var(0)
            // (the bound x), so the η-rule doesn't fire.
            Term::app(Term::Var(1), Term::Var(2)),
        );
        assert!(!def_eq(&lam_not_eta, &f));
    }

    #[test]
    fn def_eq_eta_rejects_when_var_escapes_into_function() {
        // λ(x : Univ(0)). (x x) has the bound variable in the
        // FUNCTION part — η would be unsound here, must be rejected.
        let lam_unsound = Term::lam(Term::universe(0), Term::app(Term::Var(0), Term::Var(0)));
        let any_other = Term::Var(0); // outer context "any other f"
        assert!(!def_eq(&lam_unsound, &any_other));
    }

    #[test]
    fn is_free_in_handles_binders() {
        // Var(0) is free in Var(0), but bound in λ(_).Var(0)
        assert!(is_free_in(&Term::Var(0), 0));
        let lam_body_zero = Term::lam(Term::universe(0), Term::Var(0));
        // Lam(_, Var(0)) — the body's Var(0) is the bound var, NOT
        // a free reference to outer Var(0). Querying outer-target=0
        // shifts to inner-target=1 inside the body, which Var(0) is
        // NOT. So the outer Var(0) is NOT free in this term.
        assert!(!is_free_in(&lam_body_zero, 0));
        // But Var(1) inside the body IS a free reference to OUTER
        // Var(0). The outer-target=0 query shifts to target=1 in
        // the body, which matches Var(1).
        let lam_body_outer = Term::lam(Term::universe(0), Term::Var(1));
        assert!(is_free_in(&lam_body_outer, 0));
    }

    #[test]
    fn shift_down_inverse_of_shift_up() {
        // shift_down . shift_up = identity on var indices that don't
        // get clobbered.
        let original = Term::lam(Term::universe(0), Term::Var(2));
        let shifted_up = shift_up(original.clone(), 1, 0);
        let shifted_back = shift_down(shifted_up, 1, 0);
        assert_eq!(shifted_back, original);
    }

    #[test]
    fn dependent_function_type_checks() {
        // Dependent identity: Π(A : Univ(0)). Π(_ : A). A
        // (the polymorphic identity type)
        let ctx = Context::new();
        let inner_pi = Term::pi(Term::Var(0), Term::Var(1)); // _ : A; result type A (now Var(1))
        let outer_pi = Term::pi(Term::universe(0), inner_pi);
        let inferred = infer(&ctx, &outer_pi).unwrap();
        // Universe(1) — outer.A : Universe(0); body of outer is Pi
        // taking Var(0) (A) and returning Var(1) (A under one
        // additional binder). Var(0) under the outer binder has
        // type Universe(0); the inner Pi forms over it, producing
        // Universe(0). Outer Pi: max(Univ(1) for type-of-A,
        // Univ(0) for body-Pi) = Universe(1).
        assert_eq!(inferred, Term::universe(1));
    }

    // =========================================================================
    // Kernel audit 2026-05-08 — defect-pinning regression tests.
    //

    // Each test below pins a load-bearing fix from the kernel audit.
    // Removing any of them would silently regress the kernel.  See
    // docs/architecture/verum-kernel-audit-2026.md for the full ledger.
    // =========================================================================

    #[test]
    fn defect_2_universe_overflow_is_rejected() {
        // Pre-fix: Universe(u32::MAX) was accepted with a release-mode wrap
        // to Universe(0), giving "Universe(MAX) : Universe(0)" — unsound.
        // Post-fix: explicit UniverseOverflow rejection.
        let ctx = Context::new();
        match infer(&ctx, &Term::universe(u32::MAX)) {
            Err(CheckError::UniverseOverflow { level }) => {
                assert_eq!(level, u32::MAX);
            }
            other => panic!("expected UniverseOverflow, got {:?}", other),
        }
    }

    #[test]
    fn defect_2_one_below_max_universe_still_typechecks() {
        // The ceiling at `u32::MAX - 1` is the largest valid level —
        // its successor `u32::MAX` fits in u32.
        let ctx = Context::new();
        let inferred = infer(&ctx, &Term::universe(u32::MAX - 1)).unwrap();
        assert_eq!(inferred, Term::universe(u32::MAX));
    }

    #[test]
    fn defect_4_claimed_type_must_be_a_type() {
        // claimed_type is a non-type term (here a free variable that
        // cannot itself have a Universe-level type).  The certificate
        // must be rejected with ClaimedTypeNotAType, not silently
        // allowed via structural coincidence with the inferred type.
        let cert = Certificate {
            term: Term::Var(0),                  // would error UnboundVariable
            claimed_type: Term::Var(0),          // also free
            metadata: Default::default(),
        };
        // The claimed_type's `infer` runs first, producing UnboundVariable.
        // (Either UnboundVariable or ClaimedTypeNotAType is acceptable —
        // both are kernel-side rejections at the right boundary.)
        match cert.verify() {
            Err(CheckError::UnboundVariable(_))
            | Err(CheckError::ClaimedTypeNotAType { .. }) => {}
            other => panic!("expected unbound-or-not-a-type, got {:?}", other),
        }
    }

    #[test]
    fn defect_4_claimed_type_well_formed_term_but_not_type() {
        // Build a closed term whose claimed_type is well-formed but
        // is NOT a type (e.g., a closed lambda — value, not type).
        // Pre-fix: kernel would call def_eq inferred-vs-claimed and
        // accept whenever they happen to match structurally.
        // Post-fix: the claimed_type's own type isn't a Universe, so
        // we reject with ClaimedTypeNotAType.
        let id_term = Term::lam(Term::universe(0), Term::Var(0));
        let cert = Certificate {
            term: id_term.clone(),
            claimed_type: id_term, // a value, not a type
            metadata: Default::default(),
        };
        match cert.verify() {
            Err(CheckError::ClaimedTypeNotAType { .. }) => {}
            other => panic!("expected ClaimedTypeNotAType, got {:?}", other),
        }
    }

    #[test]
    fn defect_3_whnf_terminates_on_omega_omega() {
        // Build the CoC encoding of Ω(Ω) where Ω = λ. App(Var(0), Var(0)).
        // Pre-fix: whnf would loop forever.
        // Post-fix: fuel exhausts and whnf returns a partially-reduced
        // term (the kernel's soundness only requires def_eq agree, not
        // full reduction).  We assert that whnf RETURNS — that's the
        // load-bearing property.
        let omega_body = Term::app(Term::Var(0), Term::Var(0));
        let omega = Term::lam(Term::universe(0), omega_body);
        let omega_omega = Term::app(omega.clone(), omega);
        // If whnf loops, this test hangs and CI kills it.  If it
        // returns at all, the fuel bound is doing its job.
        let _result = whnf(omega_omega);
    }

    #[test]
    fn defect_1_eta_under_beta_reduces_argument() {
        // Verify the η rule fires for a redex whose argument is not
        // syntactically `Var(0)` but β-reduces to it.
        //   λ(x : U(0)). f ((λy. y) x) ≡_{βη} f
        // Pre-fix: argument-side `(λy.y) x` is App(Lam, Var(0)) — not
        // matching `Var(0)` syntactically; eta_match returned false
        // so the rule didn't fire.
        // Post-fix: whnf the argument, β-reducing `(λy.y) x` to `x`
        // (= Var(0) after the descent), and the rule fires.
        let f_outer = Term::Var(0); // outer f
        let inner_id = Term::lam(Term::universe(0), Term::Var(0)); // λy.y at depth 0
        // Inside the outer lambda, indices shift up by 1, so:
        //  - bound x → Var(0)
        //  - outer f → Var(1)
        //  - inner_id (closed) stays the same (no free vars)
        //  - inner_id applied to bound x: App(inner_id_shifted, Var(0))
        //    where inner_id_shifted is shift_up(inner_id, 1, 0) = inner_id (closed).
        let lam_eta_via_beta = Term::lam(
            Term::universe(0),
            Term::app(
                Term::Var(1), // outer f, shifted
                Term::app(inner_id.clone(), Term::Var(0)),
            ),
        );
        assert!(
            def_eq(&lam_eta_via_beta, &f_outer),
            "η rule should fire after β-reducing the argument"
        );
    }

    #[test]
    fn polymorphic_identity_type_checks() {
        // λ(A : Univ(0)). λ(x : A). x
        //  has type Π(A : Univ(0)). Π(_ : A). A
        let ctx = Context::new();
        let body = Term::lam(Term::Var(0), Term::Var(0));
        let id = Term::lam(Term::universe(0), body);
        let inferred = infer(&ctx, &id).unwrap();
        // Type: Pi(Univ(0), Pi(Var(0), Var(1)))
        // Inner-Pi body Var(1) refers to A in the outer Pi's binder.
        let expected_type = Term::pi(Term::universe(0), Term::pi(Term::Var(0), Term::Var(1)));
        assert!(
            def_eq(&inferred, &expected_type),
            "polymorphic id expected type, got {:?}",
            inferred,
        );
    }

    // =========================================================================
    // FV-19 — Universe polymorphism (Level variables, Succ, Max).
    //
    // Each test pins a specific algebraic property of the universe-level
    // calculus or its interaction with the kernel rules.  These are the
    // load-bearing tests for the post-FV-19 universe-polymorphic kernel
    // surface.  Removing any of them silently widens the trust extension.
    // =========================================================================

    #[test]
    fn level_concrete_succ_increments() {
        // T-Univ on a concrete carrier mirrors u32 successor.
        assert_eq!(
            Level::Concrete(0).checked_succ(),
            Some(Level::Concrete(1)),
        );
        assert_eq!(
            Level::Concrete(42).checked_succ(),
            Some(Level::Concrete(43)),
        );
    }

    #[test]
    fn level_concrete_max_overflow_rejected() {
        // DEFECT-2 mirror at the level-algebra layer: u32::MAX has no
        // u32 successor — the symbolic carrier exists, but the
        // CONCRETE arm must not silently wrap.
        assert_eq!(Level::Concrete(u32::MAX).checked_succ(), None);
        // One below the ceiling still typechecks.
        assert_eq!(
            Level::Concrete(u32::MAX - 1).checked_succ(),
            Some(Level::Concrete(u32::MAX)),
        );
    }

    #[test]
    fn level_var_succ_stays_symbolic() {
        // `succ(u)` on a level variable wraps in `Succ(...)` — no
        // overflow possible because the structure is unbounded.
        let u = Level::Var("u".to_string());
        assert_eq!(
            u.checked_succ(),
            Some(Level::Succ(Box::new(Level::Var("u".to_string())))),
        );
    }

    #[test]
    fn level_max_concrete_reduces() {
        // max(C(a), C(b)) → C(a.max(b)) — the concrete-arithmetic case
        // collapses to a single Concrete summand.
        assert_eq!(
            Level::Concrete(3).max_with(Level::Concrete(7)),
            Level::Concrete(7),
        );
        assert_eq!(
            Level::Concrete(0).max_with(Level::Concrete(0)),
            Level::Concrete(0),
        );
    }

    #[test]
    fn level_max_zero_is_identity() {
        // max(0, x) = x for every x — pinned both for concrete and
        // symbolic carriers.
        let u = Level::Var("u".to_string());
        assert_eq!(Level::Concrete(0).max_with(u.clone()), u);
        assert_eq!(u.clone().max_with(Level::Concrete(0)), u);
    }

    #[test]
    fn level_max_idempotent() {
        // max(x, x) = x — idempotency, the load-bearing property
        // for level equality decisions over recurring variables.
        let u = Level::Var("u".to_string());
        assert_eq!(u.clone().max_with(u.clone()), u);
        assert_eq!(
            Level::Concrete(5).max_with(Level::Concrete(5)),
            Level::Concrete(5),
        );
    }

    #[test]
    fn level_max_factors_common_succ() {
        // max(succ(a), succ(b)) = succ(max(a, b)) — the algebraic
        // identity the normaliser uses to canonicalise nested
        // succ-of-max forms.
        let u = Level::Var("u".to_string());
        let v = Level::Var("v".to_string());
        let lhs = Level::Succ(Box::new(u.clone()))
            .max_with(Level::Succ(Box::new(v.clone())));
        let rhs = Level::Succ(Box::new(u.max_with(v)));
        assert!(level_eq(&lhs, &rhs));
    }

    #[test]
    fn level_max_commutative_via_normalisation() {
        // max(u, v) = max(v, u) — the canonical-form sort makes
        // these decidably equal even though the structural enums
        // are not literally `==`.
        let u = Level::Var("u".to_string());
        let v = Level::Var("v".to_string());
        assert!(level_eq(
            &u.clone().max_with(v.clone()),
            &v.max_with(u),
        ));
    }

    #[test]
    fn level_max_associative_via_normalisation() {
        // max(a, max(b, c)) = max(max(a, b), c) — flattening the
        // nested `Max`es into a single canonical form yields the
        // same structural shape regardless of the build order.
        let a = Level::Var("a".to_string());
        let b = Level::Var("b".to_string());
        let c = Level::Var("c".to_string());
        let left = a.clone().max_with(b.clone().max_with(c.clone()));
        let right = a.max_with(b).max_with(c);
        assert!(level_eq(&left, &right));
    }

    #[test]
    fn universe_var_lives_in_succ_var() {
        // T-Univ on a level variable: Universe(u) : Universe(succ(u)).
        // The kernel checks the body without instantiating `u`.
        let ctx = Context::new();
        let term = Term::universe_var("u");
        let inferred = infer(&ctx, &term).unwrap();
        let expected = Term::Universe(Level::Succ(Box::new(Level::Var("u".to_string()))));
        assert!(
            def_eq(&inferred, &expected),
            "Universe(u) expected to live in Universe(succ(u)), got {:?}",
            inferred,
        );
    }

    #[test]
    fn pi_form_takes_max_of_polymorphic_levels() {
        // Π(_ : Type@u). Type@v : Type@max(succ(u), succ(v))
        //                       = Type@succ(max(u, v))     (by the
        //                                                   succ-max
        //                                                   factor rule)
        let ctx = Context::new();
        let pi = Term::pi(Term::universe_var("u"), Term::universe_var("v"));
        let inferred = infer(&ctx, &pi).unwrap();
        // Expected: Universe(succ(max(u, v))) (canonical form)
        let expected_level = Level::Succ(Box::new(
            Level::Var("u".to_string()).max_with(Level::Var("v".to_string())),
        ));
        let expected = Term::Universe(expected_level);
        assert!(
            def_eq(&inferred, &expected),
            "Pi over polymorphic levels expected Type@succ(max(u, v)), got {:?}",
            inferred,
        );
    }

    #[test]
    fn pi_form_concrete_polymorphic_mix() {
        // Π(_ : Type@0). Type@u : Type@max(1, succ(u)) = Type@succ(u)
        //   (since max(1, succ(u)) reduces to succ(u) when we pull
        //   the common succ — both summands are succ-of-something —
        //   actually max(1, succ(u)) = max(succ(0), succ(u))
        //                            = succ(max(0, u))
        //                            = succ(u))
        let ctx = Context::new();
        let pi = Term::pi(Term::universe(0), Term::universe_var("u"));
        let inferred = infer(&ctx, &pi).unwrap();
        let expected_level = Level::Succ(Box::new(Level::Var("u".to_string())));
        let expected = Term::Universe(expected_level);
        assert!(
            def_eq(&inferred, &expected),
            "Pi over Type@0/Type@u expected Type@succ(u), got {:?}",
            inferred,
        );
    }

    #[test]
    fn polymorphic_identity_at_level_var_typechecks() {
        // λ(A : Type@u). λ(x : A). x
        //  has type Π(A : Type@u). Π(_ : A). A
        // The schema is well-typed for every `u` — no instantiation
        // required.
        let ctx = Context::new();
        let body = Term::lam(Term::Var(0), Term::Var(0));
        let id = Term::lam(Term::universe_var("u"), body);
        let inferred = infer(&ctx, &id).unwrap();
        let expected_type = Term::pi(
            Term::universe_var("u"),
            Term::pi(Term::Var(0), Term::Var(1)),
        );
        assert!(
            def_eq(&inferred, &expected_type),
            "polymorphic id at Type@u expected type, got {:?}",
            inferred,
        );
    }

    #[test]
    fn level_eq_decides_structural_equivalence() {
        // The decision procedure agrees on canonical forms: max(u, v)
        // and max(v, u) compare equal; max(0, u) and u compare equal.
        let u = Level::Var("u".to_string());
        let v = Level::Var("v".to_string());
        assert!(level_eq(
            &u.clone().max_with(v.clone()),
            &v.clone().max_with(u.clone()),
        ));
        assert!(level_eq(&Level::Concrete(0).max_with(u.clone()), &u));
        assert!(level_eq(
            &u.clone().max_with(u.clone()),
            &u,
        ));
        // Distinct variables compare unequal.
        assert!(!level_eq(&u, &v));
        // Concrete and variable compare unequal.
        assert!(!level_eq(&u, &Level::Concrete(0)));
    }

    #[test]
    fn level_normalize_is_idempotent() {
        // Normalisation is a fixed-point operation: `normalize(x) =
        // normalize(normalize(x))` for every `x`.  Pinned with a few
        // representative shapes.
        let cases = vec![
            Level::Concrete(0),
            Level::Concrete(42),
            Level::Var("u".to_string()),
            Level::Succ(Box::new(Level::Var("u".to_string()))),
            Level::Max(
                Box::new(Level::Var("u".to_string())),
                Box::new(Level::Var("v".to_string())),
            ),
            Level::Max(
                Box::new(Level::Succ(Box::new(Level::Var("u".to_string())))),
                Box::new(Level::Succ(Box::new(Level::Var("v".to_string())))),
            ),
        ];
        for c in cases {
            let once = c.clone().normalize();
            let twice = once.clone().normalize();
            assert_eq!(once, twice, "normalize is non-idempotent on {:?}", c);
        }
    }

    #[test]
    fn shifted_by_concrete_adds() {
        // `shifted_by(by)` on concrete adds `by` (saturating).
        assert_eq!(
            Level::Concrete(3).shifted_by(2),
            Level::Concrete(5),
        );
        // Saturation at u32::MAX.
        assert_eq!(
            Level::Concrete(u32::MAX).shifted_by(1),
            Level::Concrete(u32::MAX),
        );
    }

    #[test]
    fn shifted_by_symbolic_wraps_in_succ() {
        // `shifted_by(by)` on a variable wraps in `by` `Succ`s.
        let u = Level::Var("u".to_string());
        let shifted = u.shifted_by(2);
        // Expected: Succ(Succ(Var("u"))) — already canonical.
        let expected = Level::Succ(Box::new(Level::Succ(Box::new(Level::Var(
            "u".to_string(),
        )))));
        assert_eq!(shifted, expected);
    }

    #[test]
    fn level_display_is_deterministic() {
        // The Display impl renders structural levels in a stable,
        // identifier-safe form — used by the SMT-bridge placeholder
        // generator.
        assert_eq!(format!("{}", Level::Concrete(0)), "0");
        assert_eq!(format!("{}", Level::Var("u".to_string())), "u");
        assert_eq!(
            format!("{}", Level::Succ(Box::new(Level::Var("u".to_string())))),
            "succ(u)",
        );
        assert_eq!(
            format!(
                "{}",
                Level::Max(
                    Box::new(Level::Var("u".to_string())),
                    Box::new(Level::Var("v".to_string())),
                ),
            ),
            "max(u, v)",
        );
    }

    #[test]
    fn certificate_with_universe_variable_verifies() {
        // The polymorphic identity λ(A : Type@u). λ(x : A). x
        // claimed at type Π(A : Type@u). Π(_ : A). A — the universe
        // variable `u` is FREE/schematic.  Both sides type-check
        // without ever instantiating `u`, and the Certificate
        // verifies cleanly.
        let term = Term::lam(
            Term::universe_var("u"),
            Term::lam(Term::Var(0), Term::Var(0)),
        );
        let claimed_type = Term::pi(
            Term::universe_var("u"),
            Term::pi(Term::Var(0), Term::Var(1)),
        );
        let cert = Certificate {
            term,
            claimed_type,
            metadata: Default::default(),
        };
        cert.verify().expect("polymorphic-identity certificate should verify");
    }

    #[test]
    fn certificate_with_distinct_universe_variables_rejects_mismatch() {
        // λ(A : Type@u). λ(x : A). x  CLAIMED at  Π(A : Type@v). Π(_ : A). A
        // With distinct universe variables, the inferred Pi-domain
        // (Type@u) and the claimed Pi-domain (Type@v) are
        // definitionally inequal — the certificate must reject.
        let term = Term::lam(
            Term::universe_var("u"),
            Term::lam(Term::Var(0), Term::Var(0)),
        );
        let claimed_type = Term::pi(
            Term::universe_var("v"),
            Term::pi(Term::Var(0), Term::Var(1)),
        );
        let cert = Certificate {
            term,
            claimed_type,
            metadata: Default::default(),
        };
        match cert.verify() {
            Err(CheckError::TypeMismatch { .. }) => {}
            other => panic!("expected TypeMismatch on (u, v) variable mismatch, got {:?}", other),
        }
    }

    // =========================================================================
    // FV-20 — Σ-types (dependent pairs).
    //
    // Each test pins one of the four new kernel rules (T-Sigma-Form,
    // T-Pair-Intro, T-Fst-Elim, T-Snd-Elim) plus the β-projection
    // identities `fst(pair(a, _)) ≡ a` and `snd(pair(_, b)) ≡ b`.
    // Together they double the reference-checker's expressive power
    // (Π-types alone can't encode "exists" propositions; with Σ
    // every dependent record + existential becomes verifiable).
    // =========================================================================

    #[test]
    fn t_sigma_form_at_concrete_universe() {
        // Σ(_ : Type@0). Type@0 : Type@1
        let ctx = Context::new();
        let sigma = Term::sigma(Term::universe(0), Term::universe(0));
        let inferred = infer(&ctx, &sigma).unwrap();
        assert!(def_eq(&inferred, &Term::universe(1)));
    }

    #[test]
    fn t_sigma_form_takes_max_universe() {
        // Σ(_ : Type@2). Type@5 : Type@6 (max(3, 6) = 6)
        let ctx = Context::new();
        let sigma = Term::sigma(Term::universe(2), Term::universe(5));
        let inferred = infer(&ctx, &sigma).unwrap();
        assert!(def_eq(&inferred, &Term::universe(6)));
    }

    #[test]
    fn t_sigma_form_polymorphic_takes_succ_of_max() {
        // Σ(_ : Type@u). Type@v : Type@succ(max(u, v))
        let ctx = Context::new();
        let sigma = Term::sigma(Term::universe_var("u"), Term::universe_var("v"));
        let inferred = infer(&ctx, &sigma).unwrap();
        let expected_level = Level::Succ(Box::new(
            Level::Var("u".to_string()).max_with(Level::Var("v".to_string())),
        ));
        assert!(def_eq(&inferred, &Term::Universe(expected_level)));
    }

    #[test]
    fn t_pair_intro_synthesises_non_dependent_sigma() {
        // Build a closed pair: (Type@0, Type@0) : Σ(_ : Type@1). Type@1
        // (each component has type Type@1; non-dependent Σ.)
        let ctx = Context::new();
        let pair = Term::pair(Term::universe(0), Term::universe(0));
        let inferred = infer(&ctx, &pair).unwrap();
        // Inferred type: Σ(_ : Type@1). Type@1 (with snd's Type@1
        // shifted under the binder — irrelevant for closed types).
        let expected = Term::sigma(Term::universe(1), Term::universe(1));
        assert!(
            def_eq(&inferred, &expected),
            "T-Pair-Intro: expected {:?}, got {:?}",
            expected,
            inferred,
        );
    }

    #[test]
    fn t_fst_elim_returns_domain() {
        // Bind p : Σ(_ : Type@0). Type@0 in context.  Fst(Var(0)) : Type@0.
        let sigma_ty = Term::sigma(Term::universe(0), Term::universe(0));
        let ctx = Context::new().extend(sigma_ty);
        let inferred = infer(&ctx, &Term::fst(Term::Var(0))).unwrap();
        assert!(def_eq(&inferred, &Term::universe(0)));
    }

    #[test]
    fn t_snd_elim_with_dependent_codomain() {
        // Build a context where p : Σ(x : Type@0). x.  Snd(p) :
        // Fst(p) — the dependent codomain `x` becomes `Fst(p)` after
        // substitution.  The inferred type is Term::Fst(Var(0)).
        let sigma_ty = Term::sigma(Term::universe(0), Term::Var(0));
        let ctx = Context::new().extend(sigma_ty);
        let inferred = infer(&ctx, &Term::snd(Term::Var(0))).unwrap();
        let expected = Term::fst(Term::Var(0));
        assert!(
            def_eq(&inferred, &expected),
            "T-Snd-Elim with dependent codomain: expected {:?}, got {:?}",
            expected,
            inferred,
        );
    }

    #[test]
    fn beta_fst_pair_collapses() {
        // β-projection: Fst(Pair(Type@0, Type@1)) ≡ Type@0.
        // Pinned via def_eq (which calls whnf on both sides).
        let pair = Term::pair(Term::universe(0), Term::universe(1));
        let projected = Term::fst(pair);
        assert!(def_eq(&projected, &Term::universe(0)));
    }

    #[test]
    fn beta_snd_pair_collapses() {
        // β-projection: Snd(Pair(Type@0, Type@1)) ≡ Type@1.
        let pair = Term::pair(Term::universe(0), Term::universe(1));
        let projected = Term::snd(pair);
        assert!(def_eq(&projected, &Term::universe(1)));
    }

    #[test]
    fn fst_on_non_sigma_is_rejected() {
        // Fst on a Universe-typed term is structurally invalid:
        // the kernel rejects with NotASigma (FV-20 error variant).
        let ctx = Context::new();
        let bad = Term::fst(Term::universe(0)); // Type@0 isn't Σ-typed
        match infer(&ctx, &bad) {
            Err(CheckError::NotASigma(_)) => {}
            other => panic!("expected NotASigma, got {:?}", other),
        }
    }

    #[test]
    fn snd_on_non_sigma_is_rejected() {
        let ctx = Context::new();
        let bad = Term::snd(Term::universe(0));
        match infer(&ctx, &bad) {
            Err(CheckError::NotASigma(_)) => {}
            other => panic!("expected NotASigma, got {:?}", other),
        }
    }

    #[test]
    fn certificate_pair_at_non_dependent_sigma_verifies() {
        // The pair (Type@0, Type@0) at type Σ(_ : Type@1). Type@1
        // verifies cleanly under the trusted-base certificate.
        let cert = Certificate {
            term: Term::pair(Term::universe(0), Term::universe(0)),
            claimed_type: Term::sigma(Term::universe(1), Term::universe(1)),
            metadata: Default::default(),
        };
        cert.verify().expect("non-dependent Σ certificate should verify");
    }

    #[test]
    fn certificate_pair_with_wrong_universe_rejects() {
        // The pair (Type@0, Type@0) claimed at Σ(_ : Type@0). Type@0
        // (Type@0 isn't itself of type Type@0 — it lives in Type@1).
        // Both kernels must reject.
        let cert = Certificate {
            term: Term::pair(Term::universe(0), Term::universe(0)),
            claimed_type: Term::sigma(Term::universe(0), Term::universe(0)),
            metadata: Default::default(),
        };
        match cert.verify() {
            Err(CheckError::TypeMismatch { .. }) => {}
            other => panic!("expected TypeMismatch on universe-mismatched pair, got {:?}", other),
        }
    }

    #[test]
    fn nested_sigma_typechecks() {
        // Σ(_ : Type@0). Σ(_ : Type@0). Type@0 : Type@1
        // (nested Σ — exercises the binder-extension recursion).
        let ctx = Context::new();
        let inner = Term::sigma(Term::universe(0), Term::universe(0));
        let outer = Term::sigma(Term::universe(0), inner);
        let inferred = infer(&ctx, &outer).unwrap();
        assert!(def_eq(&inferred, &Term::universe(1)));
    }

    #[test]
    fn sigma_pi_universe_compose() {
        // Π(_ : Σ(_ : Type@0). Type@0). Type@0 : Type@1
        // — Σ as a domain of Π exercises the cross-rule interaction.
        let ctx = Context::new();
        let sigma = Term::sigma(Term::universe(0), Term::universe(0));
        let pi = Term::pi(sigma, Term::universe(0));
        let inferred = infer(&ctx, &pi).unwrap();
        assert!(def_eq(&inferred, &Term::universe(1)));
    }

    #[test]
    fn polymorphic_sigma_certificate_verifies() {
        // Build the polymorphic dependent-pair existence claim:
        //   λ(A : Type@u). λ(x : A). (x, x) : Π(A : Type@u). Π(_ : A). Σ(_ : A). A
        // Reduces to a value-level claim that's universe-polymorphic.
        let term = Term::lam(
            Term::universe_var("u"),
            Term::lam(Term::Var(0), Term::pair(Term::Var(0), Term::Var(0))),
        );
        let claimed_type = Term::pi(
            Term::universe_var("u"),
            Term::pi(
                Term::Var(0),
                Term::sigma(Term::Var(1), Term::Var(2)),
            ),
        );
        let cert = Certificate {
            term,
            claimed_type,
            metadata: Default::default(),
        };
        cert.verify().expect("polymorphic Σ certificate should verify");
    }
}
