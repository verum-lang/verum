//! Security Pattern Detection for Meta Linter
//!
//! Detects CWE-mapped security vulnerabilities in meta code:
//! - SQL Injection (CWE-89)
//! - Command Injection (CWE-78)
//! - Path Traversal (CWE-22)
//! - Dynamic Code Execution (CWE-94)
//! - Format String (CWE-134)
//! - Sensitive Data Exposure (CWE-200)
//! - Buffer Overflow (CWE-119)
//!
//! Meta linter: static analysis of meta code for unsafe patterns (unbounded
//! recursion, infinite loops, unsafe interpolation without @safe attribute).

use std::collections::HashSet;

use verum_ast::Span;
use verum_ast::expr::Expr;
use verum_common::{Maybe, Text};

use super::dataflow::{AnalysisContext, ExternalInputChecker};
use super::patterns::UnsafePatternKind;
use super::results::{LintResult, UnsafePattern};

/// Security pattern detector for meta code
pub struct SecurityDetector {
    /// Known I/O functions (blacklist)
    io_functions: HashSet<String>,
}

impl SecurityDetector {
    /// Create a new security detector
    pub fn new() -> Self {
        let mut io_functions = HashSet::new();
        // Known I/O functions (forbidden in meta context)
        io_functions.insert("File.read".to_string());
        io_functions.insert("File.write".to_string());
        io_functions.insert("File.open".to_string());
        io_functions.insert("fs.read".to_string());
        io_functions.insert("fs.write".to_string());
        io_functions.insert("net.connect".to_string());
        io_functions.insert("http.get".to_string());
        io_functions.insert("http.post".to_string());
        io_functions.insert("std.env.var".to_string());
        io_functions.insert("std.env.args".to_string());
        io_functions.insert("std.process.spawn".to_string());
        io_functions.insert("std.time.now".to_string());
        io_functions.insert("random".to_string());

        Self { io_functions }
    }

    /// Check if a function name represents an I/O operation
    pub fn is_io_function(&self, func_name: &str) -> bool {
        // Check known I/O functions
        if self.io_functions.contains(func_name) {
            return true;
        }

        // Check for common I/O patterns
        func_name.contains("read")
            || func_name.contains("write")
            || func_name.contains("open")
            || func_name.contains("close")
            || func_name.starts_with("File.")
            || func_name.starts_with("fs.")
            || func_name.starts_with("net.")
            || func_name.starts_with("http.")
            || func_name.contains("socket")
            || func_name.contains("connect")
    }

    /// Check if a method name represents an I/O operation
    pub fn is_io_method(&self, method_name: &str) -> bool {
        matches!(
            method_name,
            "read"
                | "write"
                | "open"
                | "close"
                | "flush"
                | "connect"
                | "listen"
                | "accept"
                | "send"
                | "recv"
        )
    }

    /// Check if a function call could be command execution
    pub fn is_command_execution(&self, func_name: &str) -> bool {
        func_name.contains("exec")
            || func_name.contains("spawn")
            || func_name.contains("shell")
            || func_name.contains("system")
            || func_name.starts_with("process.")
            || func_name.starts_with("os.")
            || func_name.contains("command")
    }

    /// Check if a function call is dynamic code execution
    pub fn is_dynamic_code_execution(&self, func_name: &str) -> bool {
        func_name == "eval"
            || func_name == "exec"
            || func_name.contains("compile")
            || func_name.contains("interpret")
            || func_name.starts_with("meta.eval")
    }

    /// Check for SQL injection vulnerability
    pub fn check_sql_injection(
        &self,
        func_name: &str,
        args: &verum_common::List<Expr>,
        ctx: &AnalysisContext,
        span: Span,
        result: &mut LintResult,
    ) {
        if func_name.contains("query")
            || func_name.ends_with(".query")
            || func_name.contains("execute")
            || func_name.ends_with(".execute")
        {
            let has_external = args
                .iter()
                .any(|arg| ExternalInputChecker::expr_uses_external(arg, ctx));
            if has_external {
                Self::detect_pattern(
                    UnsafePatternKind::SqlInjection,
                    Text::from("SQL query with external input may be vulnerable to injection"),
                    span,
                    Maybe::Some(Text::from(
                        "Use parameterized queries or prepared statements",
                    )),
                    result,
                );
            }
        }
    }

    /// Check for command injection vulnerability
    pub fn check_command_injection(
        &self,
        func_name: &str,
        args: &verum_common::List<Expr>,
        ctx: &AnalysisContext,
        span: Span,
        result: &mut LintResult,
    ) {
        if self.is_command_execution(func_name) {
            let has_external = args
                .iter()
                .any(|arg| ExternalInputChecker::expr_uses_external(arg, ctx));
            if has_external {
                Self::detect_pattern(
                    UnsafePatternKind::CommandInjection,
                    Text::from(
                        "Command execution with external input may be vulnerable to injection",
                    ),
                    span,
                    Maybe::Some(Text::from(
                        "Validate and sanitize input, avoid shell execution",
                    )),
                    result,
                );
            }
        }
    }

    /// Check for dynamic code execution
    pub fn check_dynamic_code_execution(
        &self,
        func_name: &str,
        span: Span,
        result: &mut LintResult,
    ) {
        if self.is_dynamic_code_execution(func_name) {
            Self::detect_pattern(
                UnsafePatternKind::DynamicCodeExecution,
                Text::from("Dynamic code execution is dangerous and should be avoided"),
                span,
                Maybe::Some(Text::from(
                    "Use safer alternatives like pattern matching or predefined operations",
                )),
                result,
            );
        }
    }

    /// Check for unsafe format with external input
    pub fn check_unsafe_format(
        &self,
        func_name: &str,
        args: &verum_common::List<Expr>,
        ctx: &AnalysisContext,
        span: Span,
        result: &mut LintResult,
    ) {
        if func_name == "format!" || func_name == "format" {
            let has_external = args
                .iter()
                .any(|arg| ExternalInputChecker::expr_uses_external(arg, ctx));
            if has_external {
                Self::detect_pattern(
                    UnsafePatternKind::UnsafeFormat,
                    Text::from(
                        "format! with external input may cause injection without proper escaping",
                    ),
                    span,
                    Maybe::Some(Text::from("Use safe formatting or escape user input")),
                    result,
                );
            }
        }
    }

    /// Check for hidden I/O operations
    pub fn check_hidden_io(&self, func_name: &str, span: Span, result: &mut LintResult) {
        if self.is_io_function(func_name) {
            Self::detect_pattern(
                UnsafePatternKind::HiddenIO,
                Text::from(format!(
                    "I/O operation not allowed in meta code: {}",
                    func_name
                )),
                span,
                Maybe::Some(Text::from(
                    "Use `using BuildAssets` context for build-time file access",
                )),
                result,
            );
        }
    }

    /// Detect and record an unsafe pattern
    fn detect_pattern(
        kind: UnsafePatternKind,
        description: Text,
        span: Span,
        suggestion: Maybe<Text>,
        result: &mut LintResult,
    ) {
        result.is_safe = false;
        result.unsafe_patterns.push(UnsafePattern {
            kind,
            description,
            span,
            suggestion,
        });
    }
}

impl Default for SecurityDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_io_function_detection() {
        let detector = SecurityDetector::new();
        assert!(detector.is_io_function("File.read"));
        assert!(detector.is_io_function("fs.write"));
        assert!(detector.is_io_function("http.get"));
        assert!(!detector.is_io_function("list_append"));
    }

    #[test]
    fn test_command_execution_detection() {
        let detector = SecurityDetector::new();
        assert!(detector.is_command_execution("process.spawn"));
        assert!(detector.is_command_execution("os.exec"));
        assert!(detector.is_command_execution("shell_command"));
        assert!(!detector.is_command_execution("list_process"));
    }

    #[test]
    fn test_dynamic_code_execution() {
        let detector = SecurityDetector::new();
        assert!(detector.is_dynamic_code_execution("eval"));
        assert!(detector.is_dynamic_code_execution("meta.eval"));
        assert!(!detector.is_dynamic_code_execution("evaluate_score"));
    }
}
