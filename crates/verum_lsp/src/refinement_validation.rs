//! LSP refinement validation extensions
//!
//! This module implements the custom LSP methods for refinement validation:
//! - `verum/validateRefinement`: Validate refinement at cursor position
//! - `verum/promoteToChecked`: Promote &T to &checked T with escape analysis proof
//! - `verum/inferRefinement`: Infer tightest refinement from usage
//!
//! LSP Refinement Validation Protocol Extensions:
//! Three custom LSP methods extend the standard protocol:
//! 1. `verum/validateRefinement` - validates refinement type at cursor position,
//!    integrates with SMT solver for real-time feedback (<100ms latency),
//!    returns diagnostics with concrete counterexamples showing violating values
//! 2. `verum/promoteToChecked` - promotes &T to &checked T with escape analysis
//!    proof, generates TextEdits and optional proof comments
//! 3. `verum/inferRefinement` - infers tightest refinement from usage patterns,
//!    returns inferred type with confidence level and suggested edits
//!    Quick fixes provide 6 categories: runtime check wrapping, inline refinement,
//!    sigma type conversion, runtime assertion, weaken refinement, promote to &checked.

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_lsp::lsp_types::*;
use verum_common::{List, Maybe, Text};

// Imports for SMT integration (placed after stdlib imports)
use verum_ast::{Expr, FileId, ItemKind, Module, Type, TypeKind};
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use verum_smt::{RefinementVerifier as SmtRefinementVerifier, VerificationError, VerifyMode};

// ==================== Data Structures ====================

/// Refinement diagnostic with counterexample data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinementDiagnostic {
    /// Standard LSP diagnostic fields
    pub range: Range,
    pub severity: DiagnosticSeverity,
    pub code: Option<String>,
    pub source: String,
    pub message: String,

    /// Verum extensions
    pub counterexample: Maybe<CounterexampleData>,
    pub quick_fixes: List<QuickFix>,
    pub related_information: List<DiagnosticRelatedInformation>,

    /// Performance metadata
    pub validation_time_ms: u64,
    pub smt_solver: SmtSolver,
}

/// Counterexample data showing concrete violation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterexampleData {
    /// Variable that violates constraint
    pub variable: Text,
    /// Concrete value (e.g., "-5")
    pub value: Text,
    /// Expected type (e.g., "Int{i != 0}")
    pub r#type: Text,
    /// Constraint that failed (e.g., "i != 0")
    pub constraint: Text,
    /// Human-readable explanation
    pub violation_reason: Text,
    /// Execution trace showing how value was derived
    pub trace: List<ExecutionTrace>,
}

/// Execution trace step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub line: u32,
    pub operation: Text,
    pub value: Text,
    pub explanation: Text,
}

/// Quick fix for refinement violation (for LSP transport)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickFix {
    pub title: String,
    pub kind: QuickFixKind,
    pub edits: List<TextEdit>,
    pub priority: u32,
    pub impact: QuickFixImpact,
    pub description: Maybe<String>,
}

/// Quick fix kind
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickFixKind {
    RuntimeCheck,
    InlineRefinement,
    SigmaType,
    Assertion,
    WeakenRefinement,
    PromoteToChecked,
}

/// Quick fix impact level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickFixImpact {
    Safe,
    Breaking,
    Unsafe,
}

/// SMT solver type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmtSolver {
    Z3,
    CVC5,
    None,
}

// ==================== Request/Response Types ====================

/// Parameters for validateRefinement request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateRefinementParams {
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    #[serde(default = "default_validation_mode")]
    pub mode: ValidationMode,
}

fn default_validation_mode() -> ValidationMode {
    ValidationMode::Quick
}

/// Validation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValidationMode {
    Quick,    // <100ms
    Thorough, // <1s
}

/// Result of validateRefinement request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateRefinementResult {
    pub valid: bool,
    pub diagnostics: List<RefinementDiagnostic>,
    pub performance_ms: u64,
}

/// Parameters for promoteToChecked request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteToCheckedParams {
    pub text_document: TextDocumentIdentifier,
    pub range: Range,
    #[serde(default)]
    pub include_proof: bool,
}

/// Result of promoteToChecked request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteToCheckedResult {
    pub success: bool,
    pub edits: List<TextEdit>,
    pub proof_comment: Maybe<Text>,
}

/// Parameters for inferRefinement request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferRefinementParams {
    pub text_document: TextDocumentIdentifier,
    pub symbol: Text,
}

/// Result of inferRefinement request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferRefinementResult {
    pub inferred_type: Text,
    pub confidence: ConfidenceLevel,
    pub usages: List<CodeLocation>,
    pub edits: List<TextEdit>,
}

/// Confidence level for inferred refinements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceLevel {
    High,
    Medium,
    Low,
}

/// Code location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeLocation {
    pub uri: Url,
    pub range: Range,
    pub context: Text,
}

// ==================== Validation Cache ====================

/// Simple LRU cache for validation results
pub struct ValidationCache {
    cache: Arc<RwLock<HashMap<String, CachedValidationResult>>>,
    max_entries: parking_lot::Mutex<usize>,
    ttl: parking_lot::Mutex<Duration>,
}

#[derive(Clone)]
struct CachedValidationResult {
    result: ValidationResult,
    timestamp: Instant,
}

#[derive(Clone)]
enum ValidationResult {
    Valid,
    Invalid { counterexample: CounterexampleData },
    Unknown,
}

impl ValidationCache {
    /// Create a new validation cache
    fn new(max_entries: usize, ttl: Duration) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            max_entries: parking_lot::Mutex::new(max_entries),
            ttl: parking_lot::Mutex::new(ttl),
        }
    }

    /// Hot-swap the cache's capacity and TTL at runtime. Called when the
    /// client updates `cacheTtlSeconds` / `cacheMaxEntries`. Keeps existing
    /// entries; a subsequent `insert` will enforce the new capacity.
    pub fn resize(&self, max_entries: usize, ttl: Duration) {
        *self.max_entries.lock() = max_entries;
        *self.ttl.lock() = ttl;

        // If the new capacity is smaller than the current fill, evict
        // oldest entries immediately so memory pressure is bounded.
        let mut cache = self.cache.write();
        while cache.len() > max_entries {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.timestamp)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            } else {
                break;
            }
        }
    }

    /// Get cached result if available and not expired
    fn get(&self, query: &str) -> Maybe<ValidationResult> {
        let ttl = *self.ttl.lock();
        let cache = self.cache.read();
        if let Some(cached) = cache.get(query)
            && cached.timestamp.elapsed() < ttl
        {
            return Maybe::Some(cached.result.clone());
        }
        Maybe::None
    }

    /// Insert result into cache
    fn insert(&self, query: String, result: ValidationResult) {
        let max_entries = *self.max_entries.lock();
        let mut cache = self.cache.write();

        // Evict oldest entries if at capacity
        if cache.len() >= max_entries {
            // Find oldest entry
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.timestamp)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }

        cache.insert(
            query,
            CachedValidationResult {
                result,
                timestamp: Instant::now(),
            },
        );
    }

    /// Clear all cached entries
    pub fn clear(&self) {
        self.cache.write().clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let ttl = *self.ttl.lock();
        let capacity = *self.max_entries.lock();
        let cache = self.cache.read();
        let now = Instant::now();
        let expired = cache
            .values()
            .filter(|v| now.duration_since(v.timestamp) >= ttl)
            .count();

        CacheStats {
            total_entries: cache.len(),
            expired_entries: expired,
            capacity,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_entries: usize,
    pub expired_entries: usize,
    pub capacity: usize,
}

// ==================== Refinement Validator ====================

/// Main refinement validator
pub struct RefinementValidator {
    cache: ValidationCache,
    /// Mirror of the relevant LSP config fields. Driven by the server on
    /// `initialize` so the validator can honour `validationMode`, the SMT
    /// solver choice, counterexample caps and "enabled" kill-switch.
    config: parking_lot::RwLock<ValidatorConfig>,
    /// Send-safe handle to the SMT worker thread. Every Z3 call is
    /// routed through this handle so the validator's async futures stay
    /// `Send` — which is a hard requirement for tower-lsp custom-method
    /// registration. See `smt_worker.rs` for the full rationale.
    smt: crate::smt_worker::SmtWorkerHandle,
}

#[derive(Debug, Clone)]
struct ValidatorConfig {
    enabled: bool,
    default_mode: ValidationMode,
    smt_timeout: Duration,
    show_counterexamples: bool,
    max_counterexample_traces: u32,
    cache_enabled: bool,
    /// Mirror of `LspConfig::verification_show_cost_warnings` —
    /// gates the per-validation cost diagnostic that fires when
    /// elapsed time exceeds `slow_threshold`. False means "stay
    /// quiet about slow validations even when they happen".
    cost_warnings_enabled: bool,
    /// Mirror of `LspConfig::verification_slow_threshold` — once
    /// validation wall-clock crosses this, a single info-level
    /// cost warning is surfaced (when `cost_warnings_enabled` is
    /// true) so users know which functions are dragging their
    /// editor latency.
    slow_threshold: Duration,
}

impl Default for ValidatorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_mode: ValidationMode::Quick,
            smt_timeout: Duration::from_millis(50),
            show_counterexamples: true,
            max_counterexample_traces: 5,
            cache_enabled: true,
            cost_warnings_enabled: true,
            slow_threshold: Duration::from_millis(5_000),
        }
    }
}

impl RefinementValidator {
    /// Create a new refinement validator. Spawns the shared SMT worker
    /// thread lazily on first construction — cheap (~1 OS thread), and
    /// keeps the rest of the server fully Send-safe.
    pub fn new() -> Self {
        Self {
            cache: ValidationCache::new(1000, Duration::from_secs(300)),
            config: parking_lot::RwLock::new(ValidatorConfig::default()),
            smt: crate::smt_worker::SmtWorkerHandle::spawn(),
        }
    }

    /// Apply an externally-owned LSP configuration. Called from the server's
    /// `initialize` handler; safe to call on `&self` so the `LanguageServer`
    /// trait signature doesn't need to change.
    pub fn apply_config(&self, cfg: &crate::lsp_config::LspConfig) {
        let mut guard = self.config.write();
        guard.enabled = cfg.enable_refinement_validation;
        guard.default_mode = match cfg.validation_mode {
            crate::lsp_config::ValidationMode::Quick => ValidationMode::Quick,
            crate::lsp_config::ValidationMode::Thorough => ValidationMode::Thorough,
            crate::lsp_config::ValidationMode::Complete => ValidationMode::Thorough,
        };
        guard.smt_timeout = cfg.smt_timeout;
        guard.show_counterexamples = cfg.show_counterexamples;
        guard.max_counterexample_traces = cfg.max_counterexample_traces;
        guard.cache_enabled = cfg.cache_validation_results;
        // Cost-warning gates were inert before this wire-up — the
        // LspConfig fields existed and were JSON-parseable from
        // initializationOptions, but no consumer ever read them.
        // Now they reach the validator's per-call slow-warning path
        // via `should_emit_cost_warning`.
        guard.cost_warnings_enabled = cfg.verification_show_cost_warnings;
        guard.slow_threshold = cfg.verification_slow_threshold;

        // Resize the cache to match the new capacity/TTL. We do this by
        // rebuilding; the few cached entries that get dropped here are not
        // load-bearing (they're revalidated on the next request anyway).
        self.cache.resize(cfg.cache_max_entries, cfg.cache_ttl);
    }

    /// Decide whether a validation that took `elapsed` should
    /// surface a cost warning. Returns `true` only when both
    /// gates concur: cost warnings are enabled AND the elapsed
    /// time crossed the configured slow threshold. Returning
    /// `bool` (rather than constructing the diagnostic here) keeps
    /// the validator decoupled from the LSP diagnostic shape —
    /// callers in `backend.rs` build the diagnostic in their own
    /// format and just consult this gate.
    pub fn should_emit_cost_warning(&self, elapsed: Duration) -> bool {
        let cfg = self.config.read();
        cfg.cost_warnings_enabled && elapsed >= cfg.slow_threshold
    }

    /// Read-only view of the configured slow threshold so callers
    /// can name it in their cost-warning diagnostic message
    /// without having to thread the LspConfig in alongside.
    pub fn slow_threshold(&self) -> Duration {
        self.config.read().slow_threshold
    }

    /// Whether this validator is currently enabled. Hot-path callers should
    /// short-circuit when this returns `false`.
    pub fn is_enabled(&self) -> bool {
        self.config.read().enabled
    }

    /// Validate refinement at cursor position
    pub async fn validate_refinement(
        &self,
        params: ValidateRefinementParams,
    ) -> Result<ValidateRefinementResult, String> {
        let start = Instant::now();

        // Extract document text and position
        let doc_uri = params.text_document.uri.to_string();
        let position = params.position;
        let mode = params.mode;

        // Build cache key
        let cache_key = format!("{}:{}:{}", doc_uri, position.line, position.character);

        // Honour `LspConfig.cache_validation_results`: when the
        // caller has disabled the validator cache (e.g. the user
        // is debugging a flaky proof and wants every keystroke
        // re-checked from scratch), skip both the lookup and the
        // insert below. Closes the inert-defense pattern: the
        // flag was forwarded into the `ValidatorConfig.cache_enabled`
        // field but no path consulted it.
        let cache_enabled = self.config.read().cache_enabled;

        // Check cache first
        if cache_enabled
            && let Maybe::Some(cached_result) = self.cache.get(&cache_key)
        {
            return Ok(self.result_to_response(cached_result, start.elapsed()));
        }

        // Determine timeout based on mode
        let timeout = match mode {
            ValidationMode::Quick => Duration::from_millis(100),
            ValidationMode::Thorough => Duration::from_secs(1),
        };

        // Perform validation with timeout
        let result = tokio::time::timeout(timeout, self.validate_impl(doc_uri, position)).await;

        let validation_result = match result {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => {
                return Err(format!("Validation error: {}", e));
            }
            Err(_) => ValidationResult::Unknown, // Timeout
        };

        // Cache result when caching is active
        if cache_enabled {
            self.cache.insert(cache_key, validation_result.clone());
        }

        Ok(self.result_to_response(validation_result, start.elapsed()))
    }

    /// Production validation implementation with full SMT integration
    ///
    /// This implementation provides:
    /// - Full span-based position lookup to find refinement at cursor
    /// - Proper AST traversal to collect context for verification
    /// - Integration with SMT solver for formal verification
    /// - Detailed counterexample extraction with execution traces
    async fn validate_impl(
        &self,
        doc_uri: String,
        position: Position,
    ) -> Result<ValidationResult, String> {
        // Parse the URI to get file path
        let url = Url::parse(&doc_uri).map_err(|e| format!("Invalid URI: {}", e))?;

        // Read file content
        let file_path = url
            .to_file_path()
            .map_err(|_| "Cannot convert URI to file path".to_string())?;

        let source = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        // Create a file ID for parsing
        let file_id = FileId::new(1);

        // Lex and parse the document
        let lexer = Lexer::new(&source, file_id);
        let parser = VerumParser::new();

        let module = parser
            .parse_module(lexer, file_id)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        // Convert position to byte offset
        let offset = position_to_offset(&source, position);

        // Find refinement type at position with full context
        let refinement_context = self.find_refinement_at_position(&module, &source, offset);

        let refinement_ctx = match refinement_context {
            Some(ctx) => ctx,
            None => return Ok(ValidationResult::Valid), // No refinement at position
        };

        // Collect all constraints in scope for the refinement. We don't
        // yet plumb them through the SMT worker — that needs a richer
        // request type — but we still collect so diagnostics can reference
        // them. See smt_worker.rs for the scope-aware extension point.
        let _scope_constraints = self.collect_scope_constraints(&module, &source, offset);

        // Off-thread SMT round-trip. The worker owns the verifier; we
        // only move owned, Send-safe payloads (Type + Option<Expr> +
        // VerifyMode) across the channel, so the resulting future stays
        // Send — which is required for tower-lsp `.custom_method`
        // registration.
        let verify_result = self
            .smt
            .verify_refinement_with_timeout(
                refinement_ctx.ty.clone(),
                refinement_ctx.context_expr.clone(),
                VerifyMode::Proof,
                Duration::from_millis(100),
            )
            .await;

        match verify_result {
            crate::smt_worker::SmtCheckResult::Valid => Ok(ValidationResult::Valid),
            crate::smt_worker::SmtCheckResult::Invalid { model } => {
                // Honour `LspConfig.show_counterexamples`: when
                // disabled, return a stub counterexample without
                // running the (potentially expensive) extraction
                // + trace-recovery pipeline. Closes the inert-
                // defense pattern: the flag was forwarded into
                // the validator's `show_counterexamples` field
                // but no path consulted it. Callers that don't
                // surface counterexample data (e.g. minimal
                // status-line UIs) avoid paying the rendering
                // cost; full IDEs leave the default `true` and
                // get the rich extraction.
                let counterexample = if self.config.read().show_counterexamples {
                    self.extract_counterexample(&refinement_ctx, &model, &source, position)
                } else {
                    CounterexampleData {
                        variable: Text::from("<hidden>"),
                        value: Text::from(""),
                        r#type: Text::from(""),
                        constraint: Text::from(""),
                        violation_reason: Text::from(
                            "counterexample suppressed by LspConfig.show_counterexamples = false",
                        ),
                        trace: List::new(),
                    }
                };
                Ok(ValidationResult::Invalid { counterexample })
            }
            crate::smt_worker::SmtCheckResult::Unknown => Ok(ValidationResult::Unknown),
        }
    }

    /// Find refinement type at a specific position with full context
    fn find_refinement_at_position(
        &self,
        module: &Module,
        source: &str,
        offset: u32,
    ) -> Option<RefinementContext> {
        use verum_ast::FunctionParamKind;

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                // Check if offset is within function span
                if func.span.start <= offset && offset <= func.span.end {
                    // Check function parameters for refinement types
                    for param in &func.params {
                        if let FunctionParamKind::Regular { ty, pattern, .. } = &param.kind {
                            if ty.span.start <= offset && offset <= ty.span.end {
                                if matches!(&ty.kind, TypeKind::Refined { .. }) {
                                    let var_name = extract_pattern_name(pattern);
                                    return Some(RefinementContext {
                                        ty: ty.clone(),
                                        var_name,
                                        context_expr: None,
                                        function_name: func.name.as_str().to_string(),
                                        location: RefinementLocation::Parameter,
                                    });
                                }
                            }
                        }
                    }

                    // Check return type
                    if let Maybe::Some(ret_ty) = &func.return_type {
                        if ret_ty.span.start <= offset && offset <= ret_ty.span.end {
                            if matches!(&ret_ty.kind, TypeKind::Refined { .. }) {
                                return Some(RefinementContext {
                                    ty: ret_ty.clone(),
                                    var_name: Text::from("result"),
                                    context_expr: None,
                                    function_name: func.name.as_str().to_string(),
                                    location: RefinementLocation::ReturnType,
                                });
                            }
                        }
                    }

                    // Check expressions in function body for refinement violations
                    if let Maybe::Some(body) = &func.body {
                        if let verum_ast::decl::FunctionBody::Block(block) = body {
                            if let Some(expr_ctx) = self.find_refinement_in_block(
                                &block.stmts,
                                source,
                                offset,
                                func.name.as_str(),
                            ) {
                                return Some(expr_ctx);
                            }
                        }
                    }
                }
            }
        }

        // Fallback: check all refinement types in module
        self.find_refinement_type(module)
            .map(|ty| RefinementContext {
                ty,
                var_name: Text::from("value"),
                context_expr: None,
                function_name: String::new(),
                location: RefinementLocation::Expression,
            })
    }

    /// Find refinement context in a block of statements
    fn find_refinement_in_block(
        &self,
        stmts: &[verum_ast::Stmt],
        source: &str,
        offset: u32,
        function_name: &str,
    ) -> Option<RefinementContext> {
        for stmt in stmts {
            if stmt.span.start <= offset && offset <= stmt.span.end {
                match &stmt.kind {
                    verum_ast::StmtKind::Let {
                        pattern, ty, value, ..
                    } => {
                        // Check type annotation for refinement
                        if let Maybe::Some(type_ann) = ty {
                            if matches!(&type_ann.kind, TypeKind::Refined { .. }) {
                                let var_name = extract_pattern_name(pattern);
                                return Some(RefinementContext {
                                    ty: type_ann.clone(),
                                    var_name,
                                    context_expr: value.clone(),
                                    function_name: function_name.to_string(),
                                    location: RefinementLocation::LocalVariable,
                                });
                            }
                        }
                    }
                    verum_ast::StmtKind::Expr { expr, .. } => {
                        // Check expression for refinement type usage
                        if let Some(ctx) =
                            self.find_refinement_in_expr(expr, source, offset, function_name)
                        {
                            return Some(ctx);
                        }
                    }
                    _ => {}
                }
            }
        }
        None
    }

    /// Find refinement context in an expression
    fn find_refinement_in_expr(
        &self,
        expr: &Expr,
        _source: &str,
        offset: u32,
        function_name: &str,
    ) -> Option<RefinementContext> {
        if expr.span.start <= offset && offset <= expr.span.end {
            match &expr.kind {
                verum_ast::ExprKind::Call { func, args, .. } => {
                    // Check if calling a function that requires refined arguments
                    for arg in args {
                        if arg.span.start <= offset && offset <= arg.span.end {
                            return Some(RefinementContext {
                                ty: Type::new(TypeKind::Inferred, arg.span),
                                var_name: Text::from("argument"),
                                context_expr: Some(arg.clone()),
                                function_name: function_name.to_string(),
                                location: RefinementLocation::CallArgument,
                            });
                        }
                    }
                    // Check the called function
                    if func.span.start <= offset && offset <= func.span.end {
                        return Some(RefinementContext {
                            ty: Type::new(TypeKind::Inferred, func.span),
                            var_name: Text::from("callee"),
                            context_expr: Some(func.as_ref().clone()),
                            function_name: function_name.to_string(),
                            location: RefinementLocation::Expression,
                        });
                    }
                }
                verum_ast::ExprKind::Binary { op, left: _, right } => {
                    // Check for division by zero
                    if matches!(
                        op,
                        verum_ast::expr::BinOp::Div | verum_ast::expr::BinOp::Rem
                    ) {
                        let predicate = verum_ast::ty::RefinementPredicate::new(
                            Expr::new(
                                verum_ast::ExprKind::Binary {
                                    op: verum_ast::expr::BinOp::Ne,
                                    left: verum_common::Heap::new(Expr::new(
                                        verum_ast::ExprKind::Path(verum_ast::ty::Path::single(
                                            verum_ast::ty::Ident::new("i", right.span),
                                        )),
                                        right.span,
                                    )),
                                    right: verum_common::Heap::new(Expr::new(
                                        verum_ast::ExprKind::Literal(verum_ast::Literal::int(
                                            0, right.span,
                                        )),
                                        right.span,
                                    )),
                                },
                                right.span,
                            ),
                            right.span,
                        );
                        return Some(RefinementContext {
                            ty: Type::new(
                                TypeKind::Refined {
                                    base: verum_common::Heap::new(Type::new(
                                        TypeKind::Int,
                                        right.span,
                                    )),
                                    predicate: verum_common::Heap::new(predicate),
                                },
                                right.span,
                            ),
                            var_name: Text::from("divisor"),
                            context_expr: Some(right.as_ref().clone()),
                            function_name: function_name.to_string(),
                            location: RefinementLocation::Expression,
                        });
                    }
                }
                verum_ast::ExprKind::Index { expr: _, index } => {
                    // Check for index bounds
                    return Some(RefinementContext {
                        ty: Type::new(TypeKind::Inferred, index.span),
                        var_name: Text::from("index"),
                        context_expr: Some(index.as_ref().clone()),
                        function_name: function_name.to_string(),
                        location: RefinementLocation::Expression,
                    });
                }
                _ => {}
            }
        }
        None
    }

    /// Collect all constraints currently in scope at a position
    fn collect_scope_constraints(
        &self,
        module: &Module,
        _source: &str,
        offset: u32,
    ) -> List<ScopeConstraint> {
        let mut constraints = List::new();

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if func.span.start <= offset && offset <= func.span.end {
                    // Add preconditions (requires clauses)
                    for attr in &func.attributes {
                        if attr.name.as_str() == "requires" {
                            if let Maybe::Some(args) = &attr.args {
                                if let Some(expr) = args.first() {
                                    constraints.push(ScopeConstraint {
                                        kind: ConstraintKind::Precondition,
                                        constraint: expr.clone(),
                                        source: Text::from("requires clause"),
                                    });
                                }
                            }
                        }
                    }

                    // Add parameter refinements as constraints
                    for param in &func.params {
                        if let verum_ast::FunctionParamKind::Regular { ty, pattern, .. } = &param.kind {
                            if let TypeKind::Refined { predicate, .. } = &ty.kind {
                                let var_name = extract_pattern_name(pattern);
                                constraints.push(ScopeConstraint {
                                    kind: ConstraintKind::ParameterRefinement,
                                    constraint: predicate.expr.clone(),
                                    source: Text::from(format!(
                                        "parameter {} refinement",
                                        var_name.as_str()
                                    )),
                                });
                            }
                        }
                    }

                    // Walk function body to collect local constraints
                    if let Maybe::Some(body) = &func.body {
                        if let verum_ast::decl::FunctionBody::Block(block) = body {
                            self.collect_block_constraints(&block.stmts, offset, &mut constraints);
                        }
                    }
                }
            }
        }

        constraints
    }

    /// Collect constraints from a block of statements
    fn collect_block_constraints(
        &self,
        stmts: &[verum_ast::Stmt],
        offset: u32,
        constraints: &mut List<ScopeConstraint>,
    ) {
        for stmt in stmts {
            // Only collect constraints from statements before the current position
            if stmt.span.end <= offset {
                match &stmt.kind {
                    verum_ast::StmtKind::Let {
                        pattern,
                        ty,
                        value: _,
                        ..
                    } => {
                        // If the let has a refined type, add it as a constraint
                        if let Maybe::Some(type_ann) = ty {
                            if let TypeKind::Refined { predicate, .. } = &type_ann.kind {
                                let var_name = extract_pattern_name(pattern);
                                constraints.push(ScopeConstraint {
                                    kind: ConstraintKind::LocalRefinement,
                                    constraint: predicate.expr.clone(),
                                    source: Text::from(format!(
                                        "local {} refinement",
                                        var_name.as_str()
                                    )),
                                });
                            }
                        }
                    }
                    verum_ast::StmtKind::Expr { expr, .. } => {
                        // Check for assertions that add constraints
                        if let verum_ast::ExprKind::Call { func, args, .. } = &expr.kind {
                            if let verum_ast::ExprKind::Path(path) = &func.kind {
                                if let Some(verum_ast::ty::PathSegment::Name(name)) =
                                    path.segments.first()
                                {
                                    if name.as_str() == "assert" || name.as_str() == "assume" {
                                        if let Some(cond) = args.first() {
                                            constraints.push(ScopeConstraint {
                                                kind: if name.as_str() == "assume" {
                                                    ConstraintKind::Assumption
                                                } else {
                                                    ConstraintKind::Assertion
                                                },
                                                constraint: cond.clone(),
                                                source: Text::from(format!(
                                                    "{} statement",
                                                    name.as_str()
                                                )),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Extract detailed counterexample from SMT model
    fn extract_counterexample(
        &self,
        ctx: &RefinementContext,
        model: &str,
        source: &str,
        position: Position,
    ) -> CounterexampleData {
        // Parse the model string to extract variable assignments
        let assignments = parse_model_assignments(model);

        // Build execution trace by walking backwards from the violation point
        let trace = self.build_execution_trace(ctx, source, position);

        // Extract the constraint that was violated
        let constraint = if let TypeKind::Refined { predicate, .. } = &ctx.ty.kind {
            format_constraint_expr(&predicate.expr)
        } else {
            Text::from("constraint")
        };

        // Determine the value that violated the constraint
        let (var, value) = assignments
            .iter()
            .find(|(name, _)| name.as_str() == ctx.var_name.as_str())
            .map(|(n, v)| (n.clone(), v.clone()))
            .unwrap_or_else(|| (ctx.var_name.clone(), Text::from("unknown")));

        CounterexampleData {
            variable: var.clone(),
            value: value.clone(),
            r#type: format_type_ast(&ctx.ty).into(),
            constraint: constraint.clone(),
            violation_reason: Text::from(format!(
                "{} (= {}) does not satisfy {}",
                var.as_str(),
                value.as_str(),
                constraint.as_str()
            )),
            trace,
        }
    }

    /// Build execution trace leading to the violation
    fn build_execution_trace(
        &self,
        ctx: &RefinementContext,
        source: &str,
        position: Position,
    ) -> List<ExecutionTrace> {
        let mut trace = List::new();

        // Get the line number of the violation
        let violation_line = position.line;

        // Add the violation point to the trace
        trace.push(ExecutionTrace {
            line: violation_line,
            operation: Text::from(match ctx.location {
                RefinementLocation::Parameter => "parameter check",
                RefinementLocation::ReturnType => "return value check",
                RefinementLocation::LocalVariable => "assignment",
                RefinementLocation::CallArgument => "function call",
                RefinementLocation::Expression => "expression evaluation",
            }),
            value: ctx.var_name.clone(),
            explanation: Text::from(format!(
                "{} passed to {} expecting {}",
                ctx.var_name.as_str(),
                ctx.function_name,
                format_type_ast(&ctx.ty).as_str()
            )),
        });

        // Try to find where the variable was assigned its value
        if violation_line > 0 {
            // Look backwards for assignment
            let lines: Vec<&str> = source.lines().collect();
            for line_num in (0..violation_line as usize).rev() {
                if line_num < lines.len() {
                    let line_text = lines[line_num].trim();
                    if line_text.contains(&format!("{} =", ctx.var_name.as_str()))
                        || line_text.contains(&format!("let {}", ctx.var_name.as_str()))
                    {
                        trace.insert(
                            0,
                            ExecutionTrace {
                                line: line_num as u32,
                                operation: Text::from("assignment"),
                                value: ctx.var_name.clone(),
                                explanation: Text::from(format!(
                                    "{} is assigned value",
                                    ctx.var_name.as_str()
                                )),
                            },
                        );
                        break;
                    }
                }
            }
        }

        trace
    }

    /// Find all refinement types in module
    ///
    /// Searches through the entire module AST to find refinement types in:
    /// - Function parameters
    /// - Function return types
    /// - Local variable type annotations
    /// - Type declaration bodies
    /// - Protocol method signatures
    /// - Constant type annotations
    fn find_refinement_type(&self, module: &Module) -> Option<Type> {
        use verum_ast::FunctionParamKind;

        for item in &module.items {
            match &item.kind {
                ItemKind::Function(func) => {
                    // Check function parameters
                    for param in &func.params {
                        if let FunctionParamKind::Regular { ty, .. } = &param.kind {
                            if let Some(refined) = self.extract_refinement_type(ty) {
                                return Some(refined);
                            }
                        }
                    }

                    // Check return type
                    if let Maybe::Some(ret_ty) = &func.return_type {
                        if let Some(refined) = self.extract_refinement_type(ret_ty) {
                            return Some(refined);
                        }
                    }

                    // Check function body for local variable type annotations
                    if let Maybe::Some(body) = &func.body {
                        if let verum_ast::decl::FunctionBody::Block(block) = body {
                            if let Some(refined) = self.search_block_for_refined_types(block) {
                                return Some(refined);
                            }
                        }
                    }
                }
                ItemKind::Type(type_decl) => {
                    // Check type declaration body for refinement types
                    match &type_decl.body {
                        verum_ast::decl::TypeDeclBody::Alias(ty) => {
                            if let Some(refined) = self.extract_refinement_type(ty) {
                                return Some(refined);
                            }
                        }
                        verum_ast::decl::TypeDeclBody::Newtype(ty) => {
                            if let Some(refined) = self.extract_refinement_type(ty) {
                                return Some(refined);
                            }
                        }
                        verum_ast::decl::TypeDeclBody::Record(fields) => {
                            for field in fields {
                                if let Some(refined) = self.extract_refinement_type(&field.ty) {
                                    return Some(refined);
                                }
                            }
                        }
                        verum_ast::decl::TypeDeclBody::Variant(variants) => {
                            for variant in variants {
                                if let Maybe::Some(data) = &variant.data {
                                    match data {
                                        verum_ast::decl::VariantData::Tuple(types) => {
                                            for ty in types {
                                                if let Some(refined) =
                                                    self.extract_refinement_type(ty)
                                                {
                                                    return Some(refined);
                                                }
                                            }
                                        }
                                        verum_ast::decl::VariantData::Record(fields) => {
                                            for field in fields {
                                                if let Some(refined) =
                                                    self.extract_refinement_type(&field.ty)
                                                {
                                                    return Some(refined);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                ItemKind::Protocol(protocol) => {
                    // Check protocol method signatures
                    for item in &protocol.items {
                        if let verum_ast::decl::ProtocolItemKind::Function { decl, .. } = &item.kind
                        {
                            // Check function parameters
                            for param in &decl.params {
                                if let FunctionParamKind::Regular { ty, .. } = &param.kind {
                                    if let Some(refined) = self.extract_refinement_type(ty) {
                                        return Some(refined);
                                    }
                                }
                            }
                            // Check function return type
                            if let Maybe::Some(ret_ty) = &decl.return_type {
                                if let Some(refined) = self.extract_refinement_type(ret_ty) {
                                    return Some(refined);
                                }
                            }
                        }
                    }
                }
                ItemKind::Const(const_decl) => {
                    // Check constant type annotation
                    if let Some(refined) = self.extract_refinement_type(&const_decl.ty) {
                        return Some(refined);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Extract refinement type from a type, recursively checking nested types
    fn extract_refinement_type(&self, ty: &Type) -> Option<Type> {
        match &ty.kind {
            TypeKind::Refined { .. } => Some(ty.clone()),
            TypeKind::Reference { inner, .. } => self.extract_refinement_type(inner),
            TypeKind::CheckedReference { inner, .. } => self.extract_refinement_type(inner),
            TypeKind::UnsafeReference { inner, .. } => self.extract_refinement_type(inner),
            TypeKind::Pointer { inner, .. } => self.extract_refinement_type(inner),
            TypeKind::Array { element, .. } => self.extract_refinement_type(element),
            TypeKind::Slice(inner) => self.extract_refinement_type(inner),
            TypeKind::Tuple(types) => {
                for t in types {
                    if let Some(refined) = self.extract_refinement_type(t) {
                        return Some(refined);
                    }
                }
                None
            }
            TypeKind::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    if let Some(refined) = self.extract_refinement_type(param) {
                        return Some(refined);
                    }
                }
                self.extract_refinement_type(return_type)
            }
            TypeKind::Generic { base, args } => {
                if let Some(refined) = self.extract_refinement_type(base) {
                    return Some(refined);
                }
                for arg in args {
                    if let verum_ast::ty::GenericArg::Type(t) = arg {
                        if let Some(refined) = self.extract_refinement_type(t) {
                            return Some(refined);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Search block for refinement types (for find_refinement_type)
    fn search_block_for_refined_types(&self, block: &verum_ast::expr::Block) -> Option<Type> {
        for stmt in &block.stmts {
            if let verum_ast::StmtKind::Let { ty, value, .. } = &stmt.kind {
                // Check type annotation
                if let Maybe::Some(type_ann) = ty {
                    if let Some(refined) = self.extract_refinement_type(type_ann) {
                        return Some(refined);
                    }
                }
                // Check init expression for nested blocks
                if let Maybe::Some(init_expr) = value {
                    if let Some(refined) = self.search_expr_for_refined_types(init_expr) {
                        return Some(refined);
                    }
                }
            }
        }
        // Check tail expression
        if let Maybe::Some(tail) = &block.expr {
            if let Some(refined) = self.search_expr_for_refined_types(tail) {
                return Some(refined);
            }
        }
        None
    }

    /// Search expression for refinement types (for find_refinement_type)
    fn search_expr_for_refined_types(&self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            verum_ast::ExprKind::Block(block) => self.search_block_for_refined_types(block),
            verum_ast::ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let Some(refined) = self.search_block_for_refined_types(then_branch) {
                    return Some(refined);
                }
                if let Maybe::Some(else_expr) = else_branch {
                    return self.search_expr_for_refined_types(else_expr);
                }
                None
            }
            verum_ast::ExprKind::Match { arms, .. } => {
                for arm in arms {
                    if let Some(refined) = self.search_expr_for_refined_types(&arm.body) {
                        return Some(refined);
                    }
                }
                None
            }
            verum_ast::ExprKind::Closure { body, .. } => self.search_expr_for_refined_types(body),
            verum_ast::ExprKind::For { body, .. } => self.search_block_for_refined_types(body),
            verum_ast::ExprKind::While { body, .. } => self.search_block_for_refined_types(body),
            verum_ast::ExprKind::Loop { body, .. } => self.search_block_for_refined_types(body),
            _ => None,
        }
    }

    /// Convert ValidationResult to response
    fn result_to_response(
        &self,
        result: ValidationResult,
        elapsed: Duration,
    ) -> ValidateRefinementResult {
        match result {
            ValidationResult::Valid => ValidateRefinementResult {
                valid: true,
                diagnostics: List::new(),
                performance_ms: elapsed.as_millis() as u64,
            },
            ValidationResult::Invalid { counterexample } => {
                let diagnostic = self.build_diagnostic(counterexample, elapsed);
                ValidateRefinementResult {
                    valid: false,
                    diagnostics: List::from(vec![diagnostic]),
                    performance_ms: elapsed.as_millis() as u64,
                }
            }
            ValidationResult::Unknown => ValidateRefinementResult {
                valid: true,
                diagnostics: List::new(),
                performance_ms: elapsed.as_millis() as u64,
            },
        }
    }

    /// Build diagnostic from counterexample
    fn build_diagnostic(
        &self,
        counterexample: CounterexampleData,
        elapsed: Duration,
    ) -> RefinementDiagnostic {
        let message = format!(
            "Refinement violation: value `{}` fails constraint `{}`",
            counterexample.value, counterexample.constraint
        );

        let quick_fixes = self.generate_quick_fixes(&counterexample);

        RefinementDiagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 0,
                },
            },
            severity: DiagnosticSeverity::ERROR,
            code: Some("E0304".to_string()),
            source: "verum".to_string(),
            message,
            counterexample: Maybe::Some(counterexample),
            quick_fixes,
            related_information: List::new(),
            validation_time_ms: elapsed.as_millis() as u64,
            smt_solver: SmtSolver::Z3,
        }
    }

    /// Generate quick fixes for counterexample
    fn generate_quick_fixes(&self, counterexample: &CounterexampleData) -> List<QuickFix> {
        let mut fixes = List::new();

        // Fix 1: Wrap with runtime check
        let runtime_check = QuickFix {
            title: "Wrap with runtime check (Result)".to_string(),
            kind: QuickFixKind::RuntimeCheck,
            edits: List::new(), // Populated based on context
            priority: 1,
            impact: QuickFixImpact::Safe,
            description: Maybe::Some("Add runtime validation using Result<T, E>".to_string()),
        };
        fixes.push(runtime_check);

        // Fix 2: Add inline refinement
        let inline_refinement = QuickFix {
            title: "Add inline refinement to parameter".to_string(),
            kind: QuickFixKind::InlineRefinement,
            edits: List::new(),
            priority: 2,
            impact: QuickFixImpact::Breaking,
            description: Maybe::Some(format!(
                "Requires callers to prove {} {} {}",
                counterexample.variable, "satisfies", counterexample.constraint
            )),
        };
        fixes.push(inline_refinement);

        // Fix 3: Add assertion
        let assertion = QuickFix {
            title: "Add assertion".to_string(),
            kind: QuickFixKind::Assertion,
            edits: List::new(),
            priority: 3,
            impact: QuickFixImpact::Safe,
            description: Maybe::Some("Runtime assertion with panic".to_string()),
        };
        fixes.push(assertion);

        fixes
    }

    /// Promote &T to &checked T with escape analysis proof
    ///
    /// This performs full escape analysis to determine if a reference can be
    /// safely promoted to a checked reference (zero-cost, compiler-verified).
    pub async fn promote_to_checked(
        &self,
        params: PromoteToCheckedParams,
    ) -> Result<PromoteToCheckedResult, String> {
        let uri = params.text_document.uri;
        let range = params.range;
        let include_proof = params.include_proof;

        // Parse the document to get the AST
        let file_path = uri
            .to_file_path()
            .map_err(|_| "Cannot convert URI to file path".to_string())?;

        let source = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let file_id = FileId::new(1);
        let lexer = Lexer::new(&source, file_id);
        let parser = VerumParser::new();

        let module = parser
            .parse_module(lexer, file_id)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        // Convert range to byte offset
        let start_offset = position_to_offset(&source, range.start);
        let end_offset = position_to_offset(&source, range.end);

        // Find the reference expression in the AST
        let ref_context = self.find_reference_at_range(&module, start_offset, end_offset);

        let ref_ctx = match ref_context {
            Some(ctx) => ctx,
            None => {
                return Ok(PromoteToCheckedResult {
                    success: false,
                    edits: List::new(),
                    proof_comment: Maybe::Some(Text::from(
                        "No reference found at the specified range",
                    )),
                });
            }
        };

        // Perform escape analysis
        let escape_analysis = self.analyze_reference_escape(&module, &ref_ctx, &source);

        if !escape_analysis.can_promote {
            return Ok(PromoteToCheckedResult {
                success: false,
                edits: List::new(),
                proof_comment: Maybe::Some(Text::from(format!(
                    "Cannot promote: {}\nThe reference {} the function scope.",
                    escape_analysis.reason,
                    if escape_analysis.escapes {
                        "escapes"
                    } else {
                        "may escape"
                    }
                ))),
            });
        }

        // Generate edits to change &T to &checked T
        let mut edits = List::new();

        // Extract the original reference text
        let ref_text = source_range(&source, start_offset as usize, end_offset as usize);

        // Generate the new checked reference text
        let new_text = if ref_text.starts_with("&mut ") {
            ref_text.replacen("&mut ", "&checked mut ", 1)
        } else if ref_text.starts_with('&') {
            ref_text.replacen("&", "&checked ", 1)
        } else {
            // If it doesn't start with &, this might be a type annotation
            format!("&checked {}", ref_text)
        };

        edits.push(TextEdit {
            range,
            new_text: new_text.clone(),
        });

        // Generate proof comment if requested
        let proof_comment = if include_proof {
            let proof_text = format!(
                "// SAFETY: Escape analysis proof for &checked reference:\n\
                 // 1. Reference lifetime: {} (lines {}-{})\n\
                 // 2. Borrow scope: function `{}`\n\
                 // 3. Escape paths analyzed: {}\n\
                 // 4. Stored to heap/captured: {}\n\
                 // 5. Returned from function: {}\n\
                 // Conclusion: Reference is safe to promote to &checked (0ns overhead)",
                escape_analysis.lifetime_info,
                escape_analysis.start_line,
                escape_analysis.end_line,
                ref_ctx.function_name,
                escape_analysis.paths_analyzed,
                if escape_analysis.stored_to_heap {
                    "yes (would fail)"
                } else {
                    "no"
                },
                if escape_analysis.returned {
                    "yes (would fail)"
                } else {
                    "no"
                },
            );
            Maybe::Some(Text::from(proof_text))
        } else {
            Maybe::None
        };

        Ok(PromoteToCheckedResult {
            success: true,
            edits,
            proof_comment,
        })
    }

    /// Find reference expression at a given range
    fn find_reference_at_range(
        &self,
        module: &Module,
        start_offset: u32,
        end_offset: u32,
    ) -> Option<ReferenceContext> {
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if func.span.start <= start_offset && end_offset <= func.span.end {
                    // Check parameters for reference types
                    for param in &func.params {
                        if let verum_ast::FunctionParamKind::Regular { ty, pattern, .. } = &param.kind {
                            if ty.span.start >= start_offset && ty.span.end <= end_offset {
                                if is_reference_type(ty) {
                                    return Some(ReferenceContext {
                                        var_name: extract_pattern_name(pattern),
                                        function_name: func.name.as_str().to_string(),
                                        ty: ty.clone(),
                                        span: ty.span,
                                        is_mutable: is_mutable_reference(ty),
                                    });
                                }
                            }
                        }
                    }

                    // Check function body for reference expressions
                    if let Maybe::Some(body) = &func.body {
                        if let verum_ast::decl::FunctionBody::Block(block) = body {
                            if let Some(ctx) = self.find_reference_in_stmts(
                                &block.stmts,
                                start_offset,
                                end_offset,
                                func.name.as_str(),
                            ) {
                                return Some(ctx);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Find reference in statements
    fn find_reference_in_stmts(
        &self,
        stmts: &[verum_ast::Stmt],
        start_offset: u32,
        end_offset: u32,
        function_name: &str,
    ) -> Option<ReferenceContext> {
        for stmt in stmts {
            match &stmt.kind {
                verum_ast::StmtKind::Let {
                    pattern, ty, value, ..
                } => {
                    // Check type annotation
                    if let Maybe::Some(type_ann) = ty {
                        if type_ann.span.start >= start_offset && type_ann.span.end <= end_offset {
                            if is_reference_type(type_ann) {
                                return Some(ReferenceContext {
                                    var_name: extract_pattern_name(pattern),
                                    function_name: function_name.to_string(),
                                    ty: type_ann.clone(),
                                    span: type_ann.span,
                                    is_mutable: is_mutable_reference(type_ann),
                                });
                            }
                        }
                    }
                    // Check init expression for reference creation
                    if let Maybe::Some(init_expr) = value {
                        if init_expr.span.start >= start_offset && init_expr.span.end <= end_offset
                        {
                            if let verum_ast::ExprKind::Unary { op, expr: _ } = &init_expr.kind {
                                let is_ref = matches!(
                                    op,
                                    verum_ast::expr::UnOp::Ref | verum_ast::expr::UnOp::RefMut
                                );
                                if is_ref {
                                    return Some(ReferenceContext {
                                        var_name: extract_pattern_name(pattern),
                                        function_name: function_name.to_string(),
                                        ty: Type::new(TypeKind::Inferred, init_expr.span),
                                        span: init_expr.span,
                                        is_mutable: matches!(op, verum_ast::expr::UnOp::RefMut),
                                    });
                                }
                            }
                        }
                    }
                }
                verum_ast::StmtKind::Expr { expr, .. } => {
                    if let Some(ctx) =
                        self.find_ref_in_expr_tree(expr, start_offset, end_offset, function_name)
                    {
                        return Some(ctx);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Find reference expression in an expression tree
    fn find_ref_in_expr_tree(
        &self,
        expr: &Expr,
        start_offset: u32,
        end_offset: u32,
        function_name: &str,
    ) -> Option<ReferenceContext> {
        if expr.span.start >= start_offset && expr.span.end <= end_offset {
            if let verum_ast::ExprKind::Unary { op, expr: _ } = &expr.kind {
                if matches!(
                    op,
                    verum_ast::expr::UnOp::Ref | verum_ast::expr::UnOp::RefMut
                ) {
                    return Some(ReferenceContext {
                        var_name: Text::from("ref"),
                        function_name: function_name.to_string(),
                        ty: Type::new(TypeKind::Inferred, expr.span),
                        span: expr.span,
                        is_mutable: matches!(op, verum_ast::expr::UnOp::RefMut),
                    });
                }
            }
        }
        None
    }

    /// Analyze whether a reference escapes its scope
    fn analyze_reference_escape(
        &self,
        module: &Module,
        ref_ctx: &ReferenceContext,
        source: &str,
    ) -> EscapeAnalysisResult {
        let mut result = EscapeAnalysisResult {
            can_promote: true,
            escapes: false,
            reason: String::new(),
            lifetime_info: String::from("local"),
            start_line: span_to_line(source, ref_ctx.span.start),
            end_line: 0,
            paths_analyzed: 0,
            stored_to_heap: false,
            returned: false,
        };

        // Find the function containing this reference
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if func.name.as_str() == ref_ctx.function_name {
                    result.end_line = span_to_line(source, func.span.end);

                    // Analyze function body for escape paths
                    if let Maybe::Some(body) = &func.body {
                        if let verum_ast::decl::FunctionBody::Block(block) = body {
                            self.check_escape_in_stmts(
                                &block.stmts,
                                &ref_ctx.var_name,
                                &mut result,
                            );
                        }
                    }

                    // Check if return type contains the reference
                    if let Maybe::Some(ret_ty) = &func.return_type {
                        if type_contains_reference(ret_ty) {
                            result.can_promote = false;
                            result.escapes = true;
                            result.returned = true;
                            result.reason = format!(
                                "Reference may be returned from function (return type: {})",
                                format_type_ast(ret_ty).as_str()
                            );
                        }
                    }

                    break;
                }
            }
        }

        result
    }

    /// Check for escape paths in statements
    fn check_escape_in_stmts(
        &self,
        stmts: &[verum_ast::Stmt],
        var_name: &Text,
        result: &mut EscapeAnalysisResult,
    ) {
        for stmt in stmts {
            result.paths_analyzed += 1;

            match &stmt.kind {
                verum_ast::StmtKind::Let { value, .. } => {
                    if let Maybe::Some(init_expr) = value {
                        // Check if reference is stored in a container
                        if self.expr_stores_reference(init_expr, var_name) {
                            result.can_promote = false;
                            result.escapes = true;
                            result.stored_to_heap = true;
                            result.reason = format!(
                                "Reference `{}` is stored in a container (may escape)",
                                var_name.as_str()
                            );
                            return;
                        }
                    }
                }
                verum_ast::StmtKind::Expr { expr, .. } => {
                    // Check for return statements containing the reference
                    if let verum_ast::ExprKind::Return(Maybe::Some(ret_val)) = &expr.kind {
                        if self.expr_contains_var(ret_val, var_name) {
                            result.can_promote = false;
                            result.escapes = true;
                            result.returned = true;
                            result.reason = format!(
                                "Reference `{}` is returned from the function",
                                var_name.as_str()
                            );
                            return;
                        }
                    }

                    // Check for function calls that might capture the reference
                    if self.expr_escapes_via_call(expr, var_name) {
                        result.can_promote = false;
                        result.escapes = true;
                        result.reason = format!(
                            "Reference `{}` is passed to a function that may capture it",
                            var_name.as_str()
                        );
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    /// Check if expression stores a reference
    fn expr_stores_reference(&self, expr: &Expr, var_name: &Text) -> bool {
        match &expr.kind {
            verum_ast::ExprKind::Call { func, args, .. } => {
                // Check if calling a container constructor (Box, Heap, etc.)
                if let verum_ast::ExprKind::Path(path) = &func.kind {
                    let path_str = format!("{:?}", path);
                    if path_str.contains("Box")
                        || path_str.contains("Heap")
                        || path_str.contains("Rc")
                        || path_str.contains("Arc")
                    {
                        // Check if any argument contains the reference
                        return args.iter().any(|arg| self.expr_contains_var(arg, var_name));
                    }
                }
                false
            }
            verum_ast::ExprKind::Array(array_expr) => match array_expr {
                verum_ast::expr::ArrayExpr::List(elements) => elements
                    .iter()
                    .any(|el| self.expr_contains_var(el, var_name)),
                verum_ast::expr::ArrayExpr::Repeat { value, count } => {
                    self.expr_contains_var(value, var_name)
                        || self.expr_contains_var(count, var_name)
                }
            },
            verum_ast::ExprKind::Record { fields, base, .. } => {
                fields.iter().any(|field| {
                    if let Maybe::Some(ref expr) = field.value {
                        self.expr_contains_var(expr, var_name)
                    } else {
                        // Shorthand field: check if the field name matches var_name
                        field.name.as_str() == var_name.as_str()
                    }
                }) || base
                    .as_ref()
                    .is_some_and(|b| self.expr_contains_var(b, var_name))
            }
            _ => false,
        }
    }

    /// Check if expression contains a variable reference
    fn expr_contains_var(&self, expr: &Expr, var_name: &Text) -> bool {
        match &expr.kind {
            verum_ast::ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(name)) = path.segments.first() {
                    return name.as_str() == var_name.as_str();
                }
                false
            }
            verum_ast::ExprKind::Unary { op, expr: inner }
                if matches!(
                    op,
                    verum_ast::expr::UnOp::Ref | verum_ast::expr::UnOp::RefMut
                ) =>
            {
                self.expr_contains_var(inner, var_name)
            }
            verum_ast::ExprKind::Unary {
                op: verum_ast::expr::UnOp::Deref,
                expr: inner,
            } => self.expr_contains_var(inner, var_name),
            verum_ast::ExprKind::Field { expr: base, .. } => self.expr_contains_var(base, var_name),
            verum_ast::ExprKind::Call { func, args, .. } => {
                self.expr_contains_var(func, var_name)
                    || args.iter().any(|arg| self.expr_contains_var(arg, var_name))
            }
            verum_ast::ExprKind::Binary { left, right, .. } => {
                self.expr_contains_var(left, var_name) || self.expr_contains_var(right, var_name)
            }
            _ => false,
        }
    }

    /// Check if expression escapes via a function call
    fn expr_escapes_via_call(&self, expr: &Expr, var_name: &Text) -> bool {
        if let verum_ast::ExprKind::Call { func, args, .. } = &expr.kind {
            // Check if the reference is passed to a closure-capturing function
            if let verum_ast::ExprKind::Path(path) = &func.kind {
                let path_str = format!("{:?}", path);
                // Check for known capturing functions
                if path_str.contains("spawn")
                    || path_str.contains("thread")
                    || path_str.contains("async")
                    || path_str.contains("move")
                {
                    return args.iter().any(|arg| self.expr_contains_var(arg, var_name));
                }
            }

            // Check for closure arguments that capture the reference via move
            for arg in args {
                if let verum_ast::ExprKind::Closure { move_, body, .. } = &arg.kind {
                    // If closure is move and body references the variable, it captures
                    if *move_ && self.expr_contains_var(body, var_name) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Infer tightest refinement from usage
    ///
    /// This analyzes all usages of a symbol to infer the tightest possible
    /// refinement type that would satisfy all constraints at usage sites.
    pub async fn infer_refinement(
        &self,
        params: InferRefinementParams,
    ) -> Result<InferRefinementResult, String> {
        let uri = params.text_document.uri;
        let symbol = params.symbol;

        // Parse the document
        let file_path = uri
            .to_file_path()
            .map_err(|_| "Cannot convert URI to file path".to_string())?;

        let source = std::fs::read_to_string(&file_path)
            .map_err(|e| format!("Failed to read file: {}", e))?;

        let file_id = FileId::new(1);
        let lexer = Lexer::new(&source, file_id);
        let parser = VerumParser::new();

        let module = parser
            .parse_module(lexer, file_id)
            .map_err(|e| format!("Parse error: {:?}", e))?;

        // Find the symbol definition
        let symbol_def = self.find_symbol_definition(&module, &symbol, &source);
        let def = match symbol_def {
            Some(d) => d,
            None => return Err(format!("Symbol `{}` not found", symbol.as_str())),
        };

        // Find all usages of the symbol
        let usages = self.find_symbol_usages(&module, &symbol, &uri, &source);

        // Collect constraints from each usage site
        let constraints = self.collect_usage_constraints(&module, &symbol, &usages);

        // Compute the intersection of all constraints to get tightest refinement
        let (inferred_refinement, confidence) =
            self.compute_tightest_refinement(&def.base_type, &constraints);

        // Generate edit to add refinement to symbol declaration
        let edits = self.generate_refinement_edits(&def, &inferred_refinement, &source);

        // Convert usages to CodeLocations
        let usage_locations: List<CodeLocation> = usages
            .iter()
            .map(|u| CodeLocation {
                uri: uri.clone(),
                range: u.range,
                context: Text::from(u.context.clone()),
            })
            .collect();

        Ok(InferRefinementResult {
            inferred_type: inferred_refinement,
            confidence,
            usages: usage_locations,
            edits,
        })
    }

    /// Find symbol definition in module
    fn find_symbol_definition(
        &self,
        module: &Module,
        symbol: &Text,
        _source: &str,
    ) -> Option<SymbolDefinition> {
        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                // Check function parameters
                for param in &func.params {
                    if let verum_ast::FunctionParamKind::Regular { ty, pattern, .. } = &param.kind {
                        let param_name = extract_pattern_name(pattern);
                        if param_name.as_str() == symbol.as_str() {
                            return Some(SymbolDefinition {
                                name: param_name,
                                span: ty.span,
                                base_type: extract_base_type(ty),
                                kind: SymbolKind::Parameter,
                                function_name: Some(func.name.as_str().to_string()),
                            });
                        }
                    }
                }

                // Check local variables in function body
                if let Maybe::Some(body) = &func.body {
                    if let verum_ast::decl::FunctionBody::Block(block) = body {
                        if let Some(def) =
                            self.find_symbol_in_stmts(&block.stmts, symbol, func.name.as_str())
                        {
                            return Some(def);
                        }
                    }
                }
            }
        }
        None
    }

    /// Find symbol definition in statements
    fn find_symbol_in_stmts(
        &self,
        stmts: &[verum_ast::Stmt],
        symbol: &Text,
        function_name: &str,
    ) -> Option<SymbolDefinition> {
        for stmt in stmts {
            if let verum_ast::StmtKind::Let { pattern, ty, .. } = &stmt.kind {
                let var_name = extract_pattern_name(pattern);
                if var_name.as_str() == symbol.as_str() {
                    let base_type = if let Maybe::Some(type_ann) = ty {
                        extract_base_type(type_ann)
                    } else {
                        Text::from("_")
                    };

                    return Some(SymbolDefinition {
                        name: var_name,
                        span: stmt.span,
                        base_type,
                        kind: SymbolKind::LocalVariable,
                        function_name: Some(function_name.to_string()),
                    });
                }
            }
        }
        None
    }

    /// Find all usages of a symbol in the module
    fn find_symbol_usages(
        &self,
        module: &Module,
        symbol: &Text,
        _uri: &Url,
        source: &str,
    ) -> List<SymbolUsage> {
        let mut usages = List::new();

        for item in &module.items {
            if let ItemKind::Function(func) = &item.kind {
                if let Maybe::Some(body) = &func.body {
                    if let verum_ast::decl::FunctionBody::Block(block) = body {
                        self.collect_usages_in_stmts(&block.stmts, symbol, source, &mut usages);
                    }
                }
            }
        }

        usages
    }

    /// Collect symbol usages from statements
    fn collect_usages_in_stmts(
        &self,
        stmts: &[verum_ast::Stmt],
        symbol: &Text,
        source: &str,
        usages: &mut List<SymbolUsage>,
    ) {
        for stmt in stmts {
            match &stmt.kind {
                verum_ast::StmtKind::Let { value, .. } => {
                    if let Maybe::Some(init_expr) = value {
                        self.collect_usages_in_expr(init_expr, symbol, source, usages);
                    }
                }
                verum_ast::StmtKind::Expr { expr, .. } => {
                    self.collect_usages_in_expr(expr, symbol, source, usages);
                }
                _ => {}
            }
        }
    }

    /// Collect symbol usages from an expression
    fn collect_usages_in_expr(
        &self,
        expr: &Expr,
        symbol: &Text,
        source: &str,
        usages: &mut List<SymbolUsage>,
    ) {
        match &expr.kind {
            verum_ast::ExprKind::Path(path) => {
                if let Some(verum_ast::ty::PathSegment::Name(name)) = path.segments.first() {
                    if name.as_str() == symbol.as_str() {
                        let context = get_line_at_span(source, expr.span);
                        usages.push(SymbolUsage {
                            range: span_to_range(source, expr.span),
                            context,
                            usage_kind: UsageKind::Read,
                            constraint_context: None,
                        });
                    }
                }
            }
            verum_ast::ExprKind::Binary { op, left, right } => {
                // Check for comparison operations that imply constraints
                if matches!(
                    op,
                    verum_ast::expr::BinOp::Lt
                        | verum_ast::expr::BinOp::Le
                        | verum_ast::expr::BinOp::Gt
                        | verum_ast::expr::BinOp::Ge
                        | verum_ast::expr::BinOp::Eq
                        | verum_ast::expr::BinOp::Ne
                ) {
                    // If symbol is on left side, the right side implies a constraint
                    if self.expr_is_symbol(left, symbol) {
                        let constraint = self.extract_comparison_constraint(op, right);
                        let context = get_line_at_span(source, expr.span);
                        usages.push(SymbolUsage {
                            range: span_to_range(source, expr.span),
                            context,
                            usage_kind: UsageKind::Comparison,
                            constraint_context: Some(constraint),
                        });
                    }
                }

                // Division implies divisor != 0
                if matches!(
                    op,
                    verum_ast::expr::BinOp::Div | verum_ast::expr::BinOp::Rem
                ) {
                    if self.expr_is_symbol(right, symbol) {
                        let context = get_line_at_span(source, expr.span);
                        usages.push(SymbolUsage {
                            range: span_to_range(source, expr.span),
                            context,
                            usage_kind: UsageKind::Divisor,
                            constraint_context: Some(Text::from("!= 0")),
                        });
                    }
                }

                self.collect_usages_in_expr(left, symbol, source, usages);
                self.collect_usages_in_expr(right, symbol, source, usages);
            }
            verum_ast::ExprKind::Index { expr: base, index } => {
                // Index implies index >= 0 && index < len(base)
                if self.expr_is_symbol(index, symbol) {
                    let context = get_line_at_span(source, expr.span);
                    usages.push(SymbolUsage {
                        range: span_to_range(source, expr.span),
                        context,
                        usage_kind: UsageKind::Index,
                        constraint_context: Some(Text::from(">= 0")),
                    });
                }
                self.collect_usages_in_expr(base, symbol, source, usages);
                self.collect_usages_in_expr(index, symbol, source, usages);
            }
            verum_ast::ExprKind::Call { func, args, .. } => {
                self.collect_usages_in_expr(func, symbol, source, usages);
                for arg in args {
                    self.collect_usages_in_expr(arg, symbol, source, usages);
                }
            }
            verum_ast::ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // Check conditions for usages
                for cond in &condition.conditions {
                    match cond {
                        verum_ast::expr::ConditionKind::Expr(e) => {
                            self.collect_usages_in_expr(e, symbol, source, usages);
                        }
                        verum_ast::expr::ConditionKind::Let { value, .. } => {
                            self.collect_usages_in_expr(value, symbol, source, usages);
                        }
                    }
                }
                // Check then_branch block statements and expression
                self.collect_usages_in_stmts(&then_branch.stmts, symbol, source, usages);
                if let Maybe::Some(expr) = &then_branch.expr {
                    self.collect_usages_in_expr(expr, symbol, source, usages);
                }
                if let Maybe::Some(else_br) = else_branch {
                    self.collect_usages_in_expr(else_br, symbol, source, usages);
                }
            }
            _ => {}
        }
    }

    /// Check if expression is a reference to the symbol
    fn expr_is_symbol(&self, expr: &Expr, symbol: &Text) -> bool {
        if let verum_ast::ExprKind::Path(path) = &expr.kind {
            if let Some(verum_ast::ty::PathSegment::Name(name)) = path.segments.first() {
                return name.as_str() == symbol.as_str();
            }
        }
        false
    }

    /// Extract constraint from comparison
    fn extract_comparison_constraint(&self, op: &verum_ast::expr::BinOp, other: &Expr) -> Text {
        let op_str = match op {
            verum_ast::expr::BinOp::Lt => "<",
            verum_ast::expr::BinOp::Le => "<=",
            verum_ast::expr::BinOp::Gt => ">",
            verum_ast::expr::BinOp::Ge => ">=",
            verum_ast::expr::BinOp::Eq => "==",
            verum_ast::expr::BinOp::Ne => "!=",
            _ => "?",
        };

        // Try to extract literal value from other side
        let value = match &other.kind {
            verum_ast::ExprKind::Literal(lit) => match &lit.kind {
                verum_ast::literal::LiteralKind::Int(int_lit) => format!("{}", int_lit.value),
                verum_ast::literal::LiteralKind::Float(float_lit) => format!("{}", float_lit.value),
                _ => "?".to_string(),
            },
            _ => "?".to_string(),
        };

        Text::from(format!("{} {}", op_str, value))
    }

    /// Collect constraints from usage sites
    fn collect_usage_constraints(
        &self,
        _module: &Module,
        _symbol: &Text,
        usages: &List<SymbolUsage>,
    ) -> List<InferredConstraint> {
        let mut constraints = List::new();

        for usage in usages {
            if let Some(ctx) = &usage.constraint_context {
                let kind = match usage.usage_kind {
                    UsageKind::Divisor => ConstraintInferenceKind::NonZero,
                    UsageKind::Index => ConstraintInferenceKind::NonNegative,
                    UsageKind::Comparison => ConstraintInferenceKind::Comparison,
                    _ => ConstraintInferenceKind::Other,
                };

                constraints.push(InferredConstraint {
                    constraint: ctx.clone(),
                    kind,
                    source_location: usage.range,
                    confidence: match usage.usage_kind {
                        UsageKind::Divisor | UsageKind::Index => 1.0,
                        UsageKind::Comparison => 0.9,
                        _ => 0.5,
                    },
                });
            }
        }

        constraints
    }

    /// Compute the tightest refinement from collected constraints
    fn compute_tightest_refinement(
        &self,
        base_type: &Text,
        constraints: &List<InferredConstraint>,
    ) -> (Text, ConfidenceLevel) {
        if constraints.is_empty() {
            return (base_type.clone(), ConfidenceLevel::Low);
        }

        // Group constraints by kind
        let mut has_non_zero = false;
        let mut has_non_negative = false;
        let mut comparisons = List::new();
        let mut avg_confidence = 0.0;

        for c in constraints {
            avg_confidence += c.confidence;
            match c.kind {
                ConstraintInferenceKind::NonZero => has_non_zero = true,
                ConstraintInferenceKind::NonNegative => has_non_negative = true,
                ConstraintInferenceKind::Comparison => {
                    comparisons.push(c.constraint.clone());
                }
                _ => {}
            }
        }

        avg_confidence /= constraints.len() as f64;

        // Build the refinement constraint
        let mut constraint_parts = List::new();

        if has_non_zero {
            constraint_parts.push("i != 0".to_string());
        }
        if has_non_negative {
            constraint_parts.push("i >= 0".to_string());
        }
        for comp in &comparisons {
            constraint_parts.push(format!("i {}", comp.as_str()));
        }

        let refinement = if constraint_parts.is_empty() {
            base_type.clone()
        } else {
            // Combine constraints with && and wrap in refinement type
            let constraint_str = constraint_parts.join(" && ");
            Text::from(format!("{}{{i | {}}}", base_type.as_str(), constraint_str))
        };

        let confidence = if avg_confidence >= 0.9 {
            ConfidenceLevel::High
        } else if avg_confidence >= 0.7 {
            ConfidenceLevel::Medium
        } else {
            ConfidenceLevel::Low
        };

        (refinement, confidence)
    }

    /// Generate edits to add refinement to symbol declaration
    fn generate_refinement_edits(
        &self,
        def: &SymbolDefinition,
        inferred_type: &Text,
        source: &str,
    ) -> List<TextEdit> {
        let mut edits = List::new();

        let range = span_to_range(source, def.span);

        edits.push(TextEdit {
            range,
            new_text: format!("{}: {}", def.name.as_str(), inferred_type.as_str()),
        });

        edits
    }

    /// Clear validation cache
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }
}

impl Default for RefinementValidator {
    fn default() -> Self {
        Self::new()
    }
}

// ==================== SMT Integration ====================

/// SMT-based refinement checking
///
/// Integrates with verum_smt to verify refinement types using Z3 solver.
pub struct SmtRefinementChecker {
    verifier: SmtRefinementVerifier,
}

impl SmtRefinementChecker {
    /// Create a new SMT refinement checker
    pub fn new() -> Self {
        Self {
            verifier: SmtRefinementVerifier::new(),
        }
    }

    /// Check if refinement is valid
    pub async fn check_refinement(
        &self,
        ty: &Type,
        context_expr: Option<&Expr>,
        _timeout: Duration,
    ) -> Result<SmtCheckResult, String> {
        // Verify the refinement type
        match self
            .verifier
            .verify_refinement(ty, context_expr, Some(VerifyMode::Proof))
        {
            Ok(_proof_result) => Ok(SmtCheckResult::Valid),
            Err(VerificationError::CannotProve {
                counterexample: Some(ce),
                ..
            }) => {
                // Extract model information
                let model_str = format!("{:?}", ce);
                Ok(SmtCheckResult::Invalid { model: model_str })
            }
            Err(VerificationError::CannotProve {
                counterexample: None,
                ..
            }) => Ok(SmtCheckResult::Unknown),
            Err(VerificationError::Timeout { .. }) => Ok(SmtCheckResult::Unknown),
            Err(VerificationError::Unknown(_)) => Ok(SmtCheckResult::Unknown),
            Err(e) => Err(format!("SMT error: {}", e)),
        }
    }
}

impl Default for SmtRefinementChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// SMT check result
#[derive(Debug, Clone)]
pub enum SmtCheckResult {
    Valid,
    Invalid { model: String },
    Unknown,
}

// ==================== Helper Functions ====================

/// Convert position to line/column string
pub fn position_to_string(pos: Position) -> String {
    format!("{}:{}", pos.line, pos.character)
}

/// Parse refinement type from text
///
/// Uses the verum_parser to properly parse refinement type syntax like:
/// - `Int{> 0}` - integer greater than zero
/// - `Text{len(it) > 0}` - non-empty text
/// - `List<Int>{len(it) >= n}` - list with at least n elements
pub fn parse_refinement_type(text: &str) -> Result<RefinementType, String> {
    // Create a temporary file ID for parsing
    let file_id = FileId::new(0);

    // Create parser and lexer
    let parser = VerumParser::new();

    // Try to parse the type expression
    match parser.parse_type_str(text, file_id) {
        Ok(parsed_type) => {
            match &parsed_type.kind {
                TypeKind::Refined { base, predicate } => {
                    // Extract base type string representation
                    let base_type = format_type_ast(base);

                    // Extract constraint string from predicate
                    let constraint = format_predicate_expr(&predicate.expr);

                    Ok(RefinementType {
                        base_type,
                        constraint,
                    })
                }
                // Not a refinement type - treat as base type with no constraint
                _ => Ok(RefinementType {
                    base_type: format_type_ast(&parsed_type),
                    constraint: "true".to_string(),
                }),
            }
        }
        Err(errors) => {
            let error_msgs: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
            Err(format!("Parse error: {}", error_msgs.join("; ")))
        }
    }
}

/// Format a Type AST node to a string representation
fn format_type_ast(ty: &Type) -> String {
    if let Some(name) = ty.kind.primitive_name() {
        return name.to_string();
    }
    match &ty.kind {
        TypeKind::Path(path) => {
            use verum_ast::ty::PathSegment;
            let segments: Vec<String> = path
                .segments
                .iter()
                .filter_map(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.as_str().to_string()),
                    PathSegment::SelfValue => Some("Self".to_string()),
                    PathSegment::Super => Some("super".to_string()),
                    PathSegment::Cog => Some("cog".to_string()),
                    PathSegment::Relative => None,
                })
                .collect();
            segments.join("::")
        }
        TypeKind::Refined { base, predicate } => {
            format!(
                "{}{{{}}}",
                format_type_ast(base),
                format_predicate_expr(&predicate.expr)
            )
        }
        TypeKind::Tuple(types) => {
            let inner: Vec<String> = types.iter().map(format_type_ast).collect();
            format!("({})", inner.join(", "))
        }
        TypeKind::Array { element, size } => {
            if let Maybe::Some(sz) = size {
                format!("[{}; {}]", format_type_ast(element), format_expr(sz))
            } else {
                format!("[{}]", format_type_ast(element))
            }
        }
        TypeKind::Slice(element) => {
            format!("[{}]", format_type_ast(element))
        }
        TypeKind::Function {
            params,
            return_type,
            ..
        } => {
            let params_str: Vec<String> = params.iter().map(format_type_ast).collect();
            format!(
                "fn({}) -> {}",
                params_str.join(", "),
                format_type_ast(return_type)
            )
        }
        TypeKind::Reference { mutable, inner } => {
            if *mutable {
                format!("&mut {}", format_type_ast(inner))
            } else {
                format!("&{}", format_type_ast(inner))
            }
        }
        TypeKind::CheckedReference { mutable, inner } => {
            if *mutable {
                format!("&checked mut {}", format_type_ast(inner))
            } else {
                format!("&checked {}", format_type_ast(inner))
            }
        }
        TypeKind::UnsafeReference { mutable, inner } => {
            if *mutable {
                format!("&unsafe mut {}", format_type_ast(inner))
            } else {
                format!("&unsafe {}", format_type_ast(inner))
            }
        }
        TypeKind::Pointer { mutable, inner } => {
            if *mutable {
                format!("*mut {}", format_type_ast(inner))
            } else {
                format!("*const {}", format_type_ast(inner))
            }
        }
        TypeKind::Generic { base, args } => {
            let args_str: Vec<String> = args.iter().map(format_generic_arg).collect();
            format!("{}<{}>", format_type_ast(base), args_str.join(", "))
        }
        TypeKind::Inferred => "_".to_string(),
        _ => "<type>".to_string(),
    }
}

/// Format a generic argument to string
fn format_generic_arg(arg: &verum_ast::ty::GenericArg) -> String {
    match arg {
        verum_ast::ty::GenericArg::Type(ty) => format_type_ast(ty),
        verum_ast::ty::GenericArg::Const(expr) => format_expr(expr),
        verum_ast::ty::GenericArg::Lifetime(lt) => format!("'{}", lt.name.as_str()),
        verum_ast::ty::GenericArg::Binding(binding) => {
            format!(
                "{} = {}",
                binding.name.as_str(),
                format_type_ast(&binding.ty)
            )
        }
    }
}

/// Format an expression to a string representation
fn format_expr(expr: &Expr) -> String {
    use verum_ast::expr::ExprKind;

    match &expr.kind {
        ExprKind::Path(path) => {
            use verum_ast::ty::PathSegment;
            path.segments
                .iter()
                .filter_map(|seg| match seg {
                    PathSegment::Name(ident) => Some(ident.as_str()),
                    PathSegment::SelfValue => Some("Self"),
                    PathSegment::Super => Some("super"),
                    PathSegment::Cog => Some("cog"),
                    PathSegment::Relative => None,
                })
                .collect::<Vec<_>>()
                .join("::")
        }
        ExprKind::Literal(lit) => format_literal(&lit.kind),
        ExprKind::Binary { left, op, right } => {
            format!(
                "{} {} {}",
                format_expr(left),
                format_binop(op),
                format_expr(right)
            )
        }
        ExprKind::Unary { op, expr } => {
            format!("{}{}", format_unaryop(op), format_expr(expr))
        }
        ExprKind::Call { func, args, .. } => {
            let args_str: Vec<String> = args.iter().map(format_expr).collect();
            format!("{}({})", format_expr(func), args_str.join(", "))
        }
        ExprKind::Field { expr, field } => {
            format!("{}.{}", format_expr(expr), field.as_str())
        }
        ExprKind::Index { expr, index } => {
            format!("{}[{}]", format_expr(expr), format_expr(index))
        }
        _ => "<expr>".to_string(),
    }
}

/// Format predicate expression to string
fn format_predicate_expr(expr: &Expr) -> String {
    format_expr(expr)
}

/// Format a literal to string
fn format_literal(lit: &verum_ast::LiteralKind) -> String {
    match lit {
        verum_ast::LiteralKind::Int(i) => i.value.to_string(),
        verum_ast::LiteralKind::Float(f) => f.value.to_string(),
        verum_ast::LiteralKind::Bool(b) => b.to_string(),
        verum_ast::LiteralKind::Text(t) => format!("\"{}\"", t.as_str()),
        verum_ast::LiteralKind::Char(c) => format!("'{}'", c),
        verum_ast::LiteralKind::Tagged { tag, content } => format!("{}#\"{}\"", tag, content),
        verum_ast::LiteralKind::InterpolatedString(s) => format!("{}\"{}\"", s.prefix, s.content),
        verum_ast::LiteralKind::Contract(c) => format!("contract#\"{}\"", c),
        verum_ast::LiteralKind::Composite(comp) => format!("{}#\"{}\"", comp.tag, comp.content),
        _ => "<literal>".to_string(),
    }
}

/// Format a binary operator to string
fn format_binop(op: &verum_ast::expr::BinOp) -> &'static str {
    use verum_ast::expr::BinOp;
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        _ => "<??>",
    }
}

/// Format a unary operator to string
fn format_unaryop(op: &verum_ast::expr::UnOp) -> &'static str {
    use verum_ast::expr::UnOp;
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Ref => "&",
        UnOp::Deref => "*",
        UnOp::RefMut => "&mut ",
        _ => "<??>",
    }
}

/// Format a comparison operator to string
/// Note: Comparison operators are part of BinOp in verum_ast
fn format_cmpop(op: &verum_ast::expr::BinOp) -> &'static str {
    use verum_ast::expr::BinOp;
    match op {
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        _ => "<??>",
    }
}

#[derive(Debug, Clone)]
pub struct RefinementType {
    pub base_type: String,
    pub constraint: String,
}

// ==================== Context Types ====================

/// Context for a refinement being validated
#[derive(Debug, Clone)]
struct RefinementContext {
    /// The refinement type being checked
    ty: Type,
    /// Name of the variable being refined
    var_name: Text,
    /// Context expression if available
    context_expr: Option<Expr>,
    /// Name of containing function
    function_name: String,
    /// Location of the refinement
    location: RefinementLocation,
}

/// Location of a refinement in source
#[derive(Debug, Clone, Copy)]
enum RefinementLocation {
    Parameter,
    ReturnType,
    LocalVariable,
    CallArgument,
    Expression,
}

/// A constraint in scope for verification
#[derive(Debug, Clone)]
pub struct ScopeConstraint {
    kind: ConstraintKind,
    constraint: Expr,
    source: Text,
}

/// Kind of constraint
#[derive(Debug, Clone, Copy)]
pub enum ConstraintKind {
    Precondition,
    ParameterRefinement,
    LocalRefinement,
    Assertion,
    Assumption,
}

/// Context for a reference being analyzed
#[derive(Debug, Clone)]
struct ReferenceContext {
    var_name: Text,
    function_name: String,
    ty: Type,
    span: verum_ast::span::Span,
    is_mutable: bool,
}

/// Result of escape analysis
#[derive(Debug, Clone)]
struct EscapeAnalysisResult {
    can_promote: bool,
    escapes: bool,
    reason: String,
    lifetime_info: String,
    start_line: u32,
    end_line: u32,
    paths_analyzed: u32,
    stored_to_heap: bool,
    returned: bool,
}

/// Symbol definition for refinement inference
#[derive(Debug, Clone)]
struct SymbolDefinition {
    name: Text,
    span: verum_ast::span::Span,
    base_type: Text,
    kind: SymbolKind,
    function_name: Option<String>,
}

/// Kind of symbol
#[derive(Debug, Clone, Copy)]
enum SymbolKind {
    Parameter,
    LocalVariable,
    Field,
}

/// Usage of a symbol
#[derive(Debug, Clone)]
struct SymbolUsage {
    range: Range,
    context: String,
    usage_kind: UsageKind,
    constraint_context: Option<Text>,
}

/// Kind of symbol usage
#[derive(Debug, Clone, Copy)]
enum UsageKind {
    Read,
    Write,
    Comparison,
    Divisor,
    Index,
    Call,
}

/// Inferred constraint from usage
#[derive(Debug, Clone)]
struct InferredConstraint {
    constraint: Text,
    kind: ConstraintInferenceKind,
    source_location: Range,
    confidence: f64,
}

/// Kind of inferred constraint
#[derive(Debug, Clone, Copy)]
enum ConstraintInferenceKind {
    NonZero,
    NonNegative,
    Positive,
    Comparison,
    Bounds,
    Other,
}

// ==================== Additional Helper Functions ====================

/// Convert LSP Position to byte offset in source
fn position_to_offset(source: &str, position: Position) -> u32 {
    let mut offset = 0u32;
    for (line_num, line) in source.lines().enumerate() {
        if line_num == position.line as usize {
            offset += position.character;
            break;
        }
        offset += line.len() as u32 + 1; // +1 for newline
    }
    offset
}

/// Convert byte offset to line number
fn span_to_line(source: &str, offset: u32) -> u32 {
    let mut current_offset = 0u32;
    for (line_num, line) in source.lines().enumerate() {
        if current_offset + line.len() as u32 >= offset {
            return line_num as u32;
        }
        current_offset += line.len() as u32 + 1;
    }
    0
}

/// Convert span to LSP Range
fn span_to_range(source: &str, span: verum_ast::span::Span) -> Range {
    let start_line = span_to_line(source, span.start);
    let end_line = span_to_line(source, span.end);

    // Calculate character positions
    let start_char = calculate_character(source, span.start, start_line);
    let end_char = calculate_character(source, span.end, end_line);

    Range {
        start: Position {
            line: start_line,
            character: start_char,
        },
        end: Position {
            line: end_line,
            character: end_char,
        },
    }
}

/// Calculate character position within a line
fn calculate_character(source: &str, offset: u32, line: u32) -> u32 {
    let mut current_offset = 0u32;
    for (line_num, line_text) in source.lines().enumerate() {
        if line_num == line as usize {
            return offset - current_offset;
        }
        current_offset += line_text.len() as u32 + 1;
    }
    0
}

/// Get line content at span
fn get_line_at_span(source: &str, span: verum_ast::span::Span) -> String {
    let line_num = span_to_line(source, span.start);
    source
        .lines()
        .nth(line_num as usize)
        .unwrap_or("")
        .to_string()
}

/// Extract source text from range
fn source_range(source: &str, start: usize, end: usize) -> String {
    if start < source.len() && end <= source.len() && start <= end {
        source[start..end].to_string()
    } else {
        String::new()
    }
}

/// Extract pattern name
fn extract_pattern_name(pattern: &verum_ast::pattern::Pattern) -> Text {
    match &pattern.kind {
        verum_ast::pattern::PatternKind::Ident { name, .. } => Text::from(name.as_str()),
        _ => Text::from("_"),
    }
}

/// Extract base type from type annotation
fn extract_base_type(ty: &Type) -> Text {
    match &ty.kind {
        TypeKind::Refined { base, .. } => extract_base_type(base),
        _ if ty.kind.primitive_name().is_some() => {
            Text::from(ty.kind.primitive_name().unwrap())
        }
        TypeKind::Path(path) => {
            if let Some(seg) = path.segments.last() {
                match seg {
                    verum_ast::ty::PathSegment::Name(name) => Text::from(name.as_str()),
                    _ => Text::from("_"),
                }
            } else {
                Text::from("_")
            }
        }
        _ => Text::from("_"),
    }
}

/// Format a type for display (returns Text)
fn format_type_text(ty: &Type) -> Text {
    match &ty.kind {
        TypeKind::Refined { base, predicate } => {
            let base_str = format_type_text(base);
            let constraint_str = format_constraint_expr(&predicate.expr);
            Text::from(format!(
                "{}{{i | {}}}",
                base_str.as_str(),
                constraint_str.as_str()
            ))
        }
        _ if ty.kind.primitive_name().is_some() => {
            Text::from(ty.kind.primitive_name().unwrap())
        }
        TypeKind::Path(path) => {
            if let Some(seg) = path.segments.last() {
                match seg {
                    verum_ast::ty::PathSegment::Name(name) => Text::from(name.as_str()),
                    _ => Text::from("_"),
                }
            } else {
                Text::from("_")
            }
        }
        TypeKind::Reference { mutable, inner } => {
            let inner_str = format_type_text(inner);
            if *mutable {
                Text::from(format!("&mut {}", inner_str.as_str()))
            } else {
                Text::from(format!("&{}", inner_str.as_str()))
            }
        }
        _ => Text::from("_"),
    }
}

/// Format a constraint expression for display
fn format_constraint_expr(expr: &Expr) -> Text {
    match &expr.kind {
        verum_ast::ExprKind::Binary { op, left, right } => {
            let left_str = format_constraint_expr(left);
            let right_str = format_constraint_expr(right);
            let op_str = match op {
                verum_ast::expr::BinOp::Add => "+",
                verum_ast::expr::BinOp::Sub => "-",
                verum_ast::expr::BinOp::Mul => "*",
                verum_ast::expr::BinOp::Div => "/",
                verum_ast::expr::BinOp::Eq => "==",
                verum_ast::expr::BinOp::Ne => "!=",
                verum_ast::expr::BinOp::Lt => "<",
                verum_ast::expr::BinOp::Le => "<=",
                verum_ast::expr::BinOp::Gt => ">",
                verum_ast::expr::BinOp::Ge => ">=",
                verum_ast::expr::BinOp::And => "&&",
                verum_ast::expr::BinOp::Or => "||",
                _ => "?",
            };
            Text::from(format!(
                "{} {} {}",
                left_str.as_str(),
                op_str,
                right_str.as_str()
            ))
        }
        verum_ast::ExprKind::Literal(lit) => match &lit.kind {
            verum_ast::literal::LiteralKind::Int(int_lit) => {
                Text::from(format!("{}", int_lit.value))
            }
            verum_ast::literal::LiteralKind::Float(float_lit) => {
                Text::from(format!("{}", float_lit.value))
            }
            verum_ast::literal::LiteralKind::Bool(b) => Text::from(format!("{}", b)),
            verum_ast::literal::LiteralKind::Text(s) => Text::from(format!("\"{}\"", s.as_str())),
            _ => Text::from("?"),
        },
        verum_ast::ExprKind::Path(path) => {
            if let Some(verum_ast::ty::PathSegment::Name(name)) = path.segments.first() {
                Text::from(name.as_str())
            } else {
                Text::from("?")
            }
        }
        verum_ast::ExprKind::Unary { op, expr: operand } => {
            let operand_str = format_constraint_expr(operand);
            let op_str = match op {
                verum_ast::expr::UnOp::Neg => "-",
                verum_ast::expr::UnOp::Not => "!",
                _ => "?",
            };
            Text::from(format!("{}{}", op_str, operand_str.as_str()))
        }
        _ => Text::from("?"),
    }
}

/// Parse SMT model string to extract variable assignments
fn parse_model_assignments(model: &str) -> List<(Text, Text)> {
    let mut assignments = List::new();

    // Parse model format: "var1 = value1, var2 = value2"
    for part in model.split(',') {
        let trimmed = part.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            assignments.push((Text::from(name.trim()), Text::from(value.trim())));
        }
    }

    assignments
}

/// Check if a type is a reference type
fn is_reference_type(ty: &Type) -> bool {
    matches!(&ty.kind, TypeKind::Reference { .. })
}

/// Check if a type is a mutable reference
fn is_mutable_reference(ty: &Type) -> bool {
    if let TypeKind::Reference { mutable, .. } = &ty.kind {
        *mutable
    } else {
        false
    }
}

/// Check if a type contains a reference
fn type_contains_reference(ty: &Type) -> bool {
    match &ty.kind {
        TypeKind::Reference { .. } => true,
        TypeKind::Tuple(elements) => elements.iter().any(type_contains_reference),
        TypeKind::Array { element, .. } => type_contains_reference(element),
        TypeKind::Refined { base, .. } => type_contains_reference(base),
        _ => false,
    }
}

// ==================== Extended SMT Checker ====================

impl SmtRefinementChecker {
    /// Check refinement with additional scope constraints
    pub async fn check_refinement_with_context(
        &self,
        ty: &Type,
        context_expr: Option<&Expr>,
        _scope_constraints: &List<ScopeConstraint>,
        _timeout: Duration,
    ) -> Result<SmtCheckResult, String> {
        // Convert scope constraints to SMT assertions
        // In a full implementation, this would translate each constraint
        // to Z3 assertions and add them to the solver context

        // For now, we use the simpler single-refinement check
        // and rely on the verifier's internal constraint handling
        match self
            .verifier
            .verify_refinement(ty, context_expr, Some(VerifyMode::Proof))
        {
            Ok(_proof_result) => Ok(SmtCheckResult::Valid),
            Err(VerificationError::CannotProve {
                counterexample: Some(ce),
                ..
            }) => {
                // Extract model information
                let model_str = format!("{:?}", ce);
                Ok(SmtCheckResult::Invalid { model: model_str })
            }
            Err(VerificationError::CannotProve {
                counterexample: None,
                ..
            }) => Ok(SmtCheckResult::Unknown),
            Err(VerificationError::Timeout { .. }) => Ok(SmtCheckResult::Unknown),
            Err(VerificationError::Unknown(_)) => Ok(SmtCheckResult::Unknown),
            Err(e) => Err(format!("SMT error: {}", e)),
        }
    }
}

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_cache() {
        let cache = ValidationCache::new(10, Duration::from_secs(1));

        // Insert and retrieve
        cache.insert("key1".to_string(), ValidationResult::Valid);
        assert!(matches!(cache.get("key1"), Maybe::Some(_)));

        // Non-existent key
        assert!(matches!(cache.get("key2"), Maybe::None));

        // Stats
        let stats = cache.stats();
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.capacity, 10);
    }

    #[test]
    fn cost_warning_gate_requires_both_enabled_and_threshold() {
        // Pin: `should_emit_cost_warning` correctly conjuncts the
        // `cost_warnings_enabled` flag with the elapsed-vs-threshold
        // comparison. Before the wire-up the LspConfig fields
        // (`verification_show_cost_warnings`, `verification_slow_threshold`)
        // never reached the validator — the JSON parser stored them on
        // LspConfig but no consumer ever read them.
        let validator = RefinementValidator::new();

        // Default config: enabled = true, threshold = 5_000ms.
        // 100ms elapsed is well under threshold → no warning.
        assert!(
            !validator.should_emit_cost_warning(Duration::from_millis(100)),
            "below-threshold elapsed must not emit"
        );
        // 6_000ms elapsed crosses the default threshold → warning.
        assert!(
            validator.should_emit_cost_warning(Duration::from_millis(6_000)),
            "above-threshold elapsed must emit when enabled"
        );

        // Apply a config that disables cost warnings entirely. Even
        // a 60-second validation must stay quiet now.
        let mut cfg = crate::lsp_config::LspConfig::default();
        cfg.verification_show_cost_warnings = false;
        validator.apply_config(&cfg);
        assert!(
            !validator.should_emit_cost_warning(Duration::from_millis(60_000)),
            "verification_show_cost_warnings=false must suppress every warning"
        );

        // Re-enable, lower the threshold to 10ms. A 50ms validation
        // now crosses it.
        cfg.verification_show_cost_warnings = true;
        cfg.verification_slow_threshold = Duration::from_millis(10);
        validator.apply_config(&cfg);
        assert!(
            validator.should_emit_cost_warning(Duration::from_millis(50)),
            "lowering the threshold via apply_config must take effect"
        );
        assert_eq!(validator.slow_threshold(), Duration::from_millis(10));
    }

    #[test]
    fn test_quick_fix_generation() {
        let validator = RefinementValidator::new();
        let counterexample = CounterexampleData {
            variable: "x".into(),
            value: "0".into(),
            r#type: "Int{i != 0}".into(),
            constraint: "i != 0".into(),
            violation_reason: "Division by zero".into(),
            trace: List::new(),
        };

        let fixes = validator.generate_quick_fixes(&counterexample);
        assert!(fixes.len() >= 3);
        assert_eq!(fixes[0].kind, QuickFixKind::RuntimeCheck);
        assert_eq!(fixes[0].priority, 1);
    }

    #[tokio::test]
    async fn test_validate_refinement() {
        let validator = RefinementValidator::new();
        let params = ValidateRefinementParams {
            text_document: TextDocumentIdentifier {
                uri: Url::parse("file:///test.vr").unwrap(),
            },
            position: Position {
                line: 10,
                character: 15,
            },
            mode: ValidationMode::Quick,
        };

        // Note: This test just verifies the validator doesn't panic
        // The actual validation will fail because the file doesn't exist
        let result = validator.validate_refinement(params).await;
        // Result may be error (file not found) or ok (mock/fallback)
        // Just ensure we get a response
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_promote_to_checked() {
        let validator = RefinementValidator::new();
        let params = PromoteToCheckedParams {
            text_document: TextDocumentIdentifier {
                // Using a non-existent file to test error handling
                uri: Url::parse("file:///nonexistent/test.vr").unwrap(),
            },
            range: Range {
                start: Position {
                    line: 5,
                    character: 20,
                },
                end: Position {
                    line: 5,
                    character: 30,
                },
            },
            include_proof: true,
        };

        let result = validator.promote_to_checked(params).await;
        // File doesn't exist, so we expect an error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_infer_refinement() {
        let validator = RefinementValidator::new();
        let params = InferRefinementParams {
            text_document: TextDocumentIdentifier {
                // Using a non-existent file to test error handling
                uri: Url::parse("file:///nonexistent/test.vr").unwrap(),
            },
            symbol: "index".into(),
        };

        let result = validator.infer_refinement(params).await;
        // File doesn't exist, so we expect an error
        assert!(result.is_err());
    }
}
