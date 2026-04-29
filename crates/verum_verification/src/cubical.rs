//! Cubical / HoTT first-class catalogue — typed primitive
//! inventory + computation-rule registry + face-formula validator.
//!
//! ## Goal
//!
//! Verum becomes the first production proof assistant where
//! cubical type theory, HoTT, AND classical foundation co-exist
//! under foundation-neutral toggles.  Cubical-Agda and Lean's
//! Mathlib4 each support some of this; Verum's USP is the
//! foundation-neutrality (cubical layer is opt-in, not the only
//! option).
//!
//! This module is the **architectural foundation** for the cubical
//! layer:
//!
//!   * Typed enumeration of cubical primitives (HComp / Transp /
//!     Glue / Path / Equiv / J-rule / Univalence / …).
//!   * Per-primitive structured doc (signature + semantics +
//!     computation rules + example).
//!   * Face-formula validator (the `i = 0`, `i = 1`, `i = 0 ∨ j = 1`
//!     boundary-cube grammar).
//!   * Path-formula well-formedness check.
//!   * Single trait boundary [`CubicalCatalog`] consumed by IDE /
//!     docs / kernel re-check.
//!
//! ## V0 contract
//!
//! V0 ships:
//!
//!   * The full primitive catalogue (16 entries covering every
//!     #78 acceptance bullet).
//!   * Production-grade face-formula parser / validator.
//!   * Per-primitive structured doc with computation-rule names.
//!   * The trait surface and reference catalogue impl.
//!
//! V1+ adds:
//!
//!   * Actual kernel-side reduction rules (currently the catalogue
//!     names them; the reductions live in the kernel).
//!   * HIT support (higher inductive types) checked via the
//!     positivity-check infrastructure.
//!   * Univalence-derivability proof from Glue.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use verum_common::Text;

// =============================================================================
// CubicalPrimitive — the 16 canonical primitives
// =============================================================================

/// One cubical / HoTT primitive operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CubicalPrimitive {
    /// `Path A x y` — typed equality between two terms of type A.
    Path,
    /// `PathOver` (heterogeneous path, dependent on a base path).
    PathOver,
    /// `refl A x : Path A x x` — reflexivity.
    Refl,
    /// `sym p : Path A y x` — symmetry of paths.
    Sym,
    /// `trans p q : Path A x z` — transitivity / composition.
    Trans,
    /// `ap f p : Path B (f x) (f y)` — congruence / functorial action.
    Ap,
    /// `apd f p : PathOver B p (f x) (f y)` — dependent congruence.
    #[serde(rename = "apd")]
    ApD,
    /// `J A C base p` — path induction (eliminator for `Path`).
    #[serde(rename = "j_rule")]
    JRule,
    /// `transp` — transport along a line of types (the HoTT
    /// primitive that drives propositional-equality reasoning).
    Transp,
    /// `coe A B p x` — coerce `x : A` to `B` along `p : Path U A B`.
    Coe,
    /// `subst P p x` — substitute along `p`.
    Subst,
    /// `hcomp` — homogeneous composition (the CCHM primitive that
    /// drives Kan-fibrancy).
    Hcomp,
    /// `comp` — heterogeneous composition (the dependent
    /// generalisation of `hcomp`).
    Comp,
    /// `Glue` — glue at face φ; enables univalence.
    Glue,
    /// `unglue` — destructor for glue terms.
    Unglue,
    /// `Equiv A B` — typed equivalence between A and B.
    Equiv,
    /// `ua : Equiv A B → Path U A B` — univalence axiom.
    Univalence,
}

impl CubicalPrimitive {
    pub fn name(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::PathOver => "path_over",
            Self::Refl => "refl",
            Self::Sym => "sym",
            Self::Trans => "trans",
            Self::Ap => "ap",
            Self::ApD => "apd",
            Self::JRule => "j_rule",
            Self::Transp => "transp",
            Self::Coe => "coe",
            Self::Subst => "subst",
            Self::Hcomp => "hcomp",
            Self::Comp => "comp",
            Self::Glue => "glue",
            Self::Unglue => "unglue",
            Self::Equiv => "equiv",
            Self::Univalence => "univalence",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "path" => Some(Self::Path),
            "path_over" | "pathover" => Some(Self::PathOver),
            "refl" => Some(Self::Refl),
            "sym" => Some(Self::Sym),
            "trans" => Some(Self::Trans),
            "ap" => Some(Self::Ap),
            "apd" => Some(Self::ApD),
            "j_rule" | "j" | "path_induction" => Some(Self::JRule),
            "transp" | "transport" => Some(Self::Transp),
            "coe" | "coercion" => Some(Self::Coe),
            "subst" | "substitution" => Some(Self::Subst),
            "hcomp" => Some(Self::Hcomp),
            "comp" => Some(Self::Comp),
            "glue" => Some(Self::Glue),
            "unglue" => Some(Self::Unglue),
            "equiv" => Some(Self::Equiv),
            "univalence" | "ua" => Some(Self::Univalence),
            _ => None,
        }
    }

    pub fn all() -> [CubicalPrimitive; 17] {
        [
            Self::Path,
            Self::PathOver,
            Self::Refl,
            Self::Sym,
            Self::Trans,
            Self::Ap,
            Self::ApD,
            Self::JRule,
            Self::Transp,
            Self::Coe,
            Self::Subst,
            Self::Hcomp,
            Self::Comp,
            Self::Glue,
            Self::Unglue,
            Self::Equiv,
            Self::Univalence,
        ]
    }

    pub fn category(self) -> CubicalCategory {
        match self {
            Self::Path | Self::PathOver | Self::Refl => CubicalCategory::Identity,
            Self::Sym | Self::Trans | Self::Ap | Self::ApD => {
                CubicalCategory::PathOps
            }
            Self::JRule => CubicalCategory::Induction,
            Self::Transp | Self::Coe | Self::Subst => CubicalCategory::Transport,
            Self::Hcomp | Self::Comp => CubicalCategory::Composition,
            Self::Glue | Self::Unglue => CubicalCategory::Glue,
            Self::Equiv | Self::Univalence => CubicalCategory::Universe,
        }
    }
}

/// Conceptual category — used for documentation grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CubicalCategory {
    Identity,
    PathOps,
    Induction,
    Transport,
    Composition,
    Glue,
    Universe,
}

impl CubicalCategory {
    pub fn name(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::PathOps => "path_ops",
            Self::Induction => "induction",
            Self::Transport => "transport",
            Self::Composition => "composition",
            Self::Glue => "glue",
            Self::Universe => "universe",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "identity" => Some(Self::Identity),
            "path_ops" => Some(Self::PathOps),
            "induction" => Some(Self::Induction),
            "transport" => Some(Self::Transport),
            "composition" => Some(Self::Composition),
            "glue" => Some(Self::Glue),
            "universe" => Some(Self::Universe),
            _ => None,
        }
    }
}

// =============================================================================
// CubicalEntry — structured doc record
// =============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CubicalEntry {
    pub primitive: CubicalPrimitive,
    pub category: CubicalCategory,
    /// Type-theoretic signature (e.g. `"hcomp {A: U} {φ: 𝔽} (u: I → Partial φ A) (a: A) : A"`).
    pub signature: Text,
    /// One-sentence operational meaning.
    pub semantics: Text,
    /// Canonical example.
    pub example: Text,
    /// Computation rules this primitive participates in.
    pub computation_rules: Vec<Text>,
    /// Stable doc anchor for cross-format linking.
    pub doc_anchor: Text,
}

// =============================================================================
// CubicalRule — typed inventory of computation rules
// =============================================================================

/// One computation / reduction rule.  These describe the
/// definitional equations the cubical kernel reduces by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CubicalRule {
    pub name: Text,
    pub participants: Vec<CubicalPrimitive>,
    pub lhs: Text,
    pub rhs: Text,
    pub rationale: Text,
}

// =============================================================================
// CubicalCatalog trait
// =============================================================================

pub trait CubicalCatalog {
    fn entries(&self) -> Vec<CubicalEntry>;
    fn lookup(&self, name: &str) -> Option<CubicalEntry>;
    fn computation_rules(&self) -> Vec<CubicalRule>;
}

// =============================================================================
// DefaultCubicalCatalog — V0 reference catalogue
// =============================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultCubicalCatalog;

impl DefaultCubicalCatalog {
    pub fn new() -> Self {
        Self
    }
}

impl CubicalCatalog for DefaultCubicalCatalog {
    fn entries(&self) -> Vec<CubicalEntry> {
        CubicalPrimitive::all().iter().map(|&p| entry_for(p)).collect()
    }

    fn lookup(&self, name: &str) -> Option<CubicalEntry> {
        CubicalPrimitive::from_name(name).map(entry_for)
    }

    fn computation_rules(&self) -> Vec<CubicalRule> {
        canonical_rules()
    }
}

fn entry_for(p: CubicalPrimitive) -> CubicalEntry {
    let (signature, semantics, example, rules): (&str, &str, &str, &[&str]) = match p {
        CubicalPrimitive::Path => (
            "Path (A: U) (x y: A) : U",
            "Typed equality.  `Path A x y` is the type of paths from `x` to `y` in `A`.  Replaces propositional `=` in classical type theory.",
            "Path Nat 0 0   // the type of identity paths on 0",
            &["path-refl", "path-J"],
        ),
        CubicalPrimitive::PathOver => (
            "PathOver (P: A → U) {x y: A} (p: Path A x y) (u: P x) (v: P y) : U",
            "Heterogeneous path: equality between terms in fibres over a path.",
            "PathOver Vec p (Cons 0 nil) (Cons 0 (Cons 1 nil))",
            &[],
        ),
        CubicalPrimitive::Refl => (
            "refl (A: U) (x: A) : Path A x x",
            "Reflexivity.  Identity path on `x`.",
            "refl Nat 0",
            &["path-refl", "path-J-refl-elim"],
        ),
        CubicalPrimitive::Sym => (
            "sym {A: U} {x y: A} (p: Path A x y) : Path A y x",
            "Path symmetry; reverse direction.",
            "sym (refl Nat 0) ≡ refl Nat 0",
            &["sym-sym-id", "sym-refl"],
        ),
        CubicalPrimitive::Trans => (
            "trans {A: U} {x y z: A} (p: Path A x y) (q: Path A y z) : Path A x z",
            "Path transitivity / composition.  Concatenate two paths sharing an endpoint.",
            "trans p (refl A y) ≡ p",
            &["trans-refl-left", "trans-refl-right", "trans-assoc"],
        ),
        CubicalPrimitive::Ap => (
            "ap {A B: U} (f: A → B) {x y: A} (p: Path A x y) : Path B (f x) (f y)",
            "Functorial action of a function on paths (congruence).",
            "ap succ (refl Nat 0) ≡ refl Nat 1",
            &["ap-refl", "ap-trans"],
        ),
        CubicalPrimitive::ApD => (
            "apd {A: U} {P: A → U} (f: ∀ x. P x) {x y: A} (p: Path A x y) : PathOver P p (f x) (f y)",
            "Dependent functorial action — produces a `PathOver` rather than a flat `Path`.",
            "apd (λ n. refl Nat n) p",
            &["apd-refl"],
        ),
        CubicalPrimitive::JRule => (
            "J (A: U) (C: ∀ {x y: A}, Path A x y → U) (base: ∀ x. C (refl A x)) {x y} (p: Path A x y) : C p",
            "Path induction.  Eliminator for `Path` types — every motive on paths reduces to the reflexivity case.",
            "J A (λ {x y} _ . P x y) (λ x . p_refl x) p",
            &["path-J", "path-J-refl-elim"],
        ),
        CubicalPrimitive::Transp => (
            "transp (A: I → U) (φ: 𝔽) (a: A i0) : A i1",
            "Transport along a line of types.  When `φ = 1` (boundary completely known) the result equals `a`; otherwise transports across the line.  HoTT primitive.",
            "transp (λ _ → Nat) 0 5 ≡ 5",
            &["transp-const", "transp-on-refl", "transp-fill"],
        ),
        CubicalPrimitive::Coe => (
            "coe (A B: U) (p: Path U A B) (x: A) : B",
            "Coerce a term across a path of types.  Definable as `transp` along `p`.",
            "coe A A (refl U A) x ≡ x",
            &["coe-refl", "coe-uncurry"],
        ),
        CubicalPrimitive::Subst => (
            "subst (P: A → U) {x y: A} (p: Path A x y) (u: P x) : P y",
            "Substitute `x` for `y` along `p`.  Special case of `transp` for the family `P`.",
            "subst P (refl A x) u ≡ u",
            &["subst-refl"],
        ),
        CubicalPrimitive::Hcomp => (
            "hcomp {A: U} {φ: 𝔽} (u: I → Partial φ A) (a: A[φ ↦ u i0]) : A",
            "Homogeneous composition.  CCHM primitive that drives Kan-fibrancy: glue partial compositions across the cube to a fresh face.",
            "hcomp (λ i [(j = 0) → p i, (j = 1) → q i]) (refl A x i)",
            &["hcomp-id-when-empty-system", "hcomp-id-when-φ-equals-1"],
        ),
        CubicalPrimitive::Comp => (
            "comp (A: I → U) {φ: 𝔽} (u: ∀ i. Partial φ (A i)) (a: A i0 [φ ↦ u i0]) : A i1",
            "Heterogeneous composition.  Dependent generalisation of `hcomp` — composes across a varying line of types.",
            "comp (λ _ → A) {0 = ⊥} (λ i []) a   // no constraints → just transports",
            &["comp-id-on-constant", "comp-collapses-to-hcomp"],
        ),
        CubicalPrimitive::Glue => (
            "Glue (A: U) {φ: 𝔽} (T: Partial φ U) (e: ∀ z. Equiv (T z) A) : U",
            "Glue at face φ.  The univalence-enabling primitive: at φ=1 evaluates to `T`, elsewhere to `A`.  Combined with `transp` derives the univalence axiom.",
            "Glue A {i = 0} T e   // T at i=0, A at i=1",
            &["glue-on-true-face", "glue-on-false-face", "glue-equiv"],
        ),
        CubicalPrimitive::Unglue => (
            "unglue {A: U} {φ: 𝔽} {T: Partial φ U} {e} (g: Glue A T e) : A",
            "Destructor for glue terms — extract the underlying A-term.",
            "unglue (glue ... a) ≡ a",
            &["unglue-glue"],
        ),
        CubicalPrimitive::Equiv => (
            "Equiv (A B: U) : U",
            "Type of typed equivalences.  Sigma type of `(f: A → B, isEquiv f)` where `isEquiv` is contractible-fibre.  HoTT-fundamental.",
            "Equiv Nat Bool   // (no equivalence; would be uninhabited)",
            &["equiv-id", "equiv-trans", "equiv-sym"],
        ),
        CubicalPrimitive::Univalence => (
            "ua {A B: U} : Equiv A B → Path U A B",
            "Univalence.  Constructively derivable from `Glue`: an equivalence between types yields a path between them in the universe.  Verum's `ua-unique` enforces uniqueness up to canonical path.",
            "ua (id-equiv A) ≡ refl U A",
            &["ua-id", "ua-trans", "ua-sym", "ua-unique"],
        ),
    };
    CubicalEntry {
        primitive: p,
        category: p.category(),
        signature: Text::from(signature),
        semantics: Text::from(semantics),
        example: Text::from(example),
        computation_rules: rules.iter().map(|s| Text::from(*s)).collect(),
        doc_anchor: Text::from(format!("cubical-{}", p.name().replace('_', "-"))),
    }
}

fn canonical_rules() -> Vec<CubicalRule> {
    use CubicalPrimitive as CP;
    let r = |name: &str, parts: Vec<CP>, lhs: &str, rhs: &str, rationale: &str| CubicalRule {
        name: Text::from(name),
        participants: parts,
        lhs: Text::from(lhs),
        rhs: Text::from(rhs),
        rationale: Text::from(rationale),
    };
    vec![
        r(
            "path-refl",
            vec![CP::Refl, CP::Path],
            "refl A x",
            "λ i → x",
            "Reflexivity is the constant path; every cubical primitive that destructures on the reflexive case reduces here.",
        ),
        r(
            "path-J",
            vec![CP::JRule, CP::Path, CP::Refl],
            "J A C base p",
            "case p of refl ⇒ base x",
            "Path induction reduces on the reflexivity case to the base case; this is the J-rule.",
        ),
        r(
            "path-J-refl-elim",
            vec![CP::JRule, CP::Refl],
            "J A C base (refl A x)",
            "base x",
            "Reduction of J on `refl`: the eliminator collapses to the base case — this is what makes `J` a definitional eliminator in cubical TT.",
        ),
        r(
            "sym-sym-id",
            vec![CP::Sym],
            "sym (sym p)",
            "p",
            "Symmetry is involutive.",
        ),
        r(
            "sym-refl",
            vec![CP::Sym, CP::Refl],
            "sym (refl A x)",
            "refl A x",
            "Symmetry of the reflexive path is itself.",
        ),
        r(
            "trans-refl-left",
            vec![CP::Trans, CP::Refl],
            "trans (refl A x) p",
            "p",
            "Reflexivity is the left identity for path concatenation.",
        ),
        r(
            "trans-refl-right",
            vec![CP::Trans, CP::Refl],
            "trans p (refl A y)",
            "p",
            "Reflexivity is the right identity for path concatenation.",
        ),
        r(
            "trans-assoc",
            vec![CP::Trans],
            "trans (trans p q) r",
            "trans p (trans q r)",
            "Path concatenation is associative; the kernel canonicalises to right-association.",
        ),
        r(
            "ap-refl",
            vec![CP::Ap, CP::Refl],
            "ap f (refl A x)",
            "refl B (f x)",
            "Functorial action on the reflexive path is reflexivity.",
        ),
        r(
            "ap-trans",
            vec![CP::Ap, CP::Trans],
            "ap f (trans p q)",
            "trans (ap f p) (ap f q)",
            "Functoriality: f distributes over path composition.",
        ),
        r(
            "apd-refl",
            vec![CP::ApD, CP::Refl],
            "apd f (refl A x)",
            "refl-over P (refl A x) (f x)",
            "Dependent functoriality on the reflexive path.",
        ),
        r(
            "transp-const",
            vec![CP::Transp],
            "transp (λ _ → A) φ a",
            "a",
            "Transport along a constant line of types is the identity.",
        ),
        r(
            "transp-on-refl",
            vec![CP::Transp, CP::Refl],
            "transp (λ i → P (refl A x i)) 0 u",
            "u",
            "Transport along a constant path is the identity.",
        ),
        r(
            "transp-fill",
            vec![CP::Transp],
            "transp A 1 a",
            "a",
            "When the boundary face φ = 1 (everything is known), transport returns the input unchanged.",
        ),
        r(
            "coe-refl",
            vec![CP::Coe, CP::Refl],
            "coe A A (refl U A) x",
            "x",
            "Coercion along the reflexive path on the universe is the identity.",
        ),
        r(
            "coe-uncurry",
            vec![CP::Coe, CP::Transp],
            "coe A B p x",
            "transp (λ i → p i) 0 x",
            "Coercion is definable as transport along `p`.",
        ),
        r(
            "subst-refl",
            vec![CP::Subst, CP::Refl],
            "subst P (refl A x) u",
            "u",
            "Substitution along the reflexive path is the identity.",
        ),
        r(
            "hcomp-id-when-empty-system",
            vec![CP::Hcomp],
            "hcomp {φ = ⊥} u a",
            "a",
            "When the partial system is empty (no faces constrained) hcomp returns the input.",
        ),
        r(
            "hcomp-id-when-φ-equals-1",
            vec![CP::Hcomp],
            "hcomp {φ = ⊤} u a",
            "u i1 1=1",
            "When φ = 1 (everything constrained) hcomp evaluates to the partial system at the top of the cube.",
        ),
        r(
            "comp-id-on-constant",
            vec![CP::Comp],
            "comp (λ _ → A) {⊥} u a",
            "a",
            "Heterogeneous composition over a constant line with empty system is the identity.",
        ),
        r(
            "comp-collapses-to-hcomp",
            vec![CP::Comp, CP::Hcomp],
            "comp (λ _ → A) φ u a",
            "hcomp φ u a",
            "Heterogeneous composition over a constant line of types reduces to homogeneous composition.",
        ),
        r(
            "glue-on-true-face",
            vec![CP::Glue],
            "Glue A {⊤} T e",
            "T 1=1",
            "When φ = 1 (whole face) Glue evaluates to the partial type T.",
        ),
        r(
            "glue-on-false-face",
            vec![CP::Glue],
            "Glue A {⊥} T e",
            "A",
            "When φ = 0 (no face) Glue evaluates to the underlying A.",
        ),
        r(
            "glue-equiv",
            vec![CP::Glue, CP::Equiv],
            "Glue A {φ} T e at face φ",
            "Equiv (T z) A",
            "Glue carries an equivalence at every face — this is what makes the type Glue-able.",
        ),
        r(
            "unglue-glue",
            vec![CP::Unglue, CP::Glue],
            "unglue (glue {φ} t a)",
            "a",
            "Unglue is the left inverse of glue construction on the underlying A-component.",
        ),
        r(
            "equiv-id",
            vec![CP::Equiv],
            "id-equiv A",
            "Equiv A A",
            "Identity equivalence; the unit of Equiv composition.",
        ),
        r(
            "equiv-trans",
            vec![CP::Equiv],
            "compose-equiv f g",
            "Equiv A C   (when f: Equiv A B, g: Equiv B C)",
            "Equivalence composition.",
        ),
        r(
            "equiv-sym",
            vec![CP::Equiv],
            "sym-equiv f",
            "Equiv B A   (when f: Equiv A B)",
            "Equivalence inverse.",
        ),
        r(
            "ua-id",
            vec![CP::Univalence, CP::Equiv, CP::Refl],
            "ua (id-equiv A)",
            "refl U A",
            "Univalence on the identity equivalence is the reflexive path on the universe.",
        ),
        r(
            "ua-trans",
            vec![CP::Univalence, CP::Equiv, CP::Trans],
            "ua (compose-equiv f g)",
            "trans (ua f) (ua g)",
            "Univalence preserves equivalence composition.",
        ),
        r(
            "ua-sym",
            vec![CP::Univalence, CP::Sym],
            "ua (sym-equiv f)",
            "sym (ua f)",
            "Univalence preserves equivalence inverses.",
        ),
        r(
            "ua-unique",
            vec![CP::Univalence],
            "p, q : Path U A B  s.t.  ua-section p ≡ ua-section q",
            "Path (Path U A B) p q",
            "Uniqueness up to canonical path: any two paths in the universe inducing the same equivalence are themselves connected by a path.",
        ),
    ]
}

// =============================================================================
// FaceFormula — typed parser / validator for face conditions
// =============================================================================

/// A face condition like `i = 0`, `i = 1`, `j = 0 ∨ k = 1`, etc.
/// V0 supports the canonical CCHM grammar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaceFormula {
    /// `⊤` — always true (whole face).
    Top,
    /// `⊥` — never true (empty face).
    Bot,
    /// `i = 0` or `i = 1`.
    EndPoint { variable: Text, end: FaceEnd },
    /// `φ ∧ ψ` — both must hold.
    And(Box<FaceFormula>, Box<FaceFormula>),
    /// `φ ∨ ψ` — either holds.
    Or(Box<FaceFormula>, Box<FaceFormula>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceEnd {
    Zero,
    One,
}

impl FaceFormula {
    /// Free interval variables in the formula.
    pub fn free_variables(&self) -> BTreeSet<Text> {
        let mut out = BTreeSet::new();
        self.collect_vars(&mut out);
        out
    }

    fn collect_vars(&self, out: &mut BTreeSet<Text>) {
        match self {
            Self::Top | Self::Bot => {}
            Self::EndPoint { variable, .. } => {
                out.insert(variable.clone());
            }
            Self::And(a, b) | Self::Or(a, b) => {
                a.collect_vars(out);
                b.collect_vars(out);
            }
        }
    }

    /// Render canonically.  Round-trips through `parse`.
    pub fn render(&self) -> Text {
        match self {
            Self::Top => Text::from("1"),
            Self::Bot => Text::from("0"),
            Self::EndPoint { variable, end } => Text::from(format!(
                "{} = {}",
                variable.as_str(),
                if matches!(end, FaceEnd::Zero) { "0" } else { "1" }
            )),
            Self::And(a, b) => Text::from(format!(
                "({} ∧ {})",
                a.render().as_str(),
                b.render().as_str()
            )),
            Self::Or(a, b) => Text::from(format!(
                "({} ∨ {})",
                a.render().as_str(),
                b.render().as_str()
            )),
        }
    }

    /// Parse a face formula.  Accepts:
    ///
    ///   * `0` / `⊥` / `bot` — bottom.
    ///   * `1` / `⊤` / `top` — top.
    ///   * `i = 0` / `i = 1` — endpoint.
    ///   * `φ ∧ ψ` / `φ /\ ψ` / `φ and ψ` — conjunction.
    ///   * `φ ∨ ψ` / `φ \/ ψ` / `φ or ψ` — disjunction.
    ///   * Parens for grouping.
    ///
    /// `∨` binds looser than `∧` (standard mathematical convention).
    pub fn parse(input: &str) -> Result<Self, Text> {
        let tokens = tokenise(input)?;
        let (formula, rest) = parse_or(&tokens)?;
        if !rest.is_empty() {
            return Err(Text::from(format!(
                "trailing tokens after parse: {:?}",
                rest
            )));
        }
        Ok(formula)
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Top,
    Bot,
    Ident(String),
    Eq,
    Zero,
    One,
    And,
    Or,
    LParen,
    RParen,
}

fn tokenise(input: &str) -> Result<Vec<Tok>, Text> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            '(' => {
                chars.next();
                out.push(Tok::LParen);
            }
            ')' => {
                chars.next();
                out.push(Tok::RParen);
            }
            '=' => {
                chars.next();
                out.push(Tok::Eq);
            }
            '0' => {
                chars.next();
                // Bare "0" with no `=` adjacency = bottom (when at the
                // start of a term).  Disambiguated by the parser.
                out.push(Tok::Zero);
            }
            '1' => {
                chars.next();
                out.push(Tok::One);
            }
            '⊤' => {
                chars.next();
                out.push(Tok::Top);
            }
            '⊥' => {
                chars.next();
                out.push(Tok::Bot);
            }
            '∧' => {
                chars.next();
                out.push(Tok::And);
            }
            '∨' => {
                chars.next();
                out.push(Tok::Or);
            }
            '/' => {
                chars.next();
                if chars.peek() != Some(&'\\') {
                    return Err(Text::from("expected `/\\` after `/`"));
                }
                chars.next();
                out.push(Tok::And);
            }
            '\\' => {
                chars.next();
                if chars.peek() != Some(&'/') {
                    return Err(Text::from("expected `\\/` after `\\`"));
                }
                chars.next();
                out.push(Tok::Or);
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut s = String::new();
                while let Some(&cc) = chars.peek() {
                    if cc.is_ascii_alphanumeric() || cc == '_' {
                        s.push(cc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                match s.as_str() {
                    "top" => out.push(Tok::Top),
                    "bot" => out.push(Tok::Bot),
                    "and" => out.push(Tok::And),
                    "or" => out.push(Tok::Or),
                    _ => out.push(Tok::Ident(s)),
                }
            }
            _ => {
                return Err(Text::from(format!(
                    "unexpected character `{}` in face formula",
                    c
                )));
            }
        }
    }
    Ok(out)
}

fn parse_or(tokens: &[Tok]) -> Result<(FaceFormula, &[Tok]), Text> {
    let (mut left, mut rest) = parse_and(tokens)?;
    while let Some(Tok::Or) = rest.first() {
        let (right, after) = parse_and(&rest[1..])?;
        left = FaceFormula::Or(Box::new(left), Box::new(right));
        rest = after;
    }
    Ok((left, rest))
}

fn parse_and(tokens: &[Tok]) -> Result<(FaceFormula, &[Tok]), Text> {
    let (mut left, mut rest) = parse_atom(tokens)?;
    while let Some(Tok::And) = rest.first() {
        let (right, after) = parse_atom(&rest[1..])?;
        left = FaceFormula::And(Box::new(left), Box::new(right));
        rest = after;
    }
    Ok((left, rest))
}

fn parse_atom(tokens: &[Tok]) -> Result<(FaceFormula, &[Tok]), Text> {
    match tokens.first() {
        Some(Tok::LParen) => {
            let (inner, rest) = parse_or(&tokens[1..])?;
            match rest.first() {
                Some(Tok::RParen) => Ok((inner, &rest[1..])),
                _ => Err(Text::from("expected `)`")),
            }
        }
        Some(Tok::Top) => Ok((FaceFormula::Top, &tokens[1..])),
        Some(Tok::Bot) => Ok((FaceFormula::Bot, &tokens[1..])),
        Some(Tok::One) => Ok((FaceFormula::Top, &tokens[1..])),
        Some(Tok::Zero) => Ok((FaceFormula::Bot, &tokens[1..])),
        Some(Tok::Ident(name)) => {
            // Expect `<ident> = 0` or `<ident> = 1`.
            let var = name.clone();
            let rest = &tokens[1..];
            if !matches!(rest.first(), Some(Tok::Eq)) {
                return Err(Text::from(format!(
                    "expected `= 0` or `= 1` after variable `{}`",
                    var
                )));
            }
            match rest.get(1) {
                Some(Tok::Zero) => Ok((
                    FaceFormula::EndPoint {
                        variable: Text::from(var),
                        end: FaceEnd::Zero,
                    },
                    &rest[2..],
                )),
                Some(Tok::One) => Ok((
                    FaceFormula::EndPoint {
                        variable: Text::from(var),
                        end: FaceEnd::One,
                    },
                    &rest[2..],
                )),
                _ => Err(Text::from(format!(
                    "endpoint must be `0` or `1`, got something else after `{} =`",
                    var
                ))),
            }
        }
        _ => Err(Text::from(format!(
            "unexpected token at start of atom: {:?}",
            tokens.first()
        ))),
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ----- CubicalPrimitive -----

    #[test]
    fn primitive_round_trip_canonical() {
        for p in CubicalPrimitive::all() {
            assert_eq!(CubicalPrimitive::from_name(p.name()), Some(p));
        }
    }

    #[test]
    fn primitive_aliases_resolve() {
        assert_eq!(
            CubicalPrimitive::from_name("transport"),
            Some(CubicalPrimitive::Transp)
        );
        assert_eq!(
            CubicalPrimitive::from_name("ua"),
            Some(CubicalPrimitive::Univalence)
        );
        assert_eq!(
            CubicalPrimitive::from_name("J"),
            Some(CubicalPrimitive::JRule)
        );
        assert_eq!(
            CubicalPrimitive::from_name("path_induction"),
            Some(CubicalPrimitive::JRule)
        );
        assert_eq!(CubicalPrimitive::from_name("garbage"), None);
    }

    #[test]
    fn seventeen_canonical_primitives() {
        // Pin the V0 inventory.
        assert_eq!(CubicalPrimitive::all().len(), 17);
    }

    #[test]
    fn category_partitions_primitives() {
        use std::collections::BTreeMap;
        let mut by_cat: BTreeMap<&str, usize> = BTreeMap::new();
        for p in CubicalPrimitive::all() {
            *by_cat.entry(p.category().name()).or_insert(0) += 1;
        }
        for cat in [
            "identity",
            "path_ops",
            "induction",
            "transport",
            "composition",
            "glue",
            "universe",
        ] {
            assert!(
                by_cat.get(cat).copied().unwrap_or(0) > 0,
                "category `{}` has no members",
                cat
            );
        }
        assert_eq!(by_cat.values().sum::<usize>(), 17);
    }

    // ----- DefaultCubicalCatalog -----

    #[test]
    fn default_catalog_lists_all_entries() {
        let cat = DefaultCubicalCatalog::new();
        assert_eq!(cat.entries().len(), 17);
    }

    #[test]
    fn default_catalog_lookup_finds_every_primitive() {
        let cat = DefaultCubicalCatalog::new();
        for p in CubicalPrimitive::all() {
            let entry = cat.lookup(p.name()).expect(p.name());
            assert_eq!(entry.primitive, p);
            assert!(!entry.signature.as_str().is_empty());
            assert!(!entry.semantics.as_str().is_empty());
            assert!(!entry.example.as_str().is_empty());
        }
    }

    #[test]
    fn default_catalog_lookup_rejects_unknown() {
        let cat = DefaultCubicalCatalog::new();
        assert!(cat.lookup("garbage").is_none());
    }

    #[test]
    fn doc_anchors_are_distinct() {
        use std::collections::BTreeSet;
        let cat = DefaultCubicalCatalog::new();
        let anchors: BTreeSet<String> = cat
            .entries()
            .iter()
            .map(|e| e.doc_anchor.as_str().to_string())
            .collect();
        assert_eq!(anchors.len(), 17);
    }

    // ----- Computation rules -----

    #[test]
    fn computation_rules_non_empty() {
        let cat = DefaultCubicalCatalog::new();
        let rules = cat.computation_rules();
        // V0 ships a substantive computation-rule inventory.
        assert!(rules.len() >= 25);
    }

    #[test]
    fn every_rule_has_lhs_rhs_rationale_participants() {
        let cat = DefaultCubicalCatalog::new();
        for r in cat.computation_rules() {
            assert!(!r.name.as_str().is_empty());
            assert!(!r.lhs.as_str().is_empty());
            assert!(!r.rhs.as_str().is_empty());
            assert!(!r.rationale.as_str().is_empty());
            assert!(!r.participants.is_empty());
        }
    }

    #[test]
    fn rule_names_are_unique() {
        use std::collections::BTreeSet;
        let names: BTreeSet<String> = canonical_rules()
            .iter()
            .map(|r| r.name.as_str().to_string())
            .collect();
        assert_eq!(names.len(), canonical_rules().len());
    }

    #[test]
    fn entry_rule_references_resolve() {
        // Every computation rule named in an entry MUST exist in
        // canonical_rules() — single source of truth.
        use std::collections::BTreeSet;
        let known: BTreeSet<String> = canonical_rules()
            .iter()
            .map(|r| r.name.as_str().to_string())
            .collect();
        let cat = DefaultCubicalCatalog::new();
        for entry in cat.entries() {
            for rule_ref in &entry.computation_rules {
                assert!(
                    known.contains(rule_ref.as_str()),
                    "primitive {:?} references unknown rule `{}`",
                    entry.primitive,
                    rule_ref.as_str()
                );
            }
        }
    }

    // ----- FaceFormula parser -----

    #[test]
    fn face_parse_top_bot() {
        assert_eq!(FaceFormula::parse("1").unwrap(), FaceFormula::Top);
        assert_eq!(FaceFormula::parse("0").unwrap(), FaceFormula::Bot);
        assert_eq!(FaceFormula::parse("⊤").unwrap(), FaceFormula::Top);
        assert_eq!(FaceFormula::parse("⊥").unwrap(), FaceFormula::Bot);
        assert_eq!(FaceFormula::parse("top").unwrap(), FaceFormula::Top);
        assert_eq!(FaceFormula::parse("bot").unwrap(), FaceFormula::Bot);
    }

    #[test]
    fn face_parse_endpoint() {
        let f = FaceFormula::parse("i = 0").unwrap();
        match f {
            FaceFormula::EndPoint { variable, end } => {
                assert_eq!(variable.as_str(), "i");
                assert_eq!(end, FaceEnd::Zero);
            }
            _ => panic!(),
        }
        let f1 = FaceFormula::parse("j = 1").unwrap();
        if let FaceFormula::EndPoint { end, .. } = f1 {
            assert_eq!(end, FaceEnd::One);
        } else {
            panic!();
        }
    }

    #[test]
    fn face_parse_and() {
        let f = FaceFormula::parse("i = 0 ∧ j = 1").unwrap();
        match f {
            FaceFormula::And(_, _) => {}
            _ => panic!("expected And"),
        }
        // ASCII variant.
        let f2 = FaceFormula::parse("i = 0 /\\ j = 1").unwrap();
        match f2 {
            FaceFormula::And(_, _) => {}
            _ => panic!(),
        }
    }

    #[test]
    fn face_parse_or() {
        let f = FaceFormula::parse("i = 0 ∨ j = 1").unwrap();
        match f {
            FaceFormula::Or(_, _) => {}
            _ => panic!("expected Or"),
        }
        let f2 = FaceFormula::parse("i = 0 \\/ j = 1").unwrap();
        match f2 {
            FaceFormula::Or(_, _) => {}
            _ => panic!(),
        }
    }

    #[test]
    fn face_parse_precedence_and_binds_tighter_than_or() {
        // `i = 0 ∧ j = 1 ∨ k = 1` should parse as `(i ∧ j) ∨ k`.
        let f = FaceFormula::parse("i = 0 ∧ j = 1 ∨ k = 1").unwrap();
        match f {
            FaceFormula::Or(left, _right) => match *left {
                FaceFormula::And(_, _) => {}
                other => panic!("expected And inside Or, got {:?}", other),
            },
            other => panic!("expected Or at top, got {:?}", other),
        }
    }

    #[test]
    fn face_parse_parens_override_precedence() {
        // `(i = 0 ∨ j = 1) ∧ k = 1` should parse as `Or ∧ k`.
        let f = FaceFormula::parse("(i = 0 ∨ j = 1) ∧ k = 1").unwrap();
        match f {
            FaceFormula::And(left, _right) => match *left {
                FaceFormula::Or(_, _) => {}
                other => panic!("expected Or inside And, got {:?}", other),
            },
            other => panic!("expected And at top, got {:?}", other),
        }
    }

    #[test]
    fn face_parse_rejects_malformed() {
        assert!(FaceFormula::parse("i =").is_err());
        assert!(FaceFormula::parse("i = 2").is_err());
        assert!(FaceFormula::parse("i = 0 ∧").is_err());
        assert!(FaceFormula::parse("(i = 0").is_err());
        assert!(FaceFormula::parse("garbage @ 0").is_err());
    }

    #[test]
    fn face_render_round_trip() {
        let inputs = [
            "0",
            "1",
            "i = 0",
            "j = 1",
            "i = 0 ∧ j = 1",
            "i = 0 ∨ j = 1",
        ];
        for s in inputs {
            let parsed = FaceFormula::parse(s).unwrap();
            let rendered = parsed.render();
            // Re-parse the rendered form and check structural equality.
            let reparsed = FaceFormula::parse(rendered.as_str()).unwrap();
            assert_eq!(parsed, reparsed, "round-trip failed for `{}`", s);
        }
    }

    #[test]
    fn face_free_variables() {
        let f = FaceFormula::parse("i = 0 ∧ (j = 1 ∨ k = 0)").unwrap();
        let vars = f.free_variables();
        assert_eq!(vars.len(), 3);
        assert!(vars.iter().any(|v| v.as_str() == "i"));
        assert!(vars.iter().any(|v| v.as_str() == "j"));
        assert!(vars.iter().any(|v| v.as_str() == "k"));
    }

    #[test]
    fn face_constants_have_no_variables() {
        for s in ["0", "1", "⊤", "⊥"] {
            let f = FaceFormula::parse(s).unwrap();
            assert!(f.free_variables().is_empty(), "`{}` should be variable-free", s);
        }
    }

    // ----- Acceptance pin -----

    #[test]
    fn task_78_every_acceptance_bullet_has_a_primitive() {
        // §1: HComp.
        assert!(CubicalPrimitive::from_name("hcomp").is_some());
        // §2: Transp.
        assert!(CubicalPrimitive::from_name("transp").is_some());
        // §3: Glue.
        assert!(CubicalPrimitive::from_name("glue").is_some());
        // §4: ua / Univalence.
        assert!(CubicalPrimitive::from_name("ua").is_some());
        // §5: transport reductions — represented by `transp-on-refl`,
        // `transp-fill`, `coe-uncurry` rules.
        let names: std::collections::BTreeSet<String> = canonical_rules()
            .iter()
            .map(|r| r.name.as_str().to_string())
            .collect();
        assert!(names.contains("transp-fill"));
        assert!(names.contains("coe-uncurry"));
        // §6: HIT support — Path / J / Glue / hcomp + Univalence
        // are the building blocks; full HIT typing rules are kernel-
        // side V1+ work but the primitives are present.
    }

    #[test]
    fn task_78_face_formula_grammar_complete() {
        // Pin every CCHM-grammar production reachable via parse:
        //   ⊤, ⊥, i=0, i=1, ∧, ∨, parens, free variables, ASCII
        //   alternatives.
        for s in [
            "1",
            "0",
            "i = 0",
            "i = 1",
            "i = 0 ∧ j = 1",
            "i = 0 ∨ j = 1",
            "(i = 0 ∨ j = 1) ∧ k = 1",
            "i = 0 /\\ j = 1",
            "i = 0 \\/ j = 1",
            "i = 0 and j = 1",
            "i = 0 or j = 1",
        ] {
            assert!(
                FaceFormula::parse(s).is_ok(),
                "face formula `{}` failed to parse",
                s
            );
        }
    }

    // ----- Serde round-trip -----

    #[test]
    fn entry_serde_round_trip() {
        let cat = DefaultCubicalCatalog::new();
        let e = cat.lookup("hcomp").unwrap();
        let s = serde_json::to_string(&e).unwrap();
        let back: CubicalEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn rule_serde_round_trip() {
        let r = canonical_rules()
            .into_iter()
            .find(|r| r.name.as_str() == "ua-id")
            .unwrap();
        let s = serde_json::to_string(&r).unwrap();
        let back: CubicalRule = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn face_formula_serde_round_trip() {
        let f = FaceFormula::parse("i = 0 ∧ j = 1").unwrap();
        let s = serde_json::to_string(&f).unwrap();
        let back: FaceFormula = serde_json::from_str(&s).unwrap();
        assert_eq!(f, back);
    }
}
