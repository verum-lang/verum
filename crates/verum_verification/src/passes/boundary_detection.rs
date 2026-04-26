//! Boundary-detection verification pass.
//!
//! Performs full call-graph analysis to detect verification
//! boundaries between functions at different verification levels.
//! Generates proof obligations at boundary crossings (proof-level
//! code calling runtime-level code, etc.) and registers them with
//! the verification context.

use std::time::Instant;

use verum_ast::Module;
use verum_common::List;

use crate::context::VerificationContext;
use crate::level::VerificationLevel;

use super::{VerificationError, VerificationPass, VerificationResult};

/// Boundary detection pass using call graph analysis.
///
/// Detects where code transitions between verification levels
/// (e.g., proof-level code calling runtime-level code) and
/// generates proof obligations at those points.
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
    /// Create a new boundary detection pass.
    pub fn new() -> Self {
        Self {
            call_graph: None,
            obligation_generator: crate::boundary::ObligationGenerator::new(),
            generate_obligations: true,
        }
    }

    /// Create with custom settings.
    pub fn with_settings(generate_obligations: bool) -> Self {
        Self {
            call_graph: None,
            obligation_generator: crate::boundary::ObligationGenerator::new(),
            generate_obligations,
        }
    }

    /// Get the generated call graph.
    pub fn call_graph(&self) -> Option<&crate::boundary::CallGraph> {
        self.call_graph.as_ref()
    }

    /// Get mutable reference to the call graph.
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

    /// V8 (#208, B7) — boundary detection is advisory: a "failure"
    /// here means the boundary annotations are inconsistent, not
    /// that the program is unsound. Mark as Informational so the
    /// pipeline doesn't short-circuit subsequent passes when the
    /// halt policy is `PipelineMode::Default`.
    fn classification(&self) -> super::PassClassification {
        super::PassClassification::Informational
    }
}
