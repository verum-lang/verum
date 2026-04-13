//! Kripke Semantics — modal worlds + accessibility.
//!
//! Kripke frames give a formal model for modal logic: a set of
//! *worlds* and an *accessibility* relation between them. A
//! formula `□φ` ("necessarily φ") holds at a world iff `φ` holds
//! at every accessible world; `◇φ` ("possibly φ") iff `φ` holds
//! at some accessible world.
//!
//! ## Frame classes
//!
//! Different modal logics correspond to constraints on the
//! accessibility relation:
//!
//! ```text
//!     K    — no constraints
//!     T    — reflexive       (every world sees itself)
//!     B    — symmetric
//!     4    — transitive      (corresponds to S4)
//!     5    — Euclidean
//!     S4   — reflexive + transitive
//!     S5   — equivalence relation (refl + sym + trans)
//! ```
//!
//! ## API
//!
//! * [`World`] — a world identifier
//! * [`KripkeFrame`] — set of worlds + accessibility edges
//! * [`Valuation`] — assigns truth values to atomic propositions
//!   per world
//! * [`evaluate`] — recursive modal-logic evaluator
//! * [`FrameClass::is_satisfied_by`] — check refl/trans/sym/etc.

use std::collections::{BTreeMap, BTreeSet};

use verum_common::Text;

/// A world in a Kripke frame.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct World {
    pub id: Text,
}

impl World {
    pub fn new(id: impl Into<Text>) -> Self {
        Self { id: id.into() }
    }
}

impl std::fmt::Display for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "w{}", self.id.as_str())
    }
}

/// A Kripke frame: worlds + accessibility relation.
#[derive(Debug, Clone, Default)]
pub struct KripkeFrame {
    worlds: BTreeSet<World>,
    /// Accessibility: w → set of worlds accessible from w.
    edges: BTreeMap<World, BTreeSet<World>>,
}

impl KripkeFrame {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_world(&mut self, w: World) {
        self.worlds.insert(w.clone());
        self.edges.entry(w).or_default();
    }

    pub fn add_edge(&mut self, from: World, to: World) {
        self.add_world(from.clone());
        self.add_world(to.clone());
        self.edges.entry(from).or_default().insert(to);
    }

    pub fn worlds(&self) -> impl Iterator<Item = &World> {
        self.worlds.iter()
    }

    pub fn accessible(&self, from: &World) -> BTreeSet<World> {
        self.edges.get(from).cloned().unwrap_or_default()
    }

    pub fn world_count(&self) -> usize {
        self.worlds.len()
    }

    /// Total number of accessibility edges.
    pub fn edge_count(&self) -> usize {
        self.edges.values().map(|s| s.len()).sum()
    }
}

/// Truth-value assignment per world for atomic propositions.
#[derive(Debug, Clone, Default)]
pub struct Valuation {
    /// (world, atom) → bool.
    truth: BTreeMap<(World, Text), bool>,
}

impl Valuation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, world: World, atom: impl Into<Text>, value: bool) {
        self.truth.insert((world, atom.into()), value);
    }

    pub fn get(&self, world: &World, atom: &Text) -> bool {
        self.truth.get(&(world.clone(), atom.clone())).copied().unwrap_or(false)
    }
}

/// A modal-logic formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModalForm {
    /// Atomic proposition.
    Atom(Text),
    /// Logical truth.
    True,
    /// Logical falsity.
    False,
    /// Negation.
    Not(Box<ModalForm>),
    /// Conjunction.
    And(Box<ModalForm>, Box<ModalForm>),
    /// Disjunction.
    Or(Box<ModalForm>, Box<ModalForm>),
    /// Implication.
    Implies(Box<ModalForm>, Box<ModalForm>),
    /// `□φ` — necessity.
    Box(Box<ModalForm>),
    /// `◇φ` — possibility.
    Diamond(Box<ModalForm>),
}

impl ModalForm {
    pub fn atom(s: impl Into<Text>) -> Self {
        Self::Atom(s.into())
    }

    pub fn not(f: ModalForm) -> Self {
        Self::Not(Box::new(f))
    }

    pub fn and(a: ModalForm, b: ModalForm) -> Self {
        Self::And(Box::new(a), Box::new(b))
    }

    pub fn or(a: ModalForm, b: ModalForm) -> Self {
        Self::Or(Box::new(a), Box::new(b))
    }

    pub fn implies(a: ModalForm, b: ModalForm) -> Self {
        Self::Implies(Box::new(a), Box::new(b))
    }

    pub fn nec(f: ModalForm) -> Self {
        Self::Box(Box::new(f))
    }

    pub fn pos(f: ModalForm) -> Self {
        Self::Diamond(Box::new(f))
    }
}

/// Evaluate a modal formula at a given world under a frame and
/// valuation. Returns the truth value.
pub fn evaluate(
    formula: &ModalForm,
    frame: &KripkeFrame,
    valuation: &Valuation,
    world: &World,
) -> bool {
    match formula {
        ModalForm::True => true,
        ModalForm::False => false,
        ModalForm::Atom(name) => valuation.get(world, name),
        ModalForm::Not(inner) => !evaluate(inner, frame, valuation, world),
        ModalForm::And(a, b) => {
            evaluate(a, frame, valuation, world) && evaluate(b, frame, valuation, world)
        }
        ModalForm::Or(a, b) => {
            evaluate(a, frame, valuation, world) || evaluate(b, frame, valuation, world)
        }
        ModalForm::Implies(a, b) => {
            !evaluate(a, frame, valuation, world) || evaluate(b, frame, valuation, world)
        }
        ModalForm::Box(inner) => {
            // True iff inner holds at every accessible world.
            for v in frame.accessible(world) {
                if !evaluate(inner, frame, valuation, &v) {
                    return false;
                }
            }
            true
        }
        ModalForm::Diamond(inner) => {
            // True iff inner holds at some accessible world.
            for v in frame.accessible(world) {
                if evaluate(inner, frame, valuation, &v) {
                    return true;
                }
            }
            false
        }
    }
}

/// A class of frames characterised by structural conditions on
/// accessibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameClass {
    /// No constraints (modal logic K).
    K,
    /// Reflexive: every world accesses itself (T).
    Reflexive,
    /// Symmetric: w → v ⇒ v → w (B).
    Symmetric,
    /// Transitive: w → v, v → u ⇒ w → u (4).
    Transitive,
    /// Reflexive + transitive (S4).
    S4,
    /// Equivalence relation: refl + sym + trans (S5).
    S5,
}

impl FrameClass {
    /// Does this frame satisfy the conditions of the given class?
    pub fn is_satisfied_by(&self, frame: &KripkeFrame) -> bool {
        let refl = || frame.worlds().all(|w| frame.accessible(w).contains(w));
        let sym = || {
            for w in frame.worlds() {
                for v in frame.accessible(w) {
                    if !frame.accessible(&v).contains(w) {
                        return false;
                    }
                }
            }
            true
        };
        let trans = || {
            for w in frame.worlds() {
                for v in frame.accessible(w) {
                    for u in frame.accessible(&v) {
                        if !frame.accessible(w).contains(&u) {
                            return false;
                        }
                    }
                }
            }
            true
        };
        match self {
            FrameClass::K => true,
            FrameClass::Reflexive => refl(),
            FrameClass::Symmetric => sym(),
            FrameClass::Transitive => trans(),
            FrameClass::S4 => refl() && trans(),
            FrameClass::S5 => refl() && sym() && trans(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> World {
        World::new(s)
    }

    fn linear_frame() -> KripkeFrame {
        // w0 → w1 → w2  (transitive closure not included)
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        f.add_edge(w("1"), w("2"));
        f
    }

    fn reflexive_frame() -> KripkeFrame {
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("0"));
        f.add_edge(w("1"), w("1"));
        f.add_edge(w("0"), w("1"));
        f.add_edge(w("1"), w("0"));
        f
    }

    #[test]
    fn world_display_uses_w_prefix() {
        assert_eq!(format!("{}", w("0")), "w0");
    }

    #[test]
    fn empty_frame_has_no_worlds() {
        let f = KripkeFrame::new();
        assert_eq!(f.world_count(), 0);
        assert_eq!(f.edge_count(), 0);
    }

    #[test]
    fn add_edge_creates_both_endpoints() {
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        assert_eq!(f.world_count(), 2);
        assert_eq!(f.edge_count(), 1);
        assert!(f.accessible(&w("0")).contains(&w("1")));
    }

    #[test]
    fn truth_holds_universally() {
        let f = linear_frame();
        let v = Valuation::new();
        assert!(evaluate(&ModalForm::True, &f, &v, &w("0")));
    }

    #[test]
    fn false_fails_universally() {
        let f = linear_frame();
        let v = Valuation::new();
        assert!(!evaluate(&ModalForm::False, &f, &v, &w("0")));
    }

    #[test]
    fn atom_uses_valuation() {
        let f = linear_frame();
        let mut v = Valuation::new();
        v.set(w("0"), "p", true);
        v.set(w("1"), "p", false);
        assert!(evaluate(&ModalForm::atom("p"), &f, &v, &w("0")));
        assert!(!evaluate(&ModalForm::atom("p"), &f, &v, &w("1")));
    }

    #[test]
    fn box_quantifies_over_accessible() {
        // w0 → w1; if p holds at w1, then □p holds at w0.
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        let mut v = Valuation::new();
        v.set(w("1"), "p", true);
        assert!(evaluate(&ModalForm::nec(ModalForm::atom("p")), &f, &v, &w("0")));
    }

    #[test]
    fn box_fails_when_some_accessible_lacks_property() {
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        f.add_edge(w("0"), w("2"));
        let mut v = Valuation::new();
        v.set(w("1"), "p", true);
        v.set(w("2"), "p", false);
        assert!(!evaluate(&ModalForm::nec(ModalForm::atom("p")), &f, &v, &w("0")));
    }

    #[test]
    fn box_holds_vacuously_when_no_accessible() {
        let mut f = KripkeFrame::new();
        f.add_world(w("0"));
        let v = Valuation::new();
        // No edges from w0 → □p is vacuously true.
        assert!(evaluate(&ModalForm::nec(ModalForm::atom("p")), &f, &v, &w("0")));
    }

    #[test]
    fn diamond_finds_witness() {
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        f.add_edge(w("0"), w("2"));
        let mut v = Valuation::new();
        v.set(w("1"), "p", false);
        v.set(w("2"), "p", true);
        assert!(evaluate(&ModalForm::pos(ModalForm::atom("p")), &f, &v, &w("0")));
    }

    #[test]
    fn diamond_fails_when_no_witness() {
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        let mut v = Valuation::new();
        v.set(w("1"), "p", false);
        assert!(!evaluate(&ModalForm::pos(ModalForm::atom("p")), &f, &v, &w("0")));
    }

    #[test]
    fn implies_evaluates_correctly() {
        let f = linear_frame();
        let mut v = Valuation::new();
        v.set(w("0"), "p", false);
        v.set(w("0"), "q", false);
        // false → false is true
        assert!(evaluate(
            &ModalForm::implies(ModalForm::atom("p"), ModalForm::atom("q")),
            &f, &v, &w("0")
        ));
    }

    #[test]
    fn k_class_always_satisfied() {
        let f = linear_frame();
        assert!(FrameClass::K.is_satisfied_by(&f));
    }

    #[test]
    fn reflexive_class_recognised() {
        let f = reflexive_frame();
        assert!(FrameClass::Reflexive.is_satisfied_by(&f));
        let g = linear_frame();
        assert!(!FrameClass::Reflexive.is_satisfied_by(&g));
    }

    #[test]
    fn s5_requires_all_three_properties() {
        // Reflexive + symmetric + transitive frame: 2-clique.
        let mut f = KripkeFrame::new();
        for from in &["0", "1"] {
            for to in &["0", "1"] {
                f.add_edge(w(from), w(to));
            }
        }
        assert!(FrameClass::S5.is_satisfied_by(&f));
        // Linear frame is not symmetric.
        assert!(!FrameClass::S5.is_satisfied_by(&linear_frame()));
    }

    #[test]
    fn transitive_class_detection() {
        // w0 → w1 → w2 and also w0 → w2 — transitive
        let mut f = KripkeFrame::new();
        f.add_edge(w("0"), w("1"));
        f.add_edge(w("1"), w("2"));
        f.add_edge(w("0"), w("2"));
        assert!(FrameClass::Transitive.is_satisfied_by(&f));

        // Without w0→w2, not transitive.
        assert!(!FrameClass::Transitive.is_satisfied_by(&linear_frame()));
    }

    #[test]
    fn nested_modalities_evaluate() {
        // w0 → w1 → w2, p holds at w2.
        // ◇◇p at w0?  Yes, via w0→w1→w2.
        let f = linear_frame();
        let mut v = Valuation::new();
        v.set(w("2"), "p", true);
        let formula = ModalForm::pos(ModalForm::pos(ModalForm::atom("p")));
        assert!(evaluate(&formula, &f, &v, &w("0")));
    }
}
