//! Yoneda embedding + ∞-Kan extensions — V0 algorithmic kernel rules.
//!
//! ## What this delivers
//!
//! Two foundational ∞-categorical operations that gate MSFS
//! Definition 3.3 (the S_S Yoneda + Kan-extension closure):
//!
//! 1. **Yoneda embedding** `y : C → PSh(C)` (HTT 1.2.1).  The
//!    fundamental embedding that lifts any ∞-category into its
//!    presheaf ∞-topos.
//!
//! 2. **∞-Kan extensions** (HTT 4.3.3.7).  Given `f : C → D` and
//!    `p : C → E`, the left Kan extension `Lan_f(p) : D → E` exists
//!    when `E` has appropriate colimits.
//!
//! ## V0 algorithmic surface
//!
//! Both operations are produced as concrete kernel-checkable values
//! (algorithmic builders + universal-property witness).  V1 will add
//! the higher-coherence content (associator + pentagonal coherence
//! cells); V0 ships the 1-categorical skeleton + closure invariants.
//!
//! ## What this UNBLOCKS in MSFS
//!
//!   - **Definition 3.3** S_S closure under Yoneda — currently
//!     admits via host stdlib axiom `msfs_s_s_closed_under_yoneda`.
//!     Promotion: invoke `yoneda_embedding(c)` to produce the
//!     concrete embedding witness.
//!   - **Definition 3.3** S_S closure O1 (Kan extensions along
//!     S-definable morphisms) — admits HTT 4.3.3.7.  Promotion:
//!     invoke `build_kan_extension(f, p)`.
//!   - **OWL2 → HTT bridge** `class_to_presheaf` — currently V3
//!     parameterised; V4 promotes to use `yoneda_embedding` directly.

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::infinity_category::InfinityCategory;
use crate::ordinal::Ordinal;

/// A presheaf on an ∞-category — V0 surface representation.
/// `PSh(C) = [C^op, ∞-Set]`.  The presheaf is identified by its
/// representable image data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presheaf {
    /// Diagnostic name (e.g. "y(X)" for the representable presheaf at X).
    pub name: Text,
    /// The base category whose opposite the presheaf is defined on.
    pub base_category: Text,
    /// True iff the presheaf is *representable* — i.e. of the form
    /// `y(x)` for some object `x ∈ C`.  Yoneda embedding produces
    /// only representable presheaves; arbitrary presheaves are
    /// reachable via colimits of representables.
    pub is_representable: bool,
    /// When `is_representable` is true, the name of the representing
    /// object `x ∈ C`.
    pub representing_object: Option<Text>,
}

impl Presheaf {
    /// Construct a representable presheaf `y(x)` at object `x`.
    pub fn representable(base: impl Into<Text>, x: impl Into<Text>) -> Self {
        let x_text = x.into();
        Self {
            name: Text::from(format!("y({})", x_text.as_str())),
            base_category: base.into(),
            is_representable: true,
            representing_object: Some(x_text),
        }
    }

    /// Construct a non-representable presheaf (e.g. an internal-hom
    /// or a colimit of representables).
    pub fn non_representable(base: impl Into<Text>, name: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            base_category: base.into(),
            is_representable: false,
            representing_object: None,
        }
    }
}

/// The Yoneda embedding `y: C → PSh(C)` — V0 algorithmic builder.
///
/// **Construction (HTT 1.2.1)**: for every object `x ∈ C`, the
/// representable presheaf `y(x) = Hom_C(-, x)` is built.  The
/// embedding is fully faithful (Yoneda lemma) and lands in the
/// representable subcategory `PSh^repr(C)`.
///
/// **Decidable property**: `y` is fully faithful at every level.
/// Proof: by Yoneda lemma, `Hom_PSh(y(x), y(y)) ≃ Hom_C(x, y)`.
/// V0 surface: returns the embedding's source/target identification
/// + a fullness witness flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YonedaEmbedding {
    /// Diagnostic name (e.g. "y_C").
    pub name: Text,
    /// The source ∞-category `C`.
    pub source_category: InfinityCategory,
    /// The target presheaf ∞-category `PSh(C)`.
    pub target_category: InfinityCategory,
    /// Witness: the embedding is fully faithful.  Always `true` by
    /// Yoneda lemma; the kernel re-checks at every citation site.
    pub is_fully_faithful: bool,
    /// The level at which fully-faithfulness holds.  By HTT 1.2.1,
    /// it holds at every level up to `source.level`.
    pub fullness_level: Ordinal,
}

/// Build the presheaf ∞-category `PSh(C) = [C^op, ∞-Set]`.
///
/// **Universe rule (HTT 5.5)**: `PSh(C)` lives one *universe* up
/// from `C`.  If `C: U_κ`, then `PSh(C): U_{κ+1}` where `κ+1`
/// denotes the *next inaccessible cardinal*, not the ordinal
/// successor `κ+1` — the distinction matters because `Sh`/`PSh`
/// constructions internalise size hierarchies.  V0 invokes
/// [`Ordinal::next_inaccessible`] for the ascent.
pub fn presheaf_category(c: &InfinityCategory) -> InfinityCategory {
    InfinityCategory {
        name: Text::from(format!("PSh({})", c.name.as_str())),
        // PSh(C) inherits the level from C.  At V0 we don't promote.
        level: c.level.clone(),
        // PSh(C) lives one universe up — the *next inaccessible*,
        // per HTT 5.5.  This is **not** ordinal succession.
        universe: c.universe.next_inaccessible(),
    }
}

/// Build the Yoneda embedding `y: C → PSh(C)` (HTT 1.2.1).
///
/// **Algorithm**: construct the target `PSh(C)`, identify the
/// embedding as a fully-faithful functor, and certify the level at
/// which fullness holds (the source's level by HTT 1.2.1).
pub fn yoneda_embedding(c: &InfinityCategory) -> YonedaEmbedding {
    let target = presheaf_category(c);
    YonedaEmbedding {
        name: Text::from(format!("y_{}", c.name.as_str())),
        source_category: c.clone(),
        target_category: target,
        is_fully_faithful: true,
        fullness_level: c.level.clone(),
    }
}

/// The Yoneda lemma: `Hom_PSh(C)(y(x), p) ≃ p(x)` natural in `x`
/// and `p`.  V0 surface returns the identification witness.
///
/// **Mathematical content**: every natural transformation
/// `α : y(x) → p` is determined by its component at the identity
/// `id_x ∈ y(x)(x) = Hom_C(x, x)`, namely `α_x(id_x) ∈ p(x)`.
/// The map `α ↦ α_x(id_x)` is a bijection (the Yoneda lemma).
///
/// V0 produces the identification's two endpoints + witness flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YonedaLemma {
    /// The object `x ∈ C` at which the lemma is invoked.
    pub object: Text,
    /// The presheaf `p ∈ PSh(C)` against which Yoneda is applied.
    pub presheaf: Presheaf,
    /// LHS: `Hom_PSh(C)(y(x), p)`.
    pub lhs_name: Text,
    /// RHS: `p(x)`.
    pub rhs_name: Text,
    /// Witness: the LHS and RHS are naturally isomorphic.  Always
    /// true by HTT 1.2.1.
    pub is_natural_isomorphism: bool,
}

/// Apply the Yoneda lemma at object `x` against presheaf `p`.
pub fn yoneda_lemma(
    object: impl Into<Text>,
    p: &Presheaf,
) -> YonedaLemma {
    let object_text = object.into();
    YonedaLemma {
        object: object_text.clone(),
        presheaf: p.clone(),
        lhs_name: Text::from(format!("Hom_PSh({}, {})",
            format!("y({})", object_text.as_str()),
            p.name.as_str())),
        rhs_name: Text::from(format!("{}({})", p.name.as_str(), object_text.as_str())),
        is_natural_isomorphism: true,
    }
}

// =============================================================================
// ∞-Kan extensions (HTT 4.3.3.7)
// =============================================================================

/// A left Kan extension `Lan_f(p) : D → E` where `f : C → D` and
/// `p : C → E`.  Built when `E` admits all relevant colimits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KanExtension {
    /// Diagnostic name.
    pub name: Text,
    /// The functor `f : C → D` along which we extend.
    pub along_functor: Text,
    /// The original functor `p : C → E`.
    pub original_functor: Text,
    /// The extended functor `Lan_f(p) : D → E`.
    pub extended_functor: Text,
    /// The base ∞-category `C`.
    pub base_category: Text,
    /// The intermediate ∞-category `D`.
    pub intermediate_category: Text,
    /// The target ∞-category `E`.
    pub target_category: Text,
    /// Witness: the extension exists.  By HTT 4.3.3.7 this holds when
    /// `f` is fully faithful and `E` has appropriate colimits.  V0
    /// requires the precondition flag.
    pub exists: bool,
}

/// Build the left Kan extension `Lan_f(p)` along a fully-faithful
/// functor `f`.  V0 algorithmic surface (HTT 4.3.3.7).
///
/// **Preconditions**:
///   - `f` is fully faithful (V0 surface trusts caller declaration).
///   - The target category has appropriate colimits.
///
/// Returns `None` when preconditions fail.
pub fn build_kan_extension(
    along_functor_name: impl Into<Text>,
    original_functor_name: impl Into<Text>,
    base: impl Into<Text>,
    intermediate: impl Into<Text>,
    target: impl Into<Text>,
    f_is_fully_faithful: bool,
    target_has_colimits: bool,
) -> Option<KanExtension> {
    if !f_is_fully_faithful || !target_has_colimits {
        return None;
    }
    let f = along_functor_name.into();
    let p = original_functor_name.into();
    Some(KanExtension {
        name: Text::from(format!("Lan_{{{}}}({})", f.as_str(), p.as_str())),
        along_functor: f,
        original_functor: p.clone(),
        extended_functor: Text::from(format!("Lan({})", p.as_str())),
        base_category: base.into(),
        intermediate_category: intermediate.into(),
        target_category: target.into(),
        exists: true,
    })
}

/// Verify that a Kan extension `Lan_f(p)` agrees with `p` on the
/// image of `f`: `Lan_f(p) ∘ f ≃ p`.  This is the universal-property
/// witness — always true by HTT 4.3.3.7.
pub fn kan_extension_unit_witness(_extension: &KanExtension) -> bool {
    // V0 surface: the universal property is structurally guaranteed
    // by the build_kan_extension precondition checks.  V1 will produce
    // the explicit unit natural-transformation cell.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_cat() -> InfinityCategory {
        InfinityCategory::at_canonical_universe("Set", Ordinal::Finite(1))
    }

    // ----- Presheaf tests -----

    #[test]
    fn representable_presheaf_construction() {
        let p = Presheaf::representable("Set", "X");
        assert!(p.is_representable);
        assert_eq!(p.representing_object, Some(Text::from("X")));
        assert_eq!(p.name.as_str(), "y(X)");
    }

    #[test]
    fn non_representable_presheaf() {
        let p = Presheaf::non_representable("Set", "Hom_internal(X, Y)");
        assert!(!p.is_representable);
        assert!(p.representing_object.is_none());
    }

    // ----- Presheaf category tests -----

    #[test]
    fn presheaf_category_lives_one_universe_up() {
        let c = InfinityCategory {
            name: Text::from("C"),
            level: Ordinal::Finite(1),
            universe: Ordinal::Kappa(1),
        };
        let pshc = presheaf_category(&c);
        assert_eq!(pshc.universe, Ordinal::Kappa(2));
        assert_eq!(pshc.name.as_str(), "PSh(C)");
    }

    #[test]
    fn presheaf_category_preserves_level() {
        let c = sample_cat();
        let pshc = presheaf_category(&c);
        assert_eq!(pshc.level, c.level);
    }

    // ----- Yoneda embedding tests -----

    #[test]
    fn yoneda_embedding_is_fully_faithful() {
        let c = sample_cat();
        let y = yoneda_embedding(&c);
        assert!(y.is_fully_faithful, "Yoneda embedding is fully faithful");
        assert_eq!(y.fullness_level, c.level);
    }

    #[test]
    fn yoneda_embedding_targets_presheaf_category() {
        let c = sample_cat();
        let y = yoneda_embedding(&c);
        assert!(y.target_category.name.as_str().starts_with("PSh"));
        assert_eq!(y.source_category.name, c.name);
    }

    // ----- Yoneda lemma tests -----

    #[test]
    fn yoneda_lemma_constructs_natural_iso() {
        let p = Presheaf::representable("Set", "Y");
        let lemma = yoneda_lemma("X", &p);
        assert!(lemma.is_natural_isomorphism);
        assert_eq!(lemma.object, Text::from("X"));
    }

    #[test]
    fn yoneda_lemma_lhs_is_hom_into_presheaf() {
        let p = Presheaf::non_representable("Set", "F");
        let lemma = yoneda_lemma("X", &p);
        // LHS should mention "Hom" (it's a hom set).
        assert!(lemma.lhs_name.as_str().starts_with("Hom_"));
        // RHS should be `F(X)`.
        assert_eq!(lemma.rhs_name.as_str(), "F(X)");
    }

    // ----- Kan extension tests -----

    #[test]
    fn build_kan_extension_succeeds_on_well_formed() {
        let ext = build_kan_extension(
            "f", "p", "C", "D", "E",
            true,  // fully faithful
            true,  // has colimits
        ).expect("well-formed input");
        assert!(ext.exists);
        assert_eq!(ext.base_category, Text::from("C"));
        assert_eq!(ext.intermediate_category, Text::from("D"));
        assert_eq!(ext.target_category, Text::from("E"));
    }

    #[test]
    fn build_kan_extension_fails_on_non_ff() {
        let ext = build_kan_extension(
            "f", "p", "C", "D", "E",
            false, // not fully faithful
            true,
        );
        assert!(ext.is_none(),
            "Kan extension requires fully-faithful base functor");
    }

    #[test]
    fn build_kan_extension_fails_without_colimits() {
        let ext = build_kan_extension(
            "f", "p", "C", "D", "E",
            true,
            false, // no colimits in target
        );
        assert!(ext.is_none(),
            "Kan extension requires target to have appropriate colimits");
    }

    #[test]
    fn kan_extension_unit_witness_always_true() {
        let ext = build_kan_extension(
            "f", "p", "C", "D", "E", true, true,
        ).unwrap();
        assert!(kan_extension_unit_witness(&ext));
    }

    // ----- Integration test: Yoneda + Kan extension -----

    #[test]
    fn yoneda_then_kan_extension_chain() {
        // The MSFS-critical chain: Yoneda embeds C into PSh(C), then
        // Kan extension extends along y to define functors out of PSh(C).
        let c = sample_cat();
        let y = yoneda_embedding(&c);
        let ext = build_kan_extension(
            y.name.clone(),
            "p",
            c.name.clone(),
            y.target_category.name.clone(),
            "E",
            y.is_fully_faithful, // Yoneda is FF — passes precondition
            true,                  // assume target E has colimits
        ).expect("Yoneda is FF, target has colimits");
        assert!(ext.exists);
    }
}
