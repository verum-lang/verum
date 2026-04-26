//! VBC codegen error types.
//!
//! This module defines errors that can occur during AST-to-VBC compilation.

use std::fmt;
use verum_ast::Span;

/// Result type for codegen operations.
pub type CodegenResult<T> = Result<T, CodegenError>;

/// Errors that can occur during VBC code generation.
#[derive(Debug)]
pub struct CodegenError {
    /// The kind of error.
    pub kind: CodegenErrorKind,
    /// Optional source location.
    pub span: Option<Span>,
    /// Optional additional context.
    pub context: Option<String>,
}

/// Categories of codegen errors.
#[derive(Debug, Clone)]
pub enum CodegenErrorKind {
    // === Expression Errors ===
    /// Unsupported expression type.
    UnsupportedExpr(String),

    /// Invalid literal value.
    InvalidLiteral(String),

    /// Invalid binary operation.
    InvalidBinaryOp(String),

    /// Invalid unary operation.
    InvalidUnaryOp(String),

    // === Variable Errors ===
    /// Undefined variable.
    UndefinedVariable(String),

    /// Variable already defined in current scope.
    VariableAlreadyDefined(String),

    /// Cannot assign to immutable variable.
    ImmutableAssignment(String),

    // === Function Errors ===
    /// Undefined function.
    UndefinedFunction(String),

    /// Wrong number of arguments.
    WrongArgumentCount {
        /// Expected argument count.
        expected: usize,
        /// Found argument count.
        found: usize,
        /// Function name.
        function: String,
    },

    /// Type mismatch in function arguments.
    ArgumentTypeMismatch {
        /// Argument position (0-indexed).
        position: usize,
        /// Expected type name.
        expected: String,
        /// Found type name.
        found: String,
    },

    // === Type Errors ===
    /// Type mismatch.
    TypeMismatch {
        /// Expected type name.
        expected: String,
        /// Found type name.
        found: String,
    },

    /// Cannot infer type.
    TypeInference(String),

    /// Invalid type for operation.
    InvalidTypeForOperation {
        /// Type name.
        ty: String,
        /// Operation name.
        operation: String,
    },

    // === Pattern Errors ===
    /// Unsupported pattern.
    UnsupportedPattern(String),

    /// Pattern does not cover all cases.
    NonExhaustivePattern(String),

    // === Control Flow Errors ===
    /// Break outside of loop.
    BreakOutsideLoop,

    /// Continue outside of loop.
    ContinueOutsideLoop,

    /// Return outside of function.
    ReturnOutsideFunction,

    /// Invalid jump target.
    InvalidJumpTarget(String),

    // === Register Errors ===
    /// Register allocation failed.
    RegisterAllocationFailed,

    /// Register overflow (too many registers needed).
    RegisterOverflow {
        /// Number of registers needed.
        needed: usize,
        /// Maximum available registers.
        max: usize,
    },

    // === Internal Errors ===
    /// Internal compiler error.
    Internal(String),

    /// Feature not yet implemented.
    NotImplemented(String),
}

impl CodegenError {
    /// Creates a new codegen error.
    pub fn new(kind: CodegenErrorKind) -> Self {
        Self {
            kind,
            span: None,
            context: None,
        }
    }

    /// Creates an error with a span.
    pub fn with_span(kind: CodegenErrorKind, span: Span) -> Self {
        Self {
            kind,
            span: Some(span),
            context: None,
        }
    }

    /// Adds context to the error.
    pub fn with_context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    /// Creates an unsupported expression error.
    pub fn unsupported_expr(desc: impl Into<String>) -> Self {
        Self::new(CodegenErrorKind::UnsupportedExpr(desc.into()))
    }

    /// Creates an undefined variable error.
    pub fn undefined_variable(name: impl Into<String>) -> Self {
        Self::new(CodegenErrorKind::UndefinedVariable(name.into()))
    }

    /// Creates an undefined function error.
    pub fn undefined_function(name: impl Into<String>) -> Self {
        Self::new(CodegenErrorKind::UndefinedFunction(name.into()))
    }

    /// Creates a type mismatch error.
    pub fn type_mismatch(expected: impl Into<String>, found: impl Into<String>) -> Self {
        Self::new(CodegenErrorKind::TypeMismatch {
            expected: expected.into(),
            found: found.into(),
        })
    }

    /// Creates an internal error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(CodegenErrorKind::Internal(msg.into()))
    }

    /// Creates a not implemented error.
    pub fn not_implemented(feature: impl Into<String>) -> Self {
        Self::new(CodegenErrorKind::NotImplemented(feature.into()))
    }

    /// Creates a register overflow error.
    pub fn register_overflow(needed: usize, max: usize) -> Self {
        Self::new(CodegenErrorKind::RegisterOverflow { needed, max })
    }

    /// Returns the undefined function name if this is an UndefinedFunction error.
    ///
    /// This is used for forward reference detection during stdlib compilation.
    /// Returns `Some(function_path)` if the error is `UndefinedFunction`, `None` otherwise.
    pub fn undefined_function_name(&self) -> Option<&str> {
        match &self.kind {
            CodegenErrorKind::UndefinedFunction(name) => Some(name),
            _ => None,
        }
    }

    /// Classifies this error into a "lenient skip" category.
    ///
    /// Used by `compile_item_lenient` and the stdlib hygiene baseline tests
    /// to distinguish:
    ///
    ///  * `BugClass` — error indicates a real codebase defect (an import
    ///    gap, wrong arity, mismatched signature, missing trait impl).
    ///    Surfacing path: warn-level trace today, hard error eventually
    ///    (#166 close-out).
    ///  * `Irreducible` — error indicates the Tier-0 interpreter genuinely
    ///    cannot compile the construct (FFI prototype, GPU shader,
    ///    not-yet-implemented language feature).  Surfacing path: debug
    ///    trace; baseline tests already exclude these by fixture choice.
    ///
    /// The split is conservative: errors that *might* be either category
    /// (notably `Internal`) classify as `BugClass` so they stay loud.
    pub fn skip_class(&self) -> SkipClass {
        match &self.kind {
            // Genuine interpreter limitations — these fire when a function
            // body references an LLVM intrinsic, an unimplemented expression
            // kind, or a feature stubbed out in the codegen path.  The
            // function is silently absent at runtime; users who call it
            // get a `FunctionNotFound` panic which is the expected and
            // documented contract.
            CodegenErrorKind::UnsupportedExpr(_)
            | CodegenErrorKind::UnsupportedPattern(_)
            | CodegenErrorKind::NotImplemented(_) => SkipClass::Irreducible,

            // Everything else is a real defect that should be fixed:
            //   * UndefinedFunction / UndefinedVariable — import gap
            //   * WrongArgumentCount / ArgumentTypeMismatch — caller / impl
            //     drift
            //   * TypeMismatch / TypeInference / InvalidTypeForOperation —
            //     type system regression
            //   * NonExhaustivePattern — coverage regression
            //   * BreakOutsideLoop / ContinueOutsideLoop /
            //     ReturnOutsideFunction — parser bug
            //   * InvalidLiteral / InvalidBinaryOp / InvalidUnaryOp —
            //     desugaring bug
            //   * RegisterAllocationFailed / RegisterOverflow — codegen
            //     resource exhaustion
            //   * ImmutableAssignment / VariableAlreadyDefined / InvalidJumpTarget
            //     — borrowck / lowering bug
            //   * Internal — by definition a bug
            _ => SkipClass::BugClass,
        }
    }
}

/// Classification used by lenient-skip diagnostics in
/// `compile_item_lenient` to distinguish recoverable interpreter
/// limitations from bug-class drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipClass {
    /// The error indicates a real defect (import gap, arity mismatch,
    /// type-system regression, codegen resource exhaustion).  Currently
    /// surfaced as a warn-level trace; #166 closes by promoting these to
    /// hard errors.
    BugClass,

    /// The error indicates the Tier-0 interpreter cannot compile the
    /// construct (FFI prototype, unimplemented language feature, GPU
    /// kernel).  Skipping is the documented contract: callers get a
    /// `FunctionNotFound` panic at runtime.
    Irreducible,
}

impl SkipClass {
    /// Short label used in trace messages.  Stable for grep/regex
    /// scraping in CI logs.
    pub fn label(self) -> &'static str {
        match self {
            SkipClass::BugClass => "bug-class",
            SkipClass::Irreducible => "irreducible",
        }
    }
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)?;
        if let Some(ref ctx) = self.context {
            write!(f, " ({})", ctx)?;
        }
        Ok(())
    }
}

impl fmt::Display for CodegenErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedExpr(e) => write!(f, "unsupported expression: {}", e),
            Self::InvalidLiteral(l) => write!(f, "invalid literal: {}", l),
            Self::InvalidBinaryOp(op) => write!(f, "invalid binary operation: {}", op),
            Self::InvalidUnaryOp(op) => write!(f, "invalid unary operation: {}", op),
            Self::UndefinedVariable(name) => write!(f, "undefined variable: {}", name),
            Self::VariableAlreadyDefined(name) => {
                write!(f, "variable already defined: {}", name)
            }
            Self::ImmutableAssignment(name) => {
                write!(f, "cannot assign to immutable variable: {}", name)
            }
            Self::UndefinedFunction(name) => write!(f, "undefined function: {}", name),
            Self::WrongArgumentCount {
                expected,
                found,
                function,
            } => {
                write!(
                    f,
                    "wrong number of arguments for {}: expected {}, found {}",
                    function, expected, found
                )
            }
            Self::ArgumentTypeMismatch {
                position,
                expected,
                found,
            } => {
                write!(
                    f,
                    "type mismatch for argument {}: expected {}, found {}",
                    position, expected, found
                )
            }
            Self::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {}, found {}", expected, found)
            }
            Self::TypeInference(msg) => write!(f, "cannot infer type: {}", msg),
            Self::InvalidTypeForOperation { ty, operation } => {
                write!(f, "invalid type {} for operation {}", ty, operation)
            }
            Self::UnsupportedPattern(p) => write!(f, "unsupported pattern: {}", p),
            Self::NonExhaustivePattern(p) => write!(f, "non-exhaustive pattern: {}", p),
            Self::BreakOutsideLoop => write!(f, "break statement outside of loop"),
            Self::ContinueOutsideLoop => write!(f, "continue statement outside of loop"),
            Self::ReturnOutsideFunction => write!(f, "return statement outside of function"),
            Self::InvalidJumpTarget(t) => write!(f, "invalid jump target: {}", t),
            Self::RegisterAllocationFailed => write!(f, "register allocation failed"),
            Self::RegisterOverflow { needed, max } => {
                write!(
                    f,
                    "register overflow: need {} registers, maximum is {}",
                    needed, max
                )
            }
            Self::Internal(msg) => write!(f, "internal compiler error: {}", msg),
            Self::NotImplemented(feature) => write!(f, "not yet implemented: {}", feature),
        }
    }
}

impl std::error::Error for CodegenError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Skip-class contract: bug-class causes (import gaps, arity drift,
    /// type-system regressions) MUST classify as `BugClass` so they stay
    /// loud — eventual #166 close-out promotes them to hard errors.
    /// `Irreducible` is reserved for the genuinely-unsupported set
    /// (FFI prototype, unimplemented language feature) where skipping
    /// is the documented Tier-0 contract.
    #[test]
    fn skip_class_classifies_bug_class() {
        let cases: Vec<CodegenErrorKind> = vec![
            CodegenErrorKind::UndefinedFunction("foo".into()),
            CodegenErrorKind::UndefinedVariable("x".into()),
            CodegenErrorKind::WrongArgumentCount {
                expected: 2,
                found: 3,
                function: "f".into(),
            },
            CodegenErrorKind::ArgumentTypeMismatch {
                position: 0,
                expected: "Int".into(),
                found: "Text".into(),
            },
            CodegenErrorKind::TypeMismatch {
                expected: "Int".into(),
                found: "Text".into(),
            },
            CodegenErrorKind::TypeInference("ambiguous".into()),
            CodegenErrorKind::NonExhaustivePattern("Maybe".into()),
            CodegenErrorKind::Internal("ICE".into()),
            CodegenErrorKind::RegisterOverflow {
                needed: 999,
                max: 256,
            },
            CodegenErrorKind::ImmutableAssignment("x".into()),
            CodegenErrorKind::BreakOutsideLoop,
            CodegenErrorKind::InvalidLiteral("0xFFFG".into()),
        ];
        for k in cases {
            let err = CodegenError::new(k);
            assert_eq!(
                err.skip_class(),
                SkipClass::BugClass,
                "{} must classify as BugClass — these surface fixable defects",
                err
            );
        }
    }

    #[test]
    fn skip_class_classifies_irreducible() {
        let cases: Vec<CodegenErrorKind> = vec![
            CodegenErrorKind::UnsupportedExpr("inline_asm".into()),
            CodegenErrorKind::UnsupportedPattern("regex".into()),
            CodegenErrorKind::NotImplemented("@gpu kernel".into()),
        ];
        for k in cases {
            let err = CodegenError::new(k);
            assert_eq!(
                err.skip_class(),
                SkipClass::Irreducible,
                "{} must classify as Irreducible — Tier-0 cannot lower these",
                err
            );
        }
    }

    #[test]
    fn skip_class_label_is_grep_stable() {
        assert_eq!(SkipClass::BugClass.label(), "bug-class");
        assert_eq!(SkipClass::Irreducible.label(), "irreducible");
    }
}
