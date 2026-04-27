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
use verum_common::Text;

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

    /// Parse from `@verify(...)` annotation.
    ///
    /// Verum has **two layers** of verification intent:
    ///
    /// 1. `VerificationLevel` (this enum) — the coarse compile-time
    ///    gradient: Runtime / Static / Proof. Drives pipeline-level
    ///    decisions (SMT on? runtime checks emitted? proof-cert required?).
    /// 2. `VerifyStrategy` in `verum_smt::verify_strategy` — the fine-grained
    ///    operational strategy: Formal / Fast / Thorough / Certified /
    ///    Synthesize. Drives per-obligation solver routing, cross-validation,
    ///    timeout scaling, synthesis dispatch.
    ///
    /// `from_annotation` accepts every grammar-legal `verify_strategy` name
    /// (grammar/verum.ebnf §2 `verify_strategy`) and projects it onto the
    /// 3-level gradient. The finer strategy nuances (`fast`/`thorough`/
    /// `certified`/`synthesize`) are handled by the SMT backend switcher
    /// via `VerifyStrategy::from_attribute_value` — callers that need the
    /// full operational strategy should use that directly.
    ///
    /// Returns `None` only for unknown names, so tooling can emit a
    /// diagnostic on truly invalid annotations (never on valid grammar).
    pub fn from_annotation(annotation: &str) -> Option<Self> {
        match annotation {
            "runtime" => Some(VerificationLevel::Runtime),
            "static" => Some(VerificationLevel::Static),
            // `proof` / `formal` are the canonical formal-verification level.
            "proof" | "formal" => Some(VerificationLevel::Proof),
            // `fast`, `thorough` / `reliable`, `certified`, `synthesize` are
            // all operational strategies that still run the full proof
            // pipeline — they collapse to the `Proof` level. The fine-grained
            // dispatch happens downstream via `VerifyStrategy`.
            "fast" | "quick" | "rapid" => Some(VerificationLevel::Proof),
            // Bounded-arithmetic (V0) — bounded-arithmetic verification still runs a
            // proof-style discharge; routing happens via `VerifyStrategy`.
            "complexity_typed" | "complexity-typed" | "complexitytyped" => {
                Some(VerificationLevel::Proof)
            }
            "thorough" | "reliable" | "robust" => Some(VerificationLevel::Proof),
            "certified" | "cross_validate" | "cross-validate" | "crossvalidate" => {
                Some(VerificationLevel::Proof)
            }
            // Coherent verification — the three coherent variants always discharge
            // through the proof pipeline. `coherent_runtime` additionally
            // emits a runtime ε-monitor; that emission decision is taken
            // by `VerifyStrategy::requires_runtime_epsilon_monitor` at the
            // operational layer, not here.
            "coherent_static" | "coherent-static" | "coherentstatic" => {
                Some(VerificationLevel::Proof)
            }
            "coherent_runtime" | "coherent-runtime" | "coherentruntime" => {
                Some(VerificationLevel::Proof)
            }
            "coherent" => Some(VerificationLevel::Proof),
            "synthesize" | "synthesis" | "synth" => Some(VerificationLevel::Proof),
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

    /// Evaluate the outcome of a proof-attempt at this level.
    ///
    /// This encodes the single policy decision that distinguishes the
    /// three levels at the compile-time / runtime boundary:
    ///
    /// | Level    | Proof succeeds | Proof fails         |
    /// |----------|----------------|---------------------|
    /// | Runtime  | no SMT call    | no SMT call         |
    /// | Static   | check elided   | fall back → runtime |
    /// | Proof    | check elided   | **hard compile fail** |
    ///
    /// ### Runtime level
    ///
    /// Runtime never attempts a proof — we pre-short-circuit here and
    /// always return `EmitRuntimeCheck`. The compiler consumer should
    /// honor this without ever invoking the SMT backend.
    ///
    /// ### Static level
    ///
    /// Static is the "best-effort optimisation" level. If the backend
    /// proves the obligation, the check is elided. If the backend
    /// times out or returns *Unknown* / *Sat*, the compiler emits the
    /// runtime check plus a `W501` warning (`soft-fail fallback:
    /// `static` became `runtime` for <name>`) — the build still
    /// succeeds. This matches the docs at
    /// `docs/verification/levels.md §2`.
    ///
    /// ### Proof level
    ///
    /// Proof is the "no fallback" contract. A failed proof is a hard
    /// compile error (`E502`), never silently demoted to runtime.
    ///
    /// The `ProofAttempt::Unattempted` variant is for call sites that
    /// want to short-circuit (e.g. when SMT is globally disabled via
    /// `--no-smt`): Runtime / Static fall back, Proof hard-fails.
    pub fn evaluate_attempt(&self, attempt: ProofAttempt) -> VerificationOutcome {
        match (self, attempt) {
            // Runtime never runs the proof — the caller shouldn't even
            // reach here with Proven / Failed. If they did, we still
            // short-circuit to runtime check emission so the policy
            // is crash-proof.
            (VerificationLevel::Runtime, _) => VerificationOutcome::EmitRuntimeCheck,

            // Static: proven → elide; failed or unattempted → runtime
            // fallback with warning.
            (VerificationLevel::Static, ProofAttempt::Proven) => {
                VerificationOutcome::ElideCheck
            }
            (VerificationLevel::Static, ProofAttempt::Failed(reason)) => {
                VerificationOutcome::FallbackWithWarning(reason)
            }
            (VerificationLevel::Static, ProofAttempt::Unattempted) => {
                VerificationOutcome::FallbackWithWarning(
                    Text::from("SMT not attempted (e.g. --no-smt)"),
                )
            }

            // Proof: proven → elide; failed → hard error; unattempted
            // → hard error (Proof's whole contract is "must prove").
            (VerificationLevel::Proof, ProofAttempt::Proven) => {
                VerificationOutcome::ElideCheck
            }
            (VerificationLevel::Proof, ProofAttempt::Failed(reason)) => {
                VerificationOutcome::HardFail(reason)
            }
            (VerificationLevel::Proof, ProofAttempt::Unattempted) => {
                VerificationOutcome::HardFail(Text::from(
                    "Proof-level obligation requires SMT; none was \
                     attempted (is the solver disabled?)",
                ))
            }
        }
    }
}

/// Input to [`VerificationLevel::evaluate_attempt`] — the result of
/// actually invoking the SMT backend (or the signal that no backend
/// was invoked at all).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofAttempt {
    /// The backend returned *Unsat* on the negated obligation — the
    /// obligation is valid.
    Proven,

    /// The backend returned *Sat*, *Unknown*, or timed out. Carries a
    /// short human-readable reason (counterexample summary, timeout,
    /// solver-unknown rationale) for the diagnostic.
    Failed(Text),

    /// No backend was invoked — most commonly because SMT is
    /// globally disabled (e.g. `--no-smt`, dev builds). Static
    /// gracefully falls back; Proof hard-fails.
    Unattempted,
}

/// Output of [`VerificationLevel::evaluate_attempt`] — the
/// instruction the compiler consumer should follow. This is
/// deliberately kept side-effect-free: the consumer translates each
/// variant into the actual VBC emission / diagnostic surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationOutcome {
    /// The obligation is proven — do not emit a runtime check.
    ElideCheck,

    /// Emit the runtime check. No diagnostic (this is the Runtime
    /// level's normal behaviour or Static's explicit "not proven but
    /// OK to run" signal when the caller pre-decided no proof).
    EmitRuntimeCheck,

    /// Emit the runtime check **and** a `W501` warning saying why
    /// the static proof did not succeed. The compiler consumer is
    /// responsible for translating this into the diagnostic surface
    /// (CLI / LSP).
    FallbackWithWarning(Text),

    /// Fail the build with an `E502` error citing the proof failure.
    /// Proof level only; Static must never return this variant.
    HardFail(Text),
}

impl VerificationOutcome {
    /// Whether this outcome requires the compiler to emit a runtime
    /// check into the generated VBC.
    pub fn requires_runtime_check(&self) -> bool {
        matches!(
            self,
            VerificationOutcome::EmitRuntimeCheck
                | VerificationOutcome::FallbackWithWarning(_)
        )
    }

    /// Whether this outcome should halt compilation.
    pub fn is_hard_failure(&self) -> bool {
        matches!(self, VerificationOutcome::HardFail(_))
    }

    /// Whether this outcome should emit a warning diagnostic.
    pub fn is_warning(&self) -> bool {
        matches!(self, VerificationOutcome::FallbackWithWarning(_))
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

#[cfg(test)]
mod attempt_tests {
    use super::*;

    // ---- Runtime level: SMT never consulted ----

    #[test]
    fn runtime_always_emits_runtime_check() {
        assert_eq!(
            VerificationLevel::Runtime.evaluate_attempt(ProofAttempt::Unattempted),
            VerificationOutcome::EmitRuntimeCheck
        );
        assert_eq!(
            VerificationLevel::Runtime.evaluate_attempt(ProofAttempt::Proven),
            VerificationOutcome::EmitRuntimeCheck
        );
        assert_eq!(
            VerificationLevel::Runtime
                .evaluate_attempt(ProofAttempt::Failed(Text::from("x"))),
            VerificationOutcome::EmitRuntimeCheck
        );
    }

    // ---- Static level: proof-attempt then soft-fail ----

    #[test]
    fn static_elides_check_when_proven() {
        assert_eq!(
            VerificationLevel::Static.evaluate_attempt(ProofAttempt::Proven),
            VerificationOutcome::ElideCheck
        );
    }

    #[test]
    fn static_falls_back_with_warning_on_proof_failure() {
        let outcome = VerificationLevel::Static
            .evaluate_attempt(ProofAttempt::Failed(Text::from("timeout")));
        match outcome {
            VerificationOutcome::FallbackWithWarning(reason) => {
                assert_eq!(reason.as_str(), "timeout");
            }
            other => panic!("expected FallbackWithWarning, got {:?}", other),
        }
    }

    #[test]
    fn static_falls_back_with_warning_on_unattempted() {
        let outcome =
            VerificationLevel::Static.evaluate_attempt(ProofAttempt::Unattempted);
        assert!(outcome.is_warning());
        assert!(outcome.requires_runtime_check());
        assert!(!outcome.is_hard_failure());
    }

    // ---- Proof level: hard-fail on anything but Proven ----

    #[test]
    fn proof_elides_check_when_proven() {
        assert_eq!(
            VerificationLevel::Proof.evaluate_attempt(ProofAttempt::Proven),
            VerificationOutcome::ElideCheck
        );
    }

    #[test]
    fn proof_hard_fails_on_proof_failure() {
        let outcome = VerificationLevel::Proof
            .evaluate_attempt(ProofAttempt::Failed(Text::from("sat model found")));
        match outcome {
            VerificationOutcome::HardFail(reason) => {
                assert_eq!(reason.as_str(), "sat model found");
            }
            other => panic!("expected HardFail, got {:?}", other),
        }
    }

    #[test]
    fn proof_hard_fails_on_unattempted() {
        let outcome =
            VerificationLevel::Proof.evaluate_attempt(ProofAttempt::Unattempted);
        assert!(outcome.is_hard_failure());
        assert!(!outcome.requires_runtime_check());
    }

    // ---- VerificationOutcome predicate consistency ----

    #[test]
    fn outcome_predicates_are_mutually_consistent() {
        let elide = VerificationOutcome::ElideCheck;
        assert!(!elide.requires_runtime_check());
        assert!(!elide.is_hard_failure());
        assert!(!elide.is_warning());

        let emit = VerificationOutcome::EmitRuntimeCheck;
        assert!(emit.requires_runtime_check());
        assert!(!emit.is_hard_failure());
        assert!(!emit.is_warning());

        let fb = VerificationOutcome::FallbackWithWarning(Text::from("r"));
        assert!(fb.requires_runtime_check());
        assert!(!fb.is_hard_failure());
        assert!(fb.is_warning());

        let hf = VerificationOutcome::HardFail(Text::from("r"));
        assert!(!hf.requires_runtime_check());
        assert!(hf.is_hard_failure());
        assert!(!hf.is_warning());
    }

    #[test]
    fn static_level_config_matches_policy() {
        // The config-side flag must agree with the evaluate_attempt
        // policy: Static allows fallback; Proof doesn't.
        assert!(VerificationLevel::Static.allows_runtime_fallback());
        assert!(!VerificationLevel::Proof.allows_runtime_fallback());
        assert!(VerificationLevel::Runtime.allows_runtime_fallback());
    }
}
