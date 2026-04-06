//! Verification Level Types
//!
//! Implements the three-level gradual verification system:
//! - Runtime: Dynamic checks with ~5-15ns overhead
//! - Static: Compile-time verification with SMT solver
//! - Proof: Formal proofs with proof certificates
//!
//! Verum has three verification levels forming a gradual spectrum:
//!
//! | Level    | Annotation           | Runtime Cost     | Compile Time | Use Case                        |
//! |----------|----------------------|------------------|--------------|---------------------------------|
//! | Runtime  | @verify(runtime)     | ~5-15ns per check| +0%          | Development, debugging          |
//! | Static   | @verify(static)      | 0ns when proven  | +10-20%      | Hot paths, simple predicates    |
//! | Proof    | @verify(proof)       | 0ns or ERROR     | +2-10x       | Critical code, contracts        |

use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Verification level for gradual verification
///
/// This represents the three-tier verification system in Verum, providing
/// a smooth spectrum from runtime checking to formal verification.
///
/// - Runtime: all safety checks executed at runtime
/// - Static: conservative static analysis proves safety, checks eliminated in AOT
/// - Proof: SMT solver proves correctness, mathematical guarantee
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VerificationLevel {
    /// Runtime validation (default)
    ///
    /// - All safety checks executed at runtime
    /// - CBGR overhead: ~100ns (Tier 0), ~15ns (Tier 1-3)
    /// - Refinement validation: ~5ns per check
    /// - Use case: Development, debugging, untrusted input
    #[default]
    Runtime,

    /// Static verification (AOT optimization)
    ///
    /// - Conservative static analysis proves safety
    /// - Checks eliminated when proven safe (0ns)
    /// - Fallback to runtime checks if proof fails
    /// - Use case: Hot paths, simple predicates, production code
    Static,

    /// Proof verification (formal)
    ///
    /// - Full SMT-based formal verification
    /// - Mathematical guarantees of correctness
    /// - Compile error if proof fails (no fallback)
    /// - Optional proof certificate generation
    /// - Use case: Critical code, financial systems, smart contracts
    Proof,
}

impl VerificationLevel {
    /// Returns true if this level requires SMT solver
    pub fn requires_smt(&self) -> bool {
        matches!(self, VerificationLevel::Static | VerificationLevel::Proof)
    }

    /// Returns true if this level allows runtime fallback
    pub fn allows_runtime_fallback(&self) -> bool {
        matches!(self, VerificationLevel::Runtime | VerificationLevel::Static)
    }

    /// Returns true if this level generates proof certificates
    pub fn generates_proof_certificate(&self) -> bool {
        matches!(self, VerificationLevel::Proof)
    }

    /// Get expected runtime overhead for this verification level
    pub fn expected_overhead_ns(&self) -> u64 {
        match self {
            VerificationLevel::Runtime => 15, // Tier 1-3 CBGR + refinement
            VerificationLevel::Static => 0,   // Proven safe
            VerificationLevel::Proof => 0,    // Proven safe
        }
    }

    /// Get expected compile-time overhead multiplier
    pub fn compile_time_multiplier(&self) -> f64 {
        match self {
            VerificationLevel::Runtime => 1.0, // No extra compilation
            VerificationLevel::Static => 1.15, // +15% for static analysis
            VerificationLevel::Proof => 5.0,   // +400% for SMT solving
        }
    }

    /// Parse from @verify annotation
    ///
    /// Examples:
    /// - `@verify(runtime)` -> Runtime
    /// - `@verify(static)` -> Static
    /// - `@verify(proof)` -> Proof
    pub fn from_annotation(annotation: &str) -> Option<Self> {
        match annotation {
            "runtime" => Some(VerificationLevel::Runtime),
            "static" => Some(VerificationLevel::Static),
            "proof" => Some(VerificationLevel::Proof),
            _ => None,
        }
    }

    /// Convert to annotation string
    pub fn to_annotation(&self) -> &'static str {
        match self {
            VerificationLevel::Runtime => "runtime",
            VerificationLevel::Static => "static",
            VerificationLevel::Proof => "proof",
        }
    }
}

impl fmt::Display for VerificationLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerificationLevel::Runtime => write!(f, "runtime"),
            VerificationLevel::Static => write!(f, "static"),
            VerificationLevel::Proof => write!(f, "proof"),
        }
    }
}

/// Verification mode combining level with configuration
///
/// This extends the basic verification level with configuration options
/// like timeout, solver choice, and proof certificate generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationMode {
    /// Base verification level
    pub level: VerificationLevel,

    /// Configuration for this mode
    pub config: VerificationConfig,
}

impl VerificationMode {
    /// Create a new verification mode with default config
    pub fn new(level: VerificationLevel) -> Self {
        Self {
            level,
            config: VerificationConfig::default_for_level(level),
        }
    }

    /// Create with custom configuration
    pub fn with_config(level: VerificationLevel, config: VerificationConfig) -> Self {
        Self { level, config }
    }

    /// Create runtime mode
    pub fn runtime() -> Self {
        Self::new(VerificationLevel::Runtime)
    }

    /// Create static mode
    pub fn static_mode() -> Self {
        Self::new(VerificationLevel::Static)
    }

    /// Create proof mode
    pub fn proof() -> Self {
        Self::new(VerificationLevel::Proof)
    }

    /// Create proof mode with timeout
    pub fn proof_with_timeout(timeout_ms: u64) -> Self {
        let mut mode = Self::proof();
        mode.config.timeout = Some(Duration::from_millis(timeout_ms));
        mode
    }
}

impl Default for VerificationMode {
    fn default() -> Self {
        Self::runtime()
    }
}

/// Configuration for verification
///
/// Provides fine-grained control over verification behavior including
/// timeouts, solver selection, and certificate generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationConfig {
    /// Timeout for SMT solver (None = no timeout)
    pub timeout: Option<Duration>,

    /// SMT solver to use (Z3 or CVC5)
    pub solver: SolverChoice,

    /// Generate proof certificate
    pub emit_certificate: bool,

    /// Enable aggressive optimization
    pub aggressive_optimization: bool,

    /// Allow runtime fallback if verification fails
    pub allow_runtime_fallback: bool,

    /// Maximum number of SMT queries
    pub max_smt_queries: Option<usize>,

    /// Enable verification caching
    pub enable_caching: bool,

    /// Verification cost budget (ms)
    pub cost_budget_ms: Option<u64>,
}

impl VerificationConfig {
    /// Create default configuration for a verification level
    pub fn default_for_level(level: VerificationLevel) -> Self {
        match level {
            VerificationLevel::Runtime => Self {
                timeout: None,
                solver: SolverChoice::None,
                emit_certificate: false,
                aggressive_optimization: false,
                allow_runtime_fallback: true,
                max_smt_queries: None,
                enable_caching: false,
                cost_budget_ms: None,
            },
            VerificationLevel::Static => Self {
                timeout: Some(Duration::from_secs(5)),
                solver: SolverChoice::Auto,
                emit_certificate: false,
                aggressive_optimization: true,
                allow_runtime_fallback: true,
                max_smt_queries: Some(100),
                enable_caching: true,
                cost_budget_ms: Some(10_000), // 10s budget
            },
            VerificationLevel::Proof => Self {
                timeout: Some(Duration::from_secs(30)),
                solver: SolverChoice::Z3,
                emit_certificate: true,
                aggressive_optimization: true,
                allow_runtime_fallback: false,
                max_smt_queries: Some(1000),
                enable_caching: true,
                cost_budget_ms: Some(60_000), // 60s budget
            },
        }
    }
}

impl Default for VerificationConfig {
    fn default() -> Self {
        Self::default_for_level(VerificationLevel::Runtime)
    }
}

/// Choice of SMT solver
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SolverChoice {
    /// No SMT solver (runtime mode)
    None,

    /// Automatically select best solver
    Auto,

    /// Use Z3 (better for quantified formulas)
    Z3,

    /// Use CVC5 (faster for quantifier-free logics)
    CVC5,
}

impl SolverChoice {
    /// Returns true if this choice uses an SMT solver
    pub fn uses_smt(&self) -> bool {
        !matches!(self, SolverChoice::None)
    }
}

impl fmt::Display for SolverChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverChoice::None => write!(f, "none"),
            SolverChoice::Auto => write!(f, "auto"),
            SolverChoice::Z3 => write!(f, "z3"),
            SolverChoice::CVC5 => write!(f, "cvc5"),
        }
    }
}

/// Runtime verification level (trait-based approach)
///
/// Marker trait for runtime verification - all checks at runtime
pub trait RuntimeLevel {
    /// Get runtime overhead in nanoseconds
    fn overhead_ns(&self) -> u64 {
        15
    }

    /// Returns true (runtime level always uses runtime checks)
    fn uses_runtime_checks(&self) -> bool {
        true
    }
}

/// Static verification level (trait-based approach)
///
/// Marker trait for static verification - SMT-based proof at compile time
pub trait StaticLevel {
    /// Returns true (static level uses SMT)
    fn uses_smt(&self) -> bool {
        true
    }

    /// Returns true (static level allows runtime fallback)
    fn allows_fallback(&self) -> bool {
        true
    }

    /// Get expected compile-time overhead
    fn compile_overhead_percent(&self) -> u32 {
        15
    }
}

/// Proof verification level (trait-based approach)
///
/// Marker trait for proof verification - formal proofs required
pub trait ProofLevel {
    /// Returns true (proof level uses SMT)
    fn uses_smt(&self) -> bool {
        true
    }

    /// Returns false (proof level does not allow fallback)
    fn allows_fallback(&self) -> bool {
        false
    }

    /// Returns true (proof level generates certificates)
    fn generates_certificate(&self) -> bool {
        true
    }

    /// Get expected compile-time overhead
    fn compile_overhead_percent(&self) -> u32 {
        400
    }
}

// Implement marker traits for VerificationLevel
impl RuntimeLevel for VerificationLevel {}
impl StaticLevel for VerificationLevel {}
impl ProofLevel for VerificationLevel {}
