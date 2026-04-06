//! Meta execution errors
//!
//! This module provides the error types used during compile-time meta function execution.
//!
//! ## Error Categories (46 total error codes)
//!
//! - **M0XX - Core meta errors** (8 codes): Basic evaluation and stage failures
//! - **M1XX - Builtin errors** (5 codes): Builtin function call failures
//! - **M2XX - Context errors** (5 codes): Missing or invalid context
//! - **M3XX - Sandbox errors** (7 codes): Resource and security violations
//! - **M4XX - Quote/Hygiene errors** (8 codes): Code generation issues
//! - **M5XX - Type-level errors** (6 codes): Type computation failures
//! - **M6XX - Const evaluation errors** (7 codes): Constant folding failures
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).
//! Meta validation: type checking, sandbox compliance, and resource limit
//! enforcement for meta functions before compile-time execution.

use verum_common::Text;

use super::builtins::RequiredContext;

/// Meta execution error
///
/// Error codes follow the M-prefix pattern:
/// - M0XX: Core meta errors (function not found, argument mismatch, type errors)
/// - M1XX: Sandbox violations (forbidden I/O, iteration limits, recursion depth)
/// - M2XX: Quote/splice errors (hygiene violations, unresolved splices)
/// - M3XX: Evaluation errors (division by zero, overflow, assertion failures)
/// - M4XX: Registration errors (duplicate macros, invalid attributes)
#[derive(Debug, Clone)]
pub enum MetaError {
    // =========================================================================
    // M0XX - Core meta errors (8 codes)
    // =========================================================================

    /// M001: Meta function not found in scope
    MetaFunctionNotFound(Text),

    /// M002: Wrong number of arguments to meta function
    MetaArityMismatch { function: Text, expected: usize, got: usize },

    /// M003: Type mismatch in meta expression
    TypeMismatch { expected: Text, found: Text },

    /// M004: Meta expression evaluation failed
    MetaEvaluationFailed { message: Text },

    /// M005: Circular dependency in meta function calls
    CircularDependency { path: Text },

    /// M006: Invalid stage level in meta(N) or quote(M)
    InvalidMetaStage { stage: i64, message: Text },

    /// M007: Stage coherence violation
    MetaStageMismatch { current: u32, target: u32, message: Text },

    /// M008: Meta function must be pure but has side effects
    MetaFunctionNotPure { function: Text, reason: Text },

    // =========================================================================
    // M1XX - Builtin errors (5 codes)
    // =========================================================================

    /// M101: Unknown builtin function
    UnknownBuiltin(Text),

    /// M102: Wrong number of arguments to builtin
    ArityMismatch { expected: usize, got: usize },

    /// M103: Argument type doesn't match builtin signature
    TypeMismatchBuiltin { function: Text, expected: Text, found: Text },

    /// M104: Builtin execution failed
    BuiltinEvalError { function: Text, message: Text },

    /// M105: Builtin not available at current stage
    BuiltinNotAvailable { function: Text, stage: u32 },

    /// M106: Assertion failed during meta execution
    AssertionFailed { message: Text },

    // =========================================================================
    // M2XX - Context errors (5 codes)
    // =========================================================================

    /// M201: Required context not declared in using clause
    MissingContext {
        function: Text,
        required: RequiredContext,
    },

    /// M202: Unknown context name in using clause
    UnknownContext(Text),

    /// M203: Operation not allowed by context capabilities
    ContextCapabilityDenied { context: Text, operation: Text },

    /// M204: Context used outside its valid scope
    ContextScopeViolation { context: Text, message: Text },

    /// M205: Same context declared multiple times
    DuplicateContext(Text),

    // =========================================================================
    // M3XX - Sandbox errors (7 codes)
    // =========================================================================

    /// M301: Operation not allowed in sandbox
    ForbiddenOperation { operation: Text, reason: Text },

    /// M302: Meta execution exceeded memory limit
    MemoryLimitExceeded { allocated: usize, limit: usize },

    /// M303: Recursion depth exceeded
    RecursionLimitExceeded { depth: usize, limit: usize },

    /// M304: Loop iteration count exceeded
    IterationLimitExceeded { count: usize, limit: usize },

    /// M305: Meta execution timed out
    TimeoutExceeded { elapsed_ms: u64, limit_ms: u64 },

    /// M306: File/network IO not allowed in meta context
    IONotAllowed { operation: Text },

    /// M307: Unsafe code not allowed in meta context
    UnsafeNotAllowed { construct: Text },

    /// M308: Path traversal attack blocked
    PathTraversalBlocked { path: Text, reason: Text },

    // =========================================================================
    // M4XX - Quote/Hygiene errors (10 codes: M400-M409)
    // =========================================================================

    /// M400: Invalid syntax inside quote block
    InvalidQuoteSyntax { message: Text },

    /// M401: $ splice used outside quote block
    UnquoteOutsideQuote,

    /// M402: Name collision that hygiene should prevent (accidental capture)
    HygieneViolation { identifier: Text, message: Text },

    /// M403: Generated symbol collision (internal error)
    GensymCollision { symbol: Text },

    /// M404: Cannot resolve binding in generated code
    ScopeResolutionFailed { name: Text, message: Text },

    /// M405: Invalid quote target stage
    QuoteStageError { target: u32, current: u32 },

    /// M406: Lift expression type cannot be lifted
    LiftTypeMismatch { ty: Text, reason: Text },

    /// M407: Malformed token tree in quote
    InvalidTokenTree { message: Text },

    /// M408: Variable captured without explicit capture clause
    CaptureNotDeclared { identifier: Text, span: verum_common::Span },

    /// M409: Mismatched lengths in $[for...] expansion
    RepetitionMismatch { first_name: Text, first_len: usize, second_name: Text, second_len: usize },

    // =========================================================================
    // M5XX - Type-level computation errors (6 codes)
    // =========================================================================

    /// M501: Type-level computation failed to reduce
    TypeReductionFailed { ty: Text, message: Text },

    /// M502: Type normalization didn't terminate
    NormalizationDiverged { ty: Text, iterations: usize },

    /// M503: SMT solver couldn't verify type constraint
    SMTVerificationFailed { constraint: Text, reason: Text },

    /// M504: Failed to construct proof term
    ProofConstructionFailed { goal: Text, message: Text },

    /// M505: Refinement predicate not satisfied
    RefinementViolation { predicate: Text, value: Text },

    /// M506: meta where constraint not satisfied
    MetaWhereUnsatisfied { constraint: Text },

    // =========================================================================
    // M6XX - Const evaluation errors (7 codes)
    // =========================================================================

    /// M601: Expression cannot be evaluated at compile time
    NonConstExpression { expr: Text, reason: Text },

    /// M602: Integer overflow in const evaluation
    ConstOverflow { operation: Text, value: Text },

    /// M603: Division by zero in const evaluation
    DivisionByZero,

    /// M604: Type mismatch in const expression
    ConstTypeMismatch { expected: Text, found: Text },

    /// M605: Index out of bounds in const evaluation
    IndexOutOfBounds { index: i128, length: usize },

    /// M606: Non-exhaustive pattern in const match
    PatternNotExhaustive { missing: Text },

    /// M607: No matching arm in const match
    NoMatchingArm { value: Text },

    // =========================================================================
    // User-initiated and other errors
    // =========================================================================

    /// Compile-time error emitted by compile_error!
    CompileError(Text),

    /// Compile-time warning emitted by compile_warning!
    CompileWarning(Text),

    /// Other error (catch-all)
    Other(Text),

    /// Parse error during meta evaluation
    ParseError(Text),
}

impl MetaError {
    /// Returns the error code for this error type
    ///
    /// Error code pattern (M-prefix):
    /// - M0XX: Core meta errors (8 codes)
    /// - M1XX: Builtin errors (5 codes)
    /// - M2XX: Context errors (5 codes)
    /// - M3XX: Sandbox errors (7 codes)
    /// - M4XX: Quote/Hygiene errors (8 codes)
    /// - M5XX: Type-level errors (6 codes)
    /// - M6XX: Const evaluation errors (7 codes)
    pub fn error_code(&self) -> &'static str {
        match self {
            // M0XX - Core meta errors
            MetaError::MetaFunctionNotFound(_) => "M001",
            MetaError::MetaArityMismatch { .. } => "M002",
            MetaError::TypeMismatch { .. } => "M003",
            MetaError::MetaEvaluationFailed { .. } => "M004",
            MetaError::CircularDependency { .. } => "M005",
            MetaError::InvalidMetaStage { .. } => "M006",
            MetaError::MetaStageMismatch { .. } => "M007",
            MetaError::MetaFunctionNotPure { .. } => "M008",

            // M1XX - Builtin errors
            MetaError::UnknownBuiltin(_) => "M101",
            MetaError::ArityMismatch { .. } => "M102",
            MetaError::TypeMismatchBuiltin { .. } => "M103",
            MetaError::BuiltinEvalError { .. } => "M104",
            MetaError::BuiltinNotAvailable { .. } => "M105",
            MetaError::AssertionFailed { .. } => "M106",

            // M2XX - Context errors
            MetaError::MissingContext { .. } => "M201",
            MetaError::UnknownContext(_) => "M202",
            MetaError::ContextCapabilityDenied { .. } => "M203",
            MetaError::ContextScopeViolation { .. } => "M204",
            MetaError::DuplicateContext(_) => "M205",

            // M3XX - Sandbox errors
            MetaError::ForbiddenOperation { .. } => "M301",
            MetaError::MemoryLimitExceeded { .. } => "M302",
            MetaError::RecursionLimitExceeded { .. } => "M303",
            MetaError::IterationLimitExceeded { .. } => "M304",
            MetaError::TimeoutExceeded { .. } => "M305",
            MetaError::IONotAllowed { .. } => "M306",
            MetaError::UnsafeNotAllowed { .. } => "M307",
            MetaError::PathTraversalBlocked { .. } => "M308",

            // M4XX - Quote/Hygiene errors
            MetaError::InvalidQuoteSyntax { .. } => "M400",
            MetaError::UnquoteOutsideQuote => "M401",
            MetaError::HygieneViolation { .. } => "M402",
            MetaError::GensymCollision { .. } => "M403",
            MetaError::ScopeResolutionFailed { .. } => "M404",
            MetaError::QuoteStageError { .. } => "M405",
            MetaError::LiftTypeMismatch { .. } => "M406",
            MetaError::InvalidTokenTree { .. } => "M407",
            MetaError::CaptureNotDeclared { .. } => "M408",
            MetaError::RepetitionMismatch { .. } => "M409",

            // M5XX - Type-level errors
            MetaError::TypeReductionFailed { .. } => "M501",
            MetaError::NormalizationDiverged { .. } => "M502",
            MetaError::SMTVerificationFailed { .. } => "M503",
            MetaError::ProofConstructionFailed { .. } => "M504",
            MetaError::RefinementViolation { .. } => "M505",
            MetaError::MetaWhereUnsatisfied { .. } => "M506",

            // M6XX - Const evaluation errors
            MetaError::NonConstExpression { .. } => "M601",
            MetaError::ConstOverflow { .. } => "M602",
            MetaError::DivisionByZero => "M603",
            MetaError::ConstTypeMismatch { .. } => "M604",
            MetaError::IndexOutOfBounds { .. } => "M605",
            MetaError::PatternNotExhaustive { .. } => "M606",
            MetaError::NoMatchingArm { .. } => "M607",

            // Other errors (no specific code)
            MetaError::CompileError(_) => "M000",
            MetaError::CompileWarning(_) => "M000",
            MetaError::Other(_) => "M000",
            MetaError::ParseError(_) => "M000",
        }
    }

    /// Returns the equivalent E-code for this meta error, for compatibility
    /// with the standard compiler error code namespace.
    ///
    /// Meta errors have their own M-prefixed codes, but some consumers expect
    /// the standard E-prefixed codes. This mapping provides the equivalent:
    /// - M003 (TypeMismatch) → E101
    /// - M201 (MissingContext) → E101
    /// - M202 (UnknownContext) → E100
    /// - M301 (ForbiddenOperation) → E103
    /// - M103 (TypeMismatchBuiltin) → E101
    /// - M001 (MetaFunctionNotFound) → E100
    pub fn equivalent_e_code(&self) -> Option<&'static str> {
        match self {
            MetaError::MetaFunctionNotFound(_) => Some("E100"),
            MetaError::TypeMismatch { .. } => Some("E101"),
            MetaError::TypeMismatchBuiltin { .. } => Some("E101"),
            MetaError::MissingContext { .. } => Some("E100/E101"),
            MetaError::UnknownContext(_) => Some("E100"),
            MetaError::ForbiddenOperation { .. } => Some("E103"),
            MetaError::IONotAllowed { .. } => Some("E103"),
            MetaError::UnsafeNotAllowed { .. } => Some("E103"),
            _ => None,
        }
    }

    /// Returns true if this error has a specific error code (not M000)
    pub fn has_specific_code(&self) -> bool {
        self.error_code() != "M000"
    }

    /// Returns the error category name
    pub fn category(&self) -> &'static str {
        match self.error_code().chars().nth(1) {
            Some('0') => "Core Meta",
            Some('1') => "Builtin",
            Some('2') => "Context",
            Some('3') => "Sandbox",
            Some('4') => "Quote/Hygiene",
            Some('5') => "Type-Level",
            Some('6') => "Const Eval",
            _ => "Other",
        }
    }

    /// Returns the compilation phase where this error is detected (1-5)
    pub fn phase(&self) -> u8 {
        match self.error_code().chars().nth(1) {
            Some('0') => match self {
                // Name resolution phase (2)
                MetaError::MetaFunctionNotFound(_) |
                MetaError::CircularDependency { .. } => 2,
                // Evaluation phase (4)
                MetaError::MetaEvaluationFailed { .. } => 4,
                // Type checking phase (3) - default for M0XX
                _ => 3,
            },
            Some('1') => 3, // Builtin errors: Type checking
            Some('2') => 2, // Context errors: Name resolution
            Some('3') => 4, // Sandbox errors: Evaluation
            Some('4') => match self {
                MetaError::InvalidQuoteSyntax { .. } |
                MetaError::UnquoteOutsideQuote |
                MetaError::InvalidTokenTree { .. } => 1, // Parser
                MetaError::CaptureNotDeclared { .. } |
                MetaError::RepetitionMismatch { .. } => 5, // Hygiene
                _ => 5, // Hygiene
            },
            Some('5') => 3, // Type-level errors: Type checking
            Some('6') => match self {
                MetaError::NonConstExpression { .. } |
                MetaError::ConstTypeMismatch { .. } => 3, // Type checking
                _ => 4, // Evaluation
            },
            _ => 0,
        }
    }
}

impl std::fmt::Display for MetaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = self.error_code();
        // Include equivalent E-code suffix for compatibility with standard error matchers
        let e_suffix = self.equivalent_e_code()
            .map(|ec| format!(" [{}]", ec))
            .unwrap_or_default();

        match self {
            // M0XX - Core meta errors
            MetaError::MetaFunctionNotFound(name) => {
                write!(f, "{}: Meta function not found: {}{}", code, name.as_str(), e_suffix)
            }
            MetaError::MetaArityMismatch { function, expected, got } => {
                write!(f, "{}: Meta function '{}': expected {} arguments, got {}",
                    code, function.as_str(), expected, got)
            }
            MetaError::TypeMismatch { expected, found } => {
                write!(f, "{}: Type mismatch: expected {}, found {}{}",
                    code, expected.as_str(), found.as_str(), e_suffix)
            }
            MetaError::MetaEvaluationFailed { message } => {
                write!(f, "{}: Meta evaluation failed: {}", code, message.as_str())
            }
            MetaError::CircularDependency { path } => {
                write!(f, "{}: Circular dependency: {}", code, path.as_str())
            }
            MetaError::InvalidMetaStage { stage, message } => {
                write!(f, "{}: Invalid meta stage {}: {}", code, stage, message.as_str())
            }
            MetaError::MetaStageMismatch { current, target, message } => {
                write!(f, "{}: Stage mismatch (current: {}, target: {}): {}",
                    code, current, target, message.as_str())
            }
            MetaError::MetaFunctionNotPure { function, reason } => {
                write!(f, "{}: Meta function '{}' not pure: {}",
                    code, function.as_str(), reason.as_str())
            }

            // M1XX - Builtin errors
            MetaError::UnknownBuiltin(name) => {
                write!(f, "{}: Unknown builtin: {}", code, name.as_str())
            }
            MetaError::ArityMismatch { expected, got } => {
                write!(f, "{}: Expected {} arguments, got {}", code, expected, got)
            }
            MetaError::TypeMismatchBuiltin { function, expected, found } => {
                write!(f, "{}: Builtin '{}': expected {}, found {}{}",
                    code, function.as_str(), expected.as_str(), found.as_str(), e_suffix)
            }
            MetaError::BuiltinEvalError { function, message } => {
                write!(f, "{}: Builtin '{}' failed: {}",
                    code, function.as_str(), message.as_str())
            }
            MetaError::BuiltinNotAvailable { function, stage } => {
                write!(f, "{}: Builtin '{}' not available at stage {}",
                    code, function.as_str(), stage)
            }
            MetaError::AssertionFailed { message } => {
                write!(f, "{}: Assertion failed: {}", code, message.as_str())
            }

            // M2XX - Context errors
            MetaError::MissingContext { function, required } => {
                write!(f, "{}: '{}' requires `using [{}]`{}",
                    code, function.as_str(), required.context_name(), e_suffix)
            }
            MetaError::UnknownContext(name) => {
                write!(f, "{}: Unknown context: {}{}", code, name.as_str(), e_suffix)
            }
            MetaError::ContextCapabilityDenied { context, operation } => {
                write!(f, "{}: Context '{}' denies operation: {}",
                    code, context.as_str(), operation.as_str())
            }
            MetaError::ContextScopeViolation { context, message } => {
                write!(f, "{}: Context '{}' scope violation: {}",
                    code, context.as_str(), message.as_str())
            }
            MetaError::DuplicateContext(name) => {
                write!(f, "{}: Duplicate context: {}", code, name.as_str())
            }

            // M3XX - Sandbox errors
            MetaError::ForbiddenOperation { operation, reason } => {
                write!(f, "{}: Forbidden '{}': {}{}",
                    code, operation.as_str(), reason.as_str(), e_suffix)
            }
            MetaError::MemoryLimitExceeded { allocated, limit } => {
                write!(f, "{}: Memory limit exceeded: {} bytes (limit: {})",
                    code, allocated, limit)
            }
            MetaError::RecursionLimitExceeded { depth, limit } => {
                write!(f, "{}: Recursion limit exceeded: depth {} (limit: {})",
                    code, depth, limit)
            }
            MetaError::IterationLimitExceeded { count, limit } => {
                write!(f, "{}: Iteration limit exceeded: {} (limit: {})",
                    code, count, limit)
            }
            MetaError::TimeoutExceeded { elapsed_ms, limit_ms } => {
                write!(f, "{}: Timeout: {}ms (limit: {}ms)",
                    code, elapsed_ms, limit_ms)
            }
            MetaError::IONotAllowed { operation } => {
                write!(f, "{}: IO not allowed: {}{}", code, operation.as_str(), e_suffix)
            }
            MetaError::UnsafeNotAllowed { construct } => {
                write!(f, "{}: Unsafe not allowed: {}{}", code, construct.as_str(), e_suffix)
            }
            MetaError::PathTraversalBlocked { path, reason } => {
                write!(
                    f,
                    "{}: Path traversal blocked for '{}': {}",
                    code,
                    path.as_str(),
                    reason.as_str()
                )
            }

            // M4XX - Quote/Hygiene errors
            MetaError::InvalidQuoteSyntax { message } => {
                write!(f, "{}: Invalid quote syntax: {}", code, message.as_str())
            }
            MetaError::UnquoteOutsideQuote => {
                write!(f, "{}: Unquote ($) outside quote block", code)
            }
            MetaError::HygieneViolation { identifier, message } => {
                write!(f, "{}: Hygiene violation for '{}': {}",
                    code, identifier.as_str(), message.as_str())
            }
            MetaError::GensymCollision { symbol } => {
                write!(f, "{}: Gensym collision: {}", code, symbol.as_str())
            }
            MetaError::ScopeResolutionFailed { name, message } => {
                write!(f, "{}: Cannot resolve '{}': {}",
                    code, name.as_str(), message.as_str())
            }
            MetaError::QuoteStageError { target, current } => {
                write!(f, "{}: Quote stage {} >= current stage {}",
                    code, target, current)
            }
            MetaError::LiftTypeMismatch { ty, reason } => {
                write!(f, "{}: Cannot lift type '{}': {}",
                    code, ty.as_str(), reason.as_str())
            }
            MetaError::InvalidTokenTree { message } => {
                write!(f, "{}: Invalid token tree: {}", code, message.as_str())
            }
            MetaError::CaptureNotDeclared { identifier, .. } => {
                write!(f, "{}: Variable '{}' captured without explicit capture clause",
                    code, identifier.as_str())
            }
            MetaError::RepetitionMismatch { first_name, first_len, second_name, second_len } => {
                write!(f, "{}: Repetition mismatch: '{}' has {} elements, '{}' has {}",
                    code, first_name.as_str(), first_len, second_name.as_str(), second_len)
            }

            // M5XX - Type-level errors
            MetaError::TypeReductionFailed { ty, message } => {
                write!(f, "{}: Type '{}' reduction failed: {}",
                    code, ty.as_str(), message.as_str())
            }
            MetaError::NormalizationDiverged { ty, iterations } => {
                write!(f, "{}: Type '{}' normalization diverged after {} iterations",
                    code, ty.as_str(), iterations)
            }
            MetaError::SMTVerificationFailed { constraint, reason } => {
                write!(f, "{}: SMT verification failed for '{}': {}",
                    code, constraint.as_str(), reason.as_str())
            }
            MetaError::ProofConstructionFailed { goal, message } => {
                write!(f, "{}: Proof construction failed for '{}': {}",
                    code, goal.as_str(), message.as_str())
            }
            MetaError::RefinementViolation { predicate, value } => {
                write!(f, "{}: Refinement '{}' violated by: {}",
                    code, predicate.as_str(), value.as_str())
            }
            MetaError::MetaWhereUnsatisfied { constraint } => {
                write!(f, "{}: meta where not satisfied: {}", code, constraint.as_str())
            }

            // M6XX - Const evaluation errors
            MetaError::NonConstExpression { expr, reason } => {
                write!(f, "{}: '{}' is not const: {}",
                    code, expr.as_str(), reason.as_str())
            }
            MetaError::ConstOverflow { operation, value } => {
                write!(f, "{}: Overflow in '{}': {}",
                    code, operation.as_str(), value.as_str())
            }
            MetaError::DivisionByZero => {
                write!(f, "{}: Division by zero", code)
            }
            MetaError::ConstTypeMismatch { expected, found } => {
                write!(f, "{}: Const type mismatch: expected {}, found {}",
                    code, expected.as_str(), found.as_str())
            }
            MetaError::IndexOutOfBounds { index, length } => {
                write!(f, "{}: Index {} out of bounds (length: {})",
                    code, index, length)
            }
            MetaError::PatternNotExhaustive { missing } => {
                write!(f, "{}: Non-exhaustive pattern, missing: {}",
                    code, missing.as_str())
            }
            MetaError::NoMatchingArm { value } => {
                write!(f, "{}: No matching arm for value: {}", code, value.as_str())
            }

            // Other errors
            MetaError::CompileError(msg) => {
                write!(f, "Compile error: {}", msg.as_str())
            }
            MetaError::CompileWarning(msg) => {
                write!(f, "Compile warning: {}", msg.as_str())
            }
            MetaError::Other(msg) => {
                write!(f, "{}", msg.as_str())
            }
            MetaError::ParseError(msg) => {
                write!(f, "Parse error: {}", msg.as_str())
            }
        }
    }
}

impl std::error::Error for MetaError {}

// ============================================================================
// Integration with Hygiene Module
// ============================================================================

impl MetaError {
    /// Convert a HygieneViolation to a MetaError
    ///
    /// This method provides explicit conversion from hygiene violations to
    /// meta errors without implementing From (which can cause type inference issues).
    pub fn from_hygiene_violation(v: crate::hygiene::HygieneViolation) -> Self {
        use crate::hygiene::HygieneViolation;

        match v {
            HygieneViolation::InvalidQuoteSyntax { message, .. } => {
                MetaError::InvalidQuoteSyntax { message }
            }
            HygieneViolation::UnquoteOutsideQuote { .. } => MetaError::UnquoteOutsideQuote,
            HygieneViolation::AccidentalCapture { captured, .. } => MetaError::HygieneViolation {
                identifier: captured.name.clone(),
                message: Text::from("accidental capture of outer binding"),
            },
            HygieneViolation::ShadowConflict { shadowed, .. } => MetaError::HygieneViolation {
                identifier: shadowed.name.clone(),
                message: Text::from("shadowing conflict across hygiene boundary"),
            },
            HygieneViolation::GensymCollision { name, .. } => {
                MetaError::GensymCollision { symbol: name }
            }
            HygieneViolation::ScopeResolutionFailed { ident, .. } => {
                MetaError::ScopeResolutionFailed {
                    name: ident,
                    message: Text::from("scope resolution failed"),
                }
            }
            HygieneViolation::StageMismatch {
                expected_stage,
                actual_stage,
                ..
            } => MetaError::QuoteStageError {
                target: actual_stage,
                current: expected_stage,
            },
            HygieneViolation::LiftTypeMismatch {
                expected, found, ..
            } => MetaError::LiftTypeMismatch {
                ty: expected,
                reason: Text::from(format!("found type {}", found.as_str())),
            },
            HygieneViolation::InvalidTokenTree { message, .. } => {
                MetaError::InvalidTokenTree { message }
            }
            HygieneViolation::CaptureNotDeclared { ident, span } => {
                MetaError::CaptureNotDeclared {
                    identifier: ident,
                    span,
                }
            }
            HygieneViolation::RepetitionMismatch {
                first_name,
                first_len,
                second_name,
                second_len,
                ..
            } => {
                MetaError::RepetitionMismatch {
                    first_name,
                    first_len,
                    second_name,
                    second_len,
                }
            }
        }
    }

    /// Convert a collection of hygiene violations to MetaErrors
    pub fn from_hygiene_violations(violations: &crate::hygiene::HygieneViolations) -> Vec<Self> {
        violations
            .iter()
            .cloned()
            .map(MetaError::from_hygiene_violation)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_codes_m0xx() {
        assert_eq!(MetaError::MetaFunctionNotFound(Text::from("foo")).error_code(), "M001");
        assert_eq!(MetaError::MetaArityMismatch {
            function: Text::from("f"), expected: 2, got: 3
        }.error_code(), "M002");
        assert_eq!(MetaError::TypeMismatch {
            expected: Text::from("Int"), found: Text::from("Text")
        }.error_code(), "M003");
        assert_eq!(MetaError::MetaEvaluationFailed {
            message: Text::from("error")
        }.error_code(), "M004");
        assert_eq!(MetaError::CircularDependency {
            path: Text::from("a -> b -> a")
        }.error_code(), "M005");
        assert_eq!(MetaError::InvalidMetaStage {
            stage: 0, message: Text::from("must be >= 1")
        }.error_code(), "M006");
        assert_eq!(MetaError::MetaStageMismatch {
            current: 1, target: 2, message: Text::from("invalid")
        }.error_code(), "M007");
        assert_eq!(MetaError::MetaFunctionNotPure {
            function: Text::from("f"), reason: Text::from("IO")
        }.error_code(), "M008");
    }

    #[test]
    fn test_error_codes_m1xx() {
        assert_eq!(MetaError::UnknownBuiltin(Text::from("foo")).error_code(), "M101");
        assert_eq!(MetaError::ArityMismatch { expected: 2, got: 3 }.error_code(), "M102");
        assert_eq!(MetaError::TypeMismatchBuiltin {
            function: Text::from("abs"), expected: Text::from("Int"), found: Text::from("Text")
        }.error_code(), "M103");
        assert_eq!(MetaError::BuiltinEvalError {
            function: Text::from("abs"), message: Text::from("error")
        }.error_code(), "M104");
        assert_eq!(MetaError::BuiltinNotAvailable {
            function: Text::from("f"), stage: 1
        }.error_code(), "M105");
    }

    #[test]
    fn test_error_codes_m2xx() {
        assert_eq!(MetaError::MissingContext {
            function: Text::from("type_name"), required: RequiredContext::MetaTypes
        }.error_code(), "M201");
        assert_eq!(MetaError::UnknownContext(Text::from("Unknown")).error_code(), "M202");
        assert_eq!(MetaError::ContextCapabilityDenied {
            context: Text::from("MetaTypes"), operation: Text::from("io")
        }.error_code(), "M203");
        assert_eq!(MetaError::ContextScopeViolation {
            context: Text::from("MetaTypes"), message: Text::from("error")
        }.error_code(), "M204");
        assert_eq!(MetaError::DuplicateContext(Text::from("MetaTypes")).error_code(), "M205");
    }

    #[test]
    fn test_error_codes_m3xx() {
        assert_eq!(MetaError::ForbiddenOperation {
            operation: Text::from("file_read"), reason: Text::from("sandbox")
        }.error_code(), "M301");
        assert_eq!(MetaError::MemoryLimitExceeded { allocated: 1000, limit: 100 }.error_code(), "M302");
        assert_eq!(MetaError::RecursionLimitExceeded { depth: 100, limit: 50 }.error_code(), "M303");
        assert_eq!(MetaError::IterationLimitExceeded { count: 1000, limit: 100 }.error_code(), "M304");
        assert_eq!(MetaError::TimeoutExceeded { elapsed_ms: 1000, limit_ms: 100 }.error_code(), "M305");
        assert_eq!(MetaError::IONotAllowed { operation: Text::from("read") }.error_code(), "M306");
        assert_eq!(MetaError::UnsafeNotAllowed { construct: Text::from("ptr") }.error_code(), "M307");
    }

    #[test]
    fn test_error_codes_m4xx() {
        assert_eq!(MetaError::InvalidQuoteSyntax { message: Text::from("error") }.error_code(), "M400");
        assert_eq!(MetaError::UnquoteOutsideQuote.error_code(), "M401");
        assert_eq!(MetaError::HygieneViolation {
            identifier: Text::from("x"), message: Text::from("collision")
        }.error_code(), "M402");
        assert_eq!(MetaError::GensymCollision { symbol: Text::from("_g1") }.error_code(), "M403");
        assert_eq!(MetaError::ScopeResolutionFailed {
            name: Text::from("x"), message: Text::from("not found")
        }.error_code(), "M404");
        assert_eq!(MetaError::QuoteStageError { target: 1, current: 1 }.error_code(), "M405");
        assert_eq!(MetaError::LiftTypeMismatch {
            ty: Text::from("Fn"), reason: Text::from("not liftable")
        }.error_code(), "M406");
        assert_eq!(MetaError::InvalidTokenTree { message: Text::from("error") }.error_code(), "M407");
        assert_eq!(MetaError::CaptureNotDeclared {
            identifier: Text::from("x"), span: verum_common::Span::default()
        }.error_code(), "M408");
        assert_eq!(MetaError::RepetitionMismatch {
            first_name: Text::from("a"), first_len: 2,
            second_name: Text::from("b"), second_len: 3
        }.error_code(), "M409");
    }

    #[test]
    fn test_error_codes_m5xx() {
        assert_eq!(MetaError::TypeReductionFailed {
            ty: Text::from("T"), message: Text::from("error")
        }.error_code(), "M501");
        assert_eq!(MetaError::NormalizationDiverged {
            ty: Text::from("T"), iterations: 1000
        }.error_code(), "M502");
        assert_eq!(MetaError::SMTVerificationFailed {
            constraint: Text::from("x > 0"), reason: Text::from("unsat")
        }.error_code(), "M503");
        assert_eq!(MetaError::ProofConstructionFailed {
            goal: Text::from("P"), message: Text::from("error")
        }.error_code(), "M504");
        assert_eq!(MetaError::RefinementViolation {
            predicate: Text::from("x > 0"), value: Text::from("-1")
        }.error_code(), "M505");
        assert_eq!(MetaError::MetaWhereUnsatisfied {
            constraint: Text::from("N > 0")
        }.error_code(), "M506");
    }

    #[test]
    fn test_error_codes_m6xx() {
        assert_eq!(MetaError::NonConstExpression {
            expr: Text::from("f()"), reason: Text::from("not const")
        }.error_code(), "M601");
        assert_eq!(MetaError::ConstOverflow {
            operation: Text::from("+"), value: Text::from("MAX + 1")
        }.error_code(), "M602");
        assert_eq!(MetaError::DivisionByZero.error_code(), "M603");
        assert_eq!(MetaError::ConstTypeMismatch {
            expected: Text::from("Int"), found: Text::from("Text")
        }.error_code(), "M604");
        assert_eq!(MetaError::IndexOutOfBounds { index: 10, length: 5 }.error_code(), "M605");
        assert_eq!(MetaError::PatternNotExhaustive {
            missing: Text::from("None")
        }.error_code(), "M606");
        assert_eq!(MetaError::NoMatchingArm {
            value: Text::from("42")
        }.error_code(), "M607");
    }

    #[test]
    fn test_error_categories() {
        assert_eq!(MetaError::MetaFunctionNotFound(Text::from("f")).category(), "Core Meta");
        assert_eq!(MetaError::UnknownBuiltin(Text::from("f")).category(), "Builtin");
        assert_eq!(MetaError::UnknownContext(Text::from("X")).category(), "Context");
        assert_eq!(MetaError::ForbiddenOperation {
            operation: Text::from("io"), reason: Text::from("")
        }.category(), "Sandbox");
        assert_eq!(MetaError::UnquoteOutsideQuote.category(), "Quote/Hygiene");
        assert_eq!(MetaError::TypeReductionFailed {
            ty: Text::from("T"), message: Text::from("")
        }.category(), "Type-Level");
        assert_eq!(MetaError::DivisionByZero.category(), "Const Eval");
    }

    #[test]
    fn test_error_phases() {
        // Parser phase (1)
        assert_eq!(MetaError::InvalidQuoteSyntax { message: Text::from("") }.phase(), 1);
        assert_eq!(MetaError::UnquoteOutsideQuote.phase(), 1);
        assert_eq!(MetaError::InvalidTokenTree { message: Text::from("") }.phase(), 1);

        // Name resolution phase (2)
        assert_eq!(MetaError::MetaFunctionNotFound(Text::from("f")).phase(), 2);
        assert_eq!(MetaError::UnknownContext(Text::from("X")).phase(), 2);

        // Type checking phase (3)
        assert_eq!(MetaError::TypeMismatch {
            expected: Text::from("Int"), found: Text::from("Text")
        }.phase(), 3);
        assert_eq!(MetaError::TypeReductionFailed {
            ty: Text::from("T"), message: Text::from("")
        }.phase(), 3);

        // Evaluation phase (4)
        assert_eq!(MetaError::DivisionByZero.phase(), 4);
        assert_eq!(MetaError::RecursionLimitExceeded { depth: 100, limit: 50 }.phase(), 4);

        // Hygiene phase (5)
        assert_eq!(MetaError::HygieneViolation {
            identifier: Text::from("x"), message: Text::from("")
        }.phase(), 5);
        assert_eq!(MetaError::ScopeResolutionFailed {
            name: Text::from("x"), message: Text::from("")
        }.phase(), 5);
        assert_eq!(MetaError::CaptureNotDeclared {
            identifier: Text::from("x"), span: verum_common::Span::default()
        }.phase(), 5);
        assert_eq!(MetaError::RepetitionMismatch {
            first_name: Text::from("a"), first_len: 2,
            second_name: Text::from("b"), second_len: 3
        }.phase(), 5);
    }
}
