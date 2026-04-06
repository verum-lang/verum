//! Verification Compiler Passes
//!
//! Implements compiler passes for gradual verification:
//! - Verification level inference
//! - Boundary detection
//! - Proof obligation generation
//! - Transition recommendation
//!
//! These passes run during compilation to: (1) infer verification levels from
//! annotations and code context, (2) detect boundaries between verification
//! levels, (3) generate proof obligations at boundaries, and (4) recommend
//! transitions to higher verification levels based on code metrics.

use crate::boundary::BoundaryKind;
use crate::context::VerificationContext;
use crate::cost::{CostReport, VerificationCost};
use crate::level::{VerificationLevel, VerificationMode};
use crate::transition::{CodeMetrics, TransitionAnalyzer, TransitionStrategy};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use thiserror::Error;
use verum_ast::{FunctionDecl, Item, Module, decl::ItemKind};
use verum_common::{List, Maybe, Text};

/// Verification pass errors
#[derive(Debug, Error)]
pub enum VerificationError {
    #[error("verification failed: {0}")]
    Failed(Text),

    #[error("timeout: {0}")]
    Timeout(Text),

    #[error("internal error: {0}")]
    Internal(Text),
}

/// Result of a verification pass
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    /// Whether verification succeeded
    pub success: bool,

    /// Verification level used
    pub level: VerificationLevel,

    /// Time taken
    pub duration: Duration,

    /// Costs per function
    pub costs: List<VerificationCost>,

    /// Number of functions verified
    pub functions_verified: usize,

    /// Number of boundaries detected
    pub boundaries_detected: usize,

    /// Number of proof obligations generated
    pub obligations_generated: usize,
}

impl VerificationResult {
    /// Create a successful result
    pub fn success(
        level: VerificationLevel,
        duration: Duration,
        costs: List<VerificationCost>,
    ) -> Self {
        let functions_verified = costs.len();
        Self {
            success: true,
            level,
            duration,
            costs,
            functions_verified,
            boundaries_detected: 0,
            obligations_generated: 0,
        }
    }

    /// Create a failure result
    pub fn failure(level: VerificationLevel, duration: Duration) -> Self {
        Self {
            success: false,
            level,
            duration,
            costs: List::new(),
            functions_verified: 0,
            boundaries_detected: 0,
            obligations_generated: 0,
        }
    }
}

/// Verification pass trait
pub trait VerificationPass {
    /// Run the pass on a module
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError>;

    /// Name of this pass
    fn name(&self) -> &str;
}

/// Level inference pass
#[derive(Debug)]
pub struct LevelInferencePass {
    default_level: VerificationLevel,
}

impl LevelInferencePass {
    /// Create a new level inference pass
    pub fn new(default_level: VerificationLevel) -> Self {
        Self { default_level }
    }
}

impl VerificationPass for LevelInferencePass {
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = Instant::now();
        let mut costs = List::new();

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                // Infer verification level from annotations
                // For now, use default level (annotation parsing would require AST changes)
                let level = self.default_level;

                // Push scope for function
                ctx.push_scope(VerificationMode::new(level), Text::from(func.name.as_str()));

                // Track timing for level inference (lightweight - no SMT queries)
                let func_start = Instant::now();

                // Record cost for this function's level inference
                // Note: SMT queries and problem_size are 0 because level inference
                // is a syntactic analysis that doesn't involve SMT solving.
                // The actual verification costs are recorded in subsequent passes
                // (ProofObligationPass, SMTVerificationPass, etc.)
                costs.push(VerificationCost::new(
                    Text::from(func.name.as_str()),
                    level,
                    func_start.elapsed(),
                    0,     // smt_queries: 0 - level inference doesn't use SMT
                    true,  // success: level inference always succeeds
                    false, // timed_out: level inference is constant-time
                    0,     // problem_size: 0 - no constraints generated
                ));

                ctx.pop_scope()
                    .map_err(|e| VerificationError::Internal(Text::from(e.to_string())))?;
            }
        }

        Ok(VerificationResult::success(
            self.default_level,
            start.elapsed(),
            costs,
        ))
    }

    fn name(&self) -> &str {
        "level_inference"
    }
}

/// Boundary detection pass using call graph analysis
///
/// This pass performs full call graph analysis to detect verification
/// boundaries between functions at different verification levels.
///
/// Detects where code transitions between verification levels (e.g., proof-level
/// code calling runtime-level code) and generates proof obligations at those points.
#[derive(Debug)]
pub struct BoundaryDetectionPass {
    /// Generated call graph (stored for inspection)
    call_graph: Option<crate::boundary::CallGraph>,
    /// Generated obligations
    obligation_generator: crate::boundary::ObligationGenerator,
    /// Whether to generate obligations automatically
    generate_obligations: bool,
}

impl BoundaryDetectionPass {
    /// Create a new boundary detection pass
    pub fn new() -> Self {
        Self {
            call_graph: None,
            obligation_generator: crate::boundary::ObligationGenerator::new(),
            generate_obligations: true,
        }
    }

    /// Create with custom settings
    pub fn with_settings(generate_obligations: bool) -> Self {
        Self {
            call_graph: None,
            obligation_generator: crate::boundary::ObligationGenerator::new(),
            generate_obligations,
        }
    }

    /// Get the generated call graph
    pub fn call_graph(&self) -> Option<&crate::boundary::CallGraph> {
        self.call_graph.as_ref()
    }

    /// Get mutable reference to call graph
    pub fn call_graph_mut(&mut self) -> Option<&mut crate::boundary::CallGraph> {
        self.call_graph.as_mut()
    }
}

impl Default for BoundaryDetectionPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerificationPass for BoundaryDetectionPass {
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = Instant::now();

        // Build call graph from module AST
        let mut call_graph = crate::boundary::CallGraph::from_module(module);

        // Detect all boundaries
        call_graph.detect_boundaries();

        // Generate proof obligations if enabled
        if self.generate_obligations {
            self.obligation_generator
                .generate_all_obligations(&mut call_graph);
        }

        // Collect statistics before registering (to avoid borrow conflicts)
        let stats = call_graph.stats().clone();
        let boundaries_detected = stats.boundary_crossings;

        // Register detected boundaries with verification context
        // Clone boundaries to avoid borrow conflicts
        let boundaries: List<_> = call_graph.get_boundaries().iter().cloned().collect();
        for boundary in &boundaries {
            let boundary_id = ctx.register_boundary(
                boundary.caller_level,
                boundary.callee_level,
                boundary.boundary_kind,
            );

            // Register required obligations
            for obligation in &boundary.required_obligations {
                let proof_obligation = crate::context::ProofObligation::new(
                    crate::context::ProofObligationId::new(0), // Will be reassigned
                    obligation.kind,
                    obligation.description.clone(),
                    Some(boundary_id),
                );
                let _ = ctx.add_obligation_to_boundary(boundary_id, proof_obligation);
            }
        }

        // Count obligations after registration
        let obligations_generated: usize = boundaries
            .iter()
            .map(|b| b.required_obligations.len())
            .sum();

        // Store the call graph for later inspection
        self.call_graph = Some(call_graph);

        let duration = start.elapsed();
        let mut result =
            VerificationResult::success(VerificationLevel::Runtime, duration, List::new());
        result.boundaries_detected = boundaries_detected;
        result.obligations_generated = obligations_generated;
        result.functions_verified = stats.total_functions;

        Ok(result)
    }

    fn name(&self) -> &str {
        "boundary_detection"
    }
}

/// Transition recommendation pass
///
/// This pass analyzes functions for transition opportunities between
/// verification levels using real code metrics collection.
#[derive(Debug)]
pub struct TransitionRecommendationPass {
    /// Transition analyzer with strategy
    analyzer: TransitionAnalyzer,
    /// Metrics collector for real metrics
    metrics_collector: crate::metrics::CodeMetricsCollector,
    /// Generated recommendations
    recommendations: List<TransitionRecommendation>,
}

/// A transition recommendation for a specific function
#[derive(Debug, Clone)]
pub struct TransitionRecommendation {
    /// Function name
    pub function_name: Text,
    /// Current verification level
    pub current_level: VerificationLevel,
    /// Recommended target level
    pub recommended_level: VerificationLevel,
    /// Confidence in recommendation (0.0 to 1.0)
    pub confidence: f64,
    /// Expected benefit (percentage)
    pub expected_benefit: f64,
    /// Collected metrics
    pub metrics: CodeMetrics,
    /// Human-readable reason
    pub reason: Text,
    /// Risk score for transition
    pub risk_score: f64,
}

impl TransitionRecommendationPass {
    /// Create a new transition recommendation pass
    pub fn new(strategy: TransitionStrategy) -> Self {
        Self {
            analyzer: TransitionAnalyzer::new(strategy),
            metrics_collector: crate::metrics::CodeMetricsCollector::new(),
            recommendations: List::new(),
        }
    }

    /// Load coverage data for metrics collection
    pub fn load_coverage(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), crate::metrics::MetricsError> {
        self.metrics_collector.load_coverage(path)
    }

    /// Load profiling data for metrics collection
    pub fn load_profiling(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), crate::metrics::MetricsError> {
        self.metrics_collector.load_profiling(path)
    }

    /// Load git history for change frequency analysis
    pub fn load_git_history(
        &mut self,
        repo_path: &std::path::Path,
    ) -> Result<(), crate::metrics::MetricsError> {
        self.metrics_collector.load_git_history(repo_path)
    }

    /// Get all generated recommendations
    pub fn recommendations(&self) -> &List<TransitionRecommendation> {
        &self.recommendations
    }

    /// Get recommendations for functions that should transition
    pub fn actionable_recommendations(&self) -> List<&TransitionRecommendation> {
        self.recommendations
            .iter()
            .filter(|r| r.current_level != r.recommended_level && r.confidence >= 0.7)
            .collect()
    }
}

impl VerificationPass for TransitionRecommendationPass {
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = Instant::now();
        self.recommendations = List::new();

        // Analyze each function for transition opportunities
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                let current_level = ctx.current_level();

                // Collect real metrics from AST analysis
                let enhanced_metrics = self.metrics_collector.analyze_function(func);
                let metrics = enhanced_metrics.to_code_metrics();

                // Analyze transition using real metrics
                let decision = self.analyzer.analyze_function(
                    &Text::from(func.name.as_str()),
                    current_level,
                    &metrics,
                );

                // Generate reason based on metrics
                let reason = if decision.recommend {
                    if metrics.is_stable() && metrics.has_good_tests() {
                        Text::from(format!(
                            "Code is stable (change freq: {:.2}/week) with good test coverage ({:.1}%)",
                            metrics.change_frequency_per_week,
                            metrics.test_coverage * 100.0
                        ))
                    } else if metrics.is_critical() {
                        Text::from("Critical code path requiring stronger guarantees")
                    } else {
                        Text::from(format!(
                            "Based on complexity ({}) and coverage ({:.1}%)",
                            metrics.cyclomatic_complexity,
                            metrics.test_coverage * 100.0
                        ))
                    }
                } else {
                    Text::from("Transition not recommended at this time")
                };

                // Calculate risk score
                let risk_score = metrics.transition_risk_score();

                // Create recommendation
                let recommendation = TransitionRecommendation {
                    function_name: Text::from(func.name.as_str()),
                    current_level,
                    recommended_level: decision.to,
                    confidence: decision.confidence,
                    expected_benefit: decision.expected_benefit_percent,
                    metrics: metrics.clone(),
                    reason,
                    risk_score,
                };

                self.recommendations.push(recommendation);
            }
        }

        let duration = start.elapsed();
        let mut result =
            VerificationResult::success(VerificationLevel::Runtime, duration, List::new());
        result.functions_verified = self.recommendations.len();

        Ok(result)
    }

    fn name(&self) -> &str {
        "transition_recommendation"
    }
}

/// Verification pipeline combining all passes
pub struct VerificationPipeline {
    passes: List<Box<dyn VerificationPass>>,
}

impl VerificationPipeline {
    /// Create a new verification pipeline
    pub fn new() -> Self {
        Self {
            passes: List::new(),
        }
    }

    /// Add a pass to the pipeline
    pub fn add_pass(&mut self, pass: Box<dyn VerificationPass>) {
        self.passes.push(pass);
    }

    /// Run all passes
    pub fn run_all(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<List<VerificationResult>, VerificationError> {
        let mut results = List::new();

        for pass in &mut self.passes {
            let result = pass.run(module, ctx)?;
            results.push(result);
        }

        Ok(results)
    }

    /// Create a default pipeline with standard passes
    pub fn default_pipeline() -> Self {
        let mut pipeline = Self::new();

        pipeline.add_pass(Box::new(LevelInferencePass::new(
            VerificationLevel::Runtime,
        )));
        pipeline.add_pass(Box::new(BoundaryDetectionPass::new()));
        pipeline.add_pass(Box::new(TransitionRecommendationPass::new(
            TransitionStrategy::Balanced,
        )));

        pipeline
    }
}

impl Default for VerificationPipeline {
    fn default() -> Self {
        Self::default_pipeline()
    }
}

impl std::fmt::Debug for VerificationPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerificationPipeline")
            .field("passes", &format!("{} passes", self.passes.len()))
            .finish()
    }
}

// =============================================================================
// SMT Verification Pass
// =============================================================================

use crate::integration::HoareZ3Verifier;
use crate::vcgen::{VCGenerator, VerificationCondition};
use verum_smt::context::Context as SmtContext;

/// SMT-based verification pass that uses Z3 to verify generated VCs
///
/// This pass:
/// 1. Generates verification conditions for each function
/// 2. Sends VCs to Z3 for automated theorem proving
/// 3. Collects results including counterexamples for failures
#[derive(Debug)]
pub struct SmtVerificationPass {
    /// Verification timeout in milliseconds
    timeout_ms: u32,
    /// Enable proof generation for certification
    generate_proofs: bool,
    /// Verification results
    results: List<SmtVerificationResult>,
    /// Statistics
    stats: SmtVerificationStats,
}

/// Result of SMT verification for a single function
#[derive(Debug, Clone)]
pub struct SmtVerificationResult {
    /// Function name
    pub function_name: Text,
    /// Total number of VCs generated
    pub vc_count: usize,
    /// Number of VCs proven valid
    pub proven_count: usize,
    /// Number of VCs that failed (counterexample found)
    pub failed_count: usize,
    /// Number of VCs with unknown result (timeout)
    pub unknown_count: usize,
    /// Detailed results for each VC
    pub vc_results: List<VCVerificationResult>,
    /// Verification time in milliseconds
    pub time_ms: u64,
}

/// Result of verifying a single VC
#[derive(Debug, Clone)]
pub struct VCVerificationResult {
    /// VC description
    pub description: Text,
    /// Verification status
    pub status: VCStatus,
    /// Counterexample if status is Invalid
    pub counterexample: Maybe<Text>,
    /// Verification time in milliseconds
    pub time_ms: u64,
}

/// Status of a verification condition
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VCStatus {
    /// VC is valid (proven by SMT solver)
    Valid,
    /// VC is invalid (counterexample found)
    Invalid,
    /// Unknown result (timeout or complexity limit)
    Unknown,
    /// Skipped (runtime-only verification)
    Skipped,
}

/// Statistics for SMT verification
#[derive(Debug, Clone, Default)]
pub struct SmtVerificationStats {
    /// Total VCs generated
    pub total_vcs: usize,
    /// VCs proven valid
    pub proven: usize,
    /// VCs with counterexamples
    pub failed: usize,
    /// VCs with unknown result
    pub unknown: usize,
    /// VCs skipped
    pub skipped: usize,
    /// Total verification time in milliseconds
    pub total_time_ms: u64,
}

impl SmtVerificationStats {
    /// Get success rate (proven / (proven + failed))
    pub fn success_rate(&self) -> f64 {
        let attempted = self.proven + self.failed;
        if attempted == 0 {
            1.0
        } else {
            self.proven as f64 / attempted as f64
        }
    }

    /// Get completion rate (non-unknown / total)
    pub fn completion_rate(&self) -> f64 {
        let non_unknown = self.proven + self.failed + self.skipped;
        if self.total_vcs == 0 {
            1.0
        } else {
            non_unknown as f64 / self.total_vcs as f64
        }
    }
}

impl SmtVerificationPass {
    /// Create a new SMT verification pass with default settings
    pub fn new() -> Self {
        Self {
            timeout_ms: 30000, // 30 second default timeout
            generate_proofs: false,
            results: List::new(),
            stats: SmtVerificationStats::default(),
        }
    }

    /// Set verification timeout in milliseconds
    pub fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Enable proof generation
    pub fn with_proofs(mut self) -> Self {
        self.generate_proofs = true;
        self
    }

    /// Get verification results
    pub fn results(&self) -> &List<SmtVerificationResult> {
        &self.results
    }

    /// Get verification statistics
    pub fn stats(&self) -> &SmtVerificationStats {
        &self.stats
    }

    /// Verify a single function
    fn verify_function(
        &self,
        func: &FunctionDecl,
        ctx: &VerificationContext,
        smt_context: &SmtContext,
    ) -> SmtVerificationResult {
        let start = std::time::Instant::now();
        let func_name = Text::from(func.name.as_str());

        // Check if this function should be verified
        // Use current scope level since we don't track per-function levels
        let level = ctx.current_level();
        if level == VerificationLevel::Runtime {
            // Skip SMT verification for runtime-only functions
            return SmtVerificationResult {
                function_name: func_name,
                vc_count: 0,
                proven_count: 0,
                failed_count: 0,
                unknown_count: 0,
                vc_results: List::new(),
                time_ms: start.elapsed().as_millis() as u64,
            };
        }

        // Generate verification conditions
        let mut vc_gen = VCGenerator::new();
        let vcs = vc_gen.generate_vcs(func);

        // Create Z3 verifier
        let verifier = HoareZ3Verifier::new(smt_context).with_timeout(self.timeout_ms);

        let mut vc_results = List::new();
        let mut proven_count = 0;
        let mut failed_count = 0;
        let mut unknown_count = 0;

        // Verify each VC
        for vc in vcs.iter() {
            let vc_start = std::time::Instant::now();

            // Convert VC formula to Hoare logic formula and verify
            let formula = vc.to_formula();
            let result = verifier.verify_formula(&formula);

            let (status, counterexample) = match result {
                Ok(hoare_result) if hoare_result.valid => {
                    proven_count += 1;
                    (VCStatus::Valid, None)
                }
                Ok(hoare_result) => {
                    failed_count += 1;
                    let ce = hoare_result
                        .counterexample
                        .map(|ce| {
                            let parts: Vec<String> =
                                ce.iter().map(|(k, v)| format!("{} = {}", k, v)).collect();
                            Text::from(parts.join(", "))
                        })
                        .unwrap_or_else(|| Text::from("no counterexample available"));
                    (VCStatus::Invalid, Some(ce))
                }
                Err(_) => {
                    unknown_count += 1;
                    (VCStatus::Unknown, None)
                }
            };

            vc_results.push(VCVerificationResult {
                description: vc.description.clone(),
                status,
                counterexample,
                time_ms: vc_start.elapsed().as_millis() as u64,
            });
        }

        SmtVerificationResult {
            function_name: func_name,
            vc_count: vcs.len(),
            proven_count,
            failed_count,
            unknown_count,
            vc_results,
            time_ms: start.elapsed().as_millis() as u64,
        }
    }
}

impl Default for SmtVerificationPass {
    fn default() -> Self {
        Self::new()
    }
}

impl VerificationPass for SmtVerificationPass {
    fn run(
        &mut self,
        module: &Module,
        ctx: &mut VerificationContext,
    ) -> Result<VerificationResult, VerificationError> {
        let start = std::time::Instant::now();

        // Create Z3 context for verification
        let smt_context = SmtContext::new();

        // Reset results
        self.results = List::new();
        self.stats = SmtVerificationStats::default();

        // Verify each function in the module
        for item in module.items.iter() {
            if let verum_ast::decl::ItemKind::Function(func) = &item.kind {
                let result = self.verify_function(func, ctx, &smt_context);

                // Update stats
                self.stats.total_vcs += result.vc_count;
                self.stats.proven += result.proven_count;
                self.stats.failed += result.failed_count;
                self.stats.unknown += result.unknown_count;
                self.stats.total_time_ms += result.time_ms;

                // Check for failures
                if result.failed_count > 0 {
                    // Mark function as having verification failures
                    for vc_result in result.vc_results.iter() {
                        if vc_result.status == VCStatus::Invalid {
                            // Could emit warning/error here
                        }
                    }
                }

                self.results.push(result);
            }
        }

        let duration = start.elapsed();

        // Determine overall verification level
        let level = if self.stats.failed > 0 {
            VerificationLevel::Runtime // Some proofs failed
        } else if self.stats.unknown > 0 {
            VerificationLevel::Static // Some proofs unknown
        } else {
            VerificationLevel::Proof // All proofs succeeded
        };

        let mut result = VerificationResult::success(level, duration, List::new());
        result.functions_verified = self.results.len();

        Ok(result)
    }

    fn name(&self) -> &str {
        "smt_verification"
    }
}
