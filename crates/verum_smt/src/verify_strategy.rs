//! # Verify Strategy Extraction
//!
//! Translates the `@verify(...)` attribute argument from a Verum function/type
//! declaration into a concrete SMT backend dispatch strategy.
//!
//! ## Grammar (verum.ebnf)
//!
//! ```ebnf
//! verify_attribute = 'verify' , '(' ,
//!     ( 'runtime' | 'static' | 'formal' | 'proof'
//!     | 'z3' | 'cvc5'
//!     | 'portfolio' | 'cross_validate' | 'certified' ) ,
//!     ')' ;
//! ```
//!
//! ## Semantics
//!
//! | Attribute Value    | BackendChoice       | Notes                                    |
//! |--------------------|---------------------|------------------------------------------|
//! | `runtime`          | (not SMT)           | Runtime assertion only, no SMT           |
//! | `static`           | (not SMT)           | Type-level check only, no SMT            |
//! | `formal`           | `Capability`        | Use the capability router (recommended)  |
//! | `proof`            | `Capability`        | Alias of formal                          |
//! | `z3`               | `Z3`                | Force Z3 backend                         |
//! | `cvc5`             | `Cvc5`              | Force CVC5 backend                       |
//! | `portfolio`        | `Portfolio`         | Parallel Z3+CVC5, first-wins             |
//! | `cross_validate`   | `Capability` + flag | Router will cross-validate this goal     |
//! | `certified`        | `Capability` + flag | Alias of cross_validate                  |
//!
//! `cross_validate` and `certified` set the `is_security_critical` flag on
//! the `ExtendedCharacteristics`, causing the router to dispatch the goal
//! to `SolverChoice::CrossValidate`.
//!
//! ## Usage
//!
//! Callers typically invoke `VerifyStrategy::from_attribute_value()` with
//! the attribute argument string, then use the returned strategy to
//! configure `BackendSwitcher` or mark a goal as security-critical.

use serde::{Deserialize, Serialize};

#[cfg(feature = "cvc5")]
use crate::backend_switcher::BackendChoice;

/// The dispatch strategy extracted from a `@verify(...)` attribute.
///
/// Beyond mapping to a `BackendChoice`, this also carries metadata about
/// whether the goal should be marked security-critical (triggering cross-
/// validation) and whether SMT verification is required at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifyStrategy {
    /// `@verify(runtime)` — insert runtime assertion, skip SMT.
    Runtime,
    /// `@verify(static)` — type-level static verification only.
    Static,
    /// `@verify(formal)` or `@verify(proof)` — route via capability-based dispatcher.
    Formal,
    /// `@verify(z3)` — force Z3 regardless of theory.
    ForceZ3,
    /// `@verify(cvc5)` — force CVC5 regardless of theory.
    ForceCvc5,
    /// `@verify(portfolio)` — run both solvers in parallel, first-wins.
    Portfolio,
    /// `@verify(cross_validate)` or `@verify(certified)` — both must agree.
    CrossValidate,
}

impl VerifyStrategy {
    /// Parse a verify-attribute argument string into a strategy.
    ///
    /// Returns `None` for unrecognized values. Case-insensitive match.
    pub fn from_attribute_value(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "runtime" => Some(Self::Runtime),
            "static" => Some(Self::Static),
            "formal" | "proof" => Some(Self::Formal),
            "z3" => Some(Self::ForceZ3),
            "cvc5" => Some(Self::ForceCvc5),
            "portfolio" => Some(Self::Portfolio),
            "cross_validate" | "cross-validate" | "crossvalidate" | "certified" => {
                Some(Self::CrossValidate)
            }
            _ => None,
        }
    }

    /// Render back to the canonical attribute-value form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Static => "static",
            Self::Formal => "formal",
            Self::ForceZ3 => "z3",
            Self::ForceCvc5 => "cvc5",
            Self::Portfolio => "portfolio",
            Self::CrossValidate => "cross_validate",
        }
    }

    /// Map the strategy to a `BackendChoice` for the switcher.
    ///
    /// Returns `None` for `Runtime` and `Static`, which do NOT use SMT at all.
    #[cfg(feature = "cvc5")]
    pub fn to_backend_choice(&self) -> Option<BackendChoice> {
        match self {
            Self::Runtime | Self::Static => None,
            Self::Formal => Some(BackendChoice::Capability),
            Self::ForceZ3 => Some(BackendChoice::Z3),
            Self::ForceCvc5 => Some(BackendChoice::Cvc5),
            Self::Portfolio => Some(BackendChoice::Portfolio),
            Self::CrossValidate => Some(BackendChoice::Capability),
        }
    }

    /// True if the strategy requires marking the goal as security-critical.
    ///
    /// When true, the switcher will set `is_security_critical = true` on the
    /// `ExtendedCharacteristics`, causing the router to dispatch to
    /// `SolverChoice::CrossValidate`.
    pub fn requires_cross_validation(&self) -> bool {
        matches!(self, Self::CrossValidate)
    }

    /// True if the strategy requires SMT verification (as opposed to
    /// runtime-only or static-only checks).
    pub fn requires_smt(&self) -> bool {
        !matches!(self, Self::Runtime | Self::Static)
    }

    /// True if the strategy forces a specific solver (disabling routing).
    pub fn is_forced(&self) -> bool {
        matches!(self, Self::ForceZ3 | Self::ForceCvc5)
    }
}

impl std::fmt::Display for VerifyStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for VerifyStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_attribute_value(s)
            .ok_or_else(|| format!("unknown verify strategy: {}", s))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_forms() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("runtime"),
            Some(VerifyStrategy::Runtime)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("formal"),
            Some(VerifyStrategy::Formal)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("portfolio"),
            Some(VerifyStrategy::Portfolio)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("cross_validate"),
            Some(VerifyStrategy::CrossValidate)
        );
    }

    #[test]
    fn parses_aliases() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("proof"),
            Some(VerifyStrategy::Formal)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("certified"),
            Some(VerifyStrategy::CrossValidate)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("cross-validate"),
            Some(VerifyStrategy::CrossValidate)
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("FORMAL"),
            Some(VerifyStrategy::Formal)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("Portfolio"),
            Some(VerifyStrategy::Portfolio)
        );
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(VerifyStrategy::from_attribute_value("unknown"), None);
        assert_eq!(VerifyStrategy::from_attribute_value(""), None);
    }

    #[test]
    fn backend_choice_mapping() {
        assert_eq!(VerifyStrategy::Runtime.to_backend_choice(), None);
        assert_eq!(VerifyStrategy::Static.to_backend_choice(), None);
        assert_eq!(
            VerifyStrategy::Formal.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
        assert_eq!(
            VerifyStrategy::ForceZ3.to_backend_choice(),
            Some(BackendChoice::Z3)
        );
        assert_eq!(
            VerifyStrategy::ForceCvc5.to_backend_choice(),
            Some(BackendChoice::Cvc5)
        );
        assert_eq!(
            VerifyStrategy::Portfolio.to_backend_choice(),
            Some(BackendChoice::Portfolio)
        );
        assert_eq!(
            VerifyStrategy::CrossValidate.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
    }

    #[test]
    fn cross_validation_flag() {
        assert!(!VerifyStrategy::Formal.requires_cross_validation());
        assert!(!VerifyStrategy::Portfolio.requires_cross_validation());
        assert!(VerifyStrategy::CrossValidate.requires_cross_validation());
    }

    #[test]
    fn smt_requirement() {
        assert!(!VerifyStrategy::Runtime.requires_smt());
        assert!(!VerifyStrategy::Static.requires_smt());
        assert!(VerifyStrategy::Formal.requires_smt());
        assert!(VerifyStrategy::Portfolio.requires_smt());
        assert!(VerifyStrategy::CrossValidate.requires_smt());
    }

    #[test]
    fn forced_detection() {
        assert!(VerifyStrategy::ForceZ3.is_forced());
        assert!(VerifyStrategy::ForceCvc5.is_forced());
        assert!(!VerifyStrategy::Formal.is_forced());
        assert!(!VerifyStrategy::Portfolio.is_forced());
    }

    #[test]
    fn roundtrip_via_display() {
        for strategy in [
            VerifyStrategy::Runtime,
            VerifyStrategy::Static,
            VerifyStrategy::Formal,
            VerifyStrategy::ForceZ3,
            VerifyStrategy::ForceCvc5,
            VerifyStrategy::Portfolio,
            VerifyStrategy::CrossValidate,
        ] {
            let s = strategy.as_str();
            let parsed = VerifyStrategy::from_attribute_value(s).unwrap();
            assert_eq!(parsed, strategy, "roundtrip failed for {:?}", strategy);
        }
    }

    #[test]
    fn from_str_errors_on_unknown() {
        use std::str::FromStr;
        assert!(VerifyStrategy::from_str("foo").is_err());
        assert!(VerifyStrategy::from_str("").is_err());
    }
}
