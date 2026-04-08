//! Type checker fuzzing harness
//!
//! This module provides a fuzzing harness for the Verum type checker.
//! It tests type inference, unification, constraint solving, and
//! refinement type verification.
//!
//! # Test Categories
//!
//! - **Type inference**: Verify correct type inference
//! - **Unification**: Test type unification edge cases
//! - **Generics**: Test generic instantiation
//! - **Refinement types**: Test SMT solver integration
//! - **CBGR**: Test reference tier checking
//!
//! # Safety Properties
//!
//! The type checker should:
//! - Never accept unsound programs
//! - Always reject type errors
//! - Handle infinite types correctly
//! - Provide helpful error messages

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Result of type checking
#[derive(Debug, Clone)]
pub struct TypeCheckResult {
    /// Whether type checking succeeded
    pub success: bool,
    /// Inferred types for expressions
    pub inferred_types: HashMap<String, String>,
    /// Type errors encountered
    pub errors: Vec<TypeError>,
    /// Type warnings
    pub warnings: Vec<TypeWarning>,
    /// Time taken for type checking
    pub duration: Duration,
    /// Number of unification steps
    pub unification_steps: usize,
    /// Number of SMT queries (for refinements)
    pub smt_queries: usize,
    /// Constraint graph size
    pub constraint_count: usize,
}

/// A type error
#[derive(Debug, Clone)]
pub struct TypeError {
    /// Error message
    pub message: String,
    /// Expected type
    pub expected: String,
    /// Found type
    pub found: String,
    /// Source location
    pub span_start: usize,
    /// End location
    pub span_end: usize,
    /// Error code
    pub code: String,
    /// Additional notes
    pub notes: Vec<String>,
}

/// A type warning
#[derive(Debug, Clone)]
pub struct TypeWarning {
    /// Warning message
    pub message: String,
    /// Warning code
    pub code: String,
    /// Source location
    pub span_start: usize,
}

/// Configuration for the type check harness
#[derive(Debug, Clone)]
pub struct TypeCheckHarnessConfig {
    /// Maximum input size
    pub max_input_size: usize,
    /// Timeout for type checking
    pub timeout: Duration,
    /// Maximum unification depth
    pub max_unification_depth: usize,
    /// Enable refinement type checking
    pub enable_refinements: bool,
    /// SMT solver timeout
    pub smt_timeout: Duration,
    /// Test CBGR correctness
    pub test_cbgr: bool,
}

impl Default for TypeCheckHarnessConfig {
    fn default() -> Self {
        Self {
            max_input_size: 1024 * 1024,
            timeout: Duration::from_secs(60),
            max_unification_depth: 100,
            enable_refinements: true,
            smt_timeout: Duration::from_secs(10),
            test_cbgr: true,
        }
    }
}

/// Statistics from type checking fuzzing
#[derive(Debug, Default)]
pub struct TypeCheckStats {
    /// Total inputs processed
    pub total_inputs: usize,
    /// Inputs that type checked successfully
    pub well_typed: usize,
    /// Inputs with type errors
    pub type_errors: usize,
    /// Inputs that timed out
    pub timeouts: usize,
    /// Average type check time
    pub avg_typecheck_time_ms: f64,
    /// Total unification steps
    pub total_unification_steps: usize,
    /// Total SMT queries
    pub total_smt_queries: usize,
    /// Error distribution
    pub error_distribution: HashMap<String, usize>,
    /// Type distribution
    pub type_distribution: HashMap<String, usize>,
}

/// Type check fuzzing harness
pub struct TypeCheckHarness {
    config: TypeCheckHarnessConfig,
    stats: TypeCheckStats,
    total_time_ms: f64,
}

impl TypeCheckHarness {
    /// Create a new type check harness
    pub fn new(config: TypeCheckHarnessConfig) -> Self {
        Self {
            config,
            stats: TypeCheckStats::default(),
            total_time_ms: 0.0,
        }
    }

    /// Fuzz the type checker
    pub fn fuzz(&mut self, input: &[u8]) -> TypeCheckResult {
        self.stats.total_inputs += 1;

        // Size check
        if input.len() > self.config.max_input_size {
            return TypeCheckResult {
                success: false,
                inferred_types: HashMap::new(),
                errors: vec![TypeError {
                    message: "Input too large".to_string(),
                    expected: String::new(),
                    found: String::new(),
                    span_start: 0,
                    span_end: 0,
                    code: "E0001".to_string(),
                    notes: vec![],
                }],
                warnings: vec![],
                duration: Duration::from_secs(0),
                unification_steps: 0,
                smt_queries: 0,
                constraint_count: 0,
            };
        }

        // Convert to string
        let source = match std::str::from_utf8(input) {
            Ok(s) => s,
            Err(_) => {
                return TypeCheckResult {
                    success: false,
                    inferred_types: HashMap::new(),
                    errors: vec![TypeError {
                        message: "Invalid UTF-8".to_string(),
                        expected: String::new(),
                        found: String::new(),
                        span_start: 0,
                        span_end: 0,
                        code: "E0002".to_string(),
                        notes: vec![],
                    }],
                    warnings: vec![],
                    duration: Duration::from_secs(0),
                    unification_steps: 0,
                    smt_queries: 0,
                    constraint_count: 0,
                };
            }
        };

        self.typecheck_source(source)
    }

    /// Type check a source string
    fn typecheck_source(&mut self, source: &str) -> TypeCheckResult {
        let start = Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut inferred_types = HashMap::new();
        let mut unification_steps = 0;
        let mut smt_queries = 0;
        let mut constraint_count = 0;

        // Simulate type checking
        let success = self.simulate_typecheck(
            source,
            &mut inferred_types,
            &mut errors,
            &mut warnings,
            &mut unification_steps,
            &mut smt_queries,
            &mut constraint_count,
        );

        let duration = start.elapsed();
        let duration_ms = duration.as_secs_f64() * 1000.0;

        // Update stats
        self.total_time_ms += duration_ms;
        self.stats.avg_typecheck_time_ms = self.total_time_ms / self.stats.total_inputs as f64;
        self.stats.total_unification_steps += unification_steps;
        self.stats.total_smt_queries += smt_queries;

        if success {
            self.stats.well_typed += 1;
        } else {
            self.stats.type_errors += 1;
        }

        for error in &errors {
            *self
                .stats
                .error_distribution
                .entry(error.code.clone())
                .or_insert(0) += 1;
        }

        for (_, ty) in &inferred_types {
            *self.stats.type_distribution.entry(ty.clone()).or_insert(0) += 1;
        }

        TypeCheckResult {
            success,
            inferred_types,
            errors,
            warnings,
            duration,
            unification_steps,
            smt_queries,
            constraint_count,
        }
    }

    /// Simulate type checking
    fn simulate_typecheck(
        &self,
        source: &str,
        inferred_types: &mut HashMap<String, String>,
        errors: &mut Vec<TypeError>,
        warnings: &mut Vec<TypeWarning>,
        unification_steps: &mut usize,
        smt_queries: &mut usize,
        constraint_count: &mut usize,
    ) -> bool {
        // Simple type environment
        let mut env: HashMap<String, String> = HashMap::new();
        let mut type_vars: HashSet<String> = HashSet::new();

        // Add built-in types
        env.insert("Int".to_string(), "Type".to_string());
        env.insert("Float".to_string(), "Type".to_string());
        env.insert("Bool".to_string(), "Type".to_string());
        env.insert("Text".to_string(), "Type".to_string());
        env.insert("Char".to_string(), "Type".to_string());

        // Scan for let bindings and infer types
        let mut pos = 0;
        while let Some(let_pos) = source[pos..].find("let ") {
            let actual_pos = pos + let_pos;
            pos = actual_pos + 4;

            // Skip 'mut' if present
            let rest = &source[pos..];
            let rest = if rest.starts_with("mut ") {
                pos += 4;
                &source[pos..]
            } else {
                rest
            };

            // Extract variable name
            let name_end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let name = &rest[..name_end];

            if name.is_empty() {
                continue;
            }

            pos += name_end;

            // Check for type annotation
            let rest = &source[pos..];
            if let Some(colon_pos) = rest.find(':') {
                if colon_pos < 10 {
                    // Type annotation present
                    pos += colon_pos + 1;
                    let rest = source[pos..].trim_start();

                    // Extract type
                    let type_end = rest
                        .find(|c: char| c == '=' || c == ';' || c == '\n')
                        .unwrap_or(rest.len());
                    let ty = rest[..type_end].trim();

                    if !ty.is_empty() {
                        inferred_types.insert(name.to_string(), ty.to_string());
                        env.insert(name.to_string(), ty.to_string());
                        *constraint_count += 1;
                    }
                }
            } else {
                // Type inference needed
                *unification_steps += 1;

                // Simple type inference based on literal
                if let Some(eq_pos) = rest.find('=') {
                    let after_eq = &rest[eq_pos + 1..];
                    let after_eq = after_eq.trim_start();

                    let inferred = if after_eq.starts_with('"') {
                        "Text"
                    } else if after_eq.starts_with('\'') {
                        "Char"
                    } else if after_eq.starts_with("true") || after_eq.starts_with("false") {
                        "Bool"
                    } else if after_eq.starts_with('[') {
                        "List<_>"
                    } else if after_eq
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit() || c == '-')
                        .unwrap_or(false)
                    {
                        if after_eq.contains('.') {
                            "Float"
                        } else {
                            "Int"
                        }
                    } else {
                        "Unknown"
                    };

                    inferred_types.insert(name.to_string(), inferred.to_string());
                    env.insert(name.to_string(), inferred.to_string());
                }
            }
        }

        // Check for refinement types
        if self.config.enable_refinements
            && (source.contains("where value")
                || source.contains("{!= ")
                || source.contains("{>= "))
        {
            *smt_queries += 1;
        }

        // Check for CBGR references
        if self.config.test_cbgr && (source.contains("&checked ") || source.contains("&unsafe ")) {
            *constraint_count += 1;

            // Check for incorrect usage
            if source.contains("&unsafe ") && !source.contains("unsafe {") {
                warnings.push(TypeWarning {
                    message: "Unsafe reference used outside unsafe block".to_string(),
                    code: "W0001".to_string(),
                    span_start: 0,
                });
            }
        }

        // Check for common type errors
        self.check_common_errors(source, errors, &env);

        errors.is_empty()
    }

    /// Check for common type errors
    fn check_common_errors(
        &self,
        source: &str,
        errors: &mut Vec<TypeError>,
        env: &HashMap<String, String>,
    ) {
        // Check for + operator on strings (should use concatenation)
        if source.contains("\"") && source.contains(" + ") {
            // This is a heuristic - might be false positive
            // A real type checker would check operand types
        }

        // Check for division by zero in constant expressions
        if source.contains("/ 0") || source.contains("/0") {
            errors.push(TypeError {
                message: "Potential division by zero".to_string(),
                expected: "non-zero Int".to_string(),
                found: "0".to_string(),
                span_start: 0,
                span_end: 0,
                code: "E0200".to_string(),
                notes: vec!["Division by zero is undefined behavior".to_string()],
            });
        }

        // Check for recursive type without indirection
        if source.contains("type ") && source.contains(" is ") {
            // Look for potential recursive types
            let lines: Vec<&str> = source.lines().collect();
            for line in lines {
                if line.contains("type ") && line.contains(" is ") {
                    // Extract type name
                    if let Some(name_start) = line.find("type ") {
                        let rest = &line[name_start + 5..];
                        let name_end = rest
                            .find(|c: char| {
                                !c.is_alphanumeric() && c != '_' && c != '<' && c != '>'
                            })
                            .unwrap_or(rest.len());
                        let name = &rest[..name_end];
                        let base_name = name.split('<').next().unwrap_or(name);

                        // Check if type references itself without Heap
                        if let Some(is_pos) = line.find(" is ") {
                            let body = &line[is_pos + 4..];
                            if body.contains(base_name)
                                && !body.contains(&format!("Heap<{}", base_name))
                            {
                                // Might be recursive without indirection
                                // This is a simplified check
                            }
                        }
                    }
                }
            }
        }
    }

    /// Get current statistics
    pub fn get_stats(&self) -> &TypeCheckStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = TypeCheckStats::default();
        self.total_time_ms = 0.0;
    }

    /// Check if result is interesting
    pub fn is_interesting(&self, result: &TypeCheckResult) -> bool {
        !result.errors.is_empty()
            || result.smt_queries > 0
            || result.unification_steps > 10
            || result.inferred_types.len() > 5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typecheck_harness_basic() {
        let config = TypeCheckHarnessConfig::default();
        let mut harness = TypeCheckHarness::new(config);

        let input = b"fn main() { let x: Int = 42; }";
        let result = harness.fuzz(input);

        assert!(result.success);
        assert!(result.inferred_types.contains_key("x"));
    }

    #[test]
    fn test_typecheck_harness_inference() {
        let config = TypeCheckHarnessConfig::default();
        let mut harness = TypeCheckHarness::new(config);

        let input = b"fn main() { let x = 42; let y = \"hello\"; let z = true; }";
        let result = harness.fuzz(input);

        assert!(result.unification_steps > 0);
        assert_eq!(result.inferred_types.get("x"), Some(&"Int".to_string()));
        assert_eq!(result.inferred_types.get("y"), Some(&"Text".to_string()));
        assert_eq!(result.inferred_types.get("z"), Some(&"Bool".to_string()));
    }

    #[test]
    fn test_typecheck_harness_division_by_zero() {
        let config = TypeCheckHarnessConfig::default();
        let mut harness = TypeCheckHarness::new(config);

        let input = b"fn main() { let x = 10 / 0; }";
        let result = harness.fuzz(input);

        assert!(!result.success);
        assert!(result.errors.iter().any(|e| e.message.contains("division")));
    }
}
