//! ATS-V Counterfactual Reasoning Engine.
//!
//! Per spec §20.4 + §22.1, evaluates a cog's architectural Shape
//! under base + alternative decisions WITHOUT actually switching
//! implementations. Comparative metric extraction at type level.
//!
//! Pipeline:
//! 1. Caller supplies (CounterfactualPair, base_shape, alt_shape).
//! 2. [`extract_metric`] walks both shapes, projects to [`MetricValue`]s.
//! 3. [`evaluate_invariant`] checks each [`ArchProposition`] from
//! `pair.stability_invariants` against both shapes — returns
//! [`InvariantStatus`].
//! 4. [`CounterfactualReport`] carries the comparative payload with
//! stable JSON schema_version=1 for agent surfaces (per spec §32.4).
//!
//! Non-destructive by construction — the engine never instantiates
//! either decision; both shapes are passed in as types.

use serde::{Deserialize, Serialize};

use crate::arch::{Capability, MsfsStratum, Shape};
use crate::arch_mtac::{ArchProposition, CounterfactualPair};

// =============================================================================
// ArchMetric — canonical metric set
// =============================================================================

/// An architectural metric that can be projected from a [`Shape`].
/// ships the canonical baseline — extending the catalog
/// requires RFC ATS-V-006 per spec §29.2.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArchMetric {
 /// Number of exposed capabilities (interface surface).
    ExposedCapabilityCount,
 /// Number of required capabilities (dependency surface).
    RequiredCapabilityCount,
 /// Sum of read-tag capabilities (exposed + required).
    ReadCapabilityCount,
 /// Sum of write-tag capabilities.
    WriteCapabilityCount,
 /// Sum of network-tag capabilities.
    NetworkCapabilityCount,
 /// Number of preserved boundary invariants.
    BoundaryInvariantCount,
 /// Number of cogs this composes with (composition degree).
    CompositionDegree,
 /// Number of consumed linear/affine resources.
    LinearResourceCount,
 /// MSFS stratum — encoded as ordinal (LFnd=0, LCls=1, LClsTop=2,
 /// LAbs=3 although LAbs is itself a defect).
    StratumOrdinal,
 /// CVE-closure completeness 0..=3 (axes discharged).
    CveCompleteness,
 /// Strict-mode flag (0 / 1).
    StrictModeFlag,
 /// Custom metric — caller-defined name; engine returns
 /// `MetricValue::Unknown` (extensibility hook
 /// self-hosted reasoning).
    Custom { name: String },
}

impl ArchMetric {
 /// Stable single-token tag for JSON serialisation.
    pub fn tag(&self) -> &'static str {
        match self {
            ArchMetric::ExposedCapabilityCount => "exposed_capability_count",
            ArchMetric::RequiredCapabilityCount => "required_capability_count",
            ArchMetric::ReadCapabilityCount => "read_capability_count",
            ArchMetric::WriteCapabilityCount => "write_capability_count",
            ArchMetric::NetworkCapabilityCount => "network_capability_count",
            ArchMetric::BoundaryInvariantCount => "boundary_invariant_count",
            ArchMetric::CompositionDegree => "composition_degree",
            ArchMetric::LinearResourceCount => "linear_resource_count",
            ArchMetric::StratumOrdinal => "stratum_ordinal",
            ArchMetric::CveCompleteness => "cve_completeness",
            ArchMetric::StrictModeFlag => "strict_mode_flag",
            ArchMetric::Custom { .. } => "custom",
        }
    }

 /// The canonical baseline metric set. The default battery
 /// `evaluate_counterfactual` runs when caller passes an empty
 /// metric list.
    pub fn baseline_set() -> Vec<ArchMetric> {
        vec![
            ArchMetric::ExposedCapabilityCount,
            ArchMetric::RequiredCapabilityCount,
            ArchMetric::ReadCapabilityCount,
            ArchMetric::WriteCapabilityCount,
            ArchMetric::NetworkCapabilityCount,
            ArchMetric::BoundaryInvariantCount,
            ArchMetric::CompositionDegree,
            ArchMetric::LinearResourceCount,
            ArchMetric::StratumOrdinal,
            ArchMetric::CveCompleteness,
            ArchMetric::StrictModeFlag,
        ]
    }
}

// =============================================================================
// MetricValue — projection target
// =============================================================================

/// Result of projecting an [`ArchMetric`] from a [`Shape`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum MetricValue {
 /// Integer projection — count-style metrics.
    Integer(i64),
 /// Floating-point projection ( reserved — ratios, etc.).
    Float(f64),
 /// Categorical — for non-ordinal metrics (e.g. stratum tag).
    Categorical(String),
 /// Engine cannot derive value for this Shape (e.g. custom metric
 /// without registered extractor).
    Unknown,
}

impl MetricValue {
 /// True iff the two values are identical projections.
    pub fn equivalent(&self, other: &MetricValue) -> bool {
        match (self, other) {
            (MetricValue::Integer(a), MetricValue::Integer(b)) => a == b,
            (MetricValue::Float(a), MetricValue::Float(b)) => (a - b).abs() < f64::EPSILON,
            (MetricValue::Categorical(a), MetricValue::Categorical(b)) => a == b,
            (MetricValue::Unknown, MetricValue::Unknown) => true,
            _ => false,
        }
    }
}

// =============================================================================
// Metric extraction — Shape → MetricValue
// =============================================================================

/// Project a metric from a Shape.
pub fn extract_metric(metric: &ArchMetric, shape: &Shape) -> MetricValue {
    match metric {
        ArchMetric::ExposedCapabilityCount => MetricValue::Integer(shape.exposes.len() as i64),
        ArchMetric::RequiredCapabilityCount => MetricValue::Integer(shape.requires.len() as i64),
        ArchMetric::ReadCapabilityCount => {
            MetricValue::Integer(count_capability(shape, |c| matches!(c, Capability::Read { .. })))
        }
        ArchMetric::WriteCapabilityCount => {
            MetricValue::Integer(count_capability(shape, |c| matches!(c, Capability::Write { .. })))
        }
        ArchMetric::NetworkCapabilityCount => MetricValue::Integer(count_capability(shape, |c| {
            matches!(c, Capability::Network { .. })
        })),
        ArchMetric::BoundaryInvariantCount => {
            MetricValue::Integer(shape.preserves.len() as i64)
        }
        ArchMetric::CompositionDegree => {
            MetricValue::Integer(shape.composes_with.len() as i64)
        }
        ArchMetric::LinearResourceCount => MetricValue::Integer(shape.consumes.len() as i64),
        ArchMetric::StratumOrdinal => MetricValue::Integer(stratum_ordinal(&shape.stratum)),
        ArchMetric::CveCompleteness => {
            MetricValue::Integer(shape.cve_closure.closure_degree() as i64)
        }
        ArchMetric::StrictModeFlag => MetricValue::Integer(if shape.strict { 1 } else { 0 }),
        ArchMetric::Custom { .. } => MetricValue::Unknown,
    }
}

fn count_capability(shape: &Shape, pred: impl Fn(&Capability) -> bool) -> i64 {
    let exposed = shape.exposes.iter().filter(|c| pred(c)).count();
    let required = shape.requires.iter().filter(|c| pred(c)).count();
    (exposed + required) as i64
}

fn stratum_ordinal(s: &MsfsStratum) -> i64 {
    match s {
        MsfsStratum::LFnd => 0,
        MsfsStratum::LCls => 1,
        MsfsStratum::LClsTop => 2,
        MsfsStratum::LAbs => 3,
    }
}

// =============================================================================
// Proposition evaluation — ArchProposition × Shape → Bool
// =============================================================================

/// True iff the proposition holds for the given Shape. 
/// covers the four canonical [`ArchProposition`] variants; `Custom`
/// is conservatively rejected (no registered evaluator).
pub fn proposition_holds(prop: &ArchProposition, shape: &Shape) -> bool {
    match prop {
        ArchProposition::HasCapability { capability_tag } => {
            let needle = capability_tag.as_str();
            shape
                .exposes
                .iter()
                .chain(shape.requires.iter())
                .any(|c| c.tag() == needle)
        }
        ArchProposition::FoundationStable => {
 // Single-shape evaluation: trivially holds (stability is
 // a binary observation across two shapes — see
 // [`evaluate_invariant`]).
            true
        }
        ArchProposition::PublicApiUnchanged => {
 // Single-shape evaluation: trivially holds (relational
 // proposition — handled by [`evaluate_invariant`]).
            true
        }
        ArchProposition::Custom { .. } => false,
    }
}

// =============================================================================
// CounterfactualReport — engine output
// =============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InvariantStatus {
 /// Proposition holds under both base + alternative.
    HoldsBoth,
 /// Holds only under base.
    HoldsBaseOnly,
 /// Holds only under alternative.
    HoldsAltOnly,
 /// Holds under neither.
    HoldsNeither,
}

impl InvariantStatus {
    pub fn tag(&self) -> &'static str {
        match self {
            InvariantStatus::HoldsBoth => "holds_both",
            InvariantStatus::HoldsBaseOnly => "holds_base_only",
            InvariantStatus::HoldsAltOnly => "holds_alt_only",
            InvariantStatus::HoldsNeither => "holds_neither",
        }
    }

 /// True iff the proposition is stable under decision swap — the
 /// only status that satisfies counterfactual stability per spec
 /// §22.2.
    pub fn is_stable(&self) -> bool {
        matches!(self, InvariantStatus::HoldsBoth)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantEvaluation {
    pub proposition: ArchProposition,
    pub status: InvariantStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricComparison {
    pub metric: ArchMetric,
    pub base: MetricValue,
    pub alt: MetricValue,
 /// True iff base != alt.
    pub diverges: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterfactualReport {
 /// Stable JSON schema version (per spec §32.4).
    pub schema_version: u32,
 /// Counterfactual-pair identifier (`pair.name`).
    pub pair_name: String,
 /// Base decision name (`pair.base.name`).
    pub base_decision: String,
 /// Alternative decision name (`pair.alternative.name`).
    pub alt_decision: String,
 /// Per-metric comparison.
    pub metric_comparisons: Vec<MetricComparison>,
 /// Per-invariant evaluation.
    pub invariant_evaluations: Vec<InvariantEvaluation>,
 /// True iff every declared stability invariant holds under both
 /// decisions (i.e. cog is counterfactually stable per §22.2).
    pub overall_stable: bool,
 /// Number of metrics that diverged.
    pub diverging_metric_count: usize,
}

// =============================================================================
// Engine entry points
// =============================================================================

/// Evaluate a single counterfactual pair against the supplied
/// metric set + base/alt shapes. Empty metric list → baseline set.
pub fn evaluate_counterfactual(
    pair: &CounterfactualPair,
    base_shape: &Shape,
    alt_shape: &Shape,
    metrics: &[ArchMetric],
) -> CounterfactualReport {
    let metrics: Vec<ArchMetric> = if metrics.is_empty() {
        ArchMetric::baseline_set()
    } else {
        metrics.to_vec()
    };

    let metric_comparisons: Vec<MetricComparison> = metrics
        .iter()
        .map(|m| {
            let base = extract_metric(m, base_shape);
            let alt = extract_metric(m, alt_shape);
            let diverges = !base.equivalent(&alt);
            MetricComparison {
                metric: m.clone(),
                base,
                alt,
                diverges,
            }
        })
        .collect();

    let invariant_evaluations: Vec<InvariantEvaluation> = pair
        .stability_invariants
        .iter()
        .map(|p| InvariantEvaluation {
            proposition: p.clone(),
            status: evaluate_invariant(p, base_shape, alt_shape),
        })
        .collect();

    let overall_stable = !invariant_evaluations.is_empty()
        && invariant_evaluations.iter().all(|e| e.status.is_stable());

    let diverging_metric_count = metric_comparisons.iter().filter(|c| c.diverges).count();

    CounterfactualReport {
        schema_version: 1,
        pair_name: pair.name.clone(),
        base_decision: pair.base.name.clone(),
        alt_decision: pair.alternative.name.clone(),
        metric_comparisons,
        invariant_evaluations,
        overall_stable,
        diverging_metric_count,
    }
}

/// Evaluate a relational proposition across two shapes.
pub fn evaluate_invariant(
    prop: &ArchProposition,
    base_shape: &Shape,
    alt_shape: &Shape,
) -> InvariantStatus {
    match prop {
        ArchProposition::FoundationStable => {
            if base_shape.foundation == alt_shape.foundation {
                InvariantStatus::HoldsBoth
            } else {
                InvariantStatus::HoldsNeither
            }
        }
        ArchProposition::PublicApiUnchanged => {
            if base_shape.exposes == alt_shape.exposes {
                InvariantStatus::HoldsBoth
            } else {
                InvariantStatus::HoldsNeither
            }
        }
        ArchProposition::HasCapability { .. } | ArchProposition::Custom { .. } => {
            let in_base = proposition_holds(prop, base_shape);
            let in_alt = proposition_holds(prop, alt_shape);
            match (in_base, in_alt) {
                (true, true) => InvariantStatus::HoldsBoth,
                (true, false) => InvariantStatus::HoldsBaseOnly,
                (false, true) => InvariantStatus::HoldsAltOnly,
                (false, false) => InvariantStatus::HoldsNeither,
            }
        }
    }
}

/// Batch-evaluate a counterfactual *set* — one base + many
/// alternatives. Per spec §22.1 (`@arch_counterfactual_set`).
pub fn evaluate_counterfactual_set(
    pair_name: &str,
    base_decision_name: &str,
    base_shape: &Shape,
    alternatives: &[(String, Shape)],
    invariants: &[ArchProposition],
    metrics: &[ArchMetric],
) -> Vec<CounterfactualReport> {
    alternatives
        .iter()
        .map(|(alt_name, alt_shape)| {
            let pair = CounterfactualPair {
                name: format!("{}::{}", pair_name, alt_name),
                base: crate::arch_mtac::Decision {
                    name: base_decision_name.to_string(),
                    options: vec![],
                    chosen: None,
                    depends_on: vec![],
                },
                alternative: crate::arch_mtac::Decision {
                    name: alt_name.clone(),
                    options: vec![],
                    chosen: None,
                    depends_on: vec![],
                },
                stability_invariants: invariants.to_vec(),
            };
            evaluate_counterfactual(&pair, base_shape, alt_shape, metrics)
        })
        .collect()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::{Capability, Foundation, ResourceTag};
    use crate::arch_mtac::Decision;

    fn pair(name: &str, invariants: Vec<ArchProposition>) -> CounterfactualPair {
        CounterfactualPair {
            name: name.into(),
            base: Decision {
                name: "use_pgsql".into(),
                options: vec![],
                chosen: None,
                depends_on: vec![],
            },
            alternative: Decision {
                name: "use_sqlite".into(),
                options: vec![],
                chosen: None,
                depends_on: vec![],
            },
            stability_invariants: invariants,
        }
    }

    #[test]
    fn extract_exposed_capability_count_matches_shape() {
        let mut s = Shape::default_for_unannotated();
        s.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        assert_eq!(
            extract_metric(&ArchMetric::ExposedCapabilityCount, &s),
            MetricValue::Integer(1)
        );
    }

    #[test]
    fn extract_metric_distinguishes_capability_kinds() {
        let mut s = Shape::default_for_unannotated();
        s.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        s.requires = vec![Capability::Write {
            resource: ResourceTag::Logger,
        }];
        assert_eq!(
            extract_metric(&ArchMetric::ReadCapabilityCount, &s),
            MetricValue::Integer(1)
        );
        assert_eq!(
            extract_metric(&ArchMetric::WriteCapabilityCount, &s),
            MetricValue::Integer(1)
        );
        assert_eq!(
            extract_metric(&ArchMetric::NetworkCapabilityCount, &s),
            MetricValue::Integer(0)
        );
    }

    #[test]
    fn custom_metric_returns_unknown() {
        let s = Shape::default_for_unannotated();
        assert_eq!(
            extract_metric(
                &ArchMetric::Custom {
                    name: "BundleSize".into()
                },
                &s,
            ),
            MetricValue::Unknown
        );
    }

    #[test]
    fn metric_value_equivalence_handles_each_arm() {
        assert!(MetricValue::Integer(3).equivalent(&MetricValue::Integer(3)));
        assert!(!MetricValue::Integer(3).equivalent(&MetricValue::Integer(4)));
        assert!(MetricValue::Float(1.0).equivalent(&MetricValue::Float(1.0)));
        assert!(MetricValue::Categorical("a".into())
            .equivalent(&MetricValue::Categorical("a".into())));
        assert!(MetricValue::Unknown.equivalent(&MetricValue::Unknown));
        assert!(!MetricValue::Integer(1).equivalent(&MetricValue::Float(1.0)));
    }

    #[test]
    fn baseline_metric_set_is_canonical_size() {
 // Pin: baseline metric battery is 11 ( catalog).
 // Adding more requires RFC ATS-V-006 per spec §29.2.
        assert_eq!(ArchMetric::baseline_set().len(), 11);
    }

    #[test]
    fn evaluate_invariant_foundation_stable() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.foundation = Foundation::ZfcTwoInacc;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.foundation = Foundation::Hott;
 // Drift across alternatives
        assert_eq!(
            evaluate_invariant(&ArchProposition::FoundationStable, &s_base, &s_alt),
            InvariantStatus::HoldsNeither
        );
 // Identity case
        assert_eq!(
            evaluate_invariant(&ArchProposition::FoundationStable, &s_base, &s_base),
            InvariantStatus::HoldsBoth
        );
    }

    #[test]
    fn evaluate_invariant_public_api_unchanged() {
        let s_base = Shape::default_for_unannotated();
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        assert_eq!(
            evaluate_invariant(&ArchProposition::PublicApiUnchanged, &s_base, &s_alt),
            InvariantStatus::HoldsNeither
        );
    }

    #[test]
    fn evaluate_invariant_has_capability_per_shape() {
        let s_base = Shape::default_for_unannotated();
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        let prop = ArchProposition::HasCapability {
            capability_tag: "read".into(),
        };
        assert_eq!(
            evaluate_invariant(&prop, &s_base, &s_alt),
            InvariantStatus::HoldsAltOnly,
        );
        assert_eq!(
            evaluate_invariant(&prop, &s_alt, &s_base),
            InvariantStatus::HoldsBaseOnly,
        );
    }

    #[test]
    fn report_marks_stable_when_all_invariants_hold_both() {
        let s = Shape::default_for_unannotated();
        let pair = pair(
            "stability_pin",
            vec![ArchProposition::FoundationStable, ArchProposition::PublicApiUnchanged],
        );
        let report = evaluate_counterfactual(&pair, &s, &s, &[]);
        assert!(report.overall_stable);
        assert_eq!(report.diverging_metric_count, 0);
        assert_eq!(report.invariant_evaluations.len(), 2);
        for ev in &report.invariant_evaluations {
            assert_eq!(ev.status, InvariantStatus::HoldsBoth);
        }
    }

    #[test]
    fn report_marks_unstable_when_any_invariant_fails() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.foundation = Foundation::ZfcTwoInacc;
        let mut s_alt = Shape::default_for_unannotated();
        s_alt.foundation = Foundation::Hott;
        let pair = pair("brittle", vec![ArchProposition::FoundationStable]);
        let report = evaluate_counterfactual(&pair, &s_base, &s_alt, &[]);
        assert!(!report.overall_stable);
        assert_eq!(
            report.invariant_evaluations[0].status,
            InvariantStatus::HoldsNeither
        );
    }

    #[test]
    fn report_no_invariants_is_unstable_by_default() {
 // Per spec §22.2: a counterfactual without declared stability
 // invariants cannot be claimed stable — the engine refuses to
 // synthesize a positive verdict from absence of evidence.
        let s = Shape::default_for_unannotated();
        let pair = pair("no_invariants", vec![]);
        let report = evaluate_counterfactual(&pair, &s, &s, &[]);
        assert!(!report.overall_stable);
    }

    #[test]
    fn metric_divergence_counted_correctly() {
        let mut s_base = Shape::default_for_unannotated();
        s_base.exposes = vec![
            Capability::Read {
                resource: ResourceTag::Logger,
            },
            Capability::Write {
                resource: ResourceTag::Logger,
            },
        ];
        let s_alt = Shape::default_for_unannotated();
        let pair = pair("diverge_metrics", vec![]);
        let report = evaluate_counterfactual(
            &pair,
            &s_base,
            &s_alt,
            &[
                ArchMetric::ExposedCapabilityCount,
                ArchMetric::ReadCapabilityCount,
                ArchMetric::WriteCapabilityCount,
                ArchMetric::NetworkCapabilityCount,
            ],
        );
        assert_eq!(report.diverging_metric_count, 3); // exposed + read + write differ; network = 0
    }

    #[test]
    fn json_round_trip_preserves_payload() {
 // serde_json is dev-only in this crate; the round-trip is a
 // pin that the report's `Serialize`/`Deserialize` derives stay
 // structurally compatible across schema_version=1.
        let s = Shape::default_for_unannotated();
        let pair = pair("json_pin", vec![ArchProposition::FoundationStable]);
        let report = evaluate_counterfactual(&pair, &s, &s, &[]);
        let json = serde_json::to_string(&report).expect("must serialise");
        let back: CounterfactualReport =
            serde_json::from_str(&json).expect("must round-trip");
        assert_eq!(back.pair_name, report.pair_name);
        assert_eq!(back.schema_version, 1);
        assert_eq!(back.diverging_metric_count, report.diverging_metric_count);
    }

    #[test]
    fn counterfactual_set_evaluates_each_alternative() {
        let s_base = Shape::default_for_unannotated();
        let mut s_a = Shape::default_for_unannotated();
        s_a.exposes = vec![Capability::Read {
            resource: ResourceTag::Logger,
        }];
        let mut s_b = Shape::default_for_unannotated();
        s_b.foundation = Foundation::Hott;
        let alternatives = vec![("a".to_string(), s_a), ("b".to_string(), s_b)];
        let reports = evaluate_counterfactual_set(
            "fw_choice",
            "vue",
            &s_base,
            &alternatives,
            &[ArchProposition::PublicApiUnchanged, ArchProposition::FoundationStable],
            &[],
        );
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].pair_name, "fw_choice::a");
        assert_eq!(reports[1].pair_name, "fw_choice::b");
 // alt_a changes API → stable=false
        assert!(!reports[0].overall_stable);
 // alt_b changes foundation → stable=false
        assert!(!reports[1].overall_stable);
    }

    #[test]
    fn architectural_pin_invariant_status_tags_are_stable() {
 // Pin: the four canonical statuses surface their stable tags
 // exactly as referenced by audit JSON consumers (per spec §32.4).
        assert_eq!(InvariantStatus::HoldsBoth.tag(), "holds_both");
        assert_eq!(InvariantStatus::HoldsBaseOnly.tag(), "holds_base_only");
        assert_eq!(InvariantStatus::HoldsAltOnly.tag(), "holds_alt_only");
        assert_eq!(InvariantStatus::HoldsNeither.tag(), "holds_neither");
    }

    #[test]
    fn architectural_pin_only_holds_both_is_stable() {
 // Pin: per spec §22.2, ONLY HoldsBoth satisfies counterfactual
 // stability — a relational property requiring presence under
 // both decisions.
        assert!(InvariantStatus::HoldsBoth.is_stable());
        assert!(!InvariantStatus::HoldsBaseOnly.is_stable());
        assert!(!InvariantStatus::HoldsAltOnly.is_stable());
        assert!(!InvariantStatus::HoldsNeither.is_stable());
    }
}
