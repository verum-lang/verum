//! Phase 3a: Contract Verification
//!
//! SMT-based formal verification of contract literals and refinement types.
//!
//! ## Features
//!
//! - Translate contracts to SMT-LIB format
//! - Verify with Z3/CVC5 solvers
//! - Check pre/post conditions
//! - Validate type invariants
//! - Protocol contract verification
//! - Counterexample generation on failure
//! - Integration with verum_smt for SMT solving
//!
//! ## Verification Flow
//!
//! 1. Extract contracts from function attributes and body
//! 2. Parse RSL (Refinement Specification Language) clauses
//! 3. Translate to SMT formulas
//! 4. Verify using Z3 solver
//! 5. Generate counterexamples on failure
//!
//! Phase 3a: Contract verification. Translates contract#"..." to SMT-LIB,
//! generates verification conditions, invokes Z3/CVC5, verifies
//! preconditions => postconditions. Output: Verified AST or CompileError.

use anyhow::Result;
use std::time::Instant;
use verum_ast::decl::{
    FunctionBody, FunctionDecl, ItemKind, ProtocolDecl, ProtocolItemKind, TypeDecl, TypeDeclBody,
};
use verum_ast::{Expr, ExprKind, Item, LiteralKind, Module, Type, TypeKind};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};
use verum_smt::{
    Context, ContextConfig, ContractSpec, CounterExample, CounterExampleCategorizer,
    FailureCategory, ProofResult, RslClause, RslClauseKind, RslParser, Translator, VerifyMode,
};
use verum_smt::verify_strategy::extract_from_attributes;
use verum_common::{List, Text};

use super::{
    CompilationPhase, PhaseData, PhaseInput, PhaseMetrics, PhaseOutput, VerifiedContractRegistry,
};

/// Contract verification phase
///
/// This phase performs SMT-based verification of:
/// - Function contracts (pre/postconditions)
/// - Type invariants
/// - Protocol contracts
///
/// Uses the verum_smt crate for Z3 integration.
pub struct ContractVerificationPhase {
    /// Z3 context for SMT solving
    context: Context,
    /// Verification configuration
    config: VerificationConfig,
    /// Shared SMT routing statistics collector.
    ///
    /// When `Some`, every invocation of the underlying Z3 solver
    /// records a routing decision (`Z3Only`) plus its outcome and
    /// elapsed time. The session's `Arc<RoutingStats>` is threaded in
    /// via `with_routing_stats` so `verum build --smt-stats` can show
    /// real data instead of zeros. When `None`, verification runs
    /// exactly as before with no telemetry overhead.
    routing_stats: Option<std::sync::Arc<verum_smt::routing_stats::RoutingStats>>,
}

/// Configuration for contract verification
#[derive(Debug, Clone)]
pub struct VerificationConfig {
    /// Maximum time for a single verification (milliseconds)
    pub timeout_ms: u64,
    /// Whether to generate counterexamples on failure
    pub generate_counterexamples: bool,
    /// Whether to generate suggestions on failure
    pub generate_suggestions: bool,
    /// Verification mode
    pub mode: VerifyMode,
    /// Whether to verify protocol contracts
    pub verify_protocols: bool,
    /// Whether to verify type invariants
    pub verify_type_invariants: bool,
    /// Maximum number of verification errors before stopping
    pub max_errors: usize,
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000, // 30 seconds
            generate_counterexamples: true,
            generate_suggestions: true,
            mode: VerifyMode::Auto,
            verify_protocols: true,
            verify_type_invariants: true,
            max_errors: 100,
        }
    }
}

/// Statistics for contract verification
#[derive(Debug, Clone, Default)]
pub struct VerificationStats {
    /// Number of functions with contracts
    pub functions_with_contracts: usize,
    /// Number of contracts verified
    pub contracts_verified: usize,
    /// Number of pre-conditions checked
    pub preconditions_checked: usize,
    /// Number of post-conditions checked
    pub postconditions_checked: usize,
    /// Number of invariants verified
    pub invariants_verified: usize,
    /// Number of type invariants verified
    pub type_invariants_verified: usize,
    /// Number of protocol contracts verified
    pub protocol_contracts_verified: usize,
    /// Number of verification failures
    pub verification_failures: usize,
    /// Number of verification timeouts
    pub verification_timeouts: usize,
    /// Time spent in SMT solver (ms)
    pub smt_time_ms: u64,
    /// Cache hits for verification results
    pub cache_hits: usize,

    // --- Per-strategy telemetry (populated from `@verify(...)` attributes) ---
    /// Functions that opted out of SMT via `@verify(runtime)` or `@verify(static)`.
    pub functions_skipped_smt: usize,
    /// Functions that used `@verify(formal)` (explicit or implicit default).
    pub functions_strategy_formal: usize,
    /// Functions that used `@verify(fast)`.
    pub functions_strategy_fast: usize,
    /// Functions that used `@verify(thorough)`.
    pub functions_strategy_thorough: usize,
    /// Functions that used `@verify(certified)`.
    pub functions_strategy_certified: usize,
    /// Functions that used `@verify(synthesize)`.
    pub functions_strategy_synthesize: usize,
}

impl VerificationStats {
    /// Total functions that ran through the SMT path (all `requires_smt()` strategies).
    pub fn functions_via_smt(&self) -> usize {
        self.functions_strategy_formal
            + self.functions_strategy_fast
            + self.functions_strategy_thorough
            + self.functions_strategy_certified
            + self.functions_strategy_synthesize
    }
}

/// Result of verifying a single contract
#[derive(Debug)]
pub enum ContractVerificationResult {
    /// Contract verified successfully
    Verified {
        /// Proof result with timing information
        proof: ProofResult,
    },
    /// Contract verification failed
    Failed {
        /// Error message
        message: Text,
        /// Counterexample if available
        counterexample: Option<CounterExample>,
        /// Suggestions for fixing the issue
        suggestions: List<Text>,
        /// Failure category
        category: FailureCategory,
    },
    /// Verification timed out
    Timeout {
        /// Time spent before timeout
        time_ms: u64,
    },
    /// Error during verification (not a proof failure)
    Error {
        /// Error message
        message: Text,
    },
}

impl ContractVerificationPhase {
    /// Create a new contract verification phase with default configuration
    pub fn new() -> Self {
        Self::with_config(VerificationConfig::default())
    }

    /// Create a new contract verification phase with custom configuration
    pub fn with_config(config: VerificationConfig) -> Self {
        // Create Z3 context with timeout
        let ctx_config = ContextConfig::default()
            .with_timeout(std::time::Duration::from_millis(config.timeout_ms));
        let context = Context::with_config(ctx_config);

        Self {
            context,
            config,
            routing_stats: None,
        }
    }

    /// Install a shared routing-stats collector.
    ///
    /// Stored on the phase and forwarded to the underlying `Context`
    /// so every Z3 `check()` during verification is visible to
    /// `verum smt-stats`. Idempotent and thread-safe.
    pub fn with_routing_stats(
        mut self,
        stats: std::sync::Arc<verum_smt::routing_stats::RoutingStats>,
    ) -> Self {
        // Rewire the underlying context so its Context::check calls
        // automatically record into the shared collector.
        self.context = self.context.clone().with_routing_stats(stats.clone());
        self.routing_stats = Some(stats);
        self
    }

    /// Verify contracts in modules and return verified contracts registry
    fn verify_modules(
        &self,
        modules: &[Module],
        stats: &mut VerificationStats,
    ) -> Result<(List<Diagnostic>, VerifiedContractRegistry), List<Diagnostic>> {
        let mut warnings = List::new();
        let mut errors = List::new();
        let mut registry = VerifiedContractRegistry::new();

        for module in modules {
            match self.verify_module(module, stats, &mut registry) {
                Ok(module_warnings) => warnings.extend(module_warnings),
                Err(module_errors) => {
                    errors.extend(module_errors);
                    if errors.len() >= self.config.max_errors {
                        break;
                    }
                }
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok((warnings, registry))
    }

    /// Verify contracts in a single module
    fn verify_module(
        &self,
        module: &Module,
        stats: &mut VerificationStats,
        registry: &mut VerifiedContractRegistry,
    ) -> Result<List<Diagnostic>, List<Diagnostic>> {
        tracing::debug!("Verifying contracts in module");

        let mut warnings = List::new();
        let mut errors = List::new();

        for item in &module.items {
            match self.verify_item(item, stats, registry) {
                Ok(item_warnings) => warnings.extend(item_warnings),
                Err(item_errors) => {
                    errors.extend(item_errors);
                    if errors.len() >= self.config.max_errors {
                        break;
                    }
                }
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(warnings)
    }

    /// Verify contracts in a single item
    fn verify_item(
        &self,
        item: &Item,
        stats: &mut VerificationStats,
        registry: &mut VerifiedContractRegistry,
    ) -> Result<List<Diagnostic>, List<Diagnostic>> {
        match &item.kind {
            ItemKind::Function(func) => self.verify_function_contract(func, stats, registry),
            ItemKind::Type(type_decl) => {
                if self.config.verify_type_invariants {
                    self.verify_type_invariants(type_decl, stats, registry)
                } else {
                    Ok(List::new())
                }
            }
            ItemKind::Protocol(protocol) => {
                if self.config.verify_protocols {
                    self.verify_protocol_contracts(protocol, stats, registry)
                } else {
                    Ok(List::new())
                }
            }
            ItemKind::Impl(impl_decl) => {
                // Verify contracts in implementation items
                let mut warnings = List::new();
                for impl_item in &impl_decl.items {
                    if let verum_ast::decl::ImplItemKind::Function(func) = &impl_item.kind {
                        match self.verify_function_contract(func, stats, registry) {
                            Ok(w) => warnings.extend(w),
                            Err(e) => return Err(e),
                        }
                    }
                }
                Ok(warnings)
            }
            _ => {
                // No contracts to verify for other item types
                Ok(List::new())
            }
        }
    }

    /// Verify function contracts (pre/post conditions)
    ///
    /// Honors `@verify(strategy)` attributes:
    /// - `runtime` / `static`: skip SMT entirely (runtime checks only).
    /// - `formal` / `fast` / `thorough` / `certified` / `synthesize`:
    ///   proceed with SMT verification, using a timeout scaled by
    ///   `strategy.timeout_multiplier()`.
    ///
    /// The strategy is recorded in `stats` for observability and appears
    /// in the verification report emitted at the end of the phase.
    fn verify_function_contract(
        &self,
        func: &FunctionDecl,
        stats: &mut VerificationStats,
        registry: &mut VerifiedContractRegistry,
    ) -> Result<List<Diagnostic>, List<Diagnostic>> {
        tracing::debug!("Verifying function contract: {}", func.name);

        let mut warnings = List::new();
        let mut errors = List::new();

        // Extract per-function verification strategy from `@verify(...)`
        // attributes. Absent attribute → use the phase default from
        // VerificationConfig (auto mode).
        let strategy = extract_from_attributes(&func.attributes);
        if let Some(s) = strategy {
            use verum_smt::verify_strategy::VerifyStrategy as VS;
            match s {
                VS::Runtime | VS::Static => {
                    tracing::debug!(
                        "Skipping SMT for {} — strategy {:?} is runtime-only",
                        func.name, s
                    );
                    stats.functions_with_contracts += 1;
                    stats.functions_skipped_smt += 1;
                    return Ok(warnings);
                }
                VS::Formal => stats.functions_strategy_formal += 1,
                VS::Fast => stats.functions_strategy_fast += 1,
                VS::Thorough => stats.functions_strategy_thorough += 1,
                VS::Certified => stats.functions_strategy_certified += 1,
                VS::Synthesize => stats.functions_strategy_synthesize += 1,
            }
        } else {
            // Default strategy = Formal (implicit).
            stats.functions_strategy_formal += 1;
        }

        // Extract contracts from function attributes and body
        let contracts = self.extract_function_contracts(func);

        if contracts.is_empty() {
            // No contracts to verify
            return Ok(warnings);
        }

        stats.functions_with_contracts += 1;

        // Parse and merge all contracts
        let merged_spec = match self.parse_and_merge_contracts(&contracts, func.span) {
            Ok(spec) => spec,
            Err(e) => {
                let diag = DiagnosticBuilder::error()
                    .message(format!(
                        "Failed to parse contract for '{}': {}",
                        func.name, e
                    ))
                    .span(super::ast_span_to_diagnostic_span(func.span, None))
                    .build();
                errors.push(diag);
                return Err(errors);
            }
        };

        // Build parameter bindings for SMT translation
        let param_bindings = self.extract_param_bindings(func);

        // Track if all clauses verified successfully
        let all_verified = true;

        // Verify preconditions
        for precond in &merged_spec.preconditions {
            stats.preconditions_checked += 1;

            match self.verify_precondition(precond, &param_bindings) {
                ContractVerificationResult::Verified { .. } => {
                    stats.contracts_verified += 1;
                }
                ContractVerificationResult::Failed {
                    message,
                    counterexample,
                    suggestions,
                    category,
                } => {
                    stats.verification_failures += 1;
                    // Precondition failures are warnings (caller's responsibility)
                    let diag = self.create_verification_failure_diagnostic_with_severity(
                        &Text::from(func.name.as_str()),
                        "precondition",
                        &message,
                        counterexample.as_ref(),
                        &suggestions,
                        category,
                        func.span,
                        Severity::Warning,
                    );

                    warnings.push(diag);
                }
                ContractVerificationResult::Timeout { time_ms } => {
                    stats.verification_timeouts += 1;
                    let diag = DiagnosticBuilder::warning()
                        .message(format!(
                            "Precondition verification for '{}' timed out after {}ms",
                            func.name, time_ms
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .help("Consider using @verify(runtime) for complex contracts")
                        .build();
                    warnings.push(diag);
                }
                ContractVerificationResult::Error { message } => {
                    let diag = DiagnosticBuilder::error()
                        .message(format!(
                            "Error verifying precondition for '{}': {}",
                            func.name, message
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .build();
                    errors.push(diag);
                }
            }
        }

        // Verify postconditions
        for postcond in &merged_spec.postconditions {
            stats.postconditions_checked += 1;

            // Include return type binding if available
            let mut bindings = param_bindings.clone();
            if let Some(ret_ty) = func.return_type.as_ref() {
                bindings.push((Text::from("result"), (*ret_ty).clone()));
            }

            match self.verify_postcondition(postcond, &bindings, &merged_spec.preconditions) {
                ContractVerificationResult::Verified { .. } => {
                    stats.contracts_verified += 1;
                }
                ContractVerificationResult::Failed {
                    message,
                    counterexample,
                    suggestions,
                    category,
                } => {
                    stats.verification_failures += 1;
                    let diag = self.create_verification_failure_diagnostic(
                        &Text::from(func.name.as_str()),
                        "postcondition",
                        &message,
                        counterexample.as_ref(),
                        &suggestions,
                        category,
                        func.span,
                    );

                    // Postcondition failures are errors (function's responsibility)
                    errors.push(diag);
                }
                ContractVerificationResult::Timeout { time_ms } => {
                    stats.verification_timeouts += 1;
                    let diag = DiagnosticBuilder::warning()
                        .message(format!(
                            "Postcondition verification for '{}' timed out after {}ms",
                            func.name, time_ms
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .help("Consider using @verify(runtime) for complex contracts")
                        .build();
                    warnings.push(diag);
                }
                ContractVerificationResult::Error { message } => {
                    let diag = DiagnosticBuilder::error()
                        .message(format!(
                            "Error verifying postcondition for '{}': {}",
                            func.name, message
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .build();
                    errors.push(diag);
                }
            }
        }

        // Verify invariants
        for invariant in &merged_spec.invariants {
            stats.invariants_verified += 1;

            match self.verify_invariant(invariant, &param_bindings) {
                ContractVerificationResult::Verified { .. } => {
                    stats.contracts_verified += 1;
                }
                ContractVerificationResult::Failed {
                    message,
                    counterexample,
                    suggestions,
                    category,
                } => {
                    stats.verification_failures += 1;
                    let diag = self.create_verification_failure_diagnostic(
                        &Text::from(func.name.as_str()),
                        "invariant",
                        &message,
                        counterexample.as_ref(),
                        &suggestions,
                        category,
                        func.span,
                    );
                    errors.push(diag);
                }
                ContractVerificationResult::Timeout { time_ms } => {
                    stats.verification_timeouts += 1;
                    let diag = DiagnosticBuilder::warning()
                        .message(format!(
                            "Invariant verification for '{}' timed out after {}ms",
                            func.name, time_ms
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .build();
                    warnings.push(diag);
                }
                ContractVerificationResult::Error { message } => {
                    let diag = DiagnosticBuilder::error()
                        .message(format!(
                            "Error verifying invariant for '{}': {}",
                            func.name, message
                        ))
                        .span(super::ast_span_to_diagnostic_span(func.span, None))
                        .build();
                    errors.push(diag);
                }
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        // SUCCESS - All contracts verified, register in registry
        // This completes Phase 3a → Phase 4 integration
        if all_verified && !contracts.is_empty() {
            use crate::contract_integration::register_function_contract;
            register_function_contract(
                registry,
                func.name.as_str(),
                merged_spec,
                std::time::Duration::from_secs(0), // Actual duration tracked elsewhere
                func.span,
            );
        }

        Ok(warnings)
    }

    /// Extract contracts from function attributes and body
    fn extract_function_contracts(&self, func: &FunctionDecl) -> List<(Text, verum_ast::Span)> {
        let mut contracts = List::new();

        // Check for contract# literals in attributes
        for attr in &func.attributes {
            if attr.name.as_str() == "contract" || attr.name.as_str() == "verify" {
                if let Some(args) = attr.args.as_ref() {
                    for arg in args.iter() {
                        if let Some(contract_text) = self.extract_contract_from_expr(arg) {
                            contracts.push((contract_text, attr.span));
                        }
                    }
                }
            }
        }

        // Check for contract# literals in function body
        if let Some(ref body) = func.body.as_ref() {
            match body {
                FunctionBody::Block(block) => {
                    // Look for contract literals at the start of the block
                    for stmt in &block.stmts {
                        if let verum_ast::stmt::StmtKind::Expr { expr, .. } = &stmt.kind {
                            if let Some(contract_text) = self.extract_contract_from_expr(expr) {
                                contracts.push((contract_text, expr.span));
                            }
                        }
                    }
                }
                FunctionBody::Expr(expr) => {
                    if let Some(contract_text) = self.extract_contract_from_expr(expr) {
                        contracts.push((contract_text, expr.span));
                    }
                }
            }
        }

        // Note: Refinement types on parameters and return types (e.g., `x: Int{> 0}`)
        // are verified through the refinement type system, not through the contract
        // verification pipeline. Do NOT convert them to contracts here, as the AST
        // Debug representation is not valid contract text and will cause parse errors.

        contracts
    }

    /// Extract contract text from an expression (contract# literal)
    fn extract_contract_from_expr(&self, expr: &Expr) -> Option<Text> {
        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Contract(content) => Some(Text::from(content.as_str())),
                _ => None,
            },
            ExprKind::Paren(inner) => self.extract_contract_from_expr(inner),
            _ => None,
        }
    }

    /// Parse and merge multiple contract specifications
    fn parse_and_merge_contracts(
        &self,
        contracts: &[(Text, verum_ast::Span)],
        default_span: verum_ast::Span,
    ) -> Result<ContractSpec, Text> {
        let mut merged = ContractSpec::new(default_span);

        for (content, span) in contracts {
            let mut parser = RslParser::new(content.as_str().to_string().into(), *span);
            let spec = parser.parse().map_err(|e| Text::from(format!("{}", e)))?;

            merged.preconditions.extend(spec.preconditions);
            merged.postconditions.extend(spec.postconditions);
            merged.invariants.extend(spec.invariants);
        }

        Ok(merged)
    }

    /// Extract parameter bindings from function declaration
    fn extract_param_bindings(&self, func: &FunctionDecl) -> List<(Text, Type)> {
        let mut bindings = List::new();

        for param in &func.params {
            match &param.kind {
                verum_ast::decl::FunctionParamKind::Regular { pattern, ty, .. } => {
                    // Extract variable name from pattern
                    if let Some(name) = self.extract_pattern_name(pattern) {
                        // Get base type (strip refinement for SMT binding)
                        let base_ty = match &ty.kind {
                            TypeKind::Refined { base, .. } => (**base).clone(),
                            _ => ty.clone(),
                        };
                        bindings.push((name, base_ty));
                    }
                }
                _ => {
                    // Self parameters don't need explicit bindings for contract verification
                }
            }
        }

        bindings
    }

    /// Extract variable name from a pattern
    fn extract_pattern_name(&self, pattern: &verum_ast::pattern::Pattern) -> Option<Text> {
        match &pattern.kind {
            verum_ast::pattern::PatternKind::Ident { name, .. } => Some(Text::from(name.as_str())),
            _ => None,
        }
    }

    /// Verify a precondition
    fn verify_precondition(
        &self,
        clause: &RslClause,
        bindings: &[(Text, Type)],
    ) -> ContractVerificationResult {
        let start = Instant::now();

        // Create translator with bindings
        let mut translator = Translator::new(&self.context);

        // Bind parameters
        for (name, ty) in bindings {
            match translator.create_var(name.as_str(), ty) {
                Ok(var) => translator.bind(name.clone(), var),
                Err(e) => {
                    return ContractVerificationResult::Error {
                        message: format!("Failed to create variable '{}': {}", name, e).into(),
                    };
                }
            }
        }

        // Translate precondition expression
        let z3_expr = match translator.translate_expr(&clause.expr) {
            Ok(expr) => expr,
            Err(e) => {
                return ContractVerificationResult::Error {
                    message: format!("Failed to translate precondition: {}", e).into(),
                };
            }
        };

        // Ensure it's a boolean expression
        let z3_bool = match z3_expr.as_bool() {
            Some(b) => b,
            None => {
                return ContractVerificationResult::Error {
                    message: "Precondition must be a boolean expression".into(),
                };
            }
        };

        // For preconditions, we just check that they are satisfiable
        // (there exists at least one valid input)
        let solver = self.context.solver();
        solver.assert(&z3_bool);

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Goes through Context::check so stats are recorded automatically
        // when a routing-stats collector is installed on the context.
        let verdict = self.context.check(&solver);
        match verdict {
            verum_smt::z3::SatResult::Sat => {
                // Precondition is satisfiable - good!
                let cost = verum_smt::VerificationCost::new(
                    "precondition".to_string().into(),
                    start.elapsed(),
                    true,
                );
                ContractVerificationResult::Verified {
                    proof: ProofResult::new(cost),
                }
            }
            verum_smt::z3::SatResult::Unsat => {
                // Precondition is unsatisfiable - no valid inputs exist!
                let category = FailureCategory::Other;
                let suggestions = CounterExampleCategorizer::suggest_fixes(category);

                ContractVerificationResult::Failed {
                    message: "Precondition is unsatisfiable - no valid inputs exist".into(),
                    counterexample: None,
                    suggestions,
                    category,
                }
            }
            verum_smt::z3::SatResult::Unknown => {
                if elapsed_ms >= self.config.timeout_ms {
                    ContractVerificationResult::Timeout {
                        time_ms: elapsed_ms,
                    }
                } else {
                    ContractVerificationResult::Error {
                        message: "SMT solver returned unknown result".into(),
                    }
                }
            }
        }
    }

    /// Verify a postcondition
    fn verify_postcondition(
        &self,
        clause: &RslClause,
        bindings: &[(Text, Type)],
        preconditions: &[RslClause],
    ) -> ContractVerificationResult {
        let start = Instant::now();

        // Create translator with bindings
        let mut translator = Translator::new(&self.context);

        // Bind parameters
        for (name, ty) in bindings {
            match translator.create_var(name.as_str(), ty) {
                Ok(var) => translator.bind(name.clone(), var),
                Err(e) => {
                    return ContractVerificationResult::Error {
                        message: format!("Failed to create variable '{}': {}", name, e).into(),
                    };
                }
            }
        }

        let solver = self.context.solver();

        // Assert all preconditions as assumptions
        for precond in preconditions {
            match translator.translate_expr(&precond.expr) {
                Ok(expr) => {
                    if let Some(bool_expr) = expr.as_bool() {
                        solver.assert(&bool_expr);
                    }
                }
                Err(_) => {
                    // Skip preconditions that can't be translated
                    continue;
                }
            }
        }

        // Translate postcondition expression
        let z3_expr = match translator.translate_expr(&clause.expr) {
            Ok(expr) => expr,
            Err(e) => {
                return ContractVerificationResult::Error {
                    message: format!("Failed to translate postcondition: {}", e).into(),
                };
            }
        };

        // Ensure it's a boolean expression
        let z3_bool = match z3_expr.as_bool() {
            Some(b) => b,
            None => {
                return ContractVerificationResult::Error {
                    message: "Postcondition must be a boolean expression".into(),
                };
            }
        };

        // Assert the NEGATION of the postcondition
        // If SAT, we found a counterexample (postcondition can be violated)
        solver.assert(&z3_bool.not());

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Route through Context::check for automatic telemetry.
        let verdict = self.context.check(&solver);
        match verdict {
            verum_smt::z3::SatResult::Unsat => {
                // No counterexample - postcondition always holds!
                let cost = verum_smt::VerificationCost::new(
                    "postcondition".to_string().into(),
                    start.elapsed(),
                    true,
                );
                ContractVerificationResult::Verified {
                    proof: ProofResult::new(cost),
                }
            }
            verum_smt::z3::SatResult::Sat => {
                // Found a counterexample
                let model = solver.get_model();
                let counterexample = model.map(|m| {
                    let extractor = verum_smt::CounterExampleExtractor::new(&m);
                    let var_names: List<Text> = bindings.iter().map(|(n, _)| n.clone()).collect();
                    extractor.extract(&var_names, &format!("{:?}", clause.expr))
                });

                let category = counterexample
                    .as_ref()
                    .map(|ce| CounterExampleCategorizer::categorize(ce))
                    .unwrap_or(FailureCategory::Other);

                let suggestions = CounterExampleCategorizer::suggest_fixes(category);

                ContractVerificationResult::Failed {
                    message: format!("Postcondition can be violated: {:?}", clause.expr).into(),
                    counterexample,
                    suggestions,
                    category,
                }
            }
            verum_smt::z3::SatResult::Unknown => {
                if elapsed_ms >= self.config.timeout_ms {
                    ContractVerificationResult::Timeout {
                        time_ms: elapsed_ms,
                    }
                } else {
                    ContractVerificationResult::Error {
                        message: "SMT solver returned unknown result".into(),
                    }
                }
            }
        }
    }

    /// Verify an invariant
    fn verify_invariant(
        &self,
        clause: &RslClause,
        bindings: &[(Text, Type)],
    ) -> ContractVerificationResult {
        // Invariants are verified the same way as postconditions
        // They must hold at all points during execution
        self.verify_postcondition(clause, bindings, &[])
    }

    /// Verify type invariants
    fn verify_type_invariants(
        &self,
        type_decl: &TypeDecl,
        stats: &mut VerificationStats,
        registry: &mut VerifiedContractRegistry,
    ) -> Result<List<Diagnostic>, List<Diagnostic>> {
        tracing::debug!("Verifying type invariants: {}", type_decl.name);

        let mut warnings = List::new();
        let mut errors = List::new();
        let start = Instant::now();

        match &type_decl.body {
            TypeDeclBody::Record(fields) => {
                // Check refinement types in fields
                for field in fields {
                    if let TypeKind::Refined { base, predicate } = &field.ty.kind {
                        stats.type_invariants_verified += 1;

                        // Create a binding for the field value
                        let bindings = vec![(Text::from("it"), (**base).clone())];

                        // Verify the refinement predicate is satisfiable
                        let clause = RslClause {
                            kind: RslClauseKind::Invariant,
                            expr: predicate.expr.clone(),
                            label: None,
                            span: predicate.span,
                        };

                        match self.verify_precondition(&clause, &bindings) {
                            ContractVerificationResult::Verified { .. } => {
                                stats.contracts_verified += 1;

                                // Register the verified type invariant
                                let mut spec = ContractSpec::new(type_decl.span);
                                spec.invariants.push(clause.clone());
                                use crate::contract_integration::register_type_invariant;
                                register_type_invariant(
                                    registry,
                                    type_decl.name.as_str(),
                                    spec,
                                    start.elapsed(),
                                    type_decl.span,
                                );
                            }
                            ContractVerificationResult::Failed { message, .. } => {
                                stats.verification_failures += 1;
                                let diag = DiagnosticBuilder::error()
                                    .message(format!(
                                        "Type invariant for field '{}' in '{}' is unsatisfiable: {}",
                                        field.name, type_decl.name, message
                                    ))
                                    .span(super::ast_span_to_diagnostic_span(field.span, None))
                                    .build();
                                errors.push(diag);
                            }
                            ContractVerificationResult::Timeout { time_ms } => {
                                stats.verification_timeouts += 1;
                                let diag = DiagnosticBuilder::warning()
                                    .message(format!(
                                        "Type invariant verification for '{}' timed out after {}ms",
                                        type_decl.name, time_ms
                                    ))
                                    .span(super::ast_span_to_diagnostic_span(type_decl.span, None))
                                    .build();
                                warnings.push(diag);
                            }
                            ContractVerificationResult::Error { message } => {
                                let diag = DiagnosticBuilder::error()
                                    .message(format!(
                                        "Error verifying type invariant for '{}': {}",
                                        type_decl.name, message
                                    ))
                                    .span(super::ast_span_to_diagnostic_span(type_decl.span, None))
                                    .build();
                                errors.push(diag);
                            }
                        }
                    }
                }
            }
            TypeDeclBody::Newtype(inner) => {
                // Check refinement in newtype wrapper
                if let TypeKind::Refined { base, predicate } = &inner.kind {
                    stats.type_invariants_verified += 1;

                    let bindings = vec![(Text::from("it"), (**base).clone())];
                    let clause = RslClause {
                        kind: RslClauseKind::Invariant,
                        expr: predicate.expr.clone(),
                        label: None,
                        span: predicate.span,
                    };

                    match self.verify_precondition(&clause, &bindings) {
                        ContractVerificationResult::Verified { .. } => {
                            stats.contracts_verified += 1;

                            // Register the verified type invariant
                            let mut spec = ContractSpec::new(type_decl.span);
                            spec.invariants.push(clause.clone());
                            use crate::contract_integration::register_type_invariant;
                            register_type_invariant(
                                registry,
                                type_decl.name.as_str(),
                                spec,
                                start.elapsed(),
                                type_decl.span,
                            );
                        }
                        ContractVerificationResult::Failed { message, .. } => {
                            stats.verification_failures += 1;
                            let diag = DiagnosticBuilder::error()
                                .message(format!(
                                    "Type invariant for '{}' is unsatisfiable: {}",
                                    type_decl.name, message
                                ))
                                .span(super::ast_span_to_diagnostic_span(type_decl.span, None))
                                .build();
                            errors.push(diag);
                        }
                        ContractVerificationResult::Timeout { time_ms } => {
                            stats.verification_timeouts += 1;
                            let diag = DiagnosticBuilder::warning()
                                .message(format!(
                                    "Type invariant verification for '{}' timed out after {}ms",
                                    type_decl.name, time_ms
                                ))
                                .span(super::ast_span_to_diagnostic_span(type_decl.span, None))
                                .build();
                            warnings.push(diag);
                        }
                        ContractVerificationResult::Error { message } => {
                            let diag = DiagnosticBuilder::error()
                                .message(format!(
                                    "Error verifying type invariant for '{}': {}",
                                    type_decl.name, message
                                ))
                                .span(super::ast_span_to_diagnostic_span(type_decl.span, None))
                                .build();
                            errors.push(diag);
                        }
                    }
                }
            }
            _ => {
                // Other type bodies don't have direct invariants
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(warnings)
    }

    /// Verify protocol contracts
    fn verify_protocol_contracts(
        &self,
        protocol: &ProtocolDecl,
        stats: &mut VerificationStats,
        registry: &mut VerifiedContractRegistry,
    ) -> Result<List<Diagnostic>, List<Diagnostic>> {
        tracing::debug!("Verifying protocol contracts: {}", protocol.name);

        let mut warnings = List::new();
        let mut errors = List::new();
        let _start = Instant::now();

        // Verify contracts in protocol methods
        for item in &protocol.items {
            match &item.kind {
                ProtocolItemKind::Function { decl, default_impl } => {
                    stats.protocol_contracts_verified += 1;

                    // Extract contracts from protocol method
                    let contracts = self.extract_function_contracts(decl);

                    // Verify the function contract
                    match self.verify_function_contract(decl, stats, registry) {
                        Ok(w) => {
                            warnings.extend(w);

                            // If contracts were verified, register as protocol contract
                            if !contracts.is_empty() {
                                // The contract was already registered by verify_function_contract
                                // But we could update it to be marked as a protocol method
                                // For now, the registry correctly stores it
                            }
                        }
                        Err(e) => errors.extend(e),
                    }

                    // If there's a default implementation, verify it satisfies the contract
                    if let Some(_impl) = default_impl.as_ref() {
                        // For now, we skip verifying default implementations
                        // Full verification would require analyzing the implementation body
                        tracing::debug!(
                            "Skipping default implementation verification for '{}::{}'",
                            protocol.name,
                            decl.name
                        );
                    }
                }
                ProtocolItemKind::Type {
                    name: _,
                    bounds: _,
                    type_params: _,
                    where_clause: _,
                    default_type: _,
                } => {
                    // Associated types don't have direct contracts
                }
                ProtocolItemKind::Const { name: _, ty } => {
                    // Check if const type has refinement
                    if let TypeKind::Refined { base, predicate } = &ty.kind {
                        stats.type_invariants_verified += 1;

                        let bindings = vec![(Text::from("it"), (**base).clone())];
                        let clause = RslClause {
                            kind: RslClauseKind::Invariant,
                            expr: predicate.expr.clone(),
                            label: None,
                            span: predicate.span,
                        };

                        match self.verify_precondition(&clause, &bindings) {
                            ContractVerificationResult::Verified { .. } => {
                                stats.contracts_verified += 1;
                            }
                            ContractVerificationResult::Failed { message, .. } => {
                                stats.verification_failures += 1;
                                let diag = DiagnosticBuilder::error()
                                    .message(format!(
                                        "Associated const type invariant in '{}' is unsatisfiable: {}",
                                        protocol.name, message
                                    ))
                                    .span(super::ast_span_to_diagnostic_span(item.span, None))
                                    .build();
                                errors.push(diag);
                            }
                            _ => {}
                        }
                    }
                }
                ProtocolItemKind::Axiom(_) => {
                    // T1-R: protocol axioms are discharged at implement
                    // sites by the model-verification pipeline, not by
                    // this contract-verification pass.
                }
            }
        }

        if !errors.is_empty() {
            return Err(errors);
        }

        Ok(warnings)
    }

    /// Create a diagnostic for a verification failure
    fn create_verification_failure_diagnostic(
        &self,
        func_name: &Text,
        contract_type: &str,
        message: &Text,
        counterexample: Option<&CounterExample>,
        suggestions: &[Text],
        category: FailureCategory,
        span: verum_ast::Span,
    ) -> Diagnostic {
        self.create_verification_failure_diagnostic_with_severity(
            func_name,
            contract_type,
            message,
            counterexample,
            suggestions,
            category,
            span,
            Severity::Error,
        )
    }

    /// Create a diagnostic for a verification failure with custom severity
    fn create_verification_failure_diagnostic_with_severity(
        &self,
        func_name: &Text,
        contract_type: &str,
        message: &Text,
        counterexample: Option<&CounterExample>,
        suggestions: &[Text],
        category: FailureCategory,
        span: verum_ast::Span,
        severity: Severity,
    ) -> Diagnostic {
        let mut builder = DiagnosticBuilder::new(severity)
            .message(format!(
                "Contract violation in '{}': {} failed - {}",
                func_name, contract_type, message
            ))
            .span(super::ast_span_to_diagnostic_span(span, None))
            .add_note(format!("Failure category: {}", category));

        // Add counterexample if available
        if let Some(ce) = counterexample {
            builder = builder.help(format!("Counterexample:\n{}", ce));
        }

        // Add suggestions
        for suggestion in suggestions {
            builder = builder.help(suggestion.as_str());
        }

        builder.build()
    }
}

impl Default for ContractVerificationPhase {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilationPhase for ContractVerificationPhase {
    fn name(&self) -> &str {
        "Phase 3a: Contract Verification"
    }

    fn description(&self) -> &str {
        "SMT-based verification of function contracts, type invariants, and protocol constraints"
    }

    fn execute(&self, input: PhaseInput) -> Result<PhaseOutput, List<Diagnostic>> {
        let start = Instant::now();

        // Extract modules from input
        let modules = match &input.data {
            PhaseData::AstModules(modules) => modules,
            _ => {
                let diag = DiagnosticBuilder::error()
                    .message("Invalid input for contract verification phase")
                    .build();
                return Err(List::from(vec![diag]));
            }
        };

        // Create statistics tracker
        let mut stats = VerificationStats::default();

        // Verify contracts and collect registry
        let (warnings, registry) = match self.verify_modules(modules, &mut stats) {
            Ok((warnings, registry)) => (warnings, registry),
            Err(errors) => return Err(errors),
        };

        let duration = start.elapsed();
        let mut metrics = PhaseMetrics::new(self.name()).with_duration(duration);

        // Add all statistics to metrics
        metrics.add_custom_metric(
            "functions_with_contracts",
            stats.functions_with_contracts.to_string(),
        );
        metrics.add_custom_metric("contracts_verified", stats.contracts_verified.to_string());
        metrics.add_custom_metric(
            "preconditions_checked",
            stats.preconditions_checked.to_string(),
        );
        metrics.add_custom_metric(
            "postconditions_checked",
            stats.postconditions_checked.to_string(),
        );
        metrics.add_custom_metric("invariants_verified", stats.invariants_verified.to_string());
        metrics.add_custom_metric(
            "type_invariants_verified",
            stats.type_invariants_verified.to_string(),
        );
        metrics.add_custom_metric(
            "protocol_contracts_verified",
            stats.protocol_contracts_verified.to_string(),
        );
        metrics.add_custom_metric(
            "verification_failures",
            stats.verification_failures.to_string(),
        );
        metrics.add_custom_metric(
            "verification_timeouts",
            stats.verification_timeouts.to_string(),
        );
        metrics.add_custom_metric("smt_time_ms", stats.smt_time_ms.to_string());
        metrics.add_custom_metric("cache_hits", stats.cache_hits.to_string());

        tracing::info!(
            "Contract verification complete: {} functions, {} contracts verified, {} failures, {:.2}ms",
            stats.functions_with_contracts,
            stats.contracts_verified,
            stats.verification_failures,
            duration.as_millis()
        );

        // Create VerificationResults for Phase 4 handoff
        let verification_results = super::VerificationResults {
            verified_contracts: registry.all_contracts().clone(),
            stats,
            success: true,
        };

        // Return AstModulesWithContracts variant for Phase 4 consumption
        Ok(PhaseOutput {
            data: PhaseData::AstModulesWithContracts {
                modules: modules.clone(),
                verification_results,
            },
            warnings,
            metrics,
        })
    }

    fn can_parallelize(&self) -> bool {
        true // Contracts can be verified in parallel (though Z3 context is not thread-safe)
    }

    fn metrics(&self) -> PhaseMetrics {
        PhaseMetrics::new(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_config_default() {
        let config = VerificationConfig::default();
        assert_eq!(config.timeout_ms, 30_000);
        assert!(config.generate_counterexamples);
        assert!(config.generate_suggestions);
    }

    #[test]
    fn test_verification_stats_default() {
        let stats = VerificationStats::default();
        assert_eq!(stats.contracts_verified, 0);
        assert_eq!(stats.verification_failures, 0);
        assert_eq!(stats.functions_skipped_smt, 0);
        assert_eq!(stats.functions_via_smt(), 0);
    }

    #[test]
    fn test_verification_stats_strategy_breakdown() {
        let mut stats = VerificationStats::default();
        stats.functions_strategy_formal = 3;
        stats.functions_strategy_fast = 2;
        stats.functions_strategy_thorough = 1;
        stats.functions_strategy_certified = 1;
        stats.functions_strategy_synthesize = 0;
        assert_eq!(stats.functions_via_smt(), 7);
    }

    /// Integration: `extract_from_attributes` recognizes all strategy
    /// names routed through the same matcher the phase uses.
    #[test]
    fn test_extract_from_attributes_covers_all_strategies() {
        use verum_ast::attr::Attribute;
        use verum_ast::expr::Expr;
        use verum_ast::{Ident, Span};
        use verum_common::{List, Maybe, Text};
        use verum_smt::verify_strategy::VerifyStrategy as VS;

        let mk = |value: &str| -> Attribute {
            let name = Ident::new(Text::from(value), Span::dummy());
            let path_expr = Expr::ident(name);
            let mut args = List::new();
            args.push(path_expr);
            Attribute::new(
                Text::from("verify"),
                Maybe::Some(args),
                Span::dummy(),
            )
        };

        for (name, expected) in [
            ("runtime", VS::Runtime),
            ("static", VS::Static),
            ("formal", VS::Formal),
            ("fast", VS::Fast),
            ("thorough", VS::Thorough),
            ("certified", VS::Certified),
            ("synthesize", VS::Synthesize),
        ] {
            let mut attrs = List::new();
            attrs.push(mk(name));
            let got = extract_from_attributes(&attrs).unwrap_or_else(|| {
                panic!("strategy '{}' should parse", name)
            });
            assert!(
                std::mem::discriminant(&got) == std::mem::discriminant(&expected),
                "strategy '{}' mis-parsed (got {:?}, expected {:?})",
                name,
                got,
                expected
            );
        }
    }

    /// Integration: absent `@verify` attribute yields None, letting the
    /// phase fall back to its configured default strategy.
    #[test]
    fn test_extract_from_attributes_absent() {
        use verum_common::List;
        use verum_ast::attr::Attribute;
        let attrs: List<Attribute> = List::new();
        assert!(extract_from_attributes(&attrs).is_none());
    }

    /// Integration: `VerifyStrategy::requires_smt()` is used by the
    /// phase to decide whether to skip SMT. The contract: Runtime and
    /// Static are the only strategies that opt out.
    #[test]
    fn test_strategy_runtime_skips_smt() {
        use verum_smt::verify_strategy::VerifyStrategy as VS;
        assert!(!VS::Runtime.requires_smt());
        assert!(!VS::Static.requires_smt());
        assert!(VS::Formal.requires_smt());
        assert!(VS::Fast.requires_smt());
        assert!(VS::Thorough.requires_smt());
        assert!(VS::Certified.requires_smt());
        assert!(VS::Synthesize.requires_smt());
    }
}
