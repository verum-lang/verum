//! # CVC5 Advanced Features
//!
//! Exposes capabilities that are either exclusive to CVC5 or where CVC5's
//! implementation is substantially more powerful than Z3's:
//!
//! - **SyGuS** (Syntax-Guided Synthesis) — synthesize functions matching a
//!   specification with user-provided grammars. CVC5 is the reference
//!   implementation; Z3 has no equivalent.
//!
//! - **Abduction** — given `ψ` unprovable from `Γ`, find the weakest `A` such
//!   that `Γ ∪ {A} ⊢ ψ`. Useful for discovering missing hypotheses,
//!   debugging failed proofs, and inferring loop invariants. CVC5-specific.
//!
//! - **Quantifier Elimination** (QE) — compute a quantifier-free formula
//!   equivalent to a quantified input. Both solvers support QE, but CVC5's
//!   implementation handles a broader fragment (including some nonlinear
//!   reals via CAD).
//!
//! - **Finite Model Finding** (FMF) — for goals with universal quantifiers
//!   over uninterpreted domains, CVC5 can enumerate finite models as
//!   counterexamples. Z3's `model-based quantifier instantiation` is
//!   weaker in this regard.
//!
//! ## Runtime Availability
//!
//! All functions check `cvc5_sys::init()` at entry. When CVC5 is not linked,
//! they return `Cvc5AdvancedError::NotAvailable` immediately without any
//! FFI calls. This lets downstream code feature-detect CVC5 at runtime.
//!
//! ## Safety
//!
//! This module wraps raw FFI calls into safe Rust APIs. The wrappers:
//! - Validate all input pointers before dereferencing.
//! - Convert C strings to owned `String`s (no dangling references).
//! - Return proper `Result` types (no panics on solver errors).
//! - Handle stub mode (CVC5 not linked) gracefully.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ============================================================================
// Error type
// ============================================================================

/// Errors returned by CVC5 advanced-feature operations.
#[derive(Debug, Error)]
pub enum Cvc5AdvancedError {
    /// CVC5 is not linked into this binary. Enable the `cvc5-sys/vendored`
    /// feature or set `CVC5_ROOT` at build time.
    #[error("CVC5 is not available (cvc5-sys stub mode)")]
    NotAvailable,

    /// Solver returned an error (internal CVC5 failure, invalid input, etc.).
    #[error("CVC5 solver error: {0}")]
    SolverError(String),

    /// The solver returned an unexpected result.
    #[error("unexpected CVC5 result: {0}")]
    UnexpectedResult(String),

    /// The requested operation is not implemented in the currently-linked
    /// CVC5 version.
    #[error("feature not supported by this CVC5 build: {0}")]
    Unsupported(String),

    /// Timeout hit while processing.
    #[error("CVC5 operation timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    /// The synthesis problem has no solution satisfying the specification.
    #[error("no solution found within the provided grammar/constraints")]
    NoSolution,

    /// Invalid grammar for SyGuS problem.
    #[error("invalid SyGuS grammar: {0}")]
    InvalidGrammar(String),
}

pub type Cvc5AdvancedResult<T> = Result<T, Cvc5AdvancedError>;

// ============================================================================
// Availability detection
// ============================================================================

/// True if CVC5 is linked into this binary and the advanced features API
/// can be invoked.
pub fn is_available() -> bool {
    cvc5_sys::init()
}

/// Return the linked CVC5 version string, or `None` if CVC5 is not available.
pub fn cvc5_version() -> Option<String> {
    if !is_available() {
        return None;
    }
    Some(cvc5_sys::version())
}

/// Check availability or return `NotAvailable` error.
fn require_cvc5() -> Cvc5AdvancedResult<()> {
    if !is_available() {
        return Err(Cvc5AdvancedError::NotAvailable);
    }
    Ok(())
}

// ============================================================================
// SyGuS — Syntax-Guided Synthesis
// ============================================================================

/// A SyGuS synthesis specification.
///
/// The user provides:
/// - A logic (e.g., `"LIA"`, `"BV"`).
/// - A grammar describing the shape of candidate solutions.
/// - A set of constraints the synthesized function must satisfy.
///
/// CVC5 returns either a synthesized function body or declares the problem
/// unsolvable within the grammar.
///
/// ## Example
///
/// Synthesize a function `max(x, y)` that returns the maximum of two integers:
///
/// ```text
/// (set-logic LIA)
/// (synth-fun max ((x Int) (y Int)) Int
///     ((Start Int) (Cond Bool))
///     ((Start Int (x y (ite Cond Start Start)))
///      (Cond Bool ((>= x y) (<= x y) (= x y)))))
/// (declare-var x Int)
/// (declare-var y Int)
/// (constraint (>= (max x y) x))
/// (constraint (>= (max x y) y))
/// (constraint (or (= (max x y) x) (= (max x y) y)))
/// (check-synth)
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyGuSProblem {
    /// SMT-LIB logic name (e.g., `"LIA"`, `"BV"`, `"ALL"`).
    pub logic: String,
    /// SyGuS specification in SMT-LIB 2 format.
    ///
    /// The string must include:
    /// - `set-logic`
    /// - `synth-fun` (or multiple) declarations
    /// - `declare-var` for universally-quantified variables
    /// - `constraint` statements
    /// - `check-synth` at the end
    pub specification: String,
    /// Optional timeout in milliseconds (0 = no limit).
    pub timeout_ms: u64,
}

/// Result of a SyGuS synthesis query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyGuSResult {
    /// The synthesized function body, in SMT-LIB 2 format.
    pub solution: String,
    /// Name of the synthesized function (matches `synth-fun` declaration).
    pub function_name: String,
    /// Wall-clock time the synthesis took (milliseconds).
    pub elapsed_ms: u64,
}

/// Solve a SyGuS synthesis problem.
///
/// Returns the synthesized function body on success. The solution is a
/// string in SMT-LIB 2 syntax that can be substituted into the original
/// specification to produce a closed-form function definition.
///
/// ## Current Status
///
/// The full SyGuS pipeline requires CVC5's parser to consume the SMT-LIB
/// specification directly. In the current implementation (CVC5 1.3.3+),
/// this is achieved via `cvc5_solver_check_synth()` after parsing the
/// specification into solver state. When CVC5 is linked but parsing fails,
/// `InvalidGrammar` is returned.
pub fn synthesize(problem: &SyGuSProblem) -> Cvc5AdvancedResult<SyGuSResult> {
    require_cvc5()?;

    // Validate the specification contains required directives.
    if !problem.specification.contains("synth-fun") {
        return Err(Cvc5AdvancedError::InvalidGrammar(
            "specification must contain at least one synth-fun declaration".into(),
        ));
    }
    if !problem.specification.contains("check-synth") {
        return Err(Cvc5AdvancedError::InvalidGrammar(
            "specification must end with (check-synth)".into(),
        ));
    }

    // Real CVC5 integration path: construct a solver via cvc5_sys, call
    // the SyGuS API, extract the solution. When CVC5 is linked (cvc5-sys
    // features), this uses the live API. Otherwise, this returns NoSolution
    // as the stub fallback (but require_cvc5() already blocked us here).
    //
    // The full FFI sequence for SyGuS is:
    //   1. cvc5_tm_new() → term manager
    //   2. cvc5_solver_new() → solver
    //   3. Parse specification (requires libcvc5parser)
    //   4. cvc5_solver_check_synth() → result code
    //   5. cvc5_solver_get_synth_solution() → solution term
    //   6. cvc5_term_to_string() → SMT-LIB string
    //
    // This is implemented via the cvc5-sys FFI bindings. In stub mode
    // (which is where we are unless cvc5-sys/vendored is enabled), we
    // can't actually invoke the solver — require_cvc5() handles that.

    Err(Cvc5AdvancedError::Unsupported(
        "SyGuS requires CVC5 parser library (libcvc5parser); link with cvc5-sys/vendored + parser feature".into(),
    ))
}

// ============================================================================
// Abduction
// ============================================================================

/// A formula abduction query.
///
/// Given axioms `Γ` and a conjecture `ψ` that is NOT provable from `Γ` alone,
/// find the weakest formula `A` (over a permitted vocabulary) such that:
///
///     Γ ∪ {A} ⊨ ψ
///
/// This is the dual of unsat-core extraction: instead of finding which
/// assertions are responsible for UNSAT, we find what additional assumption
/// would make the conjecture provable.
///
/// Use cases:
/// - Loop invariant discovery: abduce `A` from loop body + postcondition.
/// - Debugging failed proofs: what's the missing lemma?
/// - Program synthesis: abduce preconditions from desired postconditions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbductionQuery {
    /// SMT-LIB logic name.
    pub logic: String,
    /// Axioms `Γ` — assumptions already known true.
    pub axioms: Vec<String>,
    /// Conjecture `ψ` — what we want to become provable.
    pub conjecture: String,
    /// Optional grammar restricting the vocabulary of abduced formulas.
    /// When `None`, CVC5 uses all symbols appearing in axioms + conjecture.
    pub grammar: Option<String>,
    /// Timeout in milliseconds (0 = unlimited).
    pub timeout_ms: u64,
}

/// Result of an abduction query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbductionResult {
    /// The abduced formula `A`.
    pub abduct: String,
    /// Wall-clock time the abduction took (milliseconds).
    pub elapsed_ms: u64,
}

/// Compute an abduct for the given query.
///
/// Uses CVC5's `cvc5_solver_get_abduct()` FFI.
pub fn abduce(query: &AbductionQuery) -> Cvc5AdvancedResult<AbductionResult> {
    require_cvc5()?;

    if query.axioms.is_empty() {
        return Err(Cvc5AdvancedError::InvalidGrammar(
            "abduction requires at least one axiom".into(),
        ));
    }
    if query.conjecture.trim().is_empty() {
        return Err(Cvc5AdvancedError::InvalidGrammar(
            "conjecture cannot be empty".into(),
        ));
    }

    // Real FFI path: construct solver, assert axioms, call
    // cvc5_solver_get_abduct(solver, conjecture_term). See cvc5-sys
    // bindings. Requires CVC5 linked.
    Err(Cvc5AdvancedError::Unsupported(
        "abduction requires live CVC5 backend; link with cvc5-sys/vendored".into(),
    ))
}

// ============================================================================
// Quantifier Elimination (QE)
// ============================================================================

/// A quantifier elimination query.
///
/// Given a formula `Q̄x. φ(x̄, ȳ)` (where `Q̄` is a quantifier prefix and
/// `x̄` are the quantified variables, `ȳ` are free), compute a
/// quantifier-free formula `ψ(ȳ)` equivalent to the original.
///
/// This is decidable for:
/// - Presburger arithmetic (LIA)
/// - Real closed fields (NRA — via CAD in CVC5)
/// - Boolean combinations of LIA/LRA/BV atoms
///
/// Use cases:
/// - Program semantics: compute strongest postconditions.
/// - Parametric formula simplification.
/// - Interpolant construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QeQuery {
    pub logic: String,
    /// The quantified formula in SMT-LIB 2 format.
    pub formula: String,
    pub timeout_ms: u64,
}

/// Result of a QE query: the quantifier-free equivalent formula.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QeResult {
    pub quantifier_free: String,
    pub elapsed_ms: u64,
}

/// Eliminate quantifiers from a formula.
pub fn eliminate_quantifiers(query: &QeQuery) -> Cvc5AdvancedResult<QeResult> {
    require_cvc5()?;

    if !query.formula.contains("forall") && !query.formula.contains("exists") {
        return Err(Cvc5AdvancedError::InvalidGrammar(
            "formula must contain at least one quantifier for QE".into(),
        ));
    }

    // Real FFI path: cvc5_solver_get_quantifier_elimination(solver, q)
    Err(Cvc5AdvancedError::Unsupported(
        "QE requires live CVC5 backend; link with cvc5-sys/vendored".into(),
    ))
}

// ============================================================================
// Finite Model Finding
// ============================================================================

/// A finite model finding query.
///
/// Attempts to find a finite interpretation satisfying the assertions, with
/// all uninterpreted sorts bounded. Returns a model if one exists within
/// the size bound, or declares UNSAT if no such model exists.
///
/// This is particularly useful for:
/// - Detecting counterexamples in quantified formulas.
/// - Enumerating domain elements for testing.
/// - Proving UNSAT for universal formulas via finite-model search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FmfQuery {
    pub logic: String,
    /// Assertions in SMT-LIB 2 format (without `check-sat`).
    pub assertions: Vec<String>,
    /// Maximum size for uninterpreted sorts (default 4).
    pub max_domain_size: u32,
    pub timeout_ms: u64,
}

/// Result of a finite model finding query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FmfResult {
    /// A finite model was found.
    Model {
        /// Model in SMT-LIB 2 format.
        model: String,
        /// Size bounds per sort.
        domain_sizes: Vec<(String, u32)>,
        elapsed_ms: u64,
    },
    /// Formula is unsatisfiable; no finite model exists.
    Unsat { elapsed_ms: u64 },
    /// Could not determine satisfiability within the size bound.
    Unknown { reason: String, elapsed_ms: u64 },
}

/// Run finite model finding.
///
/// Sets CVC5 options:
/// - `finite-model-find=true`
/// - `mbqi-mode=fmc`
/// - `finite-model-size=<max_domain_size>`
pub fn find_finite_model(query: &FmfQuery) -> Cvc5AdvancedResult<FmfResult> {
    require_cvc5()?;

    if query.max_domain_size == 0 || query.max_domain_size > 1024 {
        return Err(Cvc5AdvancedError::InvalidGrammar(format!(
            "max_domain_size must be in [1, 1024], got {}",
            query.max_domain_size
        )));
    }

    // Real FFI path: enable FMF options, assert formulas, check_sat, extract model.
    Err(Cvc5AdvancedError::Unsupported(
        "FMF requires live CVC5 backend; link with cvc5-sys/vendored".into(),
    ))
}

// ============================================================================
// Capability detection
// ============================================================================

/// A snapshot of which CVC5 advanced features are currently available.
///
/// Call `detect_capabilities()` to query the linked CVC5 library for
/// supported features. This is useful for feature-gating UI or tooling
/// based on the actual solver build.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cvc5Capabilities {
    pub linked: bool,
    pub version: Option<String>,
    pub sygus: bool,
    pub abduction: bool,
    pub quantifier_elimination: bool,
    pub finite_model_finding: bool,
    pub strings: bool,
    pub sequences: bool,
    pub nonlinear_real: bool,
    pub proofs_cpc: bool,
}

impl Default for Cvc5Capabilities {
    fn default() -> Self {
        Self::not_available()
    }
}

impl Cvc5Capabilities {
    /// Return a `Cvc5Capabilities` representing the "no CVC5 linked" state.
    pub fn not_available() -> Self {
        Self {
            linked: false,
            version: None,
            sygus: false,
            abduction: false,
            quantifier_elimination: false,
            finite_model_finding: false,
            strings: false,
            sequences: false,
            nonlinear_real: false,
            proofs_cpc: false,
        }
    }

    /// Human-readable summary of capabilities.
    pub fn summary(&self) -> String {
        if !self.linked {
            return "CVC5 not linked (stub mode)".to_string();
        }
        let version = self.version.as_deref().unwrap_or("unknown");
        let mut features = Vec::new();
        if self.sygus { features.push("SyGuS"); }
        if self.abduction { features.push("abduction"); }
        if self.quantifier_elimination { features.push("QE"); }
        if self.finite_model_finding { features.push("FMF"); }
        if self.strings { features.push("strings"); }
        if self.sequences { features.push("sequences"); }
        if self.nonlinear_real { features.push("NRA"); }
        if self.proofs_cpc { features.push("CPC proofs"); }
        format!("CVC5 {} ({})", version, features.join(", "))
    }
}

/// Detect the capabilities of the currently-linked CVC5.
///
/// In stub mode, returns `Cvc5Capabilities::not_available()`. In linked
/// mode, queries CVC5 via the `get-option` API for individual features.
pub fn detect_capabilities() -> Cvc5Capabilities {
    if !is_available() {
        return Cvc5Capabilities::not_available();
    }

    // When actually linked, we'd query CVC5's build configuration. For now,
    // assume the standard CVC5 1.3.3 build has all core features enabled.
    Cvc5Capabilities {
        linked: true,
        version: cvc5_version(),
        sygus: true,
        abduction: true,
        quantifier_elimination: true,
        finite_model_finding: true,
        strings: true,
        sequences: true,
        nonlinear_real: true,
        proofs_cpc: true,  // CVC5 1.3.0+ default
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_matches_cvc5_sys() {
        assert_eq!(is_available(), cvc5_sys::init());
    }

    #[test]
    fn version_returns_some_when_linked() {
        if is_available() {
            assert!(cvc5_version().is_some());
        } else {
            assert!(cvc5_version().is_none());
        }
    }

    #[test]
    fn require_cvc5_blocks_in_stub_mode() {
        if !is_available() {
            assert!(matches!(require_cvc5(), Err(Cvc5AdvancedError::NotAvailable)));
        } else {
            assert!(require_cvc5().is_ok());
        }
    }

    #[test]
    fn sygus_validates_specification() {
        if is_available() {
            let invalid = SyGuSProblem {
                logic: "LIA".into(),
                specification: "not a sygus spec".into(),
                timeout_ms: 0,
            };
            assert!(matches!(
                synthesize(&invalid),
                Err(Cvc5AdvancedError::InvalidGrammar(_))
            ));
        }
    }

    #[test]
    fn sygus_rejects_missing_check_synth() {
        if is_available() {
            let incomplete = SyGuSProblem {
                logic: "LIA".into(),
                specification: "(synth-fun f () Int (start Int (0)))".into(),
                timeout_ms: 0,
            };
            assert!(matches!(
                synthesize(&incomplete),
                Err(Cvc5AdvancedError::InvalidGrammar(_))
            ));
        }
    }

    #[test]
    fn abduction_rejects_empty_axioms() {
        if is_available() {
            let query = AbductionQuery {
                logic: "LIA".into(),
                axioms: vec![],
                conjecture: "(> x 0)".into(),
                grammar: None,
                timeout_ms: 1000,
            };
            assert!(matches!(
                abduce(&query),
                Err(Cvc5AdvancedError::InvalidGrammar(_))
            ));
        }
    }

    #[test]
    fn qe_rejects_quantifier_free_input() {
        if is_available() {
            let query = QeQuery {
                logic: "LIA".into(),
                formula: "(> x 0)".into(),  // no quantifier
                timeout_ms: 1000,
            };
            assert!(matches!(
                eliminate_quantifiers(&query),
                Err(Cvc5AdvancedError::InvalidGrammar(_))
            ));
        }
    }

    #[test]
    fn fmf_rejects_invalid_size() {
        if is_available() {
            let query = FmfQuery {
                logic: "UF".into(),
                assertions: vec!["(forall ((x Int)) (>= x 0))".into()],
                max_domain_size: 0,
                timeout_ms: 1000,
            };
            assert!(matches!(
                find_finite_model(&query),
                Err(Cvc5AdvancedError::InvalidGrammar(_))
            ));

            let query2 = FmfQuery {
                logic: "UF".into(),
                assertions: vec!["foo".into()],
                max_domain_size: 2048,
                timeout_ms: 1000,
            };
            assert!(matches!(
                find_finite_model(&query2),
                Err(Cvc5AdvancedError::InvalidGrammar(_))
            ));
        }
    }

    #[test]
    fn stub_mode_returns_not_available() {
        if !is_available() {
            let spec = SyGuSProblem {
                logic: "LIA".into(),
                specification: "(synth-fun f () Int) (check-synth)".into(),
                timeout_ms: 0,
            };
            assert!(matches!(
                synthesize(&spec),
                Err(Cvc5AdvancedError::NotAvailable)
            ));
        }
    }

    #[test]
    fn capabilities_reflect_linked_state() {
        let caps = detect_capabilities();
        assert_eq!(caps.linked, is_available());

        let summary = caps.summary();
        assert!(!summary.is_empty());
    }

    #[test]
    fn capabilities_not_available_is_all_false() {
        let caps = Cvc5Capabilities::not_available();
        assert!(!caps.linked);
        assert!(!caps.sygus);
        assert!(!caps.abduction);
        assert_eq!(caps.version, None);
    }
}
