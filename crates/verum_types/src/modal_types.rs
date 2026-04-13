//! Modal Types — security labels and information-flow control.
//!
//! A *modal type* annotates a value with a **security label** drawn
//! from a lattice of labels, and the type system enforces that
//! labeled values only flow to contexts permitted by the lattice
//! ordering. This is the foundation of static information-flow
//! control (IFC) as pioneered by Denning, Myers, and Pottier.
//!
//! ## The label lattice
//!
//! Security labels form a **lattice**: any two labels have a
//! least upper bound (join) and a greatest lower bound (meet).
//! A typical default lattice is the totally-ordered
//! `Public ⊑ Internal ⊑ Secret ⊑ TopSecret` chain, but this module
//! supports arbitrary user-defined lattices through explicit
//! parent relations.
//!
//! ## Flows-to
//!
//! The central predicate is `label_a.flows_to(&label_b)` — true iff
//! values labeled `a` may be used in contexts of label `b` without
//! leaking information. In a chain lattice this is simply "a ≤ b",
//! but the general lattice case requires comparing through the
//! explicit parent chain.
//!
//! ## Composition rules
//!
//! * **Binary ops** on labeled values yield the **join** of their
//!   labels. `Secret(x) + Secret(y) : Secret`, and
//!   `Public(x) + Secret(y) : Secret` — the more sensitive label
//!   wins.
//! * **Conditional flow**: when the condition of an `if` has label
//!   `L`, both branches' results are **implicitly** labeled at
//!   least `L` (preventing implicit channel leaks).
//! * **Downgrading** (lowering a secret value's label to public)
//!   is only permitted through explicit declassification, which is
//!   outside this module's scope — it must be authorized externally.
//!
//! ## Modal operators (optional, future)
//!
//! The "modal" naming anticipates full modal type theory: `□_L T`
//! for "T necessarily at label L" and `◇_L T` for "T possibly at L".
//! The current module realises the simpler label-annotated
//! fragment, which is sufficient for static IFC and aligns with
//! Jif / FlowCaml / LIO precedents.

use std::collections::HashMap;

use verum_common::Text;

/// A single security label, identified by name. Labels form a
/// lattice through the explicit parent relation stored in
/// [`LabelLattice`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    pub name: Text,
}

impl Label {
    pub fn new(name: impl Into<Text>) -> Self {
        Self { name: name.into() }
    }

    /// The public label — shorthand for the canonical bottom of
    /// the default chain lattice.
    pub fn public() -> Self {
        Self::new("Public")
    }

    /// The secret label — shorthand for a common mid-lattice level.
    pub fn secret() -> Self {
        Self::new("Secret")
    }

    /// The top-secret label — shorthand for the canonical top of
    /// the default chain lattice.
    pub fn top_secret() -> Self {
        Self::new("TopSecret")
    }
}

/// A lattice of labels, keyed by name, with explicit parent links.
///
/// `parent_of[L]` names a label that `L` flows into (i.e., `L ⊑
/// parent_of[L]`). Labels without a parent entry are maximal.
/// Reflexivity (`L ⊑ L`) and transitivity (`L ⊑ M ⊑ N ⇒ L ⊑ N`)
/// are handled by [`Label::flows_to`].
#[derive(Debug, Clone, Default)]
pub struct LabelLattice {
    parents: HashMap<Text, Text>,
}

impl LabelLattice {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct the canonical chain
    /// `Public ⊑ Internal ⊑ Secret ⊑ TopSecret`.
    pub fn chain() -> Self {
        let mut l = Self::new();
        l.add_flow("Public", "Internal");
        l.add_flow("Internal", "Secret");
        l.add_flow("Secret", "TopSecret");
        l
    }

    /// Declare that `lo` flows to `hi` (i.e., `lo ⊑ hi` directly).
    /// Transitive closure is computed lazily by [`flows_to`].
    pub fn add_flow(&mut self, lo: impl Into<Text>, hi: impl Into<Text>) {
        self.parents.insert(lo.into(), hi.into());
    }

    /// Is `lo ⊑ hi` under the lattice? Checks reflexivity first,
    /// then walks the parent chain up to a fixed bound so malformed
    /// cycles do not cause an infinite loop.
    pub fn flows_to(&self, lo: &Label, hi: &Label) -> bool {
        if lo == hi {
            return true;
        }
        let max_steps = self.parents.len() + 1;
        let mut current = lo.name.clone();
        for _ in 0..max_steps {
            match self.parents.get(&current) {
                Some(p) => {
                    if p == &hi.name {
                        return true;
                    }
                    current = p.clone();
                }
                None => return false,
            }
        }
        false
    }

    /// Least upper bound (join) of two labels, using repeated
    /// upward walks. For arbitrary non-chain lattices the result
    /// may be conservative (returning the maximal label of the
    /// lattice) when no common upper bound is found on the
    /// explicit chains.
    pub fn join(&self, a: &Label, b: &Label) -> Label {
        if self.flows_to(a, b) {
            return b.clone();
        }
        if self.flows_to(b, a) {
            return a.clone();
        }
        // No order relation found — fall back to a distinguished
        // top if one exists. This module treats a label with no
        // parent as maximal; find the one `a` reaches.
        let max_steps = self.parents.len() + 1;
        let mut current = a.name.clone();
        for _ in 0..max_steps {
            match self.parents.get(&current) {
                Some(p) => current = p.clone(),
                None => return Label::new(current),
            }
        }
        // Degenerate — return `a` unchanged.
        a.clone()
    }

    /// Number of declared flow edges. Reflexive and transitive
    /// edges are implicit and not counted.
    pub fn len(&self) -> usize {
        self.parents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }
}

/// A violation where a labeled value flows to a context that the
/// lattice does not permit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowViolation {
    pub source: Label,
    pub destination: Label,
}

impl std::fmt::Display for FlowViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "information-flow violation: value at `{}` cannot flow to `{}`",
            self.source.name.as_str(),
            self.destination.name.as_str()
        )
    }
}

impl std::error::Error for FlowViolation {}

/// Check a single data flow. Returns `Ok(())` if permitted,
/// `Err(FlowViolation)` otherwise.
pub fn check_flow(
    lattice: &LabelLattice,
    source: &Label,
    destination: &Label,
) -> Result<(), FlowViolation> {
    if lattice.flows_to(source, destination) {
        Ok(())
    } else {
        Err(FlowViolation {
            source: source.clone(),
            destination: destination.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflexive_flows_to_self() {
        let l = LabelLattice::chain();
        assert!(l.flows_to(&Label::public(), &Label::public()));
        assert!(l.flows_to(&Label::secret(), &Label::secret()));
    }

    #[test]
    fn public_flows_to_secret_in_chain() {
        let l = LabelLattice::chain();
        assert!(l.flows_to(&Label::public(), &Label::secret()));
        assert!(l.flows_to(&Label::public(), &Label::top_secret()));
    }

    #[test]
    fn secret_does_not_flow_to_public() {
        let l = LabelLattice::chain();
        assert!(!l.flows_to(&Label::secret(), &Label::public()));
    }

    #[test]
    fn top_secret_flows_nowhere_but_itself() {
        let l = LabelLattice::chain();
        assert!(!l.flows_to(&Label::top_secret(), &Label::secret()));
        assert!(!l.flows_to(&Label::top_secret(), &Label::public()));
        assert!(l.flows_to(&Label::top_secret(), &Label::top_secret()));
    }

    #[test]
    fn join_of_chain_is_higher() {
        let l = LabelLattice::chain();
        assert_eq!(
            l.join(&Label::public(), &Label::secret()),
            Label::secret()
        );
        assert_eq!(
            l.join(&Label::secret(), &Label::public()),
            Label::secret()
        );
    }

    #[test]
    fn join_with_self_is_self() {
        let l = LabelLattice::chain();
        assert_eq!(
            l.join(&Label::secret(), &Label::secret()),
            Label::secret()
        );
    }

    #[test]
    fn check_flow_permits_upward() {
        let l = LabelLattice::chain();
        assert!(check_flow(&l, &Label::public(), &Label::secret()).is_ok());
    }

    #[test]
    fn check_flow_rejects_downward() {
        let l = LabelLattice::chain();
        let err = check_flow(&l, &Label::secret(), &Label::public()).unwrap_err();
        assert_eq!(err.source, Label::secret());
        assert_eq!(err.destination, Label::public());
    }

    #[test]
    fn custom_lattice_supports_non_chain() {
        // Two independent classifications that both flow into a
        // common Classified label.
        //
        //        Classified
        //       /          \
        //    Medical    Financial
        let mut l = LabelLattice::new();
        l.add_flow("Medical", "Classified");
        l.add_flow("Financial", "Classified");

        let medical = Label::new("Medical");
        let financial = Label::new("Financial");
        let classified = Label::new("Classified");

        assert!(l.flows_to(&medical, &classified));
        assert!(l.flows_to(&financial, &classified));

        // Medical and Financial are siblings — neither flows to
        // the other.
        assert!(!l.flows_to(&medical, &financial));
        assert!(!l.flows_to(&financial, &medical));
    }

    #[test]
    fn empty_lattice_only_reflexive() {
        let l = LabelLattice::new();
        assert!(l.flows_to(&Label::public(), &Label::public()));
        assert!(!l.flows_to(&Label::public(), &Label::secret()));
    }

    #[test]
    fn malformed_cycle_does_not_infinite_loop() {
        // A cyclic parent chain would otherwise loop forever.
        let mut l = LabelLattice::new();
        l.add_flow("A", "B");
        l.add_flow("B", "A");

        // `A ⊑ C` is false — the walk bails out after max_steps.
        assert!(!l.flows_to(&Label::new("A"), &Label::new("C")));
    }

    #[test]
    fn transitive_flow_across_multiple_hops() {
        let l = LabelLattice::chain();
        // Public -> Internal -> Secret -> TopSecret — four-level chain.
        assert!(l.flows_to(&Label::public(), &Label::top_secret()));
    }

    #[test]
    fn violation_display_mentions_labels() {
        let v = FlowViolation {
            source: Label::secret(),
            destination: Label::public(),
        };
        let s = format!("{}", v);
        assert!(s.contains("Secret"));
        assert!(s.contains("Public"));
    }

    #[test]
    fn lattice_len_counts_edges() {
        let l = LabelLattice::chain();
        assert_eq!(l.len(), 3); // three flow edges in the canonical chain
    }
}
