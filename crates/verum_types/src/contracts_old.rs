//! Dependent Contracts with `old()` — pre-state references.
//!
//! Postconditions in mutating functions often need to refer to the
//! *prior* value of state, not the current value. Eiffel, JML,
//! Dafny, and SPARK all use the `old()` syntax for this:
//!
//! ```text
//!     fn increment(&mut self)
//!         ensures self.count == old(self.count) + 1
//!     { self.count += 1 }
//! ```
//!
//! Without `old()`, postconditions can't say "the new value
//! is one more than what it was before" — they can only describe
//! the new state in isolation. With `old()`, contracts capture
//! *change* directly.
//!
//! ## Operational model
//!
//! At verification time, every `old(e)` in the postcondition is
//! evaluated against the *pre-state* snapshot. Concretely:
//!
//! 1. Before the function body executes, take a snapshot of every
//!    location mentioned inside `old(...)` expressions.
//! 2. After the body, evaluate the postcondition with `old(e)`
//!    bound to the snapshot value.
//!
//! ## Frame computation
//!
//! A function's *frame* is the set of locations it may modify. A
//! sound contract system requires every modified location's prior
//! value to be either:
//!
//! * Mentioned by `old(...)` in the postcondition (so the contract
//!   says how it changed), or
//! * Permitted to take any value (no constraint asserted).
//!
//! Locations *not* in the frame must equal their `old` value
//! after execution — this is the **frame property** that makes
//! local reasoning sound.
//!
//! ## API
//!
//! * [`Snapshot`] — pre-state snapshot keyed by location name.
//! * [`OldExpr`] — abstract representation of `old(expr)`.
//! * [`Postcondition`] — a contract clause that may reference
//!   `old(...)` expressions.
//! * [`Frame`] — the set of locations a function may modify.
//! * [`compute_frame_obligations`] — given a postcondition and a
//!   declared frame, returns the locations whose `old` value
//!   must be snapshot to verify the postcondition.

use std::collections::{HashMap, HashSet};

use verum_common::{List, Text};

/// A pre-state snapshot: maps a location name to its prior value
/// (represented opaquely as a textual symbol for this analysis core).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    values: HashMap<Text, Text>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that location `loc` had value `value` before execution.
    pub fn record(&mut self, loc: impl Into<Text>, value: impl Into<Text>) {
        self.values.insert(loc.into(), value.into());
    }

    /// Look up a snapshotted value.
    pub fn lookup(&self, loc: &Text) -> Option<&Text> {
        self.values.get(loc)
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// Abstract representation of `old(expr)` clauses inside a
/// postcondition. We store only the location name being snapshot —
/// arbitrary expression structure is opaque to this module.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OldExpr {
    pub location: Text,
}

impl OldExpr {
    pub fn new(loc: impl Into<Text>) -> Self {
        Self {
            location: loc.into(),
        }
    }
}

/// A postcondition: a list of `OldExpr` references it makes plus
/// (opaquely) the locations it constrains in the post-state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Postcondition {
    pub old_refs: List<OldExpr>,
    pub mentions_post: List<Text>,
}

impl Postcondition {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_old(mut self, loc: impl Into<Text>) -> Self {
        self.old_refs.push(OldExpr::new(loc));
        self
    }

    pub fn with_post(mut self, loc: impl Into<Text>) -> Self {
        self.mentions_post.push(loc.into());
        self
    }

    /// Distinct locations referenced via `old(...)`.
    pub fn old_locations(&self) -> HashSet<Text> {
        self.old_refs.iter().map(|o| o.location.clone()).collect()
    }
}

/// A function's frame: the set of locations it may modify.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    pub modifies: HashSet<Text>,
}

impl Frame {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, loc: impl Into<Text>) -> Self {
        self.modifies.insert(loc.into());
        self
    }

    pub fn contains(&self, loc: &Text) -> bool {
        self.modifies.contains(loc)
    }

    pub fn len(&self) -> usize {
        self.modifies.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modifies.is_empty()
    }
}

/// Frame analysis result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameAnalysis {
    /// Locations whose pre-value must be snapshot to evaluate the
    /// postcondition (i.e., locations referenced by `old(...)`).
    pub locations_to_snapshot: HashSet<Text>,
    /// Modified locations that the postcondition does *not*
    /// describe via `old(...)`. These are unconstrained by the
    /// contract — a soundness warning candidate.
    pub unconstrained_modifications: HashSet<Text>,
    /// `old(...)` references to locations *not* in the frame.
    /// These are tautologically equal to the post-value (the
    /// location is not modified) — a redundancy candidate.
    pub redundant_old_refs: HashSet<Text>,
}

impl FrameAnalysis {
    pub fn is_clean(&self) -> bool {
        self.unconstrained_modifications.is_empty()
            && self.redundant_old_refs.is_empty()
    }
}

/// Given a postcondition and a declared frame, compute the
/// snapshot obligations and report any unconstrained modifications
/// or redundant `old(...)` references.
pub fn compute_frame_obligations(
    post: &Postcondition,
    frame: &Frame,
) -> FrameAnalysis {
    let old_locs = post.old_locations();

    // Locations that must be snapshot: every old-ref'd location.
    let to_snapshot: HashSet<Text> = old_locs.clone();

    // Modified locations not mentioned by old(...).
    // A clean contract should reference every modified location's
    // old value (otherwise the postcondition can't constrain how
    // it changed).
    let unconstrained: HashSet<Text> = frame
        .modifies
        .iter()
        .filter(|loc| !old_locs.contains(*loc))
        .cloned()
        .collect();

    // old(...) references to locations not in the frame are
    // redundant: if the location isn't modified, old(loc) == loc,
    // so the postcondition could refer to the post-value directly.
    let redundant: HashSet<Text> = old_locs
        .iter()
        .filter(|loc| !frame.contains(loc))
        .cloned()
        .collect();

    FrameAnalysis {
        locations_to_snapshot: to_snapshot,
        unconstrained_modifications: unconstrained,
        redundant_old_refs: redundant,
    }
}

/// Take a snapshot of the locations dictated by a frame analysis.
/// Pre-values are read from an externally-supplied store callback.
pub fn snapshot_frame<F>(
    analysis: &FrameAnalysis,
    mut read_store: F,
) -> Snapshot
where
    F: FnMut(&Text) -> Text,
{
    let mut snap = Snapshot::new();
    let mut locs: Vec<&Text> = analysis.locations_to_snapshot.iter().collect();
    locs.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    for loc in locs {
        let value = read_store(loc);
        snap.record(loc.clone(), value);
    }
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Text {
        Text::from(s)
    }

    #[test]
    fn snapshot_records_and_looks_up() {
        let mut s = Snapshot::new();
        s.record("count", "5");
        assert_eq!(s.lookup(&t("count")), Some(&t("5")));
        assert_eq!(s.lookup(&t("missing")), None);
    }

    #[test]
    fn empty_postcondition_has_no_old_refs() {
        let p = Postcondition::new();
        assert!(p.old_locations().is_empty());
    }

    #[test]
    fn postcondition_collects_distinct_old_locations() {
        let p = Postcondition::new()
            .with_old("count")
            .with_old("name")
            .with_old("count"); // duplicate
        let locs = p.old_locations();
        assert_eq!(locs.len(), 2);
        assert!(locs.contains(&t("count")));
        assert!(locs.contains(&t("name")));
    }

    #[test]
    fn frame_with_one_loc() {
        let f = Frame::new().with("count");
        assert!(f.contains(&t("count")));
        assert!(!f.contains(&t("other")));
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn analysis_clean_contract_passes() {
        // Function modifies `count`; postcondition references
        // `old(count)`. Clean.
        let p = Postcondition::new().with_old("count").with_post("count");
        let f = Frame::new().with("count");
        let a = compute_frame_obligations(&p, &f);

        assert!(a.is_clean());
        assert!(a.locations_to_snapshot.contains(&t("count")));
    }

    #[test]
    fn analysis_flags_unconstrained_modification() {
        // Function modifies `count` AND `name`, but postcondition
        // only references `old(count)`. `name` is unconstrained.
        let p = Postcondition::new().with_old("count");
        let f = Frame::new().with("count").with("name");
        let a = compute_frame_obligations(&p, &f);

        assert!(!a.is_clean());
        assert!(a.unconstrained_modifications.contains(&t("name")));
        assert!(!a.unconstrained_modifications.contains(&t("count")));
    }

    #[test]
    fn analysis_flags_redundant_old_ref() {
        // Postcondition uses `old(immutable)` but `immutable` is
        // not in the frame — the reference is redundant.
        let p = Postcondition::new().with_old("immutable");
        let f = Frame::new().with("count");
        let a = compute_frame_obligations(&p, &f);

        assert!(a.redundant_old_refs.contains(&t("immutable")));
        assert!(!a.redundant_old_refs.contains(&t("count")));
    }

    #[test]
    fn analysis_can_be_both_unclean_dimensions() {
        let p = Postcondition::new().with_old("not_modified");
        let f = Frame::new().with("modified_but_not_constrained");
        let a = compute_frame_obligations(&p, &f);

        assert!(!a.is_clean());
        assert_eq!(a.unconstrained_modifications.len(), 1);
        assert_eq!(a.redundant_old_refs.len(), 1);
    }

    #[test]
    fn snapshot_frame_reads_via_callback() {
        let mut p = Postcondition::new();
        p = p.with_old("a").with_old("b");
        let f = Frame::new().with("a").with("b");
        let analysis = compute_frame_obligations(&p, &f);

        let snap = snapshot_frame(&analysis, |loc| {
            Text::from(format!("val_of_{}", loc.as_str()))
        });

        assert_eq!(snap.lookup(&t("a")), Some(&t("val_of_a")));
        assert_eq!(snap.lookup(&t("b")), Some(&t("val_of_b")));
    }

    #[test]
    fn snapshot_frame_processes_locations_alphabetically() {
        // Deterministic ordering for proof-stability runs: the
        // callback receives locations in alphabetical order.
        let mut p = Postcondition::new();
        p = p.with_old("zeta").with_old("alpha").with_old("mu");
        let f = Frame::new().with("zeta").with("alpha").with("mu");
        let analysis = compute_frame_obligations(&p, &f);

        let mut order = Vec::new();
        let _ = snapshot_frame(&analysis, |loc| {
            order.push(loc.as_str().to_string());
            Text::from("v")
        });

        assert_eq!(order, vec!["alpha", "mu", "zeta"]);
    }

    #[test]
    fn empty_frame_with_empty_postcondition_is_clean() {
        let p = Postcondition::new();
        let f = Frame::new();
        let a = compute_frame_obligations(&p, &f);
        assert!(a.is_clean());
        assert!(a.locations_to_snapshot.is_empty());
    }

    #[test]
    fn frame_with_multiple_modifications_constrained_by_old() {
        let p = Postcondition::new()
            .with_old("x")
            .with_old("y")
            .with_old("z");
        let f = Frame::new().with("x").with("y").with("z");
        let a = compute_frame_obligations(&p, &f);

        assert!(a.is_clean());
        assert_eq!(a.locations_to_snapshot.len(), 3);
    }

    #[test]
    fn old_ref_not_in_frame_is_redundant_but_still_snapshotted() {
        // We still snapshot it (the postcondition asks for it),
        // but flag it as redundant.
        let p = Postcondition::new().with_old("untouched");
        let f = Frame::new();
        let a = compute_frame_obligations(&p, &f);

        assert!(a.locations_to_snapshot.contains(&t("untouched")));
        assert!(a.redundant_old_refs.contains(&t("untouched")));
    }

    #[test]
    fn postcondition_mentions_post_for_completeness() {
        let p = Postcondition::new().with_post("count");
        assert_eq!(p.mentions_post.len(), 1);
        assert_eq!(p.mentions_post[0].as_str(), "count");
    }
}
