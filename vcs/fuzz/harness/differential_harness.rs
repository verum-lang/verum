//! Differential testing harness for Verum
//!
//! This module implements differential testing between different execution
//! tiers of the Verum compiler. It compares results from:
//!
//! - **Tier 0 (Interpreter)**: Direct AST interpretation
//! - **Tier 1 (Bytecode)**: Bytecode compilation and VM execution
//! - **Tier 2 (JIT)**: Just-in-time compiled execution
//! - **Tier 3 (AOT)**: Ahead-of-time compiled native code
//!
//! Any discrepancy between tiers indicates a compiler bug.
//!
//! # Usage
//!
//! ```rust,no_run
//! use verum_fuzz::harness::{DifferentialHarness, DiffError};
//!
//! let harness = DifferentialHarness::new();
//! let source_code = "fn main() { let x = 1; }";
//! let result = harness.test(source_code);
//!
//! match result {
//!     Ok(_) => println!("All tiers agree"),
//!     Err(DiffError::ResultMismatch { .. }) => println!("Bug found!"),
//!     Err(e) => println!("Error: {:?}", e),
//! }
//! ```

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

/// Value representation for comparison across tiers
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<Value>),
    Map(HashMap<String, Value>),
    Tuple(Vec<Value>),
    Struct {
        name: String,
        fields: HashMap<String, Value>,
    },
    Enum {
        variant: String,
        data: Option<Box<Value>>,
    },
    Error(String),
}

impl Value {
    /// Check approximate equality for floats
    pub fn approx_eq(&self, other: &Value, epsilon: f64) -> bool {
        match (self, other) {
            (Value::Float(a), Value::Float(b)) => (a - b).abs() < epsilon,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approx_eq(y, epsilon))
            }
            (Value::Map(a), Value::Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).map_or(false, |v2| v.approx_eq(v2, epsilon)))
            }
            (Value::Tuple(a), Value::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.approx_eq(y, epsilon))
            }
            _ => self == other,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{:.6}", n),
            Value::Text(s) => write!(f, "\"{}\"", s),
            Value::List(items) => {
                write!(f, "[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, "]")
            }
            Value::Map(map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in map.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "\"{}\": {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", item)?;
                }
                write!(f, ")")
            }
            Value::Struct { name, fields } => {
                write!(f, "{} {{ ", name)?;
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, " }}")
            }
            Value::Enum { variant, data } => {
                write!(f, "{}", variant)?;
                if let Some(d) = data {
                    write!(f, "({})", d)?;
                }
                Ok(())
            }
            Value::Error(msg) => write!(f, "Error: {}", msg),
        }
    }
}

/// Execution tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Direct AST interpretation
    Interpreter,
    /// Bytecode VM execution
    Bytecode,
    /// Just-in-time compilation
    Jit,
    /// Ahead-of-time compilation
    Aot,
}

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tier::Interpreter => write!(f, "Interpreter (Tier 0)"),
            Tier::Bytecode => write!(f, "Bytecode (Tier 1)"),
            Tier::Jit => write!(f, "JIT (Tier 2)"),
            Tier::Aot => write!(f, "AOT (Tier 3)"),
        }
    }
}

/// Result of executing a program on a specific tier
#[derive(Debug, Clone)]
pub struct TierResult {
    /// The execution tier
    pub tier: Tier,
    /// The result value (or error)
    pub value: Value,
    /// Execution time
    pub duration: Duration,
    /// Memory usage in bytes
    pub memory_used: usize,
    /// Whether execution completed successfully
    pub success: bool,
    /// Stdout output
    pub stdout: String,
    /// Stderr output
    pub stderr: String,
}

/// Error types for differential testing
#[derive(Debug)]
pub enum DiffError {
    /// Parse error
    ParseError {
        message: String,
        location: Option<(usize, usize)>,
    },
    /// Type checking error
    TypeError { message: String },
    /// Result mismatch between tiers
    ResultMismatch {
        tier1: Tier,
        result1: Value,
        tier2: Tier,
        result2: Value,
    },
    /// Output mismatch (stdout/stderr)
    OutputMismatch {
        tier1: Tier,
        output1: String,
        tier2: Tier,
        output2: String,
    },
    /// Timeout on a tier
    Timeout { tier: Tier, duration: Duration },
    /// Crash on a tier
    Crash { tier: Tier, message: String },
    /// One tier succeeded, another failed
    BehaviorMismatch {
        succeeded: Vec<Tier>,
        failed: Vec<(Tier, String)>,
    },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffError::ParseError { message, location } => {
                write!(f, "Parse error: {}", message)?;
                if let Some((line, col)) = location {
                    write!(f, " at {}:{}", line, col)?;
                }
                Ok(())
            }
            DiffError::TypeError { message } => write!(f, "Type error: {}", message),
            DiffError::ResultMismatch {
                tier1,
                result1,
                tier2,
                result2,
            } => {
                write!(
                    f,
                    "Result mismatch:\n  {}: {}\n  {}: {}",
                    tier1, result1, tier2, result2
                )
            }
            DiffError::OutputMismatch {
                tier1,
                output1,
                tier2,
                output2,
            } => {
                write!(
                    f,
                    "Output mismatch:\n  {}: {:?}\n  {}: {:?}",
                    tier1, output1, tier2, output2
                )
            }
            DiffError::Timeout { tier, duration } => {
                write!(f, "Timeout on {}: {:?}", tier, duration)
            }
            DiffError::Crash { tier, message } => {
                write!(f, "Crash on {}: {}", tier, message)
            }
            DiffError::BehaviorMismatch { succeeded, failed } => {
                write!(
                    f,
                    "Behavior mismatch:\n  Succeeded: {:?}\n  Failed: {:?}",
                    succeeded, failed
                )
            }
        }
    }
}

impl std::error::Error for DiffError {}

/// Configuration for differential testing
#[derive(Debug, Clone)]
pub struct DifferentialConfig {
    /// Tiers to test
    pub tiers: Vec<Tier>,
    /// Timeout per tier
    pub timeout: Duration,
    /// Epsilon for floating-point comparison
    pub float_epsilon: f64,
    /// Whether to compare stdout output
    pub compare_stdout: bool,
    /// Whether to compare stderr output
    pub compare_stderr: bool,
    /// Maximum memory per tier (bytes)
    pub max_memory: usize,
    /// Whether to continue testing if a tier fails
    pub continue_on_failure: bool,
}

impl Default for DifferentialConfig {
    fn default() -> Self {
        Self {
            tiers: vec![Tier::Interpreter, Tier::Aot],
            timeout: Duration::from_secs(10),
            float_epsilon: 1e-10,
            compare_stdout: true,
            compare_stderr: false,
            max_memory: 100 * 1024 * 1024, // 100 MB
            continue_on_failure: true,
        }
    }
}

/// Differential testing harness
pub struct DifferentialHarness {
    config: DifferentialConfig,
    // In production, these would be actual compiler/runtime references
    // interpreter: Interpreter,
    // bytecode_compiler: BytecodeCompiler,
    // jit_compiler: JitCompiler,
    // aot_compiler: AotCompiler,
}

impl DifferentialHarness {
    /// Create a new differential harness with default configuration
    pub fn new() -> Self {
        Self::with_config(DifferentialConfig::default())
    }

    /// Create a harness with custom configuration
    pub fn with_config(config: DifferentialConfig) -> Self {
        Self { config }
    }

    /// Test a source program across all configured tiers
    pub fn test(&self, source: &str) -> Result<Vec<TierResult>, DiffError> {
        // Step 1: Parse the source
        let ast = self.parse(source)?;

        // Step 2: Type check
        self.type_check(&ast)?;

        // Step 3: Run on each tier
        let mut results = Vec::new();
        let mut errors = Vec::new();

        for &tier in &self.config.tiers {
            match self.execute_tier(tier, source) {
                Ok(result) => results.push(result),
                Err(e) => {
                    if self.config.continue_on_failure {
                        errors.push((tier, format!("{:?}", e)));
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        // Step 4: Check for behavior mismatch
        if !errors.is_empty() && !results.is_empty() {
            let succeeded: Vec<_> = results.iter().map(|r| r.tier).collect();
            return Err(DiffError::BehaviorMismatch {
                succeeded,
                failed: errors,
            });
        }

        // Step 5: Compare results across tiers
        self.compare_results(&results)?;

        Ok(results)
    }

    /// Parse source code into AST
    fn parse(&self, source: &str) -> Result<Ast, DiffError> {
        // In production: verum_parser::parse(source)
        // Placeholder implementation
        if source.trim().is_empty() {
            return Err(DiffError::ParseError {
                message: "Empty source".to_string(),
                location: None,
            });
        }

        if !source.contains("fn ") {
            return Err(DiffError::ParseError {
                message: "No function definition found".to_string(),
                location: Some((1, 1)),
            });
        }

        Ok(Ast {
            source: source.to_string(),
        })
    }

    /// Type check the AST
    fn type_check(&self, _ast: &Ast) -> Result<(), DiffError> {
        // In production: verum_types::check(ast)
        // Placeholder - always succeeds
        Ok(())
    }

    /// Execute on a specific tier
    fn execute_tier(&self, tier: Tier, source: &str) -> Result<TierResult, DiffError> {
        let start = Instant::now();

        // Capture output
        let mut stdout = String::new();
        let stderr = String::new();

        // Execute based on tier
        let (value, memory) = match tier {
            Tier::Interpreter => self.run_interpreter(source)?,
            Tier::Bytecode => self.run_bytecode(source)?,
            Tier::Jit => self.run_jit(source)?,
            Tier::Aot => self.run_aot(source)?,
        };

        let duration = start.elapsed();

        // Check timeout
        if duration > self.config.timeout {
            return Err(DiffError::Timeout { tier, duration });
        }

        Ok(TierResult {
            tier,
            value,
            duration,
            memory_used: memory,
            success: true,
            stdout,
            stderr,
        })
    }

    /// Run in interpreter mode
    fn run_interpreter(&self, source: &str) -> Result<(Value, usize), DiffError> {
        // In production:
        // let ast = verum_parser::parse(source)?;
        // let result = verum_interpreter::Interpreter::new().run(&ast)?;
        // return Ok((result.into_value(), result.memory_used()));

        // Placeholder
        Ok((Value::Unit, 1024))
    }

    /// Run in bytecode mode
    fn run_bytecode(&self, source: &str) -> Result<(Value, usize), DiffError> {
        // In production:
        // let ast = verum_parser::parse(source)?;
        // let bytecode = verum_codegen::BytecodeCompiler::compile(&ast)?;
        // let result = verum_runtime::VM::new().run(&bytecode)?;
        // return Ok((result.into_value(), result.memory_used()));

        // Placeholder
        Ok((Value::Unit, 2048))
    }

    /// Run in JIT mode
    fn run_jit(&self, source: &str) -> Result<(Value, usize), DiffError> {
        // In production:
        // let ast = verum_parser::parse(source)?;
        // let compiled = verum_codegen::JitCompiler::compile(&ast)?;
        // let result = compiled.execute()?;
        // return Ok((result.into_value(), result.memory_used()));

        // Placeholder
        Ok((Value::Unit, 4096))
    }

    /// Run in AOT mode
    fn run_aot(&self, source: &str) -> Result<(Value, usize), DiffError> {
        // In production:
        // let ast = verum_parser::parse(source)?;
        // let binary = verum_codegen::AotCompiler::compile(&ast)?;
        // let result = binary.execute()?;
        // return Ok((result.into_value(), result.memory_used()));

        // Placeholder
        Ok((Value::Unit, 8192))
    }

    /// Compare results across tiers
    fn compare_results(&self, results: &[TierResult]) -> Result<(), DiffError> {
        if results.len() < 2 {
            return Ok(());
        }

        let reference = &results[0];

        for other in &results[1..] {
            // Compare values
            if !reference
                .value
                .approx_eq(&other.value, self.config.float_epsilon)
            {
                return Err(DiffError::ResultMismatch {
                    tier1: reference.tier,
                    result1: reference.value.clone(),
                    tier2: other.tier,
                    result2: other.value.clone(),
                });
            }

            // Compare stdout if configured
            if self.config.compare_stdout && reference.stdout != other.stdout {
                return Err(DiffError::OutputMismatch {
                    tier1: reference.tier,
                    output1: reference.stdout.clone(),
                    tier2: other.tier,
                    output2: other.stdout.clone(),
                });
            }

            // Compare stderr if configured
            if self.config.compare_stderr && reference.stderr != other.stderr {
                return Err(DiffError::OutputMismatch {
                    tier1: reference.tier,
                    output1: reference.stderr.clone(),
                    tier2: other.tier,
                    output2: other.stderr.clone(),
                });
            }
        }

        Ok(())
    }

    /// Run differential testing on multiple inputs
    pub fn test_batch(&self, sources: &[&str]) -> BatchResult {
        let mut passed = 0;
        let mut failed = 0;
        let mut errors = Vec::new();

        for (i, source) in sources.iter().enumerate() {
            match self.test(source) {
                Ok(_) => passed += 1,
                Err(e) => {
                    failed += 1;
                    errors.push((i, e));
                }
            }
        }

        BatchResult {
            total: sources.len(),
            passed,
            failed,
            errors,
        }
    }

    /// Minimize a failing test case
    pub fn minimize(&self, source: &str) -> Option<String> {
        // Check that it actually fails
        if self.test(source).is_ok() {
            return None;
        }

        let mut current = source.to_string();

        // Try removing lines
        loop {
            let lines: Vec<&str> = current.lines().collect();
            let mut made_progress = false;

            for i in 0..lines.len() {
                let candidate: String = lines
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, line)| *line)
                    .collect::<Vec<_>>()
                    .join("\n");

                if !candidate.is_empty() && self.test(&candidate).is_err() {
                    current = candidate;
                    made_progress = true;
                    break;
                }
            }

            if !made_progress {
                break;
            }
        }

        Some(current)
    }
}

impl Default for DifferentialHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// Placeholder AST type
#[derive(Debug)]
struct Ast {
    source: String,
}

/// Result of batch testing
#[derive(Debug)]
pub struct BatchResult {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errors: Vec<(usize, DiffError)>,
}

impl BatchResult {
    pub fn success_rate(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.passed as f64 / self.total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_creation() {
        let harness = DifferentialHarness::new();
        assert!(!harness.config.tiers.is_empty());
    }

    #[test]
    fn test_value_display() {
        assert_eq!(format!("{}", Value::Int(42)), "42");
        assert_eq!(format!("{}", Value::Bool(true)), "true");
        assert_eq!(format!("{}", Value::Text("hello".to_string())), "\"hello\"");
        assert_eq!(
            format!("{}", Value::List(vec![Value::Int(1), Value::Int(2)])),
            "[1, 2]"
        );
    }

    #[test]
    fn test_value_approx_eq() {
        let a = Value::Float(1.0);
        let b = Value::Float(1.0 + 1e-11);
        assert!(a.approx_eq(&b, 1e-10));

        let c = Value::Float(1.0 + 1e-9);
        assert!(!a.approx_eq(&c, 1e-10));
    }

    #[test]
    fn test_batch_result() {
        let result = BatchResult {
            total: 10,
            passed: 8,
            failed: 2,
            errors: vec![],
        };
        assert_eq!(result.success_rate(), 0.8);
    }

    #[test]
    fn test_empty_batch() {
        let result = BatchResult {
            total: 0,
            passed: 0,
            failed: 0,
            errors: vec![],
        };
        assert_eq!(result.success_rate(), 1.0);
    }
}
