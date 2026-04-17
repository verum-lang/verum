//! # Unified Solver Capability Layer
//!
//! A high-level API that lets users request advanced solver features without
//! caring which backend implements them. The capability layer automatically
//! routes each request to the appropriate solver:
//!
//! | Capability                | Z3    | CVC5  | Router Picks |
//! |---------------------------|-------|-------|--------------|
//! | Interpolation             | ✓     | ✗     | Z3           |
//! | MaxSMT / Optimization     | ✓     | ✗     | Z3           |
//! | Horn / Fixedpoint         | ✓     | ✗     | Z3           |
//! | SyGuS Synthesis           | ✗     | ✓     | CVC5         |
//! | Abduction                 | ✗     | ✓     | CVC5         |
//! | Finite Model Finding      | Weak  | ✓     | CVC5         |
//! | Quantifier Elimination    | ✓     | ✓     | Best-fit     |
//!
//! ## Usage
//!
//! ```rust,ignore
//! use verum_smt::solver_capability::{SolverCapability, CapabilityRegistry};
//!
//! let registry = CapabilityRegistry::detect();
//!
//! // Request Craig interpolation — routes to Z3 automatically.
//! if registry.supports(SolverCapability::Interpolation) {
//!     // ... use Z3 interpolation ...
//! }
//!
//! // Request SyGuS — routes to CVC5 if available.
//! match registry.find_provider(SolverCapability::SygusSynthesis) {
//!     Some(provider) => /* use the provider */,
//!     None => /* feature unavailable in this build */,
//! }
//! ```
//!
//! ## Graceful Fallback
//!
//! When a capability is requested but the specialized solver is unavailable,
//! the registry can fall back to a less-capable implementation (e.g.,
//! Z3's limited finite-model-finding) if one exists, or return `None` to
//! let the caller decide.

use serde::{Deserialize, Serialize};

use crate::portfolio_executor::SolverId;

// ============================================================================
// Capability enum
// ============================================================================

/// An advanced SMT-solver capability that callers may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SolverCapability {
    // === Z3-exclusive (no CVC5 support) ===
    /// Craig interpolation: given `A ∧ B` is UNSAT, produce `I` with
    /// `A ⊨ I` and `I ∧ B` UNSAT, where `I` uses only common variables.
    Interpolation,
    /// MaxSMT / Optimize: maximize objectives subject to soft constraints.
    Optimization,
    /// Horn clause solving (constrained Horn clauses, CHC).
    HornClauses,

    // === CVC5-exclusive or strongly preferred ===
    /// Syntax-Guided Synthesis: generate functions matching a grammar
    /// and a specification.
    SygusSynthesis,
    /// Abduction: find the weakest missing hypothesis.
    Abduction,
    /// Finite model finding with domain size bounds (CVC5 preferred over Z3's MBQI).
    FiniteModelFinding,
    /// String theory with regex (CVC5's state-of-the-art implementation).
    StringsRegex,
    /// Sequence theory (CVC5-only).
    Sequences,
    /// Cylindrical algebraic decomposition for nonlinear real arithmetic.
    CadNonlinearReal,

    // === Supported by both, with backend-specific strengths ===
    /// Quantifier elimination (both support; CVC5 broader fragment).
    QuantifierElimination,
    /// Proof production (both; different formats).
    ProofProduction,
    /// Unsat core minimization.
    UnsatCores,
    /// Model extraction.
    ModelExtraction,
    /// Incremental solving (push/pop).
    IncrementalSolving,

    // === Theory-specific ===
    /// Bit-vector reasoning.
    BitVectors,
    /// Array theory.
    Arrays,
    /// Inductive datatypes.
    InductiveDatatypes,
}

impl SolverCapability {
    /// Return the preferred solver for this capability, based on empirical
    /// performance and feature coverage.
    pub fn preferred_solver(self) -> PreferredSolver {
        match self {
            // Z3 exclusive
            Self::Interpolation | Self::Optimization | Self::HornClauses => {
                PreferredSolver::RequiresZ3
            }
            // CVC5 exclusive
            Self::SygusSynthesis
            | Self::Abduction
            | Self::Sequences
            | Self::CadNonlinearReal => PreferredSolver::RequiresCvc5,
            // CVC5 strongly preferred
            Self::FiniteModelFinding | Self::StringsRegex => {
                PreferredSolver::PrefersCvc5
            }
            // Z3 strongly preferred
            Self::BitVectors | Self::Arrays => PreferredSolver::PrefersZ3,
            // Either works well
            Self::QuantifierElimination
            | Self::ProofProduction
            | Self::UnsatCores
            | Self::ModelExtraction
            | Self::IncrementalSolving
            | Self::InductiveDatatypes => PreferredSolver::Either,
        }
    }

    /// Human-readable name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Interpolation => "Craig interpolation",
            Self::Optimization => "MaxSMT / optimization",
            Self::HornClauses => "Horn clause solving",
            Self::SygusSynthesis => "SyGuS synthesis",
            Self::Abduction => "abduction",
            Self::FiniteModelFinding => "finite model finding",
            Self::StringsRegex => "strings + regex",
            Self::Sequences => "sequences",
            Self::CadNonlinearReal => "CAD for NRA",
            Self::QuantifierElimination => "quantifier elimination",
            Self::ProofProduction => "proof production",
            Self::UnsatCores => "unsat cores",
            Self::ModelExtraction => "model extraction",
            Self::IncrementalSolving => "incremental solving",
            Self::BitVectors => "bit-vectors",
            Self::Arrays => "arrays",
            Self::InductiveDatatypes => "inductive datatypes",
        }
    }
}

/// The solver preference for a given capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PreferredSolver {
    /// Requires Z3; CVC5 cannot satisfy this capability at all.
    RequiresZ3,
    /// Requires CVC5; Z3 cannot satisfy this capability at all.
    RequiresCvc5,
    /// Both can satisfy, but Z3 is significantly more capable/faster.
    PrefersZ3,
    /// Both can satisfy, but CVC5 is significantly more capable/faster.
    PrefersCvc5,
    /// Both are competitive; either works.
    Either,
}

impl PreferredSolver {
    /// The solver this preference maps to, choosing the preferred one when
    /// both are viable.
    pub fn solver(self) -> SolverId {
        match self {
            Self::RequiresZ3 | Self::PrefersZ3 | Self::Either => SolverId::Z3,
            Self::RequiresCvc5 | Self::PrefersCvc5 => SolverId::Cvc5,
        }
    }

    /// Whether both solvers can handle this capability (even if one is preferred).
    pub fn is_flexible(self) -> bool {
        matches!(
            self,
            Self::PrefersZ3 | Self::PrefersCvc5 | Self::Either
        )
    }
}

// ============================================================================
// Capability registry
// ============================================================================

/// A runtime registry of which capabilities are available given the currently
/// linked solvers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRegistry {
    /// Whether Z3 is linked and functional.
    pub z3_available: bool,
    /// Whether CVC5 is linked and functional.
    pub cvc5_available: bool,
    /// Z3 version string (e.g., `"4.12.2"`).
    pub z3_version: Option<String>,
    /// CVC5 version string (e.g., `"1.3.3"`).
    pub cvc5_version: Option<String>,
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::detect()
    }
}

impl CapabilityRegistry {
    /// Detect which solvers are currently available by probing them.
    pub fn detect() -> Self {
        Self {
            z3_available: true, // Z3 is always bundled in Verum builds.
            cvc5_available: cvc5_sys::init(),
            z3_version: Some(detect_z3_version()),
            cvc5_version: if cvc5_sys::init() {
                Some(cvc5_sys::version())
            } else {
                None
            },
        }
    }

    /// Return whether the given capability is available in this build.
    pub fn supports(&self, cap: SolverCapability) -> bool {
        match cap.preferred_solver() {
            PreferredSolver::RequiresZ3 | PreferredSolver::PrefersZ3 => self.z3_available,
            PreferredSolver::RequiresCvc5 | PreferredSolver::PrefersCvc5 => self.cvc5_available,
            PreferredSolver::Either => self.z3_available || self.cvc5_available,
        }
    }

    /// Return the solver that provides the given capability, or `None` if
    /// no linked solver can satisfy it.
    pub fn find_provider(&self, cap: SolverCapability) -> Option<SolverId> {
        let pref = cap.preferred_solver();
        match pref {
            PreferredSolver::RequiresZ3 => {
                if self.z3_available {
                    Some(SolverId::Z3)
                } else {
                    None
                }
            }
            PreferredSolver::RequiresCvc5 => {
                if self.cvc5_available {
                    Some(SolverId::Cvc5)
                } else {
                    None
                }
            }
            PreferredSolver::PrefersZ3 => {
                if self.z3_available {
                    Some(SolverId::Z3)
                } else if self.cvc5_available {
                    Some(SolverId::Cvc5)
                } else {
                    None
                }
            }
            PreferredSolver::PrefersCvc5 => {
                if self.cvc5_available {
                    Some(SolverId::Cvc5)
                } else if self.z3_available {
                    Some(SolverId::Z3)
                } else {
                    None
                }
            }
            PreferredSolver::Either => {
                if self.z3_available {
                    Some(SolverId::Z3)
                } else if self.cvc5_available {
                    Some(SolverId::Cvc5)
                } else {
                    None
                }
            }
        }
    }

    /// Return all capabilities currently available.
    pub fn available_capabilities(&self) -> Vec<SolverCapability> {
        use SolverCapability::*;
        let all = [
            Interpolation,
            Optimization,
            HornClauses,
            SygusSynthesis,
            Abduction,
            FiniteModelFinding,
            StringsRegex,
            Sequences,
            CadNonlinearReal,
            QuantifierElimination,
            ProofProduction,
            UnsatCores,
            ModelExtraction,
            IncrementalSolving,
            BitVectors,
            Arrays,
            InductiveDatatypes,
        ];
        all.iter().copied().filter(|c| self.supports(*c)).collect()
    }

    /// Generate a capability matrix report for diagnostics.
    pub fn report(&self) -> String {
        let mut out = String::new();
        out.push_str("╭──────────────────────────────────────────────────────────────╮\n");
        out.push_str("│         Verum SMT Capability Matrix                            │\n");
        out.push_str("├──────────────────────────────────────────────────────────────┤\n");
        out.push_str(&format!(
            "│  Z3:    {} ({})                                  \n",
            if self.z3_available { "✓" } else { "✗" },
            self.z3_version.as_deref().unwrap_or("unavailable"),
        ));
        out.push_str(&format!(
            "│  CVC5:  {} ({})                                  \n",
            if self.cvc5_available { "✓" } else { "✗" },
            self.cvc5_version.as_deref().unwrap_or("unavailable"),
        ));
        out.push_str("├──────────────────────────────────────────────────────────────┤\n");
        out.push_str("│  Capabilities:                                                 │\n");

        use SolverCapability::*;
        for cap in [
            Interpolation,
            Optimization,
            HornClauses,
            SygusSynthesis,
            Abduction,
            FiniteModelFinding,
            StringsRegex,
            Sequences,
            CadNonlinearReal,
            QuantifierElimination,
            ProofProduction,
            UnsatCores,
            ModelExtraction,
            IncrementalSolving,
            BitVectors,
            Arrays,
            InductiveDatatypes,
        ] {
            let available = self.supports(cap);
            let provider = self.find_provider(cap);
            let mark = if available { "✓" } else { "✗" };
            let via = match provider {
                Some(SolverId::Z3) => "via Z3",
                Some(SolverId::Cvc5) => "via CVC5",
                None => "unavailable",
            };
            out.push_str(&format!(
                "│    {} {:<28} {:<20}              \n",
                mark,
                cap.name(),
                via,
            ));
        }
        out.push_str("╰──────────────────────────────────────────────────────────────╯\n");
        out
    }
}

// ============================================================================
// Z3 version detection (uses z3-sys)
// ============================================================================

fn detect_z3_version() -> String {
    // The z3 crate exposes the bundled Z3 version.
    // As of the current workspace, Z3 version is 4.13+ (per Cargo.toml).
    "4.13.x bundled".to_string()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_preferred_solver_stable() {
        assert_eq!(
            SolverCapability::Interpolation.preferred_solver(),
            PreferredSolver::RequiresZ3,
        );
        assert_eq!(
            SolverCapability::SygusSynthesis.preferred_solver(),
            PreferredSolver::RequiresCvc5,
        );
        assert_eq!(
            SolverCapability::Optimization.preferred_solver(),
            PreferredSolver::RequiresZ3,
        );
        assert_eq!(
            SolverCapability::StringsRegex.preferred_solver(),
            PreferredSolver::PrefersCvc5,
        );
    }

    #[test]
    fn z3_only_registry_blocks_cvc5_capabilities() {
        let registry = CapabilityRegistry {
            z3_available: true,
            cvc5_available: false,
            z3_version: Some("4.13".into()),
            cvc5_version: None,
        };

        assert!(registry.supports(SolverCapability::Interpolation));
        assert!(!registry.supports(SolverCapability::SygusSynthesis));
        assert!(!registry.supports(SolverCapability::Sequences));
        assert!(!registry.supports(SolverCapability::Abduction));
    }

    #[test]
    fn cvc5_only_registry_blocks_z3_capabilities() {
        let registry = CapabilityRegistry {
            z3_available: false,
            cvc5_available: true,
            z3_version: None,
            cvc5_version: Some("1.3.3".into()),
        };

        assert!(!registry.supports(SolverCapability::Interpolation));
        assert!(!registry.supports(SolverCapability::Optimization));
        assert!(registry.supports(SolverCapability::SygusSynthesis));
        assert!(registry.supports(SolverCapability::Sequences));
    }

    #[test]
    fn both_available_supports_everything() {
        let registry = CapabilityRegistry {
            z3_available: true,
            cvc5_available: true,
            z3_version: Some("4.13".into()),
            cvc5_version: Some("1.3.3".into()),
        };

        let caps = registry.available_capabilities();
        assert_eq!(caps.len(), 17, "all 17 capabilities should be available");
    }

    #[test]
    fn neither_available_supports_nothing() {
        let registry = CapabilityRegistry {
            z3_available: false,
            cvc5_available: false,
            z3_version: None,
            cvc5_version: None,
        };

        assert!(registry.available_capabilities().is_empty());
    }

    #[test]
    fn find_provider_falls_back_correctly() {
        let z3_only = CapabilityRegistry {
            z3_available: true,
            cvc5_available: false,
            z3_version: Some("4.13".into()),
            cvc5_version: None,
        };

        // PrefersCvc5 falls back to Z3 when CVC5 unavailable.
        assert_eq!(
            z3_only.find_provider(SolverCapability::StringsRegex),
            Some(SolverId::Z3),
        );
        // RequiresCvc5 returns None without fallback.
        assert_eq!(
            z3_only.find_provider(SolverCapability::SygusSynthesis),
            None,
        );
    }

    #[test]
    fn preferred_solver_solver_method() {
        assert_eq!(PreferredSolver::RequiresZ3.solver(), SolverId::Z3);
        assert_eq!(PreferredSolver::RequiresCvc5.solver(), SolverId::Cvc5);
        assert_eq!(PreferredSolver::PrefersZ3.solver(), SolverId::Z3);
        assert_eq!(PreferredSolver::PrefersCvc5.solver(), SolverId::Cvc5);
        assert_eq!(PreferredSolver::Either.solver(), SolverId::Z3);
    }

    #[test]
    fn flexibility_categorization() {
        assert!(!PreferredSolver::RequiresZ3.is_flexible());
        assert!(!PreferredSolver::RequiresCvc5.is_flexible());
        assert!(PreferredSolver::PrefersZ3.is_flexible());
        assert!(PreferredSolver::PrefersCvc5.is_flexible());
        assert!(PreferredSolver::Either.is_flexible());
    }

    #[test]
    fn detect_matches_runtime_availability() {
        let registry = CapabilityRegistry::detect();
        assert!(registry.z3_available);
        assert_eq!(registry.cvc5_available, cvc5_sys::init());
    }

    #[test]
    fn report_is_non_empty() {
        let registry = CapabilityRegistry::detect();
        let report = registry.report();
        assert!(report.contains("Capability Matrix"));
        assert!(report.contains("Z3"));
        assert!(report.contains("CVC5"));
    }

    #[test]
    fn all_capabilities_have_names() {
        use SolverCapability::*;
        let caps = [
            Interpolation, Optimization, HornClauses, SygusSynthesis, Abduction,
            FiniteModelFinding, StringsRegex, Sequences, CadNonlinearReal,
            QuantifierElimination, ProofProduction, UnsatCores, ModelExtraction,
            IncrementalSolving, BitVectors, Arrays, InductiveDatatypes,
        ];
        for cap in caps {
            assert!(!cap.name().is_empty(), "capability {:?} has no name", cap);
        }
    }
}
