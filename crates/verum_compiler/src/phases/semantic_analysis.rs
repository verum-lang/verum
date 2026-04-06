//! Phase 4: Semantic Analysis
//!
//! Bidirectional type checking with refinement types.
//!
//! This phase supports two modes:
//! 1. **StdlibBootstrap**: Compiling stdlib itself (uses minimal context with builtins only)
//! 2. **NormalBuild**: Compiling user code (loads stdlib types from embedded stdlib.vbca)
//!
//! Phase 4: Semantic analysis. Name resolution with profile awareness,
//! bidirectional type checking, refinement subsumption (syntactic + SMT),
//! reference validation (&mut exclusive, & shared), context resolution.
//! Supports both normal compilation (pre-loaded stdlib) and bootstrap mode.

use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_common::List;
use verum_types::core_metadata::CoreMetadata;
use verum_types::TypeChecker;

use super::{
    CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput, VerifiedContractRegistry,
};

pub struct SemanticAnalysisPhase {
    /// Verified contracts from Phase 3a (if available)
    verified_contracts: Option<VerifiedContractRegistry>,
    /// Stdlib metadata for type checking (None = StdlibBootstrap mode)
    stdlib_metadata: Option<Arc<CoreMetadata>>,
    /// Number of user modules (from the end of the modules list).
    /// When core sources are prepended, this tells the phase how many modules
    /// at the end are user code (vs stdlib). Errors in stdlib modules are non-fatal.
    /// None means all modules are treated as user code.
    user_module_count: Option<usize>,
}

impl SemanticAnalysisPhase {
    /// Create a new semantic analysis phase in StdlibBootstrap mode.
    ///
    /// Uses minimal context with builtins only (no external stdlib).
    pub fn new() -> Self {
        Self {
            verified_contracts: None,
            stdlib_metadata: None,
            user_module_count: None,
        }
    }

    /// Create a new semantic analysis phase in NormalBuild mode.
    ///
    /// Uses pre-compiled stdlib types from the provided metadata.
    pub fn with_core(stdlib: Arc<CoreMetadata>) -> Self {
        Self {
            verified_contracts: None,
            stdlib_metadata: Some(stdlib),
            user_module_count: None,
        }
    }

    /// Create a semantic analysis phase with verified contracts
    pub fn with_contracts(mut self, registry: VerifiedContractRegistry) -> Self {
        self.verified_contracts = Some(registry);
        self
    }

    /// Set stdlib metadata (builder pattern)
    pub fn with_core_metadata(mut self, stdlib: Arc<CoreMetadata>) -> Self {
        self.stdlib_metadata = Some(stdlib);
        self
    }

    /// Set the number of user modules (from the end of the modules list).
    /// Modules before this count are treated as stdlib (errors are non-fatal).
    pub fn with_user_module_count(mut self, count: usize) -> Self {
        self.user_module_count = Some(count);
        self
    }
}

impl Default for SemanticAnalysisPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for SemanticAnalysisPhase {
    fn name(&self) -> &str {
        "Phase 4: Semantic Analysis"
    }

    fn description(&self) -> &str {
        "Bidirectional type checking (3x faster than Hindley-Milner)"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract modules AND contracts (if available)
        let (modules, contracts_opt) = match &input.data {
            PhaseData::AstModulesWithContracts {
                modules,
                verification_results,
            } => (modules, Some(verification_results.clone())),
            PhaseData::AstModules(modules) => (modules, None),
            _ => {
                let diag = DiagnosticBuilder::new(Severity::Error)
                    .message("Invalid input for semantic analysis phase".to_string())
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Log if contracts are available
        if let Some(ref contracts) = contracts_opt {
            tracing::info!(
                "Semantic analysis with {} verified contracts",
                contracts.verified_contracts.len()
            );
            tracing::debug!(
                "Contract breakdown: {} functions, {} types, {} protocols",
                contracts.stats.functions_with_contracts,
                contracts.stats.type_invariants_verified,
                contracts.stats.protocol_contracts_verified
            );
        } else {
            tracing::info!("Semantic analysis without contract verification");
        }

        // Create a new type checker for this phase
        //
        // Mode selection:
        // - StdlibBootstrap (stdlib_metadata = None): Uses minimal context + builtins only
        // - NormalBuild (stdlib_metadata = Some): Loads stdlib types from pre-compiled metadata
        let mut phase_checker = match &self.stdlib_metadata {
            Some(stdlib) => {
                tracing::info!(
                    "Semantic analysis with stdlib metadata: {} types, {} protocols, {} functions",
                    stdlib.types.len(),
                    stdlib.protocols.len(),
                    stdlib.functions.len()
                );
                // Clone the metadata from Arc for TypeChecker (it takes ownership)
                TypeChecker::new_with_core(stdlib.as_ref().clone())
            }
            None => {
                // Compiling stdlib itself - use minimal context, types are registered
                // dynamically as stdlib .vr files are parsed.
                tracing::info!("Semantic analysis for stdlib compilation (minimal context)");
                TypeChecker::with_minimal_context()
            }
        };

        // Register built-in types and functions (Int, Bool, print, assert, etc.)
        // NOTE: In NormalBuild mode, these may already be loaded from stdlib metadata,
        // but register_builtins() is idempotent and ensures core intrinsics are available.
        phase_checker.register_builtins();

        // If contracts are available, enable contract-aware type checking
        // This allows the type checker to leverage verified preconditions/postconditions
        // to strengthen type inference and validate call sites
        if let Some(ref contracts) = contracts_opt {
            // Store contract count for metrics
            // Note: Full contract integration with TypeChecker requires adding
            // a contract_context field to TypeChecker (see contract_integration.rs)
            // For now, we log availability and pass through contracts unchanged
            tracing::debug!(
                "Type checker initialized with {} verified contracts available",
                contracts.verified_contracts.len()
            );
        }

        let all_warnings = List::new();

        // Multi-pass type checking:
        // Pass 0a: Register all type NAMES (placeholders) - enables forward references
        // Pass 0b: Resolve all type BODIES (now all names are known)
        // Pass 1: Register all function signatures
        // Pass 2: Register protocol declarations
        // Pass 3: Register protocol implementations
        // Pass 4: Type check functions and expressions
        //
        // When core sources are prepended (e.g., by run_common_pipeline), the first
        // N-1 modules are stdlib and the last module is user code. Stdlib registration
        // errors are non-fatal (logged as debug) to match pipeline.rs behavior.
        // Only user module errors are fatal.

        // Determine which modules are stdlib vs user code.
        // User module count is set by the caller; default: all modules are user code.
        let total_modules = modules.len();
        let user_module_count = self.user_module_count.unwrap_or(total_modules);
        let stdlib_count = total_modules.saturating_sub(user_module_count);

        // Pass 0a: Register all type NAMES only (enables forward references to types)
        // This allows type A to reference type B even if B is defined later
        for module in modules {
            for item in &module.items {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    phase_checker.register_type_name_only(type_decl);
                }
            }
        }

        // Pass 0b: Resolve all type BODIES (now all type names are known)
        for (i, module) in modules.iter().enumerate() {
            let is_stdlib = i < stdlib_count;
            for item in &module.items {
                if let verum_ast::ItemKind::Type(type_decl) = &item.kind {
                    if let Err(e) = phase_checker.register_type_declaration(type_decl) {
                        if is_stdlib {
                            tracing::debug!("Stdlib type registration error (non-fatal): {:?}", e);
                        } else {
                            let diag = e.to_diagnostic();
                            return Err(List::from(vec![diag]));
                        }
                    }
                }
            }
        }

        // Pass 1: Register all function signatures (enables forward calls)
        for (i, module) in modules.iter().enumerate() {
            let is_stdlib = i < stdlib_count;
            for item in &module.items {
                if let verum_ast::ItemKind::Function(func) = &item.kind {
                    if let Err(e) = phase_checker.register_function_signature(func) {
                        if is_stdlib {
                            tracing::debug!("Stdlib function registration error (non-fatal): {:?}", e);
                        } else {
                            let diag = e.to_diagnostic();
                            return Err(List::from(vec![diag]));
                        }
                    }
                }
            }
        }

        // Pass 2: Register protocols
        for (i, module) in modules.iter().enumerate() {
            let is_stdlib = i < stdlib_count;
            for item in &module.items {
                if let verum_ast::ItemKind::Protocol(protocol_decl) = &item.kind {
                    if let Err(e) = phase_checker.register_protocol(protocol_decl) {
                        if is_stdlib {
                            tracing::debug!("Stdlib protocol registration error (non-fatal): {:?}", e);
                        } else {
                            let diag = e.to_diagnostic();
                            return Err(List::from(vec![diag]));
                        }
                    }
                }
            }
        }

        // Pass 3: Register protocol implementations
        for (i, module) in modules.iter().enumerate() {
            let is_stdlib = i < stdlib_count;
            for item in &module.items {
                if let verum_ast::ItemKind::Impl(impl_decl) = &item.kind {
                    if let Err(e) = phase_checker.register_impl_block(impl_decl) {
                        if is_stdlib {
                            tracing::debug!("Stdlib impl registration error (non-fatal): {:?}", e);
                        } else {
                            let diag = e.to_diagnostic();
                            return Err(List::from(vec![diag]));
                        }
                    }
                }
            }
        }

        // Pass 4: Type check each item
        //
        // Only type-check USER modules, not stdlib modules. Stdlib modules
        // are registered for their type/function/protocol declarations but
        // their function bodies are not checked here (they were validated
        // during stdlib compilation).
        for module in modules.iter().skip(stdlib_count) {
            for item in &module.items {
                match phase_checker.check_item(item) {
                    Ok(()) => {
                        // Success
                    }
                    Err(type_error) => {
                        // Convert type error to diagnostic
                        let diag = type_error.to_diagnostic();
                        return Err(List::from(vec![diag]));
                    }
                }
            }
        }

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);

        // Add type checker metrics
        let type_metrics = phase_checker.metrics.clone();
        metrics.add_custom_metric("synth_count", type_metrics.synth_count.to_string());
        metrics.add_custom_metric("check_count", type_metrics.check_count.to_string());
        metrics.add_custom_metric("unify_count", type_metrics.unify_count.to_string());
        metrics.add_custom_metric(
            "refinement_checks",
            type_metrics.refinement_checks.to_string(),
        );
        metrics.add_custom_metric("protocol_checks", type_metrics.protocol_checks.to_string());

        // Add contract metrics if available
        if let Some(ref contracts) = contracts_opt {
            metrics.add_custom_metric(
                "verified_contracts",
                contracts.verified_contracts.len().to_string(),
            );
        }

        tracing::info!(
            "Semantic analysis complete: {} modules, {:.2}ms",
            modules.len(),
            duration.as_millis()
        );
        tracing::debug!("{}", type_metrics.report());

        Ok(PhaseOutput {
            data: input.data, // Pass through AST modules (with or without contracts)
            warnings: all_warnings,
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        false // Type checking has dependencies
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}
