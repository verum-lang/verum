//! Rank diagnostics for a declared diagram in an accessible category.
//! A κ-accessible category admits small κ-filtered colimits. A diagram
//! reported λ-filtered meets that rank condition when κ ≤ λ. Constructing
//! an apex, its cocone, or evidence of presentability requires further data.

use serde::{Deserialize, Serialize};
use verum_common::Text;

use crate::ordinal::Ordinal;

/// Description of a diagram and its reported filtering rank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LambdaFilteredDiagram {
    /// Diagnostic name.
    pub name: Text,
    /// The indexing category I's name.
    pub index_category: Text,
    /// The target category C's name.
    pub target_category: Text,
    /// The cofinality bound λ — the regular cardinal at which the
    /// diagram is filtered.
    pub lambda: Ordinal,
    /// The objects `D(i)` for `i ∈ I`, by index name.
    pub diagram_objects: Vec<(Text, Text)>,
    /// Filtering reported by the producer; checking arrows requires the diagram itself.
    pub is_lambda_filtered: bool,
}

impl LambdaFilteredDiagram {
    /// Construct a fresh λ-filtered diagram fixture.
    pub fn new(
        name: impl Into<Text>,
        index_category: impl Into<Text>,
        target_category: impl Into<Text>,
        lambda: Ordinal,
        diagram_objects: Vec<(Text, Text)>,
        is_lambda_filtered: bool,
    ) -> Self {
        Self {
            name: name.into(),
            index_category: index_category.into(),
            target_category: target_category.into(),
            lambda,
            diagram_objects,
            is_lambda_filtered,
        }
    }
}

/// A κ-accessible category — the target for the AR 1.26 closure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KappaAccessibleCategory {
    /// Diagnostic name (e.g. "Set", "Cat", "(∞,1)-Cat").
    pub name: Text,
    /// The accessibility level κ. Must be a regular cardinal.
    pub kappa: Ordinal,
}

impl KappaAccessibleCategory {
    /// Construct a rank descriptor from an infinite regular cardinal notation.
    pub fn at_kappa(name: impl Into<Text>, kappa: Ordinal) -> Result<Self, FilteredDiagramError> {
        if !infinite_regular(&kappa) {
            return Err(FilteredDiagramError::InvalidCategoryRank);
        }
        Ok(Self {
            name: name.into(),
            kappa,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilteredDiagramError {
    InvalidCategoryRank,
    InvalidDiagramRank,
    TargetMismatch,
    NoReportedObject,
    FilteringUnreported,
    InsufficientFilteringRank,
}

fn infinite_regular(level: &Ordinal) -> bool {
    Ordinal::Omega.le(level) && level.is_regular()
}

/// Check whether the reported source and target meet the rank hypotheses.
/// A successful result carries no claim about the constructed object.
pub fn check_filtered_diagram_scope(
    diagram: &LambdaFilteredDiagram,
    target: &KappaAccessibleCategory,
) -> Result<(), FilteredDiagramError> {
    if !infinite_regular(&target.kappa) {
        return Err(FilteredDiagramError::InvalidCategoryRank);
    }
    if !infinite_regular(&diagram.lambda) {
        return Err(FilteredDiagramError::InvalidDiagramRank);
    }
    if diagram.target_category != target.name {
        return Err(FilteredDiagramError::TargetMismatch);
    }
    if diagram.diagram_objects.is_empty() {
        return Err(FilteredDiagramError::NoReportedObject);
    }
    if !diagram.is_lambda_filtered {
        return Err(FilteredDiagramError::FilteringUnreported);
    }
    if !target.kappa.le(&diagram.lambda) {
        return Err(FilteredDiagramError::InsufficientFilteringRank);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagram(lambda: Ordinal) -> LambdaFilteredDiagram {
        LambdaFilteredDiagram::new("D", "I", "C", lambda, vec![("i".into(), "x".into())], true)
    }

    #[test]
    fn reported_filtering_must_be_at_least_the_required_rank() {
        let target = KappaAccessibleCategory::at_kappa("C", Ordinal::Kappa(1)).unwrap();
        for rank in [Ordinal::Kappa(1), Ordinal::Kappa(2)] {
            assert_eq!(
                check_filtered_diagram_scope(&diagram(rank), &target),
                Ok(())
            );
        }
        assert_eq!(
            check_filtered_diagram_scope(&diagram(Ordinal::Omega), &target),
            Err(FilteredDiagramError::InsufficientFilteringRank)
        );
    }

    #[test]
    fn target_identity_and_reported_filtering_are_required() {
        let target = KappaAccessibleCategory::at_kappa("C", Ordinal::Omega).unwrap();
        let mut d = diagram(Ordinal::Omega);
        d.target_category = "other".into();
        assert_eq!(
            check_filtered_diagram_scope(&d, &target),
            Err(FilteredDiagramError::TargetMismatch)
        );
        d.target_category = target.name.clone();
        d.is_lambda_filtered = false;
        assert_eq!(
            check_filtered_diagram_scope(&d, &target),
            Err(FilteredDiagramError::FilteringUnreported)
        );
        d.is_lambda_filtered = true;
        d.diagram_objects.clear();
        assert_eq!(
            check_filtered_diagram_scope(&d, &target),
            Err(FilteredDiagramError::NoReportedObject)
        );
    }

    #[test]
    fn finite_singular_and_unchecked_wire_ranks_are_refused() {
        let valid = KappaAccessibleCategory::at_kappa("C", Ordinal::Omega).unwrap();
        for rank in [
            Ordinal::Finite(1),
            Ordinal::OmegaPlus(1),
            Ordinal::OmegaSquared,
        ] {
            assert_eq!(
                KappaAccessibleCategory::at_kappa("C", rank.clone()),
                Err(FilteredDiagramError::InvalidCategoryRank)
            );
            let unchecked = KappaAccessibleCategory {
                name: "C".into(),
                kappa: rank.clone(),
            };
            assert_eq!(
                check_filtered_diagram_scope(&diagram(Ordinal::Omega), &unchecked),
                Err(FilteredDiagramError::InvalidCategoryRank)
            );
            assert_eq!(
                check_filtered_diagram_scope(&diagram(rank), &valid),
                Err(FilteredDiagramError::InvalidDiagramRank)
            );
        }
    }
}
