//! Comprehensive test oracles for fuzzing
//!
//! This module provides multiple oracle types for detecting bugs:
//!
//! # Oracle Types
//!
//! - **Crash Oracle**: Detects crashes, panics, and abnormal termination
//! - **Differential Oracle**: Compares execution across tiers (Tier 0 vs Tier 3)
//! - **Memory Safety Oracle**: Detects use-after-free, double-free, leaks
//! - **Type Safety Oracle**: Detects type confusion and unsoundness
//! - **SMT Oracle**: Verifies refinement type contracts
//! - **Timeout Oracle**: Detects infinite loops and hangs
//!
//! # Comparison Rules
//!
//! - Floating point: Use epsilon comparison
//! - Collections: Order-sensitive for lists, order-insensitive for sets
//! - Errors: Match error types, not messages
//! - Performance: Not compared (expected to differ)
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_vfuzz::oracle::{OracleRunner, OracleConfig};
//!
//! let config = OracleConfig::default();
//! let mut runner = OracleRunner::new(config);
//!
//! let result = runner.check_all("fn main() { 42 }");
//! if let Some(violation) = result.violation {
//!     println!("Oracle violation: {:?}", violation);
//! }
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Execution tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionTier {
    /// Tier 0: Interpreter (reference implementation)
    Tier0,
    /// Tier 1: Baseline JIT
    Tier1,
    /// Tier 2: Optimizing JIT
    Tier2,
    /// Tier 3: AOT native compilation
    Tier3,
}

impl std::fmt::Display for ExecutionTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionTier::Tier0 => write!(f, "Tier0 (Interpreter)"),
            ExecutionTier::Tier1 => write!(f, "Tier1 (JIT-Base)"),
            ExecutionTier::Tier2 => write!(f, "Tier2 (JIT-Opt)"),
            ExecutionTier::Tier3 => write!(f, "Tier3 (AOT)"),
        }
    }
}

/// Result of executing a program on a tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierResult {
    /// The tier used
    pub tier: ExecutionTier,
    /// Whether execution succeeded
    pub success: bool,
    /// Return value (if any, serialized)
    pub return_value: Option<SerializedValue>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Error type (if failed)
    pub error: Option<ExecutionError>,
    /// Execution duration
    pub duration: Duration,
    /// Memory usage in bytes
    pub memory_bytes: usize,
}

/// A serialized value for comparison
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SerializedValue {
    /// Unit value
    Unit,
    /// Boolean value
    Bool(bool),
    /// Integer value
    Int(i64),
    /// Floating point value
    Float(f64),
    /// String value
    Text(String),
    /// List of values
    List(Vec<SerializedValue>),
    /// Map of values
    Map(Vec<(SerializedValue, SerializedValue)>),
    /// Tuple of values
    Tuple(Vec<SerializedValue>),
    /// Optional value
    Maybe(Option<Box<SerializedValue>>),
    /// Custom type (name + fields)
    Struct(String, HashMap<String, SerializedValue>),
    /// Variant value
    Variant(String, Box<SerializedValue>),
}

impl SerializedValue {
    /// Compare two values with configured tolerance
    pub fn compare(&self, other: &SerializedValue, config: &CompareConfig) -> CompareResult {
        match (self, other) {
            (SerializedValue::Unit, SerializedValue::Unit) => CompareResult::Equal,

            (SerializedValue::Bool(a), SerializedValue::Bool(b)) => {
                if a == b {
                    CompareResult::Equal
                } else {
                    CompareResult::ValueMismatch(format!("{} vs {}", a, b))
                }
            }

            (SerializedValue::Int(a), SerializedValue::Int(b)) => {
                if a == b {
                    CompareResult::Equal
                } else {
                    CompareResult::ValueMismatch(format!("{} vs {}", a, b))
                }
            }

            (SerializedValue::Float(a), SerializedValue::Float(b)) => {
                // Handle NaN
                if a.is_nan() && b.is_nan() {
                    return CompareResult::Equal;
                }

                // Epsilon comparison
                if (a - b).abs() <= config.float_epsilon {
                    CompareResult::Equal
                } else if config.allow_float_relative_error {
                    let max = a.abs().max(b.abs());
                    if max > 0.0 && (a - b).abs() / max <= config.float_relative_epsilon {
                        CompareResult::Equal
                    } else {
                        CompareResult::ValueMismatch(format!("{} vs {} (rel)", a, b))
                    }
                } else {
                    CompareResult::ValueMismatch(format!("{} vs {} (abs)", a, b))
                }
            }

            (SerializedValue::Text(a), SerializedValue::Text(b)) => {
                if a == b {
                    CompareResult::Equal
                } else {
                    CompareResult::ValueMismatch(format!("'{}' vs '{}'", a, b))
                }
            }

            (SerializedValue::List(a), SerializedValue::List(b)) => {
                if a.len() != b.len() {
                    return CompareResult::LengthMismatch(a.len(), b.len());
                }

                for (i, (va, vb)) in a.iter().zip(b.iter()).enumerate() {
                    let result = va.compare(vb, config);
                    if result != CompareResult::Equal {
                        return CompareResult::ElementMismatch(i, Box::new(result));
                    }
                }

                CompareResult::Equal
            }

            (SerializedValue::Tuple(a), SerializedValue::Tuple(b)) => {
                if a.len() != b.len() {
                    return CompareResult::LengthMismatch(a.len(), b.len());
                }

                for (i, (va, vb)) in a.iter().zip(b.iter()).enumerate() {
                    let result = va.compare(vb, config);
                    if result != CompareResult::Equal {
                        return CompareResult::ElementMismatch(i, Box::new(result));
                    }
                }

                CompareResult::Equal
            }

            (SerializedValue::Maybe(a), SerializedValue::Maybe(b)) => match (a, b) {
                (None, None) => CompareResult::Equal,
                (Some(va), Some(vb)) => va.compare(vb, config),
                _ => CompareResult::ValueMismatch("Some vs None".to_string()),
            },

            (
                SerializedValue::Struct(name_a, fields_a),
                SerializedValue::Struct(name_b, fields_b),
            ) => {
                if name_a != name_b {
                    return CompareResult::TypeMismatch(name_a.clone(), name_b.clone());
                }

                for (key, va) in fields_a {
                    if let Some(vb) = fields_b.get(key) {
                        let result = va.compare(vb, config);
                        if result != CompareResult::Equal {
                            return CompareResult::FieldMismatch(key.clone(), Box::new(result));
                        }
                    } else {
                        return CompareResult::MissingField(key.clone());
                    }
                }

                CompareResult::Equal
            }

            (SerializedValue::Variant(name_a, val_a), SerializedValue::Variant(name_b, val_b)) => {
                if name_a != name_b {
                    return CompareResult::ValueMismatch(format!("{} vs {}", name_a, name_b));
                }
                val_a.compare(val_b, config)
            }

            (SerializedValue::Map(a), SerializedValue::Map(b)) => {
                if a.len() != b.len() {
                    return CompareResult::LengthMismatch(a.len(), b.len());
                }

                // For maps, order doesn't matter
                for (key_a, val_a) in a {
                    let found = b
                        .iter()
                        .find(|(k, _)| k.compare(key_a, config) == CompareResult::Equal);
                    if let Some((_, val_b)) = found {
                        let result = val_a.compare(val_b, config);
                        if result != CompareResult::Equal {
                            return CompareResult::MapValueMismatch(Box::new(result));
                        }
                    } else {
                        return CompareResult::MapKeyMissing;
                    }
                }

                CompareResult::Equal
            }

            _ => CompareResult::TypeMismatch(
                format!("{:?}", std::mem::discriminant(self)),
                format!("{:?}", std::mem::discriminant(other)),
            ),
        }
    }
}

/// Execution error
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionError {
    /// Compilation error
    CompileError(String),
    /// Runtime error
    RuntimeError(String),
    /// Timeout
    Timeout,
    /// Crash
    Crash(String),
    /// Out of memory
    OutOfMemory,
}

/// Configuration for value comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareConfig {
    /// Epsilon for floating point absolute comparison
    pub float_epsilon: f64,
    /// Whether to allow relative float comparison
    pub allow_float_relative_error: bool,
    /// Epsilon for floating point relative comparison
    pub float_relative_epsilon: f64,
    /// Whether to compare stdout
    pub compare_stdout: bool,
    /// Whether errors must match exactly
    pub strict_error_matching: bool,
}

impl Default for CompareConfig {
    fn default() -> Self {
        Self {
            float_epsilon: 1e-10,
            allow_float_relative_error: true,
            float_relative_epsilon: 1e-6,
            compare_stdout: true,
            strict_error_matching: false,
        }
    }
}

/// Configuration for differential oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialOracleConfig {
    /// Timeout in milliseconds for each tier execution
    pub timeout_ms: u64,
    /// Whether to compare output between tiers
    pub compare_output: bool,
    /// Whether to compare side effects
    pub compare_side_effects: bool,
    /// Reference tier
    pub reference_tier: ExecutionTier,
    /// Tiers to test against reference
    pub test_tiers: Vec<ExecutionTier>,
    /// Comparison configuration
    pub compare_config: CompareConfig,
}

impl Default for DifferentialOracleConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 5000,
            compare_output: true,
            compare_side_effects: true,
            reference_tier: ExecutionTier::Tier0,
            test_tiers: vec![ExecutionTier::Tier3],
            compare_config: CompareConfig::default(),
        }
    }
}

/// Result of comparing two values
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CompareResult {
    /// Values are equal
    Equal,
    /// Values differ
    ValueMismatch(String),
    /// Types differ
    TypeMismatch(String, String),
    /// Lengths differ (for collections)
    LengthMismatch(usize, usize),
    /// Element at index differs
    ElementMismatch(usize, Box<CompareResult>),
    /// Field differs
    FieldMismatch(String, Box<CompareResult>),
    /// Missing field
    MissingField(String),
    /// Map key missing
    MapKeyMissing,
    /// Map value differs
    MapValueMismatch(Box<CompareResult>),
}

/// Result of differential testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialResult {
    /// The input program
    pub input: String,
    /// Results from each tier
    pub tier_results: HashMap<ExecutionTier, TierResult>,
    /// Whether all tiers agreed
    pub consistent: bool,
    /// Mismatches found
    pub mismatches: Vec<TierMismatch>,
    /// Timestamp
    pub timestamp: u64,
}

/// A mismatch between tiers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierMismatch {
    /// Reference tier (usually Tier 0)
    pub reference_tier: ExecutionTier,
    /// Mismatched tier
    pub mismatched_tier: ExecutionTier,
    /// Kind of mismatch
    pub kind: MismatchKind,
    /// Details
    pub details: String,
}

/// Kind of mismatch
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MismatchKind {
    /// Return values differ
    ReturnValue,
    /// Standard output differs
    Stdout,
    /// One succeeded, one failed
    SuccessFailure,
    /// Different error types
    ErrorType,
    /// Different error messages (if strict matching)
    ErrorMessage,
}

/// The differential oracle
pub struct DifferentialOracle {
    /// Reference tier for comparison
    reference_tier: ExecutionTier,
    /// Tiers to test against reference
    test_tiers: Vec<ExecutionTier>,
    /// Comparison configuration
    config: CompareConfig,
    /// Statistics
    stats: OracleStats,
}

impl Default for DifferentialOracle {
    fn default() -> Self {
        Self::new(DifferentialOracleConfig::default())
    }
}

impl DifferentialOracle {
    /// Create a new oracle with config
    pub fn new(config: DifferentialOracleConfig) -> Self {
        Self {
            reference_tier: config.reference_tier,
            test_tiers: config.test_tiers,
            config: config.compare_config,
            stats: OracleStats::default(),
        }
    }

    /// Create an oracle with custom configuration
    pub fn with_config(
        reference_tier: ExecutionTier,
        test_tiers: Vec<ExecutionTier>,
        config: CompareConfig,
    ) -> Self {
        Self {
            reference_tier,
            test_tiers,
            config,
            stats: OracleStats::default(),
        }
    }

    /// Compare results from all tiers
    pub fn compare(
        &mut self,
        input: &str,
        results: HashMap<ExecutionTier, TierResult>,
    ) -> DifferentialResult {
        self.stats.total_comparisons += 1;

        let reference_result = results.get(&self.reference_tier);
        let mut mismatches = Vec::new();

        if let Some(ref_result) = reference_result {
            for tier in &self.test_tiers {
                if let Some(tier_result) = results.get(tier) {
                    if let Some(mismatch) = self.compare_tier_results(ref_result, tier_result) {
                        mismatches.push(mismatch);
                    }
                }
            }
        }

        let consistent = mismatches.is_empty();

        if !consistent {
            self.stats.mismatches_found += mismatches.len();
        }

        DifferentialResult {
            input: input.to_string(),
            tier_results: results,
            consistent,
            mismatches,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        }
    }

    /// Compare two tier results
    fn compare_tier_results(
        &self,
        reference: &TierResult,
        other: &TierResult,
    ) -> Option<TierMismatch> {
        // Check success/failure mismatch
        if reference.success != other.success {
            return Some(TierMismatch {
                reference_tier: reference.tier,
                mismatched_tier: other.tier,
                kind: MismatchKind::SuccessFailure,
                details: format!(
                    "Reference {} but other {}",
                    if reference.success {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    if other.success { "succeeded" } else { "failed" }
                ),
            });
        }

        // If both failed, check error types
        if !reference.success {
            if let (Some(ref_err), Some(other_err)) = (&reference.error, &other.error) {
                if self.config.strict_error_matching && ref_err != other_err {
                    return Some(TierMismatch {
                        reference_tier: reference.tier,
                        mismatched_tier: other.tier,
                        kind: MismatchKind::ErrorType,
                        details: format!("{:?} vs {:?}", ref_err, other_err),
                    });
                }
                // For non-strict matching, only compare error variants
                if std::mem::discriminant(ref_err) != std::mem::discriminant(other_err) {
                    return Some(TierMismatch {
                        reference_tier: reference.tier,
                        mismatched_tier: other.tier,
                        kind: MismatchKind::ErrorType,
                        details: format!("{:?} vs {:?}", ref_err, other_err),
                    });
                }
            }
            return None; // Both failed with compatible errors
        }

        // Compare return values
        if let (Some(ref_val), Some(other_val)) = (&reference.return_value, &other.return_value) {
            let result = ref_val.compare(other_val, &self.config);
            if result != CompareResult::Equal {
                return Some(TierMismatch {
                    reference_tier: reference.tier,
                    mismatched_tier: other.tier,
                    kind: MismatchKind::ReturnValue,
                    details: format!("{:?}", result),
                });
            }
        }

        // Compare stdout if configured
        if self.config.compare_stdout && reference.stdout != other.stdout {
            return Some(TierMismatch {
                reference_tier: reference.tier,
                mismatched_tier: other.tier,
                kind: MismatchKind::Stdout,
                details: format!(
                    "stdout differs:\n  reference: {}\n  other: {}",
                    reference.stdout, other.stdout
                ),
            });
        }

        None
    }

    /// Get statistics
    pub fn stats(&self) -> &OracleStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = OracleStats::default();
    }
}

/// Oracle statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OracleStats {
    /// Total comparisons performed
    pub total_comparisons: usize,
    /// Mismatches found
    pub mismatches_found: usize,
    /// Return value mismatches
    pub return_value_mismatches: usize,
    /// Stdout mismatches
    pub stdout_mismatches: usize,
    /// Success/failure mismatches
    pub success_failure_mismatches: usize,
    /// Error type mismatches
    pub error_type_mismatches: usize,
}

/// Builder for testing multiple tiers
pub struct DifferentialTestBuilder {
    input: String,
    results: HashMap<ExecutionTier, TierResult>,
}

impl DifferentialTestBuilder {
    /// Create a new builder
    pub fn new(input: &str) -> Self {
        Self {
            input: input.to_string(),
            results: HashMap::new(),
        }
    }

    /// Add a tier result
    pub fn with_tier_result(mut self, result: TierResult) -> Self {
        self.results.insert(result.tier, result);
        self
    }

    /// Run comparison with the oracle
    pub fn compare(self, oracle: &mut DifferentialOracle) -> DifferentialResult {
        oracle.compare(&self.input, self.results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_comparison() {
        let config = CompareConfig::default();

        // Equal integers
        let a = SerializedValue::Int(42);
        let b = SerializedValue::Int(42);
        assert_eq!(a.compare(&b, &config), CompareResult::Equal);

        // Different integers
        let a = SerializedValue::Int(42);
        let b = SerializedValue::Int(43);
        assert!(matches!(
            a.compare(&b, &config),
            CompareResult::ValueMismatch(_)
        ));

        // Float with epsilon
        let a = SerializedValue::Float(1.0);
        let b = SerializedValue::Float(1.0 + 1e-12);
        assert_eq!(a.compare(&b, &config), CompareResult::Equal);

        // NaN comparison
        let a = SerializedValue::Float(f64::NAN);
        let b = SerializedValue::Float(f64::NAN);
        assert_eq!(a.compare(&b, &config), CompareResult::Equal);
    }

    #[test]
    fn test_list_comparison() {
        let config = CompareConfig::default();

        // Equal lists
        let a = SerializedValue::List(vec![SerializedValue::Int(1), SerializedValue::Int(2)]);
        let b = SerializedValue::List(vec![SerializedValue::Int(1), SerializedValue::Int(2)]);
        assert_eq!(a.compare(&b, &config), CompareResult::Equal);

        // Different lengths
        let a = SerializedValue::List(vec![SerializedValue::Int(1)]);
        let b = SerializedValue::List(vec![SerializedValue::Int(1), SerializedValue::Int(2)]);
        assert!(matches!(
            a.compare(&b, &config),
            CompareResult::LengthMismatch(1, 2)
        ));
    }

    #[test]
    fn test_differential_oracle() {
        let mut oracle = DifferentialOracle::default();

        let tier0_result = TierResult {
            tier: ExecutionTier::Tier0,
            success: true,
            return_value: Some(SerializedValue::Int(42)),
            stdout: String::new(),
            stderr: String::new(),
            error: None,
            duration: Duration::from_millis(100),
            memory_bytes: 1024,
        };

        let tier3_result = TierResult {
            tier: ExecutionTier::Tier3,
            success: true,
            return_value: Some(SerializedValue::Int(42)),
            stdout: String::new(),
            stderr: String::new(),
            error: None,
            duration: Duration::from_millis(10),
            memory_bytes: 512,
        };

        let mut results = HashMap::new();
        results.insert(ExecutionTier::Tier0, tier0_result);
        results.insert(ExecutionTier::Tier3, tier3_result);

        let diff_result = oracle.compare("fn main() { 42 }", results);
        assert!(diff_result.consistent);
        assert!(diff_result.mismatches.is_empty());
    }

    #[test]
    fn test_mismatch_detection() {
        let mut oracle = DifferentialOracle::default();

        let tier0_result = TierResult {
            tier: ExecutionTier::Tier0,
            success: true,
            return_value: Some(SerializedValue::Int(42)),
            stdout: String::new(),
            stderr: String::new(),
            error: None,
            duration: Duration::from_millis(100),
            memory_bytes: 1024,
        };

        let tier3_result = TierResult {
            tier: ExecutionTier::Tier3,
            success: true,
            return_value: Some(SerializedValue::Int(43)), // Different value!
            stdout: String::new(),
            stderr: String::new(),
            error: None,
            duration: Duration::from_millis(10),
            memory_bytes: 512,
        };

        let mut results = HashMap::new();
        results.insert(ExecutionTier::Tier0, tier0_result);
        results.insert(ExecutionTier::Tier3, tier3_result);

        let diff_result = oracle.compare("fn main() { bug() }", results);
        assert!(!diff_result.consistent);
        assert_eq!(diff_result.mismatches.len(), 1);
        assert_eq!(diff_result.mismatches[0].kind, MismatchKind::ReturnValue);
    }

    #[test]
    fn test_success_failure_mismatch() {
        let mut oracle = DifferentialOracle::default();

        let tier0_result = TierResult {
            tier: ExecutionTier::Tier0,
            success: true,
            return_value: Some(SerializedValue::Int(42)),
            stdout: String::new(),
            stderr: String::new(),
            error: None,
            duration: Duration::from_millis(100),
            memory_bytes: 1024,
        };

        let tier3_result = TierResult {
            tier: ExecutionTier::Tier3,
            success: false, // Crashed!
            return_value: None,
            stdout: String::new(),
            stderr: "crash".to_string(),
            error: Some(ExecutionError::Crash("segfault".to_string())),
            duration: Duration::from_millis(10),
            memory_bytes: 512,
        };

        let mut results = HashMap::new();
        results.insert(ExecutionTier::Tier0, tier0_result);
        results.insert(ExecutionTier::Tier3, tier3_result);

        let diff_result = oracle.compare("fn main() { crash() }", results);
        assert!(!diff_result.consistent);
        assert_eq!(diff_result.mismatches[0].kind, MismatchKind::SuccessFailure);
    }
}

// ============================================================================
// Additional Oracle Types
// ============================================================================

/// Type of oracle violation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OracleViolation {
    /// Program crashed unexpectedly
    Crash(CrashViolation),
    /// Differential test found mismatch
    Differential(DifferentialViolation),
    /// Memory safety violation
    MemorySafety(MemorySafetyViolation),
    /// Type safety violation
    TypeSafety(TypeSafetyViolation),
    /// SMT verification failure
    SmtVerification(SmtViolation),
    /// Program exceeded timeout
    Timeout(TimeoutViolation),
}

/// Details of a crash violation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CrashViolation {
    /// Type of crash
    pub crash_type: CrashType,
    /// Error message
    pub message: String,
    /// Stack trace (if available)
    pub stack_trace: Option<String>,
    /// Exit code (if process exited)
    pub exit_code: Option<i32>,
    /// Signal that caused crash (Unix)
    pub signal: Option<i32>,
}

/// Type of crash
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CrashType {
    /// Rust panic
    Panic,
    /// Segmentation fault
    Segfault,
    /// Stack overflow
    StackOverflow,
    /// Abort signal
    Abort,
    /// Floating point exception
    FloatingPointException,
    /// Illegal instruction
    IllegalInstruction,
    /// Bus error
    BusError,
    /// Unknown crash
    Unknown,
}

/// Details of a differential violation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DifferentialViolation {
    /// Reference tier
    pub reference_tier: ExecutionTier,
    /// Tested tier with different behavior
    pub test_tier: ExecutionTier,
    /// Description of the difference
    pub difference: String,
    /// Reference output
    pub reference_output: String,
    /// Test output
    pub test_output: String,
}

/// Memory safety violation types
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MemorySafetyViolation {
    /// Use after free
    UseAfterFree {
        address: String,
        allocation_site: Option<String>,
        free_site: Option<String>,
        use_site: String,
    },
    /// Double free
    DoubleFree {
        address: String,
        first_free: String,
        second_free: String,
    },
    /// Memory leak
    MemoryLeak {
        address: String,
        size_bytes: usize,
        allocation_site: String,
    },
    /// Buffer overflow
    BufferOverflow {
        buffer_address: String,
        buffer_size: usize,
        access_offset: usize,
        access_size: usize,
    },
    /// Null pointer dereference
    NullPointerDeref { location: String },
    /// Uninitialized memory read
    UninitializedRead {
        address: String,
        size_bytes: usize,
        location: String,
    },
    /// CBGR generation check failure
    CbgrGenerationMismatch {
        expected_generation: u64,
        actual_generation: u64,
        reference_location: String,
    },
    /// CBGR epoch capability violation
    CbgrEpochViolation { message: String },
}

/// Type safety violation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TypeSafetyViolation {
    /// Kind of type safety issue
    pub kind: TypeSafetyKind,
    /// Expected type
    pub expected_type: String,
    /// Actual type
    pub actual_type: String,
    /// Location in source
    pub location: String,
    /// Additional details
    pub details: Option<String>,
}

/// Kind of type safety violation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TypeSafetyKind {
    /// Type confusion at runtime
    TypeConfusion,
    /// Type cast failed
    InvalidCast,
    /// Refinement type predicate violated
    RefinementViolation,
    /// Variant type mismatch
    VariantMismatch,
    /// Generic instantiation error
    GenericInstantiationError,
    /// Trait method not found at runtime
    MissingTraitMethod,
}

/// SMT verification violation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtViolation {
    /// Kind of SMT issue
    pub kind: SmtViolationKind,
    /// The contract that was violated
    pub contract: String,
    /// Counter-example (if available)
    pub counter_example: Option<HashMap<String, String>>,
    /// SMT solver output
    pub solver_output: Option<String>,
}

/// Kind of SMT violation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SmtViolationKind {
    /// Precondition not satisfied
    PreconditionViolation,
    /// Postcondition not satisfied
    PostconditionViolation,
    /// Assertion failed
    AssertionFailed,
    /// Loop invariant violated
    InvariantViolation,
    /// Decreases clause not satisfied
    DecreasesViolation,
    /// Modifies clause violated
    ModifiesViolation,
    /// Solver timeout
    SolverTimeout,
    /// Solver returned unknown
    SolverUnknown,
}

/// Timeout violation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeoutViolation {
    /// Timeout threshold in milliseconds
    pub timeout_ms: u64,
    /// Actual elapsed time when killed
    pub elapsed_ms: u64,
    /// Phase during which timeout occurred
    pub phase: TimeoutPhase,
    /// Possible cause (heuristic)
    pub possible_cause: Option<String>,
}

/// Phase during which timeout occurred
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeoutPhase {
    /// Lexing phase
    Lexing,
    /// Parsing phase
    Parsing,
    /// Type checking phase
    TypeChecking,
    /// SMT solving
    SmtSolving,
    /// Code generation
    CodeGeneration,
    /// Runtime execution
    Execution,
    /// Unknown phase
    Unknown,
}

// ============================================================================
// Crash Oracle
// ============================================================================

/// Oracle that detects crashes and abnormal termination
pub struct CrashOracle {
    /// Configuration
    config: CrashOracleConfig,
    /// Statistics
    stats: CrashOracleStats,
}

/// Configuration for crash oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrashOracleConfig {
    /// Whether to capture stack traces
    pub capture_stack_trace: bool,
    /// Whether to check for assertion failures
    pub check_assertions: bool,
    /// Patterns indicating expected errors (not bugs)
    pub expected_error_patterns: Vec<String>,
}

impl Default for CrashOracleConfig {
    fn default() -> Self {
        Self {
            capture_stack_trace: true,
            check_assertions: true,
            expected_error_patterns: vec![
                "syntax error".to_string(),
                "type error".to_string(),
                "unresolved".to_string(),
            ],
        }
    }
}

/// Statistics for crash oracle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CrashOracleStats {
    /// Total checks performed
    pub total_checks: usize,
    /// Crashes detected
    pub crashes_detected: usize,
    /// Panics detected
    pub panics_detected: usize,
    /// Segfaults detected
    pub segfaults_detected: usize,
    /// Stack overflows detected
    pub stack_overflows_detected: usize,
}

impl Default for CrashOracle {
    fn default() -> Self {
        Self::new(CrashOracleConfig::default())
    }
}

impl CrashOracle {
    /// Create a new crash oracle
    pub fn new(config: CrashOracleConfig) -> Self {
        Self {
            config,
            stats: CrashOracleStats::default(),
        }
    }

    /// Check for crashes in execution result
    pub fn check(&mut self, result: &OracleExecutionResult) -> Option<CrashViolation> {
        self.stats.total_checks += 1;

        // Check exit code
        if let Some(exit_code) = result.exit_code {
            if exit_code != 0 {
                return self.analyze_exit_code(exit_code, &result.stderr);
            }
        }

        // Check for panic patterns in stderr
        if result.stderr.contains("panicked at") || result.stderr.contains("thread 'main' panicked")
        {
            self.stats.panics_detected += 1;
            return Some(CrashViolation {
                crash_type: CrashType::Panic,
                message: self.extract_panic_message(&result.stderr),
                stack_trace: self.extract_stack_trace(&result.stderr),
                exit_code: result.exit_code,
                signal: None,
            });
        }

        // Check for stack overflow
        if result.stderr.contains("stack overflow") {
            self.stats.stack_overflows_detected += 1;
            return Some(CrashViolation {
                crash_type: CrashType::StackOverflow,
                message: "Stack overflow detected".to_string(),
                stack_trace: self.extract_stack_trace(&result.stderr),
                exit_code: result.exit_code,
                signal: None,
            });
        }

        None
    }

    /// Analyze an exit code to determine crash type
    fn analyze_exit_code(&mut self, exit_code: i32, stderr: &str) -> Option<CrashViolation> {
        // Check if this is an expected error
        for pattern in &self.config.expected_error_patterns {
            if stderr.to_lowercase().contains(&pattern.to_lowercase()) {
                return None;
            }
        }

        let (crash_type, signal) = match exit_code {
            // Unix signals
            134 => (CrashType::Abort, Some(6)), // SIGABRT
            136 => (CrashType::FloatingPointException, Some(8)), // SIGFPE
            139 => {
                self.stats.segfaults_detected += 1;
                (CrashType::Segfault, Some(11)) // SIGSEGV
            }
            132 => (CrashType::IllegalInstruction, Some(4)), // SIGILL
            138 => (CrashType::BusError, Some(7)),           // SIGBUS
            _ if exit_code > 128 => (CrashType::Unknown, Some(exit_code - 128)),
            _ => (CrashType::Unknown, None),
        };

        self.stats.crashes_detected += 1;

        Some(CrashViolation {
            crash_type,
            message: format!("Process exited with code {}", exit_code),
            stack_trace: self.extract_stack_trace(stderr),
            exit_code: Some(exit_code),
            signal,
        })
    }

    /// Extract panic message from stderr
    fn extract_panic_message(&self, stderr: &str) -> String {
        let re = regex::Regex::new(r"panicked at '([^']+)'").unwrap();
        if let Some(cap) = re.captures(stderr) {
            return cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }

        // Try alternate format
        let re2 = regex::Regex::new(r"panicked at ([^,]+),").unwrap();
        if let Some(cap) = re2.captures(stderr) {
            return cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }

        "Unknown panic".to_string()
    }

    /// Extract stack trace from stderr
    fn extract_stack_trace(&self, stderr: &str) -> Option<String> {
        if !self.config.capture_stack_trace {
            return None;
        }

        // Look for stack trace markers
        if let Some(start) = stderr.find("stack backtrace:") {
            return Some(stderr[start..].to_string());
        }

        if let Some(start) = stderr.find("   0:") {
            return Some(stderr[start..].to_string());
        }

        None
    }

    /// Get statistics
    pub fn stats(&self) -> &CrashOracleStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = CrashOracleStats::default();
    }
}

/// Extended execution result with more metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleExecutionResult {
    /// Whether execution completed successfully
    pub success: bool,
    /// Exit code (if process-based execution)
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Return value
    pub return_value: Option<SerializedValue>,
    /// Execution duration
    pub duration: Duration,
    /// Memory usage in bytes
    pub memory_bytes: usize,
    /// Execution tier
    pub tier: ExecutionTier,
}

// ============================================================================
// Memory Safety Oracle
// ============================================================================

/// Oracle that detects memory safety violations
pub struct MemorySafetyOracle {
    /// Configuration
    config: MemorySafetyConfig,
    /// Statistics
    stats: MemorySafetyStats,
    /// Track allocations (for leak detection)
    allocations: HashMap<String, AllocationInfo>,
}

/// Configuration for memory safety oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySafetyConfig {
    /// Enable use-after-free detection
    pub detect_use_after_free: bool,
    /// Enable double-free detection
    pub detect_double_free: bool,
    /// Enable memory leak detection
    pub detect_leaks: bool,
    /// Enable buffer overflow detection
    pub detect_buffer_overflow: bool,
    /// Enable CBGR violation detection
    pub detect_cbgr_violations: bool,
    /// Minimum leak size to report (bytes)
    pub min_leak_size: usize,
}

impl Default for MemorySafetyConfig {
    fn default() -> Self {
        Self {
            detect_use_after_free: true,
            detect_double_free: true,
            detect_leaks: true,
            detect_buffer_overflow: true,
            detect_cbgr_violations: true,
            min_leak_size: 1024,
        }
    }
}

/// Allocation tracking info
#[derive(Debug, Clone)]
struct AllocationInfo {
    address: String,
    size: usize,
    allocation_site: String,
    freed: bool,
    free_site: Option<String>,
}

/// Statistics for memory safety oracle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemorySafetyStats {
    /// Total checks
    pub total_checks: usize,
    /// Use-after-free detected
    pub use_after_free: usize,
    /// Double-free detected
    pub double_free: usize,
    /// Memory leaks detected
    pub leaks: usize,
    /// Buffer overflows detected
    pub buffer_overflows: usize,
    /// CBGR violations detected
    pub cbgr_violations: usize,
}

impl Default for MemorySafetyOracle {
    fn default() -> Self {
        Self::new(MemorySafetyConfig::default())
    }
}

impl MemorySafetyOracle {
    /// Create a new memory safety oracle
    pub fn new(config: MemorySafetyConfig) -> Self {
        Self {
            config,
            stats: MemorySafetyStats::default(),
            allocations: HashMap::new(),
        }
    }

    /// Check sanitizer output for memory safety violations
    pub fn check_sanitizer_output(&mut self, output: &str) -> Vec<MemorySafetyViolation> {
        self.stats.total_checks += 1;
        let mut violations = Vec::new();

        // Check for AddressSanitizer output
        if output.contains("AddressSanitizer") {
            if let Some(v) = self.parse_asan_output(output) {
                violations.push(v);
            }
        }

        // Check for CBGR violation patterns
        if self.config.detect_cbgr_violations {
            if let Some(v) = self.check_cbgr_violations(output) {
                violations.push(v);
            }
        }

        violations
    }

    /// Parse AddressSanitizer output
    fn parse_asan_output(&mut self, output: &str) -> Option<MemorySafetyViolation> {
        if output.contains("heap-use-after-free") {
            self.stats.use_after_free += 1;
            return Some(MemorySafetyViolation::UseAfterFree {
                address: self.extract_address(output).unwrap_or_default(),
                allocation_site: self.extract_allocation_site(output),
                free_site: self.extract_free_site(output),
                use_site: self.extract_use_site(output).unwrap_or_default(),
            });
        }

        if output.contains("double-free") {
            self.stats.double_free += 1;
            return Some(MemorySafetyViolation::DoubleFree {
                address: self.extract_address(output).unwrap_or_default(),
                first_free: self.extract_first_free(output).unwrap_or_default(),
                second_free: self.extract_second_free(output).unwrap_or_default(),
            });
        }

        if output.contains("heap-buffer-overflow") || output.contains("stack-buffer-overflow") {
            self.stats.buffer_overflows += 1;
            return Some(MemorySafetyViolation::BufferOverflow {
                buffer_address: self.extract_address(output).unwrap_or_default(),
                buffer_size: self.extract_buffer_size(output).unwrap_or(0),
                access_offset: self.extract_access_offset(output).unwrap_or(0),
                access_size: self.extract_access_size(output).unwrap_or(0),
            });
        }

        if output.contains("SEGV on unknown address") || output.contains("null pointer") {
            return Some(MemorySafetyViolation::NullPointerDeref {
                location: self.extract_crash_location(output).unwrap_or_default(),
            });
        }

        None
    }

    /// Check for CBGR violations in output
    fn check_cbgr_violations(&mut self, output: &str) -> Option<MemorySafetyViolation> {
        // Check for generation mismatch
        let gen_re =
            regex::Regex::new(r"CBGR generation mismatch: expected (\d+), got (\d+) at (.+)")
                .unwrap();

        if let Some(cap) = gen_re.captures(output) {
            self.stats.cbgr_violations += 1;
            return Some(MemorySafetyViolation::CbgrGenerationMismatch {
                expected_generation: cap
                    .get(1)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0),
                actual_generation: cap
                    .get(2)
                    .and_then(|m| m.as_str().parse().ok())
                    .unwrap_or(0),
                reference_location: cap
                    .get(3)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
            });
        }

        // Check for epoch violations
        if output.contains("CBGR epoch") || output.contains("epoch capability") {
            self.stats.cbgr_violations += 1;
            return Some(MemorySafetyViolation::CbgrEpochViolation {
                message: output
                    .lines()
                    .find(|l| l.contains("CBGR") || l.contains("epoch"))
                    .unwrap_or("")
                    .to_string(),
            });
        }

        None
    }

    // Helper extraction functions
    fn extract_address(&self, output: &str) -> Option<String> {
        let re = regex::Regex::new(r"0x[0-9a-fA-F]+").unwrap();
        re.find(output).map(|m| m.as_str().to_string())
    }

    fn extract_allocation_site(&self, output: &str) -> Option<String> {
        let re = regex::Regex::new(r"allocated by thread.*\n\s*#\d+\s+(.+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    fn extract_free_site(&self, output: &str) -> Option<String> {
        let re = regex::Regex::new(r"freed by thread.*\n\s*#\d+\s+(.+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    fn extract_use_site(&self, output: &str) -> Option<String> {
        let re = regex::Regex::new(r"READ of size.*\n\s*#\d+\s+(.+)").unwrap();
        re.captures(output)
            .or_else(|| {
                let re2 = regex::Regex::new(r"WRITE of size.*\n\s*#\d+\s+(.+)").unwrap();
                re2.captures(output)
            })
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    fn extract_first_free(&self, output: &str) -> Option<String> {
        self.extract_free_site(output)
    }

    fn extract_second_free(&self, output: &str) -> Option<String> {
        // Look for the second free in double-free output
        let re = regex::Regex::new(r"previously freed.*\n\s*#\d+\s+(.+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    fn extract_buffer_size(&self, output: &str) -> Option<usize> {
        let re = regex::Regex::new(r"buffer of size (\d+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).and_then(|m| m.as_str().parse().ok()))
    }

    fn extract_access_offset(&self, output: &str) -> Option<usize> {
        let re = regex::Regex::new(r"at offset (\d+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).and_then(|m| m.as_str().parse().ok()))
    }

    fn extract_access_size(&self, output: &str) -> Option<usize> {
        let re = regex::Regex::new(r"(?:READ|WRITE) of size (\d+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).and_then(|m| m.as_str().parse().ok()))
    }

    fn extract_crash_location(&self, output: &str) -> Option<String> {
        let re = regex::Regex::new(r"#0\s+(.+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    /// Get statistics
    pub fn stats(&self) -> &MemorySafetyStats {
        &self.stats
    }

    /// Reset state and statistics
    pub fn reset(&mut self) {
        self.stats = MemorySafetyStats::default();
        self.allocations.clear();
    }
}

// ============================================================================
// Type Safety Oracle
// ============================================================================

/// Oracle that detects type safety violations at runtime
pub struct TypeSafetyOracle {
    /// Configuration
    config: TypeSafetyConfig,
    /// Statistics
    stats: TypeSafetyStats,
}

/// Configuration for type safety oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSafetyConfig {
    /// Enable type confusion detection
    pub detect_type_confusion: bool,
    /// Enable cast failure detection
    pub detect_cast_failures: bool,
    /// Enable refinement violation detection
    pub detect_refinement_violations: bool,
    /// Enable variant mismatch detection
    pub detect_variant_mismatches: bool,
}

impl Default for TypeSafetyConfig {
    fn default() -> Self {
        Self {
            detect_type_confusion: true,
            detect_cast_failures: true,
            detect_refinement_violations: true,
            detect_variant_mismatches: true,
        }
    }
}

/// Statistics for type safety oracle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypeSafetyStats {
    /// Total checks
    pub total_checks: usize,
    /// Type confusion detected
    pub type_confusion: usize,
    /// Cast failures detected
    pub cast_failures: usize,
    /// Refinement violations detected
    pub refinement_violations: usize,
    /// Variant mismatches detected
    pub variant_mismatches: usize,
}

impl Default for TypeSafetyOracle {
    fn default() -> Self {
        Self::new(TypeSafetyConfig::default())
    }
}

impl TypeSafetyOracle {
    /// Create a new type safety oracle
    pub fn new(config: TypeSafetyConfig) -> Self {
        Self {
            config,
            stats: TypeSafetyStats::default(),
        }
    }

    /// Check runtime output for type safety violations
    pub fn check(&mut self, output: &str, stderr: &str) -> Option<TypeSafetyViolation> {
        self.stats.total_checks += 1;

        // Check for type confusion
        if self.config.detect_type_confusion {
            if let Some(v) = self.check_type_confusion(output, stderr) {
                self.stats.type_confusion += 1;
                return Some(v);
            }
        }

        // Check for cast failures
        if self.config.detect_cast_failures {
            if let Some(v) = self.check_cast_failure(stderr) {
                self.stats.cast_failures += 1;
                return Some(v);
            }
        }

        // Check for refinement violations
        if self.config.detect_refinement_violations {
            if let Some(v) = self.check_refinement_violation(stderr) {
                self.stats.refinement_violations += 1;
                return Some(v);
            }
        }

        // Check for variant mismatches
        if self.config.detect_variant_mismatches {
            if let Some(v) = self.check_variant_mismatch(stderr) {
                self.stats.variant_mismatches += 1;
                return Some(v);
            }
        }

        None
    }

    fn check_type_confusion(&self, _output: &str, stderr: &str) -> Option<TypeSafetyViolation> {
        let re = regex::Regex::new(r"type confusion: expected (\S+), got (\S+) at (.+)").unwrap();

        if let Some(cap) = re.captures(stderr) {
            return Some(TypeSafetyViolation {
                kind: TypeSafetyKind::TypeConfusion,
                expected_type: cap
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                actual_type: cap
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                location: cap
                    .get(3)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                details: None,
            });
        }
        None
    }

    fn check_cast_failure(&self, stderr: &str) -> Option<TypeSafetyViolation> {
        let re = regex::Regex::new(r"invalid cast from (\S+) to (\S+)").unwrap();

        if let Some(cap) = re.captures(stderr) {
            return Some(TypeSafetyViolation {
                kind: TypeSafetyKind::InvalidCast,
                expected_type: cap
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                actual_type: cap
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                location: String::new(),
                details: None,
            });
        }
        None
    }

    fn check_refinement_violation(&self, stderr: &str) -> Option<TypeSafetyViolation> {
        if stderr.contains("refinement predicate failed")
            || stderr.contains("refinement type violation")
        {
            let re = regex::Regex::new(r"refinement.*: (.+) does not satisfy (.+)").unwrap();

            let (actual, expected) = if let Some(cap) = re.captures(stderr) {
                (
                    cap.get(1)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                    cap.get(2)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                )
            } else {
                ("unknown".to_string(), "unknown refinement".to_string())
            };

            return Some(TypeSafetyViolation {
                kind: TypeSafetyKind::RefinementViolation,
                expected_type: expected,
                actual_type: actual,
                location: String::new(),
                details: Some(stderr.to_string()),
            });
        }
        None
    }

    fn check_variant_mismatch(&self, stderr: &str) -> Option<TypeSafetyViolation> {
        let re = regex::Regex::new(r"variant mismatch: expected (\S+), got (\S+)").unwrap();

        if let Some(cap) = re.captures(stderr) {
            return Some(TypeSafetyViolation {
                kind: TypeSafetyKind::VariantMismatch,
                expected_type: cap
                    .get(1)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                actual_type: cap
                    .get(2)
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_default(),
                location: String::new(),
                details: None,
            });
        }
        None
    }

    /// Get statistics
    pub fn stats(&self) -> &TypeSafetyStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = TypeSafetyStats::default();
    }
}

// ============================================================================
// SMT Oracle
// ============================================================================

/// Oracle that verifies SMT/refinement type contracts
pub struct SmtOracle {
    /// Configuration
    config: SmtOracleConfig,
    /// Statistics
    stats: SmtOracleStats,
}

/// Configuration for SMT oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtOracleConfig {
    /// Timeout for SMT solving in milliseconds
    pub solver_timeout_ms: u64,
    /// Whether to generate counter-examples
    pub generate_counter_examples: bool,
    /// Whether to check preconditions
    pub check_preconditions: bool,
    /// Whether to check postconditions
    pub check_postconditions: bool,
    /// Whether to check loop invariants
    pub check_invariants: bool,
}

impl Default for SmtOracleConfig {
    fn default() -> Self {
        Self {
            solver_timeout_ms: 30000,
            generate_counter_examples: true,
            check_preconditions: true,
            check_postconditions: true,
            check_invariants: true,
        }
    }
}

/// Statistics for SMT oracle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SmtOracleStats {
    /// Total verifications
    pub total_verifications: usize,
    /// Successful verifications
    pub verified: usize,
    /// Failed verifications
    pub failed: usize,
    /// Timeouts
    pub timeouts: usize,
    /// Unknown results
    pub unknown: usize,
}

impl Default for SmtOracle {
    fn default() -> Self {
        Self::new(SmtOracleConfig::default())
    }
}

impl SmtOracle {
    /// Create a new SMT oracle
    pub fn new(config: SmtOracleConfig) -> Self {
        Self {
            config,
            stats: SmtOracleStats::default(),
        }
    }

    /// Check SMT verification output for violations
    pub fn check(&mut self, verification_output: &str) -> Option<SmtViolation> {
        self.stats.total_verifications += 1;

        // Check for timeout
        if verification_output.contains("solver timeout")
            || verification_output.contains("Z3 timeout")
        {
            self.stats.timeouts += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::SolverTimeout,
                contract: String::new(),
                counter_example: None,
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Check for precondition violation
        if verification_output.contains("precondition failed")
            || verification_output.contains("requires clause violated")
        {
            self.stats.failed += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::PreconditionViolation,
                contract: self.extract_contract(verification_output),
                counter_example: self.extract_counter_example(verification_output),
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Check for postcondition violation
        if verification_output.contains("postcondition failed")
            || verification_output.contains("ensures clause violated")
        {
            self.stats.failed += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::PostconditionViolation,
                contract: self.extract_contract(verification_output),
                counter_example: self.extract_counter_example(verification_output),
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Check for assertion failure
        if verification_output.contains("assertion failed") {
            self.stats.failed += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::AssertionFailed,
                contract: self.extract_contract(verification_output),
                counter_example: self.extract_counter_example(verification_output),
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Check for invariant violation
        if verification_output.contains("invariant violated")
            || verification_output.contains("loop invariant failed")
        {
            self.stats.failed += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::InvariantViolation,
                contract: self.extract_contract(verification_output),
                counter_example: self.extract_counter_example(verification_output),
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Check for decreases violation
        if verification_output.contains("decreases clause failed")
            || verification_output.contains("termination check failed")
        {
            self.stats.failed += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::DecreasesViolation,
                contract: self.extract_contract(verification_output),
                counter_example: self.extract_counter_example(verification_output),
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Check for unknown
        if verification_output.contains("unknown") && !verification_output.contains("verified") {
            self.stats.unknown += 1;
            return Some(SmtViolation {
                kind: SmtViolationKind::SolverUnknown,
                contract: String::new(),
                counter_example: None,
                solver_output: Some(verification_output.to_string()),
            });
        }

        // Success
        self.stats.verified += 1;
        None
    }

    fn extract_contract(&self, output: &str) -> String {
        let re = regex::Regex::new(r"contract: (.+)").unwrap();
        re.captures(output)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            .unwrap_or_default()
    }

    fn extract_counter_example(&self, output: &str) -> Option<HashMap<String, String>> {
        if !self.config.generate_counter_examples {
            return None;
        }

        let mut counter_example = HashMap::new();

        // Look for variable assignments in counter-example
        let re = regex::Regex::new(r"(\w+)\s*=\s*([^\n]+)").unwrap();
        for cap in re.captures_iter(output) {
            if let (Some(var), Some(val)) = (cap.get(1), cap.get(2)) {
                counter_example.insert(var.as_str().to_string(), val.as_str().to_string());
            }
        }

        if counter_example.is_empty() {
            None
        } else {
            Some(counter_example)
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &SmtOracleStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = SmtOracleStats::default();
    }
}

// ============================================================================
// Timeout Oracle
// ============================================================================

/// Oracle that detects infinite loops and hangs
pub struct TimeoutOracle {
    /// Configuration
    config: TimeoutOracleConfig,
    /// Statistics
    stats: TimeoutOracleStats,
}

/// Configuration for timeout oracle
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutOracleConfig {
    /// Timeout for lexing in milliseconds
    pub lexer_timeout_ms: u64,
    /// Timeout for parsing in milliseconds
    pub parser_timeout_ms: u64,
    /// Timeout for type checking in milliseconds
    pub typecheck_timeout_ms: u64,
    /// Timeout for SMT solving in milliseconds
    pub smt_timeout_ms: u64,
    /// Timeout for code generation in milliseconds
    pub codegen_timeout_ms: u64,
    /// Timeout for execution in milliseconds
    pub execution_timeout_ms: u64,
}

impl Default for TimeoutOracleConfig {
    fn default() -> Self {
        Self {
            lexer_timeout_ms: 1000,
            parser_timeout_ms: 5000,
            typecheck_timeout_ms: 10000,
            smt_timeout_ms: 30000,
            codegen_timeout_ms: 10000,
            execution_timeout_ms: 60000,
        }
    }
}

/// Statistics for timeout oracle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeoutOracleStats {
    /// Total executions monitored
    pub total_executions: usize,
    /// Lexer timeouts
    pub lexer_timeouts: usize,
    /// Parser timeouts
    pub parser_timeouts: usize,
    /// Type checking timeouts
    pub typecheck_timeouts: usize,
    /// SMT timeouts
    pub smt_timeouts: usize,
    /// Codegen timeouts
    pub codegen_timeouts: usize,
    /// Execution timeouts
    pub execution_timeouts: usize,
}

impl Default for TimeoutOracle {
    fn default() -> Self {
        Self::new(TimeoutOracleConfig::default())
    }
}

impl TimeoutOracle {
    /// Create a new timeout oracle
    pub fn new(config: TimeoutOracleConfig) -> Self {
        Self {
            config,
            stats: TimeoutOracleStats::default(),
        }
    }

    /// Get the timeout for a given phase
    pub fn get_timeout(&self, phase: TimeoutPhase) -> Duration {
        Duration::from_millis(match phase {
            TimeoutPhase::Lexing => self.config.lexer_timeout_ms,
            TimeoutPhase::Parsing => self.config.parser_timeout_ms,
            TimeoutPhase::TypeChecking => self.config.typecheck_timeout_ms,
            TimeoutPhase::SmtSolving => self.config.smt_timeout_ms,
            TimeoutPhase::CodeGeneration => self.config.codegen_timeout_ms,
            TimeoutPhase::Execution => self.config.execution_timeout_ms,
            TimeoutPhase::Unknown => self.config.execution_timeout_ms,
        })
    }

    /// Check if execution exceeded timeout
    pub fn check(&mut self, phase: TimeoutPhase, elapsed: Duration) -> Option<TimeoutViolation> {
        self.stats.total_executions += 1;

        let timeout = self.get_timeout(phase);
        let elapsed_ms = elapsed.as_millis() as u64;
        let timeout_ms = timeout.as_millis() as u64;

        if elapsed >= timeout {
            // Update stats
            match phase {
                TimeoutPhase::Lexing => self.stats.lexer_timeouts += 1,
                TimeoutPhase::Parsing => self.stats.parser_timeouts += 1,
                TimeoutPhase::TypeChecking => self.stats.typecheck_timeouts += 1,
                TimeoutPhase::SmtSolving => self.stats.smt_timeouts += 1,
                TimeoutPhase::CodeGeneration => self.stats.codegen_timeouts += 1,
                TimeoutPhase::Execution => self.stats.execution_timeouts += 1,
                TimeoutPhase::Unknown => self.stats.execution_timeouts += 1,
            }

            return Some(TimeoutViolation {
                timeout_ms,
                elapsed_ms,
                phase,
                possible_cause: self.guess_cause(phase, elapsed_ms),
            });
        }

        None
    }

    fn guess_cause(&self, phase: TimeoutPhase, elapsed_ms: u64) -> Option<String> {
        match phase {
            TimeoutPhase::Lexing => {
                Some("Possible infinite loop in lexer (deeply nested comments?)".to_string())
            }
            TimeoutPhase::Parsing => {
                Some("Possible exponential backtracking in parser".to_string())
            }
            TimeoutPhase::TypeChecking => {
                Some("Possible cyclic type inference or exponential unification".to_string())
            }
            TimeoutPhase::SmtSolving => {
                if elapsed_ms > 60000 {
                    Some("SMT solver struggling with complex quantifiers".to_string())
                } else {
                    Some("SMT solver timeout on verification condition".to_string())
                }
            }
            TimeoutPhase::CodeGeneration => {
                Some("Possible infinite expansion in code generation".to_string())
            }
            TimeoutPhase::Execution => Some("Possible infinite loop in generated code".to_string()),
            TimeoutPhase::Unknown => None,
        }
    }

    /// Get statistics
    pub fn stats(&self) -> &TimeoutOracleStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = TimeoutOracleStats::default();
    }
}

// ============================================================================
// Unified Oracle Runner
// ============================================================================

/// Unified runner that combines all oracles
pub struct OracleRunner {
    /// Crash oracle
    pub crash_oracle: CrashOracle,
    /// Differential oracle
    pub differential_oracle: DifferentialOracle,
    /// Memory safety oracle
    pub memory_oracle: MemorySafetyOracle,
    /// Type safety oracle
    pub type_oracle: TypeSafetyOracle,
    /// SMT oracle
    pub smt_oracle: SmtOracle,
    /// Timeout oracle
    pub timeout_oracle: TimeoutOracle,
    /// Configuration
    config: OracleRunnerConfig,
    /// Combined statistics
    stats: OracleRunnerStats,
}

/// Configuration for oracle runner
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleRunnerConfig {
    /// Enable crash oracle
    pub enable_crash: bool,
    /// Enable differential oracle
    pub enable_differential: bool,
    /// Enable memory safety oracle
    pub enable_memory_safety: bool,
    /// Enable type safety oracle
    pub enable_type_safety: bool,
    /// Enable SMT oracle
    pub enable_smt: bool,
    /// Enable timeout oracle
    pub enable_timeout: bool,
}

impl Default for OracleRunnerConfig {
    fn default() -> Self {
        Self {
            enable_crash: true,
            enable_differential: true,
            enable_memory_safety: true,
            enable_type_safety: true,
            enable_smt: true,
            enable_timeout: true,
        }
    }
}

/// Combined statistics for all oracles
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OracleRunnerStats {
    /// Total checks across all oracles
    pub total_checks: usize,
    /// Total violations found
    pub violations_found: usize,
    /// Violations by type
    pub by_type: HashMap<String, usize>,
}

impl Default for OracleRunner {
    fn default() -> Self {
        Self::new(OracleRunnerConfig::default())
    }
}

impl OracleRunner {
    /// Create a new oracle runner with default configuration
    pub fn new(config: OracleRunnerConfig) -> Self {
        Self {
            crash_oracle: CrashOracle::default(),
            differential_oracle: DifferentialOracle::default(),
            memory_oracle: MemorySafetyOracle::default(),
            type_oracle: TypeSafetyOracle::default(),
            smt_oracle: SmtOracle::default(),
            timeout_oracle: TimeoutOracle::default(),
            config,
            stats: OracleRunnerStats::default(),
        }
    }

    /// Check all enabled oracles and return violations
    pub fn check_all(&mut self, result: &OracleExecutionResult) -> Vec<OracleViolation> {
        self.stats.total_checks += 1;
        let mut violations = Vec::new();

        // Check crash oracle
        if self.config.enable_crash {
            if let Some(v) = self.crash_oracle.check(result) {
                violations.push(OracleViolation::Crash(v));
            }
        }

        // Check memory safety
        if self.config.enable_memory_safety {
            for v in self.memory_oracle.check_sanitizer_output(&result.stderr) {
                violations.push(OracleViolation::MemorySafety(v));
            }
        }

        // Check type safety
        if self.config.enable_type_safety {
            if let Some(v) = self.type_oracle.check(&result.stdout, &result.stderr) {
                violations.push(OracleViolation::TypeSafety(v));
            }
        }

        // Update stats
        self.stats.violations_found += violations.len();
        for v in &violations {
            let type_name = match v {
                OracleViolation::Crash(_) => "crash",
                OracleViolation::Differential(_) => "differential",
                OracleViolation::MemorySafety(_) => "memory_safety",
                OracleViolation::TypeSafety(_) => "type_safety",
                OracleViolation::SmtVerification(_) => "smt",
                OracleViolation::Timeout(_) => "timeout",
            };
            *self.stats.by_type.entry(type_name.to_string()).or_insert(0) += 1;
        }

        violations
    }

    /// Check for timeout violation
    pub fn check_timeout(
        &mut self,
        phase: TimeoutPhase,
        elapsed: Duration,
    ) -> Option<OracleViolation> {
        if self.config.enable_timeout {
            self.timeout_oracle
                .check(phase, elapsed)
                .map(OracleViolation::Timeout)
        } else {
            None
        }
    }

    /// Check SMT verification output
    pub fn check_smt(&mut self, output: &str) -> Option<OracleViolation> {
        if self.config.enable_smt {
            self.smt_oracle
                .check(output)
                .map(OracleViolation::SmtVerification)
        } else {
            None
        }
    }

    /// Run differential oracle
    pub fn check_differential(
        &mut self,
        input: &str,
        results: HashMap<ExecutionTier, TierResult>,
    ) -> Option<OracleViolation> {
        if !self.config.enable_differential {
            return None;
        }

        let diff_result = self.differential_oracle.compare(input, results);

        if !diff_result.consistent && !diff_result.mismatches.is_empty() {
            let mismatch = &diff_result.mismatches[0];
            Some(OracleViolation::Differential(DifferentialViolation {
                reference_tier: mismatch.reference_tier,
                test_tier: mismatch.mismatched_tier,
                difference: format!("{:?}", mismatch.kind),
                reference_output: mismatch.details.clone(),
                test_output: String::new(),
            }))
        } else {
            None
        }
    }

    /// Get combined statistics
    pub fn stats(&self) -> &OracleRunnerStats {
        &self.stats
    }

    /// Reset all oracles
    pub fn reset(&mut self) {
        self.crash_oracle.reset_stats();
        self.differential_oracle.reset_stats();
        self.memory_oracle.reset();
        self.type_oracle.reset_stats();
        self.smt_oracle.reset_stats();
        self.timeout_oracle.reset_stats();
        self.stats = OracleRunnerStats::default();
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    #[test]
    fn test_crash_oracle_panic() {
        let mut oracle = CrashOracle::default();

        // Use exit code 0 so we actually test the panic detection logic (not exit code analysis)
        let result = OracleExecutionResult {
            success: false,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: "thread 'main' panicked at 'index out of bounds'".to_string(),
            return_value: None,
            duration: Duration::from_millis(100),
            memory_bytes: 1024,
            tier: ExecutionTier::Tier0,
        };

        let violation = oracle.check(&result);
        assert!(violation.is_some());
        assert_eq!(violation.unwrap().crash_type, CrashType::Panic);
    }

    #[test]
    fn test_memory_safety_oracle_asan() {
        let mut oracle = MemorySafetyOracle::default();

        let asan_output = r#"
AddressSanitizer: heap-use-after-free on address 0x1234
READ of size 8 at 0x1234 thread T0
    #0 test_func at test.rs:10
"#;

        let violations = oracle.check_sanitizer_output(asan_output);
        assert_eq!(violations.len(), 1);
        assert!(matches!(
            violations[0],
            MemorySafetyViolation::UseAfterFree { .. }
        ));
    }

    #[test]
    fn test_timeout_oracle() {
        let mut oracle = TimeoutOracle::default();

        // Within timeout
        let result = oracle.check(TimeoutPhase::Lexing, Duration::from_millis(500));
        assert!(result.is_none());

        // Exceeds timeout
        let result = oracle.check(TimeoutPhase::Lexing, Duration::from_millis(2000));
        assert!(result.is_some());
        assert_eq!(result.unwrap().phase, TimeoutPhase::Lexing);
    }

    #[test]
    fn test_smt_oracle() {
        let mut oracle = SmtOracle::default();

        let output = "verification: postcondition failed for function foo";
        let result = oracle.check(output);
        assert!(result.is_some());
        assert!(matches!(
            result.unwrap().kind,
            SmtViolationKind::PostconditionViolation
        ));
    }

    #[test]
    fn test_type_safety_oracle() {
        let mut oracle = TypeSafetyOracle::default();

        let stderr = "type confusion: expected Int, got Text at main.vr:10";
        let result = oracle.check("", stderr);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().kind, TypeSafetyKind::TypeConfusion);
        assert_eq!(result.as_ref().unwrap().expected_type, "Int");
        assert_eq!(result.as_ref().unwrap().actual_type, "Text");
    }

    #[test]
    fn test_oracle_runner() {
        let mut runner = OracleRunner::default();

        let result = OracleExecutionResult {
            success: true,
            exit_code: Some(0),
            stdout: "42".to_string(),
            stderr: String::new(),
            return_value: Some(SerializedValue::Int(42)),
            duration: Duration::from_millis(100),
            memory_bytes: 1024,
            tier: ExecutionTier::Tier0,
        };

        let violations = runner.check_all(&result);
        assert!(violations.is_empty());
    }
}
