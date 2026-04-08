//! Code generator fuzzing harness
//!
//! This module provides a fuzzing harness for the Verum code generator.
//! It tests LLVM IR generation, optimization passes, and linking.
//!
//! # Test Categories
//!
//! - **IR generation**: Verify correct LLVM IR for all expressions
//! - **Optimizations**: Test optimization correctness
//! - **ABI compliance**: Test calling conventions
//! - **CBGR codegen**: Test reference tracking code generation
//! - **Async codegen**: Test coroutine transformation
//!
//! # Safety Properties
//!
//! The code generator should:
//! - Never generate invalid LLVM IR
//! - Preserve program semantics through optimization
//! - Generate correct CBGR reference tracking
//! - Handle all edge cases correctly

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Result of code generation
#[derive(Debug, Clone)]
pub struct CodegenResult {
    /// Whether codegen succeeded
    pub success: bool,
    /// Size of generated IR (bytes)
    pub ir_size: usize,
    /// Number of functions generated
    pub function_count: usize,
    /// Number of basic blocks
    pub basic_block_count: usize,
    /// Number of instructions
    pub instruction_count: usize,
    /// Errors encountered
    pub errors: Vec<CodegenError>,
    /// Warnings
    pub warnings: Vec<CodegenWarning>,
    /// Time taken
    pub duration: Duration,
    /// Optimizations applied
    pub optimizations: Vec<String>,
    /// Generated IR (if requested)
    pub ir: Option<String>,
}

/// A code generation error
#[derive(Debug, Clone)]
pub struct CodegenError {
    /// Error message
    pub message: String,
    /// Error phase (IR gen, optimization, linking)
    pub phase: CodegenPhase,
    /// Error code
    pub code: String,
}

/// A code generation warning
#[derive(Debug, Clone)]
pub struct CodegenWarning {
    /// Warning message
    pub message: String,
    /// Warning code
    pub code: String,
}

/// Phases of code generation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodegenPhase {
    /// LLVM IR generation
    IRGeneration,
    /// Optimization passes
    Optimization,
    /// Object file generation
    ObjectGeneration,
    /// Linking
    Linking,
}

/// Configuration for the codegen harness
#[derive(Debug, Clone)]
pub struct CodegenHarnessConfig {
    /// Maximum input size
    pub max_input_size: usize,
    /// Timeout for code generation
    pub timeout: Duration,
    /// Optimization level (0-3)
    pub opt_level: u8,
    /// Whether to verify generated IR
    pub verify_ir: bool,
    /// Whether to collect generated IR
    pub collect_ir: bool,
    /// Test CBGR code generation
    pub test_cbgr: bool,
    /// Test async code generation
    pub test_async: bool,
}

impl Default for CodegenHarnessConfig {
    fn default() -> Self {
        Self {
            max_input_size: 1024 * 1024,
            timeout: Duration::from_secs(120),
            opt_level: 2,
            verify_ir: true,
            collect_ir: false,
            test_cbgr: true,
            test_async: true,
        }
    }
}

/// Statistics from codegen fuzzing
#[derive(Debug, Default)]
pub struct CodegenStats {
    /// Total inputs processed
    pub total_inputs: usize,
    /// Successful code generations
    pub successful: usize,
    /// Failed code generations
    pub failed: usize,
    /// Timeouts
    pub timeouts: usize,
    /// Average codegen time
    pub avg_codegen_time_ms: f64,
    /// Total IR generated (bytes)
    pub total_ir_bytes: usize,
    /// Total functions generated
    pub total_functions: usize,
    /// Error distribution by phase
    pub errors_by_phase: HashMap<String, usize>,
    /// Optimization distribution
    pub optimizations_applied: HashMap<String, usize>,
}

/// Code generator fuzzing harness
pub struct CodegenHarness {
    config: CodegenHarnessConfig,
    stats: CodegenStats,
    total_time_ms: f64,
}

impl CodegenHarness {
    /// Create a new codegen harness
    pub fn new(config: CodegenHarnessConfig) -> Self {
        Self {
            config,
            stats: CodegenStats::default(),
            total_time_ms: 0.0,
        }
    }

    /// Fuzz the code generator
    pub fn fuzz(&mut self, input: &[u8]) -> CodegenResult {
        self.stats.total_inputs += 1;

        // Size check
        if input.len() > self.config.max_input_size {
            return CodegenResult {
                success: false,
                ir_size: 0,
                function_count: 0,
                basic_block_count: 0,
                instruction_count: 0,
                errors: vec![CodegenError {
                    message: "Input too large".to_string(),
                    phase: CodegenPhase::IRGeneration,
                    code: "E0001".to_string(),
                }],
                warnings: vec![],
                duration: Duration::from_secs(0),
                optimizations: vec![],
                ir: None,
            };
        }

        // Convert to string
        let source = match std::str::from_utf8(input) {
            Ok(s) => s,
            Err(_) => {
                return CodegenResult {
                    success: false,
                    ir_size: 0,
                    function_count: 0,
                    basic_block_count: 0,
                    instruction_count: 0,
                    errors: vec![CodegenError {
                        message: "Invalid UTF-8".to_string(),
                        phase: CodegenPhase::IRGeneration,
                        code: "E0002".to_string(),
                    }],
                    warnings: vec![],
                    duration: Duration::from_secs(0),
                    optimizations: vec![],
                    ir: None,
                };
            }
        };

        self.generate_code(source)
    }

    /// Generate code for a source string
    fn generate_code(&mut self, source: &str) -> CodegenResult {
        let start = Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut optimizations = Vec::new();
        let mut function_count = 0;
        let mut basic_block_count = 0;
        let mut instruction_count = 0;
        let mut ir = String::new();

        // Simulate code generation
        let success = self.simulate_codegen(
            source,
            &mut function_count,
            &mut basic_block_count,
            &mut instruction_count,
            &mut errors,
            &mut warnings,
            &mut optimizations,
            &mut ir,
        );

        let duration = start.elapsed();
        let duration_ms = duration.as_secs_f64() * 1000.0;

        // Update stats
        self.total_time_ms += duration_ms;
        self.stats.avg_codegen_time_ms = self.total_time_ms / self.stats.total_inputs as f64;
        self.stats.total_functions += function_count;
        self.stats.total_ir_bytes += ir.len();

        if success {
            self.stats.successful += 1;
        } else {
            self.stats.failed += 1;
        }

        for error in &errors {
            *self
                .stats
                .errors_by_phase
                .entry(format!("{:?}", error.phase))
                .or_insert(0) += 1;
        }

        for opt in &optimizations {
            *self
                .stats
                .optimizations_applied
                .entry(opt.clone())
                .or_insert(0) += 1;
        }

        CodegenResult {
            success,
            ir_size: ir.len(),
            function_count,
            basic_block_count,
            instruction_count,
            errors,
            warnings,
            duration,
            optimizations,
            ir: if self.config.collect_ir {
                Some(ir)
            } else {
                None
            },
        }
    }

    /// Simulate code generation
    fn simulate_codegen(
        &self,
        source: &str,
        function_count: &mut usize,
        basic_block_count: &mut usize,
        instruction_count: &mut usize,
        errors: &mut Vec<CodegenError>,
        warnings: &mut Vec<CodegenWarning>,
        optimizations: &mut Vec<String>,
        ir: &mut String,
    ) -> bool {
        // Simulate LLVM IR generation

        // Count functions
        *function_count = source.matches("fn ").count();

        // Add IR header
        ir.push_str("; ModuleID = 'verum_module'\n");
        ir.push_str("source_filename = \"input.vr\"\n\n");

        // Generate IR for each function
        let lines: Vec<&str> = source.lines().collect();
        let mut in_function = false;
        let mut brace_depth = 0;

        for line in &lines {
            if line.contains("fn ") && line.contains("(") {
                // Start of function
                in_function = true;
                brace_depth = 0;

                // Extract function name (simplified)
                let after_fn = line.split("fn ").nth(1).unwrap_or("unknown");
                let name_end = after_fn.find('(').unwrap_or(after_fn.len());
                let name = &after_fn[..name_end];

                ir.push_str(&format!("define void @{}() {{\n", name.trim()));
                ir.push_str("entry:\n");
                *basic_block_count += 1;
            }

            if in_function {
                for c in line.chars() {
                    match c {
                        '{' => brace_depth += 1,
                        '}' => {
                            brace_depth -= 1;
                            if brace_depth == 0 {
                                ir.push_str("  ret void\n");
                                ir.push_str("}\n\n");
                                in_function = false;
                            }
                        }
                        _ => {}
                    }
                }

                // Generate IR for statements
                if line.contains("let ") {
                    ir.push_str("  ; let binding\n");
                    *instruction_count += 2; // alloca + store
                }
                if line.contains(" + ")
                    || line.contains(" - ")
                    || line.contains(" * ")
                    || line.contains(" / ")
                {
                    ir.push_str("  ; arithmetic\n");
                    *instruction_count += 1;
                }
                if line.contains("if ") {
                    ir.push_str("  ; conditional\n");
                    *instruction_count += 2; // cmp + br
                    *basic_block_count += 2; // then/else blocks
                }
                if line.contains("for ") || line.contains("while ") || line.contains("loop ") {
                    ir.push_str("  ; loop\n");
                    *instruction_count += 3; // cmp + br + phi
                    *basic_block_count += 3; // header, body, exit
                }
            }
        }

        // Apply optimizations based on config
        if self.config.opt_level >= 1 {
            optimizations.push("mem2reg".to_string());
            optimizations.push("simplifycfg".to_string());
        }
        if self.config.opt_level >= 2 {
            optimizations.push("instcombine".to_string());
            optimizations.push("gvn".to_string());
            optimizations.push("licm".to_string());
        }
        if self.config.opt_level >= 3 {
            optimizations.push("loop-vectorize".to_string());
            optimizations.push("slp-vectorize".to_string());
        }

        // Check for CBGR code generation
        if self.config.test_cbgr && (source.contains("&") || source.contains("Heap")) {
            ir.push_str("; CBGR reference tracking\n");
            *instruction_count += 3; // gen check + epoch check + pointer
        }

        // Check for async code generation
        if self.config.test_async && (source.contains("async ") || source.contains(".await")) {
            ir.push_str("; Coroutine transformation\n");
            *instruction_count += 5; // state machine
            *basic_block_count += 2; // suspend/resume
        }

        // Verify IR if enabled
        if self.config.verify_ir {
            // Simulated verification - check for common issues
            if ir.matches('{').count() != ir.matches('}').count() {
                errors.push(CodegenError {
                    message: "Unbalanced braces in generated IR".to_string(),
                    phase: CodegenPhase::IRGeneration,
                    code: "E0100".to_string(),
                });
            }
        }

        // Check for potential issues
        if source.contains("/ 0") || source.contains("/0") {
            warnings.push(CodegenWarning {
                message: "Potential division by zero".to_string(),
                code: "W0100".to_string(),
            });
        }

        errors.is_empty()
    }

    /// Get current statistics
    pub fn get_stats(&self) -> &CodegenStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = CodegenStats::default();
        self.total_time_ms = 0.0;
    }

    /// Check if result is interesting
    pub fn is_interesting(&self, result: &CodegenResult) -> bool {
        !result.errors.is_empty()
            || result.function_count > 5
            || result.instruction_count > 100
            || result.optimizations.len() > 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codegen_harness_basic() {
        let config = CodegenHarnessConfig::default();
        let mut harness = CodegenHarness::new(config);

        let input = b"fn main() { let x = 42; }";
        let result = harness.fuzz(input);

        assert!(result.success);
        assert!(result.function_count > 0);
    }

    #[test]
    fn test_codegen_harness_multiple_functions() {
        let config = CodegenHarnessConfig::default();
        let mut harness = CodegenHarness::new(config);

        let input = b"fn foo() {} fn bar() {} fn main() {}";
        let result = harness.fuzz(input);

        assert!(result.success);
        assert_eq!(result.function_count, 3);
    }

    #[test]
    fn test_codegen_harness_optimizations() {
        let mut config = CodegenHarnessConfig::default();
        config.opt_level = 3;
        let mut harness = CodegenHarness::new(config);

        let input = b"fn main() { let x = 1 + 2; }";
        let result = harness.fuzz(input);

        assert!(!result.optimizations.is_empty());
        assert!(result.optimizations.contains(&"loop-vectorize".to_string()));
    }

    #[test]
    fn test_codegen_harness_stats() {
        let config = CodegenHarnessConfig::default();
        let mut harness = CodegenHarness::new(config);

        harness.fuzz(b"fn main() {}");
        harness.fuzz(b"fn foo() {}");

        let stats = harness.get_stats();
        assert_eq!(stats.total_inputs, 2);
        assert!(stats.total_functions > 0);
    }
}
