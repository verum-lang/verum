//! Level-inference verification pass.
//!
//! Walks every function declaration in the module and assigns the
//! current verification level. Lightweight: no SMT, no kernel
//! invocation. Records per-function `VerificationCost` entries.

use std::time::Instant;

use verum_ast::{Module, decl::ItemKind};
use verum_common::{List, Text};

use crate::context::VerificationContext;
use crate::cost::VerificationCost;
use crate::level::{VerificationLevel, VerificationMode};

use super::{VerificationError, VerificationPass, VerificationResult};

/// Level inference pass.
#[derive(Debug)]
pub struct LevelInferencePass {
    default_level: VerificationLevel,
}

impl LevelInferencePass {
    /// Create a new level inference pass.
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
