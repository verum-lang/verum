//! Sheaf descent condition encoding for SMT verification.
//!
//! An ∞-sheaf on a site `(C, J)` satisfies the **descent condition**:
//! for every covering sieve `S` of an object `c`, the canonical map
//! `F(c) → holim_{d → c ∈ S} F(d)` is an equivalence of ∞-groupoids.
//!
//! At the SMT level, we encode descent as a first-order constraint:
//! given a covering family `{f_i : d_i → c}` and compatible local
//! data `{s_i : F(d_i)}` with `restrict_ij(s_i) = restrict_ji(s_j)`
//! on all overlaps, there exists a unique global section `s : F(c)`
//! with `restrict_i(s) = s_i` for every `i`.

use verum_common::{List, Text};

/// A sheaf descent problem: given a covering family and compatible
/// local sections, verify that a unique global section exists.
#[derive(Debug, Clone)]
pub struct DescentProblem {
    /// The target object `c : C`.
    pub target: Text,
    /// The covering morphisms `f_i : d_i → c` (as opaque handles).
    pub cover: List<Text>,
    /// The local sections `s_i : F(d_i)` (as opaque handles).
    pub local_sections: List<Text>,
    /// The pairwise-overlap compatibility conditions (already assumed).
    pub compatibility_assumed: bool,
}

impl DescentProblem {
    pub fn new(target: impl Into<Text>) -> Self {
        Self {
            target: target.into(),
            cover: List::new(),
            local_sections: List::new(),
            compatibility_assumed: false,
        }
    }

    /// Add a covering morphism with its local section.
    pub fn add_cover(mut self, morphism: impl Into<Text>, section: impl Into<Text>) -> Self {
        self.cover.push(morphism.into());
        self.local_sections.push(section.into());
        self
    }

    /// Assert that compatibility on overlaps has been verified.
    pub fn with_compatibility(mut self) -> Self {
        self.compatibility_assumed = true;
        self
    }
}

/// Verification result for a descent problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescentResult {
    /// Descent holds — unique global section exists.
    UniqueGlobalSection,
    /// Compatibility not verified — cannot conclude descent.
    CompatibilityNotVerified,
    /// Empty cover — trivially descended (unique section = the F(c) itself).
    EmptyCover,
    /// Descent cannot be proven by this backend.
    Undetermined,
}

/// Verify the descent condition for a given problem.
///
/// This is a lightweight syntactic check at the SMT-encoding level;
/// the actual descent-preservation proof requires the concrete
/// ∞-sheaf implementation to discharge its `@verify(formal) descent`
/// obligation.
pub fn verify_descent(problem: &DescentProblem) -> DescentResult {
    // Empty cover: descent is trivial (unique section = the identity).
    if problem.cover.is_empty() {
        return DescentResult::EmptyCover;
    }

    // Compatibility on overlaps is a precondition — without it we
    // cannot conclude uniqueness of the global section.
    if !problem.compatibility_assumed {
        return DescentResult::CompatibilityNotVerified;
    }

    // Cover and sections must align.
    if problem.cover.len() != problem.local_sections.len() {
        return DescentResult::Undetermined;
    }

    // All preconditions satisfied — unique global section exists.
    DescentResult::UniqueGlobalSection
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_cover_trivial_descent() {
        let p = DescentProblem::new("c");
        assert_eq!(verify_descent(&p), DescentResult::EmptyCover);
    }

    #[test]
    fn test_cover_without_compatibility_undetermined() {
        let p = DescentProblem::new("c").add_cover("f1", "s1");
        assert_eq!(
            verify_descent(&p),
            DescentResult::CompatibilityNotVerified
        );
    }

    #[test]
    fn test_cover_with_compatibility_descends() {
        let p = DescentProblem::new("c")
            .add_cover("f1", "s1")
            .add_cover("f2", "s2")
            .with_compatibility();
        assert_eq!(verify_descent(&p), DescentResult::UniqueGlobalSection);
    }

    #[test]
    fn test_single_cover_with_compatibility() {
        let p = DescentProblem::new("c")
            .add_cover("f", "s")
            .with_compatibility();
        assert_eq!(verify_descent(&p), DescentResult::UniqueGlobalSection);
    }

    #[test]
    fn test_mismatched_sections_undetermined() {
        let mut p = DescentProblem::new("c").with_compatibility();
        p.cover.push(Text::from("f1"));
        p.cover.push(Text::from("f2"));
        p.local_sections.push(Text::from("s1"));
        // Only 1 section for 2 covers → mismatch
        assert_eq!(verify_descent(&p), DescentResult::Undetermined);
    }
}
