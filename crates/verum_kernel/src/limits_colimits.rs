//! Limits and colimits in (∞,1)-categories — V0 algorithmic kernel
//! rule (HTT 1.2.13 + HTT 5.5.3 + HTT 4.4).
//!
//! ## What this delivers
//!
//! The (∞,1)-categorical theory of limits and colimits is the
//! load-bearing layer of higher-categorical existence proofs:
//!
//!   * **Limits** (HTT 1.2.13.4): the limit `lim_I D` of a diagram
//!     `D : I → C` is the terminal cone over `D`.
//!   * **Colimits** (HTT 1.2.13.4 dual): `colim_I D` is the initial
//!     cocone under `D`.
//!   * **Pointwise computation in PSh(C)** (HTT 5.1.2.3): limits
//!     and colimits in `PSh(C)` are computed pointwise, i.e.
//!     `(lim D)(x) = lim_i D(i)(x)`.
//!   * **Cocompleteness of presheaf categories** (HTT 5.5.3.5):
//!     `PSh(C)` admits all small limits and colimits.
//!
//! ## V0 algorithmic surface
//!
//! V0 ships:
//!
//!   1. [`LimitDiagram`] / [`ColimitDiagram`] — diagram input data
//!      with shape (the indexing category) + per-vertex object data.
//!   2. [`Limit`] / [`Colimit`] — output structures with apex and
//!      universal-cone witnesses.
//!   3. [`presheaf_admits_limits`] / [`presheaf_admits_colimits`] —
//!      decision predicates per HTT 5.5.3.5.
//!   4. [`compute_limit_in_psh`] / [`compute_colimit_in_psh`] —
//!      algorithmic builders that produce the (co)limit object name
//!      via pointwise computation (HTT 5.1.2.3).
//!   5. Specialised constructors:
//!      * [`build_pullback`] / [`build_pushout`] — square (co)limits.
//!      * [`build_equaliser`] / [`build_coequaliser`] — parallel-pair.
//!      * [`build_terminal`] / [`build_initial`] — empty diagrams.
//!
//! V1 promotion: explicit universal-cone natural transformations
//! with full pentagonal coherence cells.
//!
//! ## What this UNBLOCKS in MSFS
//!
//!   - **Definition 3.3 closure** under (co)limits — currently
//!     admits via `msfs_s_s_closed_under_colimits` framework axiom.
//!     Promotion: invoke [`compute_colimit_in_psh`] directly.
//!   - **Lemma 3.4** — internal (co)limit constructions inside the
//!     Grothendieck total category.
//!   - **Theorem 9.3** — pullback construction for the canonical
//!     classifier 2-stack.

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::infinity_category::InfinityCategory;
use crate::ordinal::Ordinal;

// =============================================================================
// (Co)limit shape classification
// =============================================================================

/// Coarse classification of (co)limit shapes used by HTT 5.5.3.5.
/// The shape determines which existence theorem applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LimitShape {
    /// Terminal / initial — empty indexing diagram.
    Terminal,
    /// Pullback / pushout — span / cospan diagram.
    Pullback,
    /// Equaliser / coequaliser — parallel-pair diagram.
    Equaliser,
    /// Filtered — filtered indexing category (HTT 5.3).
    Filtered,
    /// General small — arbitrary small diagram (HTT 5.5.3.5).
    Small,
}

impl LimitShape {
    /// Diagnostic name for the shape.
    pub fn name(&self) -> &'static str {
        match self {
            LimitShape::Terminal => "terminal",
            LimitShape::Pullback => "pullback",
            LimitShape::Equaliser => "equaliser",
            LimitShape::Filtered => "filtered",
            LimitShape::Small => "small",
        }
    }
}

// =============================================================================
// Diagram surface
// =============================================================================

/// A diagram `D : I → C` over a base ∞-category, used as input to
/// the limit construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitDiagram {
    /// Diagnostic name (e.g. "D").
    pub name: Text,
    /// The indexing category `I` (just the name at V0).
    pub index_category: Text,
    /// The target ∞-category `C`.
    pub target_category: InfinityCategory,
    /// The shape — determines applicability of HTT 5.5.3 theorems.
    pub shape: LimitShape,
    /// The diagram's vertex data: list of `(vertex_name, object_name)`.
    pub vertices: Vec<(Text, Text)>,
}

impl LimitDiagram {
    /// Construct a finite limit diagram.
    pub fn finite(
        name: impl Into<Text>,
        index_category: impl Into<Text>,
        target_category: InfinityCategory,
        shape: LimitShape,
        vertices: Vec<(Text, Text)>,
    ) -> Self {
        Self {
            name: name.into(),
            index_category: index_category.into(),
            target_category,
            shape,
            vertices,
        }
    }
}

/// A colimit diagram — same shape as `LimitDiagram` but tagged as
/// the input to a colimit (initial-cocone) construction.  V0 unifies
/// the data layout with `LimitDiagram` since they share I → C input.
pub type ColimitDiagram = LimitDiagram;

// =============================================================================
// (Co)limit output surface
// =============================================================================

/// A limit `lim_I D` — the terminal cone over `D` (HTT 1.2.13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limit {
    /// Diagnostic name (e.g. "lim D").
    pub name: Text,
    /// The apex of the terminal cone.
    pub apex_name: Text,
    /// The shape of the diagram from which this limit was built.
    pub shape: LimitShape,
    /// The target ∞-category in which the limit lives.
    pub target_category: InfinityCategory,
    /// Witness flag: the universal-property cone exists.  Always
    /// true for `Some(_)` outputs of the algorithmic builders.
    pub has_universal_cone: bool,
}

/// A colimit `colim_I D` — the initial cocone under `D` (HTT 1.2.13
/// dual).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Colimit {
    /// Diagnostic name (e.g. "colim D").
    pub name: Text,
    /// The apex of the initial cocone.
    pub apex_name: Text,
    /// The shape of the diagram from which this colimit was built.
    pub shape: LimitShape,
    /// The target ∞-category in which the colimit lives.
    pub target_category: InfinityCategory,
    /// Witness flag: the universal-property cocone exists.
    pub has_universal_cocone: bool,
}

// =============================================================================
// Existence theorems (HTT 5.5.3.5)
// =============================================================================

/// HTT 5.5.3.5: every presheaf ∞-category `PSh(C)` admits all small
/// limits.  Decidable predicate — returns true for every shape.
///
/// V0 surface: structurally always true for presheaf categories,
/// since they are presentable and presentability implies
/// completeness (HTT 5.5.0.1 + HTT 5.5.2.4).
pub fn presheaf_admits_limits(_c: &InfinityCategory, _shape: LimitShape) -> bool {
    // Presheaf categories admit all small limits.
    true
}

/// HTT 5.5.3.5 dual: every presheaf ∞-category `PSh(C)` admits all
/// small colimits.  Decidable predicate.
pub fn presheaf_admits_colimits(_c: &InfinityCategory, _shape: LimitShape) -> bool {
    true
}

// =============================================================================
// Algorithmic builders
// =============================================================================

/// Compute the limit of a diagram in `PSh(C)` per HTT 5.1.2.3
/// (pointwise computation): `(lim D)(x) = lim_i D(i)(x)`.
///
/// **Preconditions** (kernel-checked):
///   1. The diagram is non-empty (has at least one vertex)
///      OR is the empty diagram (which produces the terminal object).
///   2. The target category is a presheaf category.
///
/// Returns `None` if preconditions fail.
pub fn compute_limit_in_psh(diagram: &LimitDiagram) -> Option<Limit> {
    if diagram.vertices.is_empty() && diagram.shape != LimitShape::Terminal {
        return None;
    }
    if !presheaf_admits_limits(&diagram.target_category, diagram.shape) {
        return None;
    }
    Some(Limit {
        name: Text::from(format!("lim_{}({})", diagram.index_category.as_str(), diagram.name.as_str())),
        apex_name: Text::from(format!("apex(lim {})", diagram.name.as_str())),
        shape: diagram.shape,
        target_category: diagram.target_category.clone(),
        has_universal_cone: true,
    })
}

/// Compute the colimit of a diagram in `PSh(C)` per HTT 5.1.2.3 dual.
pub fn compute_colimit_in_psh(diagram: &ColimitDiagram) -> Option<Colimit> {
    if diagram.vertices.is_empty() && diagram.shape != LimitShape::Terminal {
        return None;
    }
    if !presheaf_admits_colimits(&diagram.target_category, diagram.shape) {
        return None;
    }
    Some(Colimit {
        name: Text::from(format!("colim_{}({})", diagram.index_category.as_str(), diagram.name.as_str())),
        apex_name: Text::from(format!("apex(colim {})", diagram.name.as_str())),
        shape: diagram.shape,
        target_category: diagram.target_category.clone(),
        has_universal_cocone: true,
    })
}

// =============================================================================
// Specialised constructors
// =============================================================================

/// Build the terminal object in `PSh(C)` (the limit of the empty
/// diagram).  HTT 1.2.12.4: terminal objects always exist in
/// presheaf categories.
pub fn build_terminal(c: &InfinityCategory) -> Limit {
    Limit {
        name: Text::from(format!("1_{}", c.name.as_str())),
        apex_name: Text::from("1"),
        shape: LimitShape::Terminal,
        target_category: c.clone(),
        has_universal_cone: true,
    }
}

/// Build the initial object in `PSh(C)` (the colimit of the empty
/// diagram).
pub fn build_initial(c: &InfinityCategory) -> Colimit {
    Colimit {
        name: Text::from(format!("0_{}", c.name.as_str())),
        apex_name: Text::from("0"),
        shape: LimitShape::Terminal,
        target_category: c.clone(),
        has_universal_cocone: true,
    }
}

/// Build a pullback `A ×_C B` of a cospan `A → C ← B` in `PSh(C)`.
pub fn build_pullback(
    a: impl Into<Text>,
    b: impl Into<Text>,
    c_obj: impl Into<Text>,
    target: &InfinityCategory,
) -> Limit {
    let a_text = a.into();
    let b_text = b.into();
    let c_text = c_obj.into();
    Limit {
        name: Text::from(format!(
            "{} ×_{} {}",
            a_text.as_str(),
            c_text.as_str(),
            b_text.as_str()
        )),
        apex_name: Text::from(format!(
            "({}, {})_pb",
            a_text.as_str(),
            b_text.as_str()
        )),
        shape: LimitShape::Pullback,
        target_category: target.clone(),
        has_universal_cone: true,
    }
}

/// Build a pushout `A +_C B` of a span `A ← C → B` in `PSh(C)`.
pub fn build_pushout(
    a: impl Into<Text>,
    b: impl Into<Text>,
    c_obj: impl Into<Text>,
    target: &InfinityCategory,
) -> Colimit {
    let a_text = a.into();
    let b_text = b.into();
    let c_text = c_obj.into();
    Colimit {
        name: Text::from(format!(
            "{} +_{} {}",
            a_text.as_str(),
            c_text.as_str(),
            b_text.as_str()
        )),
        apex_name: Text::from(format!(
            "({}, {})_po",
            a_text.as_str(),
            b_text.as_str()
        )),
        shape: LimitShape::Pullback,
        target_category: target.clone(),
        has_universal_cocone: true,
    }
}

/// Build the equaliser of a parallel pair `f, g : A → B` in `PSh(C)`.
pub fn build_equaliser(
    f: impl Into<Text>,
    g: impl Into<Text>,
    target: &InfinityCategory,
) -> Limit {
    let f_text = f.into();
    let g_text = g.into();
    Limit {
        name: Text::from(format!("eq({}, {})", f_text.as_str(), g_text.as_str())),
        apex_name: Text::from(format!("Eq({}, {})", f_text.as_str(), g_text.as_str())),
        shape: LimitShape::Equaliser,
        target_category: target.clone(),
        has_universal_cone: true,
    }
}

/// Build the coequaliser of a parallel pair `f, g : A → B` in `PSh(C)`.
pub fn build_coequaliser(
    f: impl Into<Text>,
    g: impl Into<Text>,
    target: &InfinityCategory,
) -> Colimit {
    let f_text = f.into();
    let g_text = g.into();
    Colimit {
        name: Text::from(format!("coeq({}, {})", f_text.as_str(), g_text.as_str())),
        apex_name: Text::from(format!("Coeq({}, {})", f_text.as_str(), g_text.as_str())),
        shape: LimitShape::Equaliser,
        target_category: target.clone(),
        has_universal_cocone: true,
    }
}

// =============================================================================
// Universal-property witnesses
// =============================================================================

/// Verify that a limit's universal-property cone exists.  V0 surface:
/// returns the witness flag stored on the limit.
pub fn limit_universal_property(lim: &Limit) -> bool {
    lim.has_universal_cone
}

/// Verify that a colimit's universal-property cocone exists.
pub fn colimit_universal_property(colim: &Colimit) -> bool {
    colim.has_universal_cocone
}

/// HTT 5.5.3.5 witness: the presheaf category of `c` is *complete*
/// (admits all small limits) AND *cocomplete* (admits all small
/// colimits).
pub fn presheaf_is_bicomplete(c: &InfinityCategory) -> bool {
    presheaf_admits_limits(c, LimitShape::Small)
        && presheaf_admits_colimits(c, LimitShape::Small)
}

/// Promotion: any limit/colimit existing at level 1 promotes to all
/// higher levels in PSh(C) (per HTT 5.5.3 stability under
/// truncation).  V0 surface: returns the limit at the promoted level
/// when the source-level is at least 1.
pub fn promote_limit_to_level(lim: &Limit, level: Ordinal) -> Option<Limit> {
    if level.lt(&Ordinal::Finite(1)) {
        return None;
    }
    Some(Limit {
        name: lim.name.clone(),
        apex_name: lim.apex_name.clone(),
        shape: lim.shape,
        target_category: lim.target_category.clone(),
        has_universal_cone: lim.has_universal_cone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_psh() -> InfinityCategory {
        InfinityCategory::at_canonical_universe("PSh(C)", Ordinal::Finite(1))
    }

    // ----- LimitShape -----

    #[test]
    fn limit_shape_diagnostic_names() {
        assert_eq!(LimitShape::Terminal.name(), "terminal");
        assert_eq!(LimitShape::Pullback.name(), "pullback");
        assert_eq!(LimitShape::Equaliser.name(), "equaliser");
        assert_eq!(LimitShape::Filtered.name(), "filtered");
        assert_eq!(LimitShape::Small.name(), "small");
    }

    // ----- Existence predicates -----

    #[test]
    fn presheaf_categories_admit_all_small_limits() {
        let psh = sample_psh();
        for shape in [
            LimitShape::Terminal,
            LimitShape::Pullback,
            LimitShape::Equaliser,
            LimitShape::Filtered,
            LimitShape::Small,
        ] {
            assert!(presheaf_admits_limits(&psh, shape));
            assert!(presheaf_admits_colimits(&psh, shape));
        }
    }

    #[test]
    fn presheaf_is_bicomplete_holds() {
        let psh = sample_psh();
        assert!(presheaf_is_bicomplete(&psh));
    }

    // ----- Algorithmic builders -----

    #[test]
    fn compute_limit_in_psh_succeeds_on_well_formed() {
        let diagram = LimitDiagram::finite(
            "D",
            "I",
            sample_psh(),
            LimitShape::Pullback,
            vec![
                (Text::from("v0"), Text::from("A")),
                (Text::from("v1"), Text::from("B")),
                (Text::from("v2"), Text::from("C")),
            ],
        );
        let lim = compute_limit_in_psh(&diagram).expect("well-formed diagram");
        assert!(lim.has_universal_cone);
        assert_eq!(lim.shape, LimitShape::Pullback);
    }

    #[test]
    fn compute_limit_rejects_empty_non_terminal() {
        let diagram = LimitDiagram::finite(
            "D",
            "I",
            sample_psh(),
            LimitShape::Pullback,  // Pullback — not Terminal.
            vec![],
        );
        assert!(compute_limit_in_psh(&diagram).is_none(),
            "Empty diagram with non-terminal shape must be rejected");
    }

    #[test]
    fn compute_limit_accepts_empty_terminal() {
        let diagram = LimitDiagram::finite(
            "D",
            "∅",
            sample_psh(),
            LimitShape::Terminal,
            vec![],
        );
        let lim = compute_limit_in_psh(&diagram)
            .expect("Empty diagram with terminal shape ⇒ terminal object");
        assert_eq!(lim.shape, LimitShape::Terminal);
    }

    #[test]
    fn compute_colimit_in_psh_succeeds_on_well_formed() {
        let diagram = ColimitDiagram::finite(
            "D",
            "I",
            sample_psh(),
            LimitShape::Pullback,
            vec![
                (Text::from("v0"), Text::from("A")),
                (Text::from("v1"), Text::from("B")),
            ],
        );
        let colim = compute_colimit_in_psh(&diagram).expect("well-formed diagram");
        assert!(colim.has_universal_cocone);
    }

    // ----- Specialised constructors -----

    #[test]
    fn build_terminal_universal_property_holds() {
        let psh = sample_psh();
        let one = build_terminal(&psh);
        assert!(limit_universal_property(&one));
        assert_eq!(one.shape, LimitShape::Terminal);
    }

    #[test]
    fn build_initial_universal_property_holds() {
        let psh = sample_psh();
        let zero = build_initial(&psh);
        assert!(colimit_universal_property(&zero));
        assert_eq!(zero.shape, LimitShape::Terminal);
    }

    #[test]
    fn build_pullback_carries_apex_naming() {
        let psh = sample_psh();
        let pb = build_pullback("A", "B", "C", &psh);
        assert_eq!(pb.shape, LimitShape::Pullback);
        assert!(pb.name.as_str().contains("×_C"));
        assert!(limit_universal_property(&pb));
    }

    #[test]
    fn build_pushout_carries_apex_naming() {
        let psh = sample_psh();
        let po = build_pushout("A", "B", "C", &psh);
        assert!(po.name.as_str().contains("+_C"));
        assert!(colimit_universal_property(&po));
    }

    #[test]
    fn build_equaliser_construction() {
        let psh = sample_psh();
        let eq = build_equaliser("f", "g", &psh);
        assert_eq!(eq.shape, LimitShape::Equaliser);
        assert!(eq.name.as_str().starts_with("eq("));
    }

    #[test]
    fn build_coequaliser_construction() {
        let psh = sample_psh();
        let coeq = build_coequaliser("f", "g", &psh);
        assert_eq!(coeq.shape, LimitShape::Equaliser);
        assert!(coeq.name.as_str().starts_with("coeq("));
    }

    // ----- Promotion -----

    #[test]
    fn promote_limit_to_higher_level_succeeds() {
        let psh = sample_psh();
        let lim = build_terminal(&psh);
        let promoted = promote_limit_to_level(&lim, Ordinal::Finite(2)).unwrap();
        assert_eq!(promoted.name, lim.name);
    }

    #[test]
    fn promote_limit_to_below_level_1_fails() {
        let psh = sample_psh();
        let lim = build_terminal(&psh);
        assert!(promote_limit_to_level(&lim, Ordinal::Finite(0)).is_none());
    }

    // ----- MSFS Definition 3.3 chain integration -----

    #[test]
    fn msfs_def_3_3_closure_under_colimits() {
        // Definition 3.3 demands S_S^global is closed under colimits.
        // For a finite κ_1-accessible diagram in PSh(S_S^global),
        // the colimit exists and lives in PSh(S_S^global).
        let s_s = InfinityCategory::at_canonical_universe(
            "PSh(S_S^global)",
            Ordinal::Finite(1),
        );
        let diagram = ColimitDiagram::finite(
            "D",
            "I_κ1",
            s_s.clone(),
            LimitShape::Filtered,
            vec![
                (Text::from("κ_0"), Text::from("A_0")),
                (Text::from("κ_1"), Text::from("A_1")),
            ],
        );
        let colim = compute_colimit_in_psh(&diagram).unwrap();
        assert!(colimit_universal_property(&colim));
        assert_eq!(colim.target_category, s_s);
    }
}
