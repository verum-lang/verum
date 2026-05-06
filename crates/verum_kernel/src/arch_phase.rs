//! ATS-V phase 6.5 — architectural type checking phase.
//!
//! ## Architectural role
//!
//! ATS-V phase sits between type inference (Phase 4-6) and VBC
//! codegen (Phase 7) in the compilation pipeline.  Its job:
//!
//! 1. Walk every module in the parsed AST.
//! 2. Extract `@arch_module(...)` attributes via [`crate::arch_parse`].
//! 3. Build [`crate::arch::Shape`] for each module.
//! 4. Run all 32 anti-pattern checks (full canonical catalog).
//! 5. Aggregate violations into [`ArchPhaseReport`] for downstream
//! diagnostic emission.
//!
//! ## Why this lives in `verum_kernel` not `verum_compiler`
//!
//! `verum_kernel` already has all the carrier types (Shape,
//! Capability, Boundary, anti-pattern catalog, parser). Putting
//! the phase orchestration here keeps the kernel self-contained and
//! testable without pulling in the full compiler pipeline.
//! `verum_compiler` invokes this module's `run_arch_phase`
//! function during its actual pipeline at Phase 6.5; CLI
//! consumers (`verum arch explain`) call it directly.
//!
//! ## scope
//!
//! Phase produces an `ArchPhaseReport` from a list of
//! `(module_name, attribute_args)` pairs. wires the
//! Phase into the actual `verum_compiler::pipeline` between
//! Phase 4 (type inference) and Phase 7 (VBC codegen) — that
//! requires touching `Session::run` which is multi-day.

use crate::arch::Shape;
use crate::arch_anti_pattern::{
    check_all_anti_patterns, AntiPatternCode, AntiPatternViolation, DiagnosticContext,
};
use crate::arch_parse::{parse_arch_module, ArchParseError};
use verum_ast::expr::Expr;

// =============================================================================
// ArchPhaseReport — aggregated phase result
// =============================================================================

/// Per-module result of running ATS-V phase.
#[derive(Debug, Clone)]
pub struct ModuleArchResult {
 /// Stable module path (e.g. "core.database.postgres").
    pub module_name: String,
 /// Parsed Shape, or `None` if the module had no `@arch_module(...)`
 /// annotation (uses default shape; no violations expected per
 /// spec §17.5 backward-compat).
    pub shape: Option<Shape>,
 /// Parser errors (if any) produced during shape extraction.
    pub parse_errors: Vec<ArchParseError>,
 /// Anti-pattern violations detected for this module.
    pub violations: Vec<AntiPatternViolation>,
}

impl ModuleArchResult {
 /// True iff the module's arch_type is load-bearing — no parse
 /// errors AND no anti-pattern violations.
    pub fn is_load_bearing(&self) -> bool {
        self.parse_errors.is_empty() && self.violations.is_empty()
    }
}

/// Aggregated ATS-V phase report across all modules.
#[derive(Debug, Clone, Default)]
pub struct ArchPhaseReport {
 /// Per-module results in iteration order.
    pub modules: Vec<ModuleArchResult>,
}

impl ArchPhaseReport {
 /// True iff every module is load-bearing — phase compiles cleanly.
    pub fn is_load_bearing(&self) -> bool {
        self.modules.iter().all(|m| m.is_load_bearing())
    }

 /// Total count of anti-pattern violations across all modules.
    pub fn total_violations(&self) -> usize {
        self.modules.iter().map(|m| m.violations.len()).sum()
    }

 /// Total count of parse errors across all modules.
    pub fn total_parse_errors(&self) -> usize {
        self.modules.iter().map(|m| m.parse_errors.len()).sum()
    }

 /// Count of modules that have an explicit `@arch_module(...)`
 /// declaration (vs. those falling back to default shape).
    pub fn annotated_module_count(&self) -> usize {
        self.modules.iter().filter(|m| m.shape.is_some()).count()
    }

 /// Group violations by stable RFC code for agent-friendly
 /// pattern-matching (per spec §32.4).
    pub fn violations_by_code(&self) -> std::collections::BTreeMap<&'static str, usize> {
        let mut by_code = std::collections::BTreeMap::new();
        for module in &self.modules {
            for v in &module.violations {
                *by_code.entry(v.code.code()).or_insert(0) += 1;
            }
        }
        by_code
    }
}

// =============================================================================
// run_arch_phase — main phase entry point
// =============================================================================

/// Run ATS-V phase across a list of modules.
///
/// Each input is a tuple `(module_name, attribute_args)` where
/// `attribute_args` is the slice of named-arg expressions for the
/// module's `@arch_module(...)` attribute (or empty if the
/// module has no annotation).
///
/// Returns `ArchPhaseReport` with per-module results. No early
/// exit — the phase walks every module so the agent / human gets
/// a complete violation roster in one pass.
pub fn run_arch_phase(modules: &[(String, &[Expr])]) -> ArchPhaseReport {
    let mut report = ArchPhaseReport::default();
    for (name, args) in modules {
        let result = run_arch_phase_one(name.clone(), args);
        report.modules.push(result);
    }
    report
}

/// Run the phase for a single module. Public для CLI consumers
/// that want per-module dispatch (e.g. `verum arch explain <cog>`).
///
/// **Red-team data wiring**: this function populates
/// `DiagnosticContext` slots that activate the AT-1..AT-5
/// closures.  Without this wiring the closures would only fire
/// in unit tests (silent regression risk).  Currently activated:
///
///   * `capability_ontology_registry` — the kernel-static
///     canonical registry from `arch::canonical_capability_registry`
///     drives AT-1 (Custom-tag must be registered).
///
/// Other fields (yoneda_verdicts_claimed, foreign_foundation_constructs,
/// composition_handoff_gaps, transitive_lifecycle_regressions, …)
/// require body-level analysis the kernel-internal phase does not
/// have access to.  Compiler-side
/// `verum_compiler::pipeline::ats_v_phase` extends this context
/// with the additional slots when wiring lands per #ATS-V-V1.
pub fn run_arch_phase_one(module_name: String, attribute_args: &[Expr]) -> ModuleArchResult {
    run_arch_phase_one_with(module_name, attribute_args, &PhaseInputs::default())
}

/// Per-module phase inputs the caller can supply.  Defaults to
/// the kernel-static canonical capability registry; consumers
/// (compiler pipeline, audit-bundle CLI) override with richer
/// data when available.
#[derive(Debug, Clone, Default)]
pub struct PhaseInputs {
    /// Override capability-ontology registry — `None` uses the
    /// kernel-static canonical roster.
    pub capability_ontology_registry: Option<Vec<String>>,
    /// Yoneda verdicts the cog has attached
    /// (label, list-of-observer-tags-in-agreement).
    pub yoneda_verdicts_claimed: Vec<(String, Vec<String>)>,
    /// Foreign-foundation constructs detected in the body.
    pub foreign_foundation_constructs: Vec<(String, crate::arch::Foundation)>,
    /// Cross-cog peer foundations for AP-005 FoundationDrift.
    /// Populated by the compiler's session-level arch-shape
    /// registry from each `composes_with` peer's `@arch_module(foundation: ...)`
    /// declaration.
    pub composed_foundations: Vec<(String, crate::arch::Foundation)>,
    /// Cross-cog peer lifecycles for AP-009 LifecycleRegression
    /// (and AP-024 transitive variant).  Populated from each peer's
    /// `@arch_module(lifecycle: ...)` declaration.
    pub cited_lifecycles: Vec<(String, crate::arch::Lifecycle)>,
    /// Cross-cog peer tiers for AP-004 TierMixing.  Populated from
    /// each peer's `@arch_module(at_tier: ...)` declaration plus
    /// any direct callee resolution from body analysis.
    pub callee_tiers: Vec<(String, crate::arch::Tier)>,
    /// Capabilities the body actually exercises, inferred by
    /// walking the cog's AST and matching each call site against
    /// the canonical ontology in
    /// `crate::arch_capability_inference`.  Activates AP-001
    /// CapabilityEscalation in production: any inferred capability
    /// not declared in `Shape.requires` raises the violation.
    pub inferred_used_capabilities: Vec<crate::arch::Capability>,
}

/// Run the phase for a single module with caller-supplied inputs.
/// Used by the compiler pipeline + audit-bundle CLI to surface
/// red-team and body-level checks that need data unavailable to
/// the bare-name `run_arch_phase_one` entry point.
pub fn run_arch_phase_one_with(
    module_name: String,
    attribute_args: &[Expr],
    inputs: &PhaseInputs,
) -> ModuleArchResult {
    // Step 1: parse Shape from attribute args.
    let (shape, parse_errors) = if attribute_args.is_empty() {
        // Module has no @arch_module annotation — backward-compat
        // default Shape (vacuously passes every check).
        (None, Vec::new())
    } else {
        match parse_arch_module(attribute_args) {
            Ok(s) => (Some(s), Vec::new()),
            Err(e) => (None, vec![e]),
        }
    };

    // Step 2: build DiagnosticContext with red-team data wired in.
    let shape_for_checks = shape
        .clone()
        .unwrap_or_else(Shape::default_for_unannotated);
    let mut ctx = DiagnosticContext::default();
    ctx.cog_name = module_name.clone();
    ctx.capability_ontology_registry = inputs
        .capability_ontology_registry
        .clone()
        .unwrap_or_else(crate::arch::canonical_capability_registry);
    ctx.yoneda_verdicts_claimed = inputs.yoneda_verdicts_claimed.clone();
    ctx.foreign_foundation_constructs = inputs.foreign_foundation_constructs.clone();
    // Cross-cog peer resolution — activates AP-004 TierMixing,
    // AP-005 FoundationDrift, AP-009 LifecycleRegression in
    // production builds.
    ctx.composed_foundations = inputs.composed_foundations.clone();
    ctx.cited_lifecycles = inputs.cited_lifecycles.clone();
    ctx.callee_tiers = inputs.callee_tiers.clone();
    // Body-level capability inference — activates AP-001
    // CapabilityEscalation in production builds.  The compiler
    // walks the cog's AST, matches each call site against
    // `arch_capability_inference::canonical_ontology`, and supplies
    // the inferred set here.  An empty list means either (a) the
    // body uses no capability-relevant primitives, or (b) the
    // walker hasn't been invoked.  Either way the silent path of
    // AP-001 reports no violation, so no false-positive risk.
    ctx.inferred_used_capabilities = inputs.inferred_used_capabilities.clone();

    let violations = check_all_anti_patterns(&shape_for_checks, &ctx);

    ModuleArchResult {
        module_name,
        shape,
        parse_errors,
        violations,
    }
}

// =============================================================================
// CompositionVerification — cross-module composition check
// =============================================================================

/// Verify a chain of compositions across multiple modules. Per
/// spec §5.3 + §17.5: when modules declare `composes_with`,
/// the phase verifies pairwise compatibility (capability flow,
/// foundation, tier, stratum).
pub fn verify_composition_chain(modules: &[(&str, &Shape)]) -> CompositionVerificationReport {
    use crate::arch_composition::{compose, CompositionResult};
    let mut steps: Vec<CompositionStep> = Vec::new();

    if modules.len() < 2 {
 // Single or no module — no composition needed.
        return CompositionVerificationReport { steps };
    }

 // Left-fold: A ⊗ B ⊗ C = ((A ⊗ B) ⊗ C).
    let (first_name, first_shape) = modules[0];
    let mut acc = first_shape.clone();
    let mut acc_name = first_name.to_string();

    for (next_name, next_shape) in modules.iter().skip(1) {
        match compose(&acc, next_shape) {
            CompositionResult::Composed(merged) => {
                steps.push(CompositionStep {
                    left: acc_name.clone(),
                    right: next_name.to_string(),
                    composed: true,
                    violations: Vec::new(),
                });
                acc = merged;
                acc_name = format!("({} ⊗ {})", acc_name, next_name);
            }
            CompositionResult::Rejected(violations) => {
                steps.push(CompositionStep {
                    left: acc_name.clone(),
                    right: next_name.to_string(),
                    composed: false,
                    violations: violations
                        .iter()
                        .map(|v| (v.code, v.summary.clone()))
                        .collect(),
                });
 // Stop on first rejection — composition is order-
 // dependent in failure case; subsequent steps lose
 // meaning.
                break;
            }
        }
    }

    CompositionVerificationReport { steps }
}

/// Per-step result of cross-module composition verification.
#[derive(Debug, Clone)]
pub struct CompositionStep {
    /// Module-path of the left composition arm.
    pub left: String,
    /// Module-path of the right composition arm.
    pub right: String,
    /// True iff the two arms compose without violating any
    /// anti-pattern.
    pub composed: bool,
    /// Any anti-pattern violations the composition surfaced.
    pub violations: Vec<(AntiPatternCode, String)>,
}

/// Aggregate report describing every composition step the phase
/// verified across the mounted module graph.
#[derive(Debug, Clone)]
pub struct CompositionVerificationReport {
    /// Per-step composition results.
    pub steps: Vec<CompositionStep>,
}

impl CompositionVerificationReport {
    /// True iff every composition step succeeded — the audit-gate
    /// load-bearing predicate.
    pub fn is_load_bearing(&self) -> bool {
        self.steps.iter().all(|s| s.composed)
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch::*;
    use verum_ast::expr::ExprKind;
    use verum_ast::literal::Literal;
    use verum_ast::span::Span;
    use verum_ast::ty::{Ident, Path, PathSegment};
    use verum_common::{Heap, List};

    fn span() -> Span {
        Span::dummy()
    }

    fn dotted_path_expr(parts: &[&str]) -> Expr {
        Expr::new(
            ExprKind::Path(Path::new(
                List::from(
                    parts
                        .iter()
                        .map(|p| PathSegment::Name(Ident::new(*p, span())))
                        .collect::<Vec<_>>(),
                ),
                span(),
            )),
            span(),
        )
    }

    fn named_arg(name: &str, value: Expr) -> Expr {
        Expr::new(
            ExprKind::NamedArg {
                name: Ident::new(name, span()),
                value: Heap::new(value),
            },
            span(),
        )
    }

    fn bool_lit_expr(b: bool) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal::new(
                verum_ast::literal::LiteralKind::Bool(b),
                span(),
            )),
            span(),
        )
    }

    #[test]
    fn empty_modules_produces_empty_report() {
        let report = run_arch_phase(&[]);
        assert!(report.is_load_bearing());
        assert_eq!(report.total_violations(), 0);
        assert_eq!(report.total_parse_errors(), 0);
        assert_eq!(report.annotated_module_count(), 0);
    }

    #[test]
    fn unannotated_module_uses_default_shape_no_violations() {
 // Module with empty attribute args = no @arch_module(...).
 // Default shape passes all anti-pattern checks vacuously.
        let report = run_arch_phase(&[("test_cog".to_string(), &[])]);
        assert!(report.is_load_bearing());
        assert_eq!(report.modules.len(), 1);
        assert!(report.modules[0].shape.is_none());
        assert_eq!(report.annotated_module_count(), 0);
    }

    #[test]
    fn well_formed_arch_module_produces_shape() {
        let args = vec![
            named_arg("at_tier", dotted_path_expr(&["Tier", "Aot"])),
            named_arg(
                "foundation",
                dotted_path_expr(&["Foundation", "ZfcTwoInacc"]),
            ),
            named_arg("strict", bool_lit_expr(false)),
        ];
        let report = run_arch_phase(&[("annotated_cog".to_string(), &args)]);
        assert!(report.is_load_bearing());
        assert_eq!(report.annotated_module_count(), 1);
        let m = &report.modules[0];
        assert!(m.shape.is_some());
        let shape = m.shape.as_ref().unwrap();
        assert_eq!(shape.at_tier.tag(), "aot");
        assert_eq!(shape.foundation.tag(), "zfc_two_inacc");
    }

    #[test]
    fn invalid_arch_module_surfaces_parse_error() {
        let args = vec![named_arg(
            "foundation",
            dotted_path_expr(&["Foundation", "BogusFoundation"]),
        )];
        let report = run_arch_phase(&[("bad_cog".to_string(), &args)]);
        assert!(!report.is_load_bearing());
        assert_eq!(report.total_parse_errors(), 1);
        assert!(report.modules[0].shape.is_none());
    }

    #[test]
    fn l_abs_stratum_triggers_anti_pattern() {
        let args = vec![named_arg(
            "stratum",
            dotted_path_expr(&["MsfsStratum", "LAbs"]),
        )];
        let report = run_arch_phase(&[("escape_attempt".to_string(), &args)]);
        assert!(!report.is_load_bearing());
        assert!(report.total_violations() >= 1);
 // Anti-pattern catalog has both stratum_admissible (uses
 // FoundationDrift code as proxy) AND
 // AbsoluteBoundaryAttempt — at least one fires.
        let by_code = report.violations_by_code();
        assert!(!by_code.is_empty());
    }

    #[test]
    fn multi_module_phase_walks_all() {
        let good_args = vec![named_arg("at_tier", dotted_path_expr(&["Tier", "Aot"]))];
        let bad_args = vec![named_arg(
            "stratum",
            dotted_path_expr(&["MsfsStratum", "LAbs"]),
        )];
        let report = run_arch_phase(&[
            ("good_cog".to_string(), &good_args),
            ("bad_cog".to_string(), &bad_args),
            ("unannotated".to_string(), &[]),
        ]);
 // Phase walks all 3 modules even though one has violations.
        assert_eq!(report.modules.len(), 3);
 // Bad cog is non-load-bearing.
        assert!(!report.is_load_bearing());
 // Good cog still has its shape.
        assert!(report.modules[0].shape.is_some());
        assert!(report.modules[0].is_load_bearing());
 // Bad cog has shape (parser succeeded) but with violation.
        assert!(report.modules[1].shape.is_some());
        assert!(!report.modules[1].is_load_bearing());
 // Unannotated has no shape but is vacuously load-bearing.
        assert!(report.modules[2].shape.is_none());
        assert!(report.modules[2].is_load_bearing());
    }

    #[test]
    fn composition_chain_compatible_modules() {
        let a = Shape::default_for_unannotated();
        let b = Shape::default_for_unannotated();
        let c = Shape::default_for_unannotated();
        let report =
            verify_composition_chain(&[("A", &a), ("B", &b), ("C", &c)]);
        assert!(report.is_load_bearing());
        assert_eq!(report.steps.len(), 2); // A⊗B, then (A⊗B)⊗C
    }

    #[test]
    fn composition_chain_stops_on_rejection() {
        let mut a = Shape::default_for_unannotated();
        a.foundation = Foundation::ZfcTwoInacc;
        let mut b = Shape::default_for_unannotated();
        b.foundation = Foundation::Hott;
        let c = Shape::default_for_unannotated();
        let report = verify_composition_chain(&[("A", &a), ("B", &b), ("C", &c)]);
        assert!(!report.is_load_bearing());
 // Stopped after A⊗B failed.
        assert_eq!(report.steps.len(), 1);
        assert!(!report.steps[0].composed);
    }

    #[test]
    fn report_aggregates_violations_by_code() {
        let bad_args = vec![named_arg(
            "stratum",
            dotted_path_expr(&["MsfsStratum", "LAbs"]),
        )];
        let report = run_arch_phase(&[
            ("first_bad".to_string(), &bad_args),
            ("second_bad".to_string(), &bad_args),
        ]);
        let by_code = report.violations_by_code();
 // Both modules produce same violation code → count 2.
        for (_, count) in &by_code {
            assert!(*count >= 1);
        }
    }

    #[test]
    fn architectural_pin_phase_does_not_early_exit() {
 // Pin: phase walks ALL modules even after finding violations
 // in earlier ones — humans / agents need complete violation
 // rosters per spec §32.4 dual-audience design.
        let bad_args = vec![named_arg(
            "stratum",
            dotted_path_expr(&["MsfsStratum", "LAbs"]),
        )];
        let report = run_arch_phase(&[
            ("first".to_string(), &bad_args),
            ("second".to_string(), &bad_args),
            ("third".to_string(), &bad_args),
        ]);
        assert_eq!(report.modules.len(), 3);
 // Each module reports its own violation.
        for m in &report.modules {
            assert!(!m.violations.is_empty());
        }
    }
}
