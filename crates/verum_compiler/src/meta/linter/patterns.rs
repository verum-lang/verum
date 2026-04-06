//! Unsafe Pattern Definitions for Meta Linter
//!
//! Defines the 18 kinds of unsafe patterns detected in meta code.
//!
//! Meta linter: static analysis of meta code for unsafe patterns (unbounded
//! recursion, infinite loops, unsafe interpolation without @safe attribute).

use verum_diagnostics::Severity;

/// Kinds of unsafe patterns in meta code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnsafePatternKind {
    // ========================================================================
    // Security patterns (CWE-mapped)
    // ========================================================================
    /// SQL injection pattern detected (CWE-89)
    SqlInjection,
    /// Command injection pattern detected (CWE-78)
    CommandInjection,
    /// Path traversal vulnerability (CWE-22)
    PathTraversal,
    /// Unrestricted eval/exec of dynamic code (CWE-94)
    DynamicCodeExecution,
    /// Direct format! with user input without escaping (CWE-134)
    UnsafeFormat,
    /// Sensitive data exposure in logs or output (CWE-200)
    SensitiveDataExposure,
    /// Unsafe memory operations (raw pointer manipulation) (CWE-119)
    UnsafeMemory,

    // ========================================================================
    // Safety patterns
    // ========================================================================
    /// String concatenation with external input (injection risk)
    StringConcatenation,
    /// Unchecked type cast that could fail
    UncheckedCast,
    /// Use of panic!/expect/unwrap that could fail at compile-time
    PanicPossible,
    /// Unbounded recursion without base case
    UnboundedRecursion,
    /// Unbounded loop (while true, loop {})
    UnboundedLoop,
    /// Hidden I/O operation
    HiddenIO,
    /// Access to runtime-only APIs
    RuntimeAccess,
    /// Mutation of global state
    GlobalMutation,
    /// Non-deterministic operation
    NonDeterministic,
    /// Excessive resource usage (memory, CPU)
    ExcessiveResourceUsage,
    /// Type confusion through unsafe casts
    TypeConfusion,
}

impl UnsafePatternKind {
    /// Get a human-readable name for this pattern
    pub fn name(&self) -> &'static str {
        match self {
            UnsafePatternKind::StringConcatenation => "String Concatenation",
            UnsafePatternKind::UnsafeFormat => "Unsafe Format",
            UnsafePatternKind::UncheckedCast => "Unchecked Cast",
            UnsafePatternKind::PanicPossible => "Possible Panic",
            UnsafePatternKind::UnboundedRecursion => "Unbounded Recursion",
            UnsafePatternKind::UnboundedLoop => "Unbounded Loop",
            UnsafePatternKind::HiddenIO => "Hidden I/O",
            UnsafePatternKind::RuntimeAccess => "Runtime Access",
            UnsafePatternKind::GlobalMutation => "Global Mutation",
            UnsafePatternKind::NonDeterministic => "Non-Deterministic",
            UnsafePatternKind::UnsafeMemory => "Unsafe Memory",
            UnsafePatternKind::DynamicCodeExecution => "Dynamic Code Execution",
            UnsafePatternKind::SensitiveDataExposure => "Sensitive Data Exposure",
            UnsafePatternKind::ExcessiveResourceUsage => "Excessive Resource Usage",
            UnsafePatternKind::TypeConfusion => "Type Confusion",
            UnsafePatternKind::SqlInjection => "SQL Injection",
            UnsafePatternKind::CommandInjection => "Command Injection",
            UnsafePatternKind::PathTraversal => "Path Traversal",
        }
    }

    /// Get severity level for this pattern
    pub fn severity(&self) -> Severity {
        match self {
            // Critical security issues - always errors
            UnsafePatternKind::SqlInjection
            | UnsafePatternKind::CommandInjection
            | UnsafePatternKind::PathTraversal
            | UnsafePatternKind::DynamicCodeExecution
            | UnsafePatternKind::UnsafeMemory => Severity::Error,

            // High severity - errors by default
            UnsafePatternKind::HiddenIO
            | UnsafePatternKind::GlobalMutation
            | UnsafePatternKind::SensitiveDataExposure => Severity::Error,

            // Medium severity - warnings
            UnsafePatternKind::StringConcatenation
            | UnsafePatternKind::UnsafeFormat
            | UnsafePatternKind::TypeConfusion => Severity::Warning,

            // Lower severity - warnings
            UnsafePatternKind::UncheckedCast
            | UnsafePatternKind::PanicPossible
            | UnsafePatternKind::UnboundedRecursion
            | UnsafePatternKind::UnboundedLoop
            | UnsafePatternKind::RuntimeAccess
            | UnsafePatternKind::NonDeterministic
            | UnsafePatternKind::ExcessiveResourceUsage => Severity::Warning,
        }
    }

    /// Check if this pattern is a security-related issue
    pub fn is_security_issue(&self) -> bool {
        matches!(
            self,
            UnsafePatternKind::SqlInjection
                | UnsafePatternKind::CommandInjection
                | UnsafePatternKind::PathTraversal
                | UnsafePatternKind::DynamicCodeExecution
                | UnsafePatternKind::UnsafeMemory
                | UnsafePatternKind::StringConcatenation
                | UnsafePatternKind::UnsafeFormat
                | UnsafePatternKind::SensitiveDataExposure
        )
    }

    /// Get CWE (Common Weakness Enumeration) ID if applicable
    pub fn cwe_id(&self) -> Option<u32> {
        match self {
            UnsafePatternKind::SqlInjection => Some(89),            // CWE-89: SQL Injection
            UnsafePatternKind::CommandInjection => Some(78),        // CWE-78: OS Command Injection
            UnsafePatternKind::PathTraversal => Some(22),           // CWE-22: Path Traversal
            UnsafePatternKind::DynamicCodeExecution => Some(94),    // CWE-94: Code Injection
            UnsafePatternKind::UnsafeFormat => Some(134),           // CWE-134: Format String
            UnsafePatternKind::SensitiveDataExposure => Some(200),  // CWE-200: Information Exposure
            UnsafePatternKind::UnsafeMemory => Some(119),           // CWE-119: Buffer Overflow
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_names() {
        assert_eq!(
            UnsafePatternKind::StringConcatenation.name(),
            "String Concatenation"
        );
        assert_eq!(UnsafePatternKind::HiddenIO.name(), "Hidden I/O");
        assert_eq!(UnsafePatternKind::SqlInjection.name(), "SQL Injection");
    }

    #[test]
    fn test_security_patterns_have_cwe() {
        assert!(UnsafePatternKind::SqlInjection.cwe_id().is_some());
        assert!(UnsafePatternKind::CommandInjection.cwe_id().is_some());
        assert!(UnsafePatternKind::PathTraversal.cwe_id().is_some());
    }

    #[test]
    fn test_security_issues_are_errors() {
        assert_eq!(UnsafePatternKind::SqlInjection.severity(), Severity::Error);
        assert_eq!(
            UnsafePatternKind::CommandInjection.severity(),
            Severity::Error
        );
    }
}
