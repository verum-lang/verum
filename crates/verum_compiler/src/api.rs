//! Public API for verum_compiler
//!
//! This module provides a clean, well-designed public API for the Verum compiler.
//! It exposes the key compilation functions that can be used by:
//! - VCS test runner (vtest)
//! - CLI tools
//! - IDE integrations (LSP)
//! - Embedding applications
//!
//! # Architecture
//!
//! The compiler is split into two main pipelines:
//!
//! ## Common Pipeline (Source → TypedAST)
//!
//! This is the core of the language implementation, covering:
//! - Lexical analysis and parsing (verum_lexer, verum_fast_parser)
//! - Macro expansion (meta-system)
//! - Type inference and checking (verum_types)
//! - Refinement types verification (SMT via verum_smt)
//! - Contract verification (SMT-based pre/post conditions)
//! - Context system validation (using/provide)
//! - CBGR reference tier analysis (verum_cbgr) - determines reference safety tiers:
//!   - Tier 0: Runtime checked (~15ns overhead)
//!   - Tier 1: Compiler-proven safe (0ns overhead)
//!   - Tier 2: Unsafe (manual proof required)
//!
//! The Common Pipeline produces `TypedAST` which can be verified against
//! the full language specification.
//!
//! ## Backend Pipeline (TypedAST → Execution)
//!
//! Handles tier-specific compilation:
//! - VBC code generation
//! - Monomorphization
//! - Interpreter / JIT / AOT execution
//!
//! # Example
//!
//! ```ignore
//! use verum_compiler::api::{parse, typecheck, run_common_pipeline};
//!
//! // Parse only
//! let ast = parse("fn main() { print(\"hello\"); }")?;
//!
//! // Parse + type check
//! let typed = typecheck("fn main() { let x: Int = 42; }")?;
//!
//! // Full common pipeline with verification
//! let config = CommonPipelineConfig::default();
//! let result = run_common_pipeline(&["main.vr"], &config)?;
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use verum_ast::Module;
use verum_common::{List, Text};
use verum_diagnostics::Diagnostic;

use crate::phases::{
    ExecutionTier, HirModule, PhaseMetrics, VerificationResults, VerifyMode as PhaseVerifyMode,
};

// ============================================================================
// Error Types
// ============================================================================

/// Compilation error type
#[derive(Debug, Clone)]
pub struct CompilationError {
    /// Error kind
    pub kind: CompilationErrorKind,
    /// Human-readable message
    pub message: Text,
    /// Associated diagnostics
    pub diagnostics: List<Diagnostic>,
}

/// Kinds of compilation errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationErrorKind {
    /// Lexer error (invalid tokens)
    LexerError,
    /// Parser error (syntax error)
    ParseError,
    /// Type checking error
    TypeError,
    /// Contract verification error
    VerificationError,
    /// Context system error
    ContextError,
    /// VBC codegen error
    CodegenError,
    /// I/O error (file not found, etc.)
    IoError,
    /// Internal compiler error
    InternalError,
}

impl std::fmt::Display for CompilationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)?;
        // Include first diagnostic message if available (truncated to 100 chars)
        if let Some(first_diag) = self.diagnostics.first() {
            let msg = first_diag.message();
            if msg.len() > 100 {
                write!(f, " - {}...", &msg[..100])?;
            } else {
                write!(f, " - {}", msg)?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for CompilationError {}

impl CompilationError {
    /// Create a new compilation error
    pub fn new(kind: CompilationErrorKind, message: impl Into<Text>) -> Self {
        Self {
            kind,
            message: message.into(),
            diagnostics: List::new(),
        }
    }

    /// Create error with diagnostics
    pub fn with_diagnostics(
        kind: CompilationErrorKind,
        message: impl Into<Text>,
        diagnostics: List<Diagnostic>,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            diagnostics,
        }
    }

    /// Create parse error
    pub fn parse_error(message: impl Into<Text>) -> Self {
        Self::new(CompilationErrorKind::ParseError, message)
    }

    /// Create type error
    pub fn type_error(message: impl Into<Text>) -> Self {
        Self::new(CompilationErrorKind::TypeError, message)
    }

    /// Create internal error
    pub fn internal(message: impl Into<Text>) -> Self {
        Self::new(CompilationErrorKind::InternalError, message)
    }
}

// ============================================================================
// Configuration Types
// ============================================================================

/// Configuration for the common pipeline (Source → TypedAST)
#[derive(Debug, Clone)]
pub struct CommonPipelineConfig {
    /// Enable SMT verification of contracts
    pub verify_contracts: bool,
    /// SMT solver timeout in milliseconds
    pub smt_timeout_ms: u64,
    /// Enable refinement type checking
    pub check_refinements: bool,
    /// Enable context system validation
    pub validate_contexts: bool,
    /// Collect detailed metrics
    pub collect_metrics: bool,
    /// Enable macro expansion
    pub expand_macros: bool,
    /// Path to the core stdlib directory. If set, core .vr files are prepended to sources.
    pub core_source_path: Option<PathBuf>,
    /// Enable cubical-type normalization in the unifier (sourced from
    /// `[types] cubical` in `verum.toml`). Default: true.
    pub cubical_enabled: bool,
    /// Enable the context / DI system (sourced from `[context] enabled`).
    /// When false, no context validation runs and `using [...]` clauses
    /// are parsed-and-ignored. Default: true.
    pub context_enabled: bool,
    /// Enable `@derive(...)` expansion (sourced from `[meta] derive`).
    pub derive_enabled: bool,
    /// Enable compile-time function evaluation (`meta fn`, `@const`).
    /// Sourced from `[meta] compile_time_functions`.
    pub compile_time_enabled: bool,
    /// Allow `unsafe { ... }` expressions (sourced from
    /// `[safety] unsafe_allowed`). When false, the safety-gate phase
    /// rejects unsafe blocks with a config-pointing diagnostic.
    pub unsafe_allowed: bool,
}

impl Default for CommonPipelineConfig {
    fn default() -> Self {
        Self {
            verify_contracts: true,
            smt_timeout_ms: 5000,
            check_refinements: true,
            validate_contexts: true,
            collect_metrics: true,
            expand_macros: true,
            core_source_path: None,
            cubical_enabled: true,
            context_enabled: true,
            derive_enabled: true,
            compile_time_enabled: true,
            unsafe_allowed: true,
        }
    }
}

impl CommonPipelineConfig {
    /// Create minimal config (fast, no verification)
    pub fn minimal() -> Self {
        Self {
            verify_contracts: false,
            smt_timeout_ms: 0,
            check_refinements: false,
            validate_contexts: false,
            collect_metrics: false,
            expand_macros: true,
            core_source_path: None,
            cubical_enabled: true,
            context_enabled: true,
            derive_enabled: true,
            compile_time_enabled: true,
            unsafe_allowed: true,
        }
    }

    /// Create strict config (full verification)
    pub fn strict() -> Self {
        Self {
            verify_contracts: true,
            smt_timeout_ms: 10000,
            check_refinements: true,
            validate_contexts: true,
            collect_metrics: true,
            expand_macros: true,
            core_source_path: None,
            cubical_enabled: true,
            context_enabled: true,
            derive_enabled: true,
            compile_time_enabled: true,
            unsafe_allowed: true,
        }
    }
}

/// Configuration for the full compiler
#[derive(Debug, Clone)]
pub struct CompilerConfig {
    /// Common pipeline configuration
    pub common: CommonPipelineConfig,
    /// Target execution tier
    pub target_tier: ExecutionTier,
    /// Optimization level (0-3)
    pub optimization_level: u8,
    /// Output type
    pub output_type: OutputType,
    /// Enable debug information
    pub debug_info: bool,
}

impl Default for CompilerConfig {
    fn default() -> Self {
        Self {
            common: CommonPipelineConfig::default(),
            target_tier: ExecutionTier::Interpreter,
            optimization_level: 2,
            output_type: OutputType::Executable,
            debug_info: false,
        }
    }
}

/// Output type for compilation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputType {
    /// Standalone executable
    Executable,
    /// Shared library (.so/.dylib/.dll)
    SharedLibrary,
    /// Static library (.a/.lib)
    StaticLibrary,
    /// VBC bytecode module
    VbcModule,
    /// Object file (.o)
    ObjectFile,
}

// ============================================================================
// Result Types
// ============================================================================

/// Result of running the common pipeline
#[derive(Debug, Clone)]
pub struct CommonPipelineResult {
    /// Typed AST modules (HIR)
    pub typed_modules: List<HirModule>,
    /// Original AST modules (for reference)
    pub ast_modules: List<Module>,
    /// Diagnostics (errors and warnings)
    pub diagnostics: List<Diagnostic>,
    /// Contract verification results
    pub verification_results: VerificationResults,
    /// Pipeline metrics
    pub metrics: PipelineMetrics,
    /// Whether the pipeline completed successfully
    pub success: bool,
}

impl CommonPipelineResult {
    /// Check if there are any errors
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity() == verum_diagnostics::Severity::Error)
    }

    /// Get error count
    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity() == verum_diagnostics::Severity::Error).count()
    }

    /// Get warning count
    pub fn warning_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.severity() == verum_diagnostics::Severity::Warning).count()
    }
}

/// Pipeline execution metrics
#[derive(Debug, Clone, Default)]
pub struct PipelineMetrics {
    /// Total pipeline duration
    pub total_duration: Duration,
    /// Per-phase metrics
    pub phase_metrics: List<PhaseMetrics>,
    /// Number of modules processed
    pub modules_processed: usize,
    /// Number of functions processed
    pub functions_processed: usize,
    /// Number of types processed
    pub types_processed: usize,
}

/// Result of full compilation
#[derive(Debug, Clone)]
pub struct CompilationResult {
    /// Common pipeline result
    pub common: CommonPipelineResult,
    /// VBC modules (if generated)
    pub vbc_modules: Option<List<crate::phases::VbcModuleData>>,
    /// Output artifacts
    pub artifacts: Option<CompilationArtifacts>,
}

/// Compilation output artifacts
#[derive(Debug, Clone)]
pub struct CompilationArtifacts {
    /// Path to main output file
    pub output_path: Option<std::path::PathBuf>,
    /// Additional generated files
    pub additional_files: List<std::path::PathBuf>,
}

// ============================================================================
// Source File Types
// ============================================================================

/// Source file for compilation
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// File path (can be virtual for in-memory sources)
    pub path: Text,
    /// Source content
    pub content: Text,
}

impl SourceFile {
    /// Create from path and content
    pub fn new(path: impl Into<Text>, content: impl Into<Text>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }

    /// Create from string (virtual file)
    pub fn from_string(content: impl Into<Text>) -> Self {
        Self {
            path: Text::from("<string>"),
            content: content.into(),
        }
    }

    /// Load from file path
    pub fn load(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;
        Ok(Self {
            path: Text::from(path.display().to_string()),
            content: Text::from(content),
        })
    }
}

impl From<&str> for SourceFile {
    fn from(content: &str) -> Self {
        Self::from_string(content)
    }
}

impl From<String> for SourceFile {
    fn from(content: String) -> Self {
        Self::from_string(content)
    }
}

// ============================================================================
// Public API Functions
// ============================================================================

/// Parse source code into AST.
///
/// This is Phase 1 of the compilation pipeline.
///
/// # Arguments
///
/// * `source` - Source code to parse
///
/// # Returns
///
/// Parsed AST module or parse error
///
/// # Example
///
/// ```ignore
/// let ast = verum_compiler::api::parse("fn main() { print(\"hello\"); }")?;
/// ```
pub fn parse(source: &str) -> Result<Module, CompilationError> {
    use verum_lexer::Lexer;
    use verum_fast_parser::VerumParser;
    use verum_ast::FileId;

    let file_id = FileId::new(0); // Virtual file
    let lexer = Lexer::new(source, file_id);
    let parser = VerumParser::new();

    parser.parse_module(lexer, file_id).map_err(|e| {
        CompilationError::parse_error(format!("Parse error: {:?}", e))
    })
}

/// Parse and type check source code.
///
/// This runs Phases 1-4 of the compilation pipeline:
/// - Phase 1: Lexical parsing
/// - Phase 2: Meta registry (minimal)
/// - Phase 3: Macro expansion
/// - Phase 4: Semantic analysis (type checking)
///
/// # Arguments
///
/// * `source` - Source code to compile
///
/// # Returns
///
/// Typed AST (HIR) module or compilation error
///
/// # Example
///
/// ```ignore
/// let typed = verum_compiler::api::typecheck("fn main() { let x: Int = 42; }")?;
/// ```
pub fn typecheck(source: &str) -> Result<HirModule, CompilationError> {
    let config = CommonPipelineConfig::minimal();
    let source_file = SourceFile::from_string(source);
    let result = run_common_pipeline(&[source_file], &config)?;

    if result.has_errors() {
        return Err(CompilationError::with_diagnostics(
            CompilationErrorKind::TypeError,
            "Type checking failed",
            result.diagnostics,
        ));
    }

    result.typed_modules.first().cloned().ok_or_else(|| {
        CompilationError::internal("No typed module produced")
    })
}

/// Run the common pipeline (Source → TypedAST).
///
/// This runs Phases 1-4b of the compilation pipeline:
/// - Phase 1: Lexical parsing
/// - Phase 2: Meta registry
/// - Phase 3: Macro expansion
/// - Phase 3a: Contract verification (if enabled)
/// - Phase 4: Semantic analysis (type inference, refinements)
/// - Phase 4b: Context validation (if enabled)
///
/// The common pipeline is the core of the language implementation and should
/// be fully verified against the language specification.
///
/// # Arguments
///
/// * `sources` - Source files to compile
/// * `config` - Pipeline configuration
///
/// # Returns
///
/// Common pipeline result containing typed AST and diagnostics
///
/// # Example
///
/// ```ignore
/// let config = CommonPipelineConfig::default();
/// let result = run_common_pipeline(&[SourceFile::load("main.vr")?], &config)?;
///
/// if result.has_errors() {
///     for diag in &result.diagnostics {
///         eprintln!("{}", diag);
///     }
/// }
/// ```
pub fn run_common_pipeline(
    sources: &[SourceFile],
    config: &CommonPipelineConfig,
) -> Result<CommonPipelineResult, CompilationError> {
    use std::time::Instant;
    use crate::phases::*;
    use verum_lexer::Lexer;
    use verum_fast_parser::VerumParser;

    let start = Instant::now();
    let user_source_count = sources.len();

    // Load core stdlib sources if configured
    let mut all_sources: Vec<SourceFile> = Vec::new();
    if let Some(ref core_path) = config.core_source_path {
        match crate::core_source::CoreSource::local(core_path) {
            Ok(core_src) => {
                let loaded = core_src.load_all_source_files();
                tracing::debug!("Loaded {} core source files", loaded.len());
                all_sources = loaded;
            }
            Err(e) => {
                tracing::warn!("Failed to load core sources from {:?}: {}", core_path, e);
            }
        }
    }
    all_sources.extend(sources.iter().cloned());
    let sources = &all_sources;

    let mut all_diagnostics: List<Diagnostic> = List::new();
    let mut phase_metrics: List<PhaseMetrics> = List::new();

    // Phase context
    let context = PhaseContext {
        profile: LanguageProfile::Application,
        target_tier: ExecutionTier::Interpreter,
        verify_mode: if config.verify_contracts {
            PhaseVerifyMode::Proof
        } else {
            PhaseVerifyMode::None
        },
        opt_level: OptimizationLevel::O0,
    };

    // Phase 1: Lexical Parsing (directly from content, not files)
    let parse_start = Instant::now();
    let mut ast_modules: List<Module> = List::new();
    let parser = VerumParser::new();

    let core_source_count = sources.len() - user_source_count;
    for (i, source) in sources.iter().enumerate() {
        let file_id = verum_ast::FileId::new(i as u32);
        let is_core_source = i < core_source_count;
        let lexer = Lexer::new(source.content.as_str(), file_id);

        match parser.parse_module(lexer, file_id) {
            Ok(module) => {
                tracing::debug!(
                    "Successfully parsed {} ({} items)",
                    source.path,
                    module.items.len()
                );
                ast_modules.push(module);
            }
            Err(errors) => {
                if is_core_source {
                    // Core/stdlib parse errors are non-fatal — log and skip the module
                    tracing::debug!(
                        "Skipping core module {} ({} parse errors)",
                        source.path, errors.len()
                    );
                } else {
                    // User source parse errors are fatal
                    for error in errors {
                        let diag = verum_diagnostics::DiagnosticBuilder::new(
                            verum_diagnostics::Severity::Error
                        )
                            .message(format!("Parse error in {}: {}", source.path, error))
                            .build();
                        all_diagnostics.push(diag);
                    }
                }
            }
        }
    }

    // If we have errors and no successful modules, fail
    if !all_diagnostics.is_empty() && ast_modules.is_empty() {
        return Err(CompilationError::with_diagnostics(
            CompilationErrorKind::ParseError,
            "Parsing failed",
            all_diagnostics,
        ));
    }

    let parse_duration = parse_start.elapsed();
    phase_metrics.push(PhaseMetrics::new("Lexical Parsing")
        .with_duration(parse_duration)
        .with_items_processed(ast_modules.len()));

    // Phase 2: Meta Registry (if macros enabled)
    let mut current_data = PhaseData::AstModules(ast_modules.clone());

    if config.expand_macros {
        let meta_phase = meta_registry_phase::MetaRegistryPhase::new();
        let meta_input = PhaseInput {
            data: current_data.clone(),
            context: context.clone(),
        };

        let meta_output = meta_phase.execute(meta_input).map_err(|diags| {
            CompilationError::with_diagnostics(
                CompilationErrorKind::InternalError,
                "Meta registry failed",
                diags,
            )
        })?;

        all_diagnostics.extend(meta_output.warnings.clone());
        phase_metrics.push(meta_output.metrics.clone());
        current_data = meta_output.data;
    }

    // Phase 3: Macro Expansion
    if config.expand_macros {
        let expansion_phase = macro_expansion::MacroExpansionPhase::new()
            .with_derive_enabled(config.derive_enabled)
            .with_compile_time_enabled(config.compile_time_enabled);
        let expansion_input = PhaseInput {
            data: current_data.clone(),
            context: context.clone(),
        };

        let expansion_output = expansion_phase.execute(expansion_input).map_err(|diags| {
            CompilationError::with_diagnostics(
                CompilationErrorKind::InternalError,
                "Macro expansion failed",
                diags,
            )
        })?;

        all_diagnostics.extend(expansion_output.warnings.clone());
        phase_metrics.push(expansion_output.metrics.clone());
        current_data = expansion_output.data;
    }

    // Phase 3a: Contract Verification (if enabled)
    let mut verification_results = VerificationResults::default();

    if config.verify_contracts {
        let contract_phase = contract_verification::ContractVerificationPhase::new();
        let contract_input = PhaseInput {
            data: current_data.clone(),
            context: context.clone(),
        };

        let contract_output = contract_phase.execute(contract_input).map_err(|diags| {
            CompilationError::with_diagnostics(
                CompilationErrorKind::VerificationError,
                "Contract verification failed",
                diags,
            )
        })?;

        all_diagnostics.extend(contract_output.warnings.clone());
        phase_metrics.push(contract_output.metrics.clone());

        // Extract verification results
        if let PhaseData::AstModulesWithContracts { modules, verification_results: vr } = contract_output.data {
            current_data = PhaseData::AstModules(modules);
            verification_results = vr;
        } else {
            current_data = contract_output.data;
        }
    }

    // Phase 3b: Safety Gate
    // Pre-typecheck AST walker that rejects language constructs
    // disabled by the current [safety] feature set. Runs before
    // semantic analysis so type-checking errors aren't compounded
    // onto gate errors.
    {
        let modules = match &current_data {
            PhaseData::AstModules(ms) => Some(ms),
            PhaseData::AstModulesWithContracts { modules, .. } => Some(modules),
            _ => None,
        };
        if let Some(modules) = modules {
            let slice: Vec<_> = modules.iter().cloned().collect();
            let gate_diags = crate::phases::safety_gate::check_unsafe_usage(
                &slice,
                config.unsafe_allowed,
            );
            if !gate_diags.is_empty() {
                return Err(CompilationError::with_diagnostics(
                    CompilationErrorKind::TypeError,
                    "Safety gate rejected disallowed language constructs",
                    gate_diags,
                ));
            }
        }
    }

    // Phase 4: Semantic Analysis (Type Checking)
    // When core sources are prepended, tell the semantic phase how many user modules
    // there are so it can treat stdlib registration errors as non-fatal.
    let semantic_phase = if config.core_source_path.is_some() {
        semantic_analysis::SemanticAnalysisPhase::new()
            .with_user_module_count(user_source_count)
            .with_cubical_enabled(config.cubical_enabled)
    } else {
        semantic_analysis::SemanticAnalysisPhase::new()
            .with_cubical_enabled(config.cubical_enabled)
    };
    let semantic_input = PhaseInput {
        data: current_data,
        context: context.clone(),
    };

    let semantic_output = semantic_phase.execute(semantic_input).map_err(|diags| {
        CompilationError::with_diagnostics(
            CompilationErrorKind::TypeError,
            "Type checking failed",
            diags,
        )
    })?;

    all_diagnostics.extend(semantic_output.warnings.clone());
    phase_metrics.push(semantic_output.metrics.clone());

    let typed_modules = match semantic_output.data {
        PhaseData::Hir(modules) => modules,
        PhaseData::AstModules(modules) => {
            // Fallback: convert AST to minimal HIR
            modules.iter().map(|m| HirModule {
                name: Text::from(format!("module_{}", m.file_id.raw())),
                items: List::new(),
                ffi_boundaries: List::new(),
                verified_contracts: List::new(),
                context_requirements: List::new(),
            }).collect()
        }
        _ => return Err(CompilationError::internal("Unexpected semantic output")),
    };

    // Phase 4b: Context Validation (if enabled)
    // Both `validate_contexts` (per-invocation override) and
    // `context_enabled` (from `[context] enabled` in `verum.toml`)
    // must be on. Disabling either skips the phase entirely.
    if config.validate_contexts && config.context_enabled {
        let context_phase = context_validation::ContextValidationPhase::new();
        let context_input = PhaseInput {
            data: PhaseData::Hir(typed_modules.clone()),
            context: context.clone(),
        };

        match context_phase.execute(context_input) {
            Ok(output) => {
                all_diagnostics.extend(output.warnings);
                phase_metrics.push(output.metrics);
            }
            Err(diags) => {
                // Context validation errors are warnings, not fatal
                all_diagnostics.extend(diags);
            }
        }
    }

    let total_duration = start.elapsed();

    let metrics = PipelineMetrics {
        total_duration,
        phase_metrics,
        modules_processed: ast_modules.len(),
        functions_processed: typed_modules.iter().map(|m| {
            m.items.iter().filter(|item| matches!(item.kind, HirItemKind::Function(_))).count()
        }).sum(),
        types_processed: typed_modules.iter().map(|m| {
            m.items.iter().filter(|item| matches!(item.kind, HirItemKind::Type(_) | HirItemKind::Protocol(_))).count()
        }).sum(),
    };

    let has_errors = all_diagnostics.iter().any(|d| d.severity() == verum_diagnostics::Severity::Error);

    Ok(CommonPipelineResult {
        typed_modules,
        ast_modules,
        diagnostics: all_diagnostics,
        verification_results,
        metrics,
        success: !has_errors,
    })
}

/// Compile source code to VBC bytecode.
///
/// This runs Phases 1-5 of the compilation pipeline:
/// - Phases 1-4b: Common pipeline
/// - Phase 5: VBC code generation
///
/// # Arguments
///
/// * `source` - Source code to compile
///
/// # Returns
///
/// VBC module or compilation error
///
/// # Example
///
/// ```ignore
/// let vbc = verum_compiler::api::compile_to_vbc("fn main() { print(42); }")?;
/// ```
pub fn compile_to_vbc(source: &str) -> Result<verum_vbc::VbcModule, CompilationError> {
    let config = CommonPipelineConfig::minimal();
    let source_file = SourceFile::from_string(source);
    let result = run_common_pipeline(&[source_file], &config)?;

    if result.has_errors() {
        return Err(CompilationError::with_diagnostics(
            CompilationErrorKind::TypeError,
            "Compilation failed",
            result.diagnostics,
        ));
    }

    // Get AST module for VBC codegen
    let ast_module = result.ast_modules.first().ok_or_else(|| {
        CompilationError::internal("No AST module produced")
    })?;

    // Run VBC codegen
    use verum_vbc::codegen::VbcCodegen;

    let mut codegen = VbcCodegen::new();
    codegen.compile_module(ast_module).map_err(|e| {
        CompilationError::new(
            CompilationErrorKind::CodegenError,
            format!("VBC codegen error: {}", e),
        )
    })
}

/// Run full compilation pipeline.
///
/// This runs all phases from source to final output:
/// - Phases 1-4b: Common pipeline
/// - Phase 5: VBC code generation
/// - Phase 6: Monomorphization (see `phases/vbc_mono.rs`)
/// - Phase 7: Execution/compilation (tier-dependent)
///
/// # Arguments
///
/// * `sources` - Source files to compile
/// * `config` - Compiler configuration
///
/// # Returns
///
/// Full compilation result including artifacts
///
/// # Example
///
/// ```ignore
/// let config = CompilerConfig {
///     target_tier: ExecutionTier::Aot,
///     ..Default::default()
/// };
/// let result = compile(&[SourceFile::load("main.vr")?], &config)?;
/// ```
pub fn compile(
    sources: &[SourceFile],
    config: &CompilerConfig,
) -> Result<CompilationResult, CompilationError> {
    // Run common pipeline
    let common_result = run_common_pipeline(sources, &config.common)?;

    if common_result.has_errors() {
        return Ok(CompilationResult {
            common: common_result,
            vbc_modules: None,
            artifacts: None,
        });
    }

    // Run VBC backend pipeline: TypedAST → VBC bytecode
    use verum_vbc::codegen::VbcCodegen;
    use crate::phases::VbcTierStats;

    let mut vbc_modules = List::new();
    let mut codegen_errors = List::new();

    for ast_module in common_result.ast_modules.iter() {
        let mut codegen = VbcCodegen::new();
        match codegen.compile_module(ast_module) {
            Ok(vbc_module) => {
                vbc_modules.push(crate::phases::VbcModuleData {
                    module: vbc_module,
                    tier_stats: VbcTierStats::default(),
                });
            }
            Err(e) => {
                codegen_errors.push(
                    verum_diagnostics::DiagnosticBuilder::error()
                        .message(format!("VBC codegen error: {}", e))
                        .build(),
                );
            }
        }
    }

    if !codegen_errors.is_empty() {
        // Return result with errors but no VBC modules
        let mut result_common = common_result;
        result_common.diagnostics.extend(codegen_errors);
        return Ok(CompilationResult {
            common: result_common,
            vbc_modules: None,
            artifacts: None,
        });
    }

    let has_modules = !vbc_modules.is_empty();

    Ok(CompilationResult {
        common: common_result,
        vbc_modules: if has_modules { Some(vbc_modules) } else { None },
        artifacts: None,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let result = parse("fn main() {}");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_error() {
        let result = parse("fn main( {}");
        assert!(result.is_err());
    }

    #[test]
    fn test_source_file_from_string() {
        let sf = SourceFile::from_string("let x = 42;");
        assert_eq!(sf.path.as_str(), "<string>");
        assert_eq!(sf.content.as_str(), "let x = 42;");
    }

    #[test]
    fn test_common_pipeline_config_default() {
        let config = CommonPipelineConfig::default();
        assert!(config.verify_contracts);
        assert!(config.expand_macros);
    }

    #[test]
    fn test_common_pipeline_config_minimal() {
        let config = CommonPipelineConfig::minimal();
        assert!(!config.verify_contracts);
        assert!(config.expand_macros);
    }

    #[test]
    fn test_compilation_error_display() {
        let err = CompilationError::parse_error("unexpected token");
        assert!(err.to_string().contains("ParseError"));
    }
}
