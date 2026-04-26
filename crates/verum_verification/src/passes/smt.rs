//! SMT-based verification pass.
//!
//! Generates verification conditions for each function and
//! discharges them through the Z3 SMT backend. Includes a kernel-
//! recheck preamble (#186) that runs the K-rules before SMT — a
//! K-rule rejection short-circuits the SMT round because no SMT
//! proof can recover from a kernel formation error.

use std::time::Instant;

use verum_ast::{FunctionDecl, Module};
use verum_common::{List, Maybe, Text};
use verum_smt::context::Context as SmtContext;

use crate::context::VerificationContext;
use crate::integration::HoareZ3Verifier;
use crate::kernel_recheck::KernelRecheck;
use crate::level::VerificationLevel;
use crate::vcgen::VCGenerator;

use super::{VerificationError, VerificationPass, VerificationResult};

/// SMT-based verification pass that uses Z3 to verify generated VCs.
///
/// 1. Generates verification conditions for each function.
/// 2. Sends VCs to Z3 for automated theorem proving.
/// 3. Collects results including counterexamples for failures.
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

/// Result of SMT verification for a single function.
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

/// Result of verifying a single VC.
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

/// Status of a verification condition.
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

/// Statistics for SMT verification.
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
    /// Get success rate (proven / (proven + failed)).
    pub fn success_rate(&self) -> f64 {
        let attempted = self.proven + self.failed;
        if attempted == 0 {
            1.0
        } else {
            self.proven as f64 / attempted as f64
        }
    }

    /// Get completion rate (non-unknown / total).
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
    /// Create a new SMT verification pass with default settings.
    pub fn new() -> Self {
        Self {
            timeout_ms: 30000, // 30 second default timeout
            generate_proofs: false,
            results: List::new(),
            stats: SmtVerificationStats::default(),
        }
    }

    /// Set verification timeout in milliseconds.
    pub fn with_timeout(mut self, timeout_ms: u32) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    /// Enable proof generation.
    pub fn with_proofs(mut self) -> Self {
        self.generate_proofs = true;
        self
    }

    /// Get verification results.
    pub fn results(&self) -> &List<SmtVerificationResult> {
        &self.results
    }

    /// Get verification statistics.
    pub fn stats(&self) -> &SmtVerificationStats {
        &self.stats
    }

    /// Verify a single function.
    fn verify_function(
        &self,
        func: &FunctionDecl,
        ctx: &VerificationContext,
        smt_context: &SmtContext,
    ) -> SmtVerificationResult {
        let start = Instant::now();
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

        // V3 (#186) — kernel-recheck preamble. Walk every
        // refinement type appearing in the function's parameters /
        // return type and run K-Refine-omega before SMT. A failed
        // K-rule surfaces as an Invalid VC with the kernel
        // diagnostic in the description; this is strictly *cheaper*
        // than SMT (linear in term size, no solver call) and a
        // K-rule failure is a hard formation error that no SMT
        // proof can recover, so it's correct to short-circuit
        // before VC generation.
        let kernel_outcomes = KernelRecheck::recheck_function(func);
        let mut preamble_results: List<VCVerificationResult> = List::new();
        let mut preamble_failures = 0usize;
        for (label, outcome) in kernel_outcomes.iter() {
            match outcome {
                Ok(()) => {
                    preamble_results.push(VCVerificationResult {
                        description: label.clone(),
                        status: VCStatus::Valid,
                        counterexample: None,
                        time_ms: 0,
                    });
                }
                Err(err) => {
                    preamble_failures += 1;
                    preamble_results.push(VCVerificationResult {
                        description: label.clone(),
                        status: VCStatus::Invalid,
                        counterexample: Some(Text::from(format!("{}", err))),
                        time_ms: 0,
                    });
                }
            }
        }
        if preamble_failures > 0 {
            // K-rule rejection is a hard error — skip SMT entirely.
            return SmtVerificationResult {
                function_name: func_name,
                vc_count: preamble_results.len(),
                proven_count: 0,
                failed_count: preamble_failures,
                unknown_count: 0,
                vc_results: preamble_results,
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
            let vc_start = Instant::now();

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
        let start = Instant::now();

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
