//! # Verify Strategy Extraction
//!
//! Translates the `@verify(...)` attribute argument from a Verum function/type
//! declaration into a concrete verification dispatch strategy.
//!
//! ## Design Principle: Solver Abstraction
//!
//! This module is the USER-FACING API for verification. It deliberately
//! exposes only **semantic strategies** (describing intent: "fast",
//! "thorough", "certified") — never specific solver backends. This keeps
//! user code independent of the underlying proof engine and lets the
//! compiler swap implementations (e.g., migrate from Z3+CVC5 to a custom
//! solver) without breaking any existing annotations.
//!
//! Backend selection happens internally based on:
//! - The strategy's intent (fast ↔ thorough tradeoff)
//! - The goal's theory signature (routed via `capability_router`)
//! - The set of linked solvers and their capabilities
//!
//! ## Grammar (verum.ebnf)
//!
//! ```ebnf
//! verify_attribute = 'verify' , '(' ,
//!     ( 'runtime' | 'static' | 'formal' | 'proof'
//!     | 'fast' | 'thorough' | 'reliable'
//!     | 'certified' | 'synthesize' ) ,
//!     ')' ;
//! ```
//!
//! ## Strategy Semantics
//!
//! | Attribute     | Intent                                          | Performance         |
//! |---------------|-------------------------------------------------|---------------------|
//! | `runtime`     | Runtime assertion (no formal proof)             | Fastest, unverified |
//! | `static`      | Static type-level check (no SMT)                | Fast, partial       |
//! | `formal`      | Formal verification with default strategy       | Balanced            |
//! | `proof`       | Alias of `formal`                               | Balanced            |
//! | `fast`        | Optimize for speed; may be incomplete on hard   | Fastest verify      |
//! | `thorough`    | Maximum completeness; race multiple strategies  | Slower, robust      |
//! | `reliable`    | Alias of `thorough`                             | Slower, robust      |
//! | `certified`   | Independent cross-verification; for certs       | Slowest, strongest  |
//! | `synthesize`  | Generate a term from a specification            | Variable            |
//!
//! ## Usage
//!
//! Callers invoke `VerifyStrategy::from_attribute_value()` with the attribute
//! argument string, then pass the returned strategy to `BackendSwitcher::
//! solve_with_strategy()`. The switcher translates semantic strategies into
//! appropriate internal backend dispatch.

use serde::{Deserialize, Serialize};

#[cfg(feature = "cvc5")]
use crate::backend_switcher::BackendChoice;

/// The semantic verification strategy from a `@verify(...)` attribute.
///
/// ## User-Facing API
///
/// This enum is the public interface for verification intent. Each variant
/// describes WHAT the user wants (speed, thoroughness, certification) —
/// NOT which specific solver or algorithm should be used. The compiler
/// maps these strategies to internal dispatch decisions.
///
/// ## Migration Stability
///
/// When the Verum compiler migrates to a custom in-house solver, existing
/// user annotations remain valid without modification. Only the internal
/// dispatch logic in `BackendSwitcher` changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifyStrategy {
    /// `@verify(runtime)` — insert runtime assertion, skip formal proof.
    Runtime,

    /// `@verify(static)` — type-level static verification only.
    Static,

    /// `@verify(formal)` or `@verify(proof)` — the default formal
    /// verification path. The compiler picks the best technique based on
    /// the goal's structure. Recommended for most cases.
    Formal,

    /// `@verify(fast)` — prioritize verification speed over completeness.
    ///
    /// Uses the fastest technique known to decide the goal's theory.
    /// May return "unknown" on difficult goals that a more thorough strategy
    /// would solve. Ideal for iterative development and tight feedback loops.
    Fast,

    /// `@verify(thorough)` or `@verify(reliable)` — prioritize completeness
    /// over speed.
    ///
    /// Runs multiple complementary techniques in parallel and accepts the
    /// first successful result. More reliable on hard goals but consumes
    /// more resources. Use for production-critical verification.
    Thorough,

    /// `@verify(certified)` — produce an independently verifiable proof
    /// certificate.
    ///
    /// Runs two independent verification techniques and requires them to
    /// agree on the result. Divergence is treated as a hard error (it
    /// indicates either a bug in a verifier or an encoding issue). Used
    /// for security-critical verification and for exporting proof artifacts
    /// to external tools (Coq / Lean / Dedukti / Metamath).
    Certified,

    /// `@verify(synthesize)` — treat the goal as a synthesis problem.
    ///
    /// Instead of just checking satisfiability, generate a term that
    /// satisfies the specification. Used for program synthesis, invariant
    /// generation, and precondition inference.
    Synthesize,
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
            "fast" | "quick" | "rapid" => Some(Self::Fast),
            "thorough" | "reliable" | "robust" => Some(Self::Thorough),
            "certified" | "cross_validate" | "cross-validate" | "crossvalidate" => {
                Some(Self::Certified)
            }
            "synthesize" | "synthesis" | "synth" => Some(Self::Synthesize),
            _ => None,
        }
    }

    /// Render back to the canonical attribute-value form.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Static => "static",
            Self::Formal => "formal",
            Self::Fast => "fast",
            Self::Thorough => "thorough",
            Self::Certified => "certified",
            Self::Synthesize => "synthesize",
        }
    }

    /// Map the strategy to an internal `BackendChoice` for the switcher.
    ///
    /// This is an INTERNAL mapping that callers outside the compiler should
    /// generally not use directly — prefer `BackendSwitcher::solve_with_strategy`.
    ///
    /// Returns `None` for strategies that don't require formal proof
    /// infrastructure (`Runtime`, `Static`).
    ///
    /// The mapping intent:
    /// - `Formal` → default capability routing (compiler picks best solver)
    /// - `Fast`   → capability routing + fast-timeout single-solver preference
    /// - `Thorough` → portfolio mode (parallel complementary strategies)
    /// - `Certified` → cross-validation (two independent techniques must agree)
    /// - `Synthesize` → synthesis backend (CVC5 SyGuS, future: custom)
    #[cfg(feature = "cvc5")]
    pub fn to_backend_choice(&self) -> Option<BackendChoice> {
        match self {
            Self::Runtime | Self::Static => None,
            // Default formal verification — router decides.
            Self::Formal => Some(BackendChoice::Capability),
            // Fast: same router but with stricter timeouts (handled separately
            // in solve_with_strategy — keep as Capability here).
            Self::Fast => Some(BackendChoice::Capability),
            // Thorough: portfolio mode.
            Self::Thorough => Some(BackendChoice::Portfolio),
            // Certified: capability router with security-critical flag so
            // the router dispatches to SolverChoice::CrossValidate.
            Self::Certified => Some(BackendChoice::Capability),
            // Synthesize: capability router — it routes to the synthesis-capable backend.
            Self::Synthesize => Some(BackendChoice::Capability),
        }
    }

    /// True if the strategy requires marking the goal as security-critical.
    ///
    /// When true, the goal is dispatched to cross-validation (both primary
    /// and secondary verification techniques must agree on the result).
    pub fn requires_cross_validation(&self) -> bool {
        matches!(self, Self::Certified)
    }

    /// True if the strategy requires formal verification infrastructure
    /// (as opposed to runtime or static checks).
    pub fn requires_smt(&self) -> bool {
        !matches!(self, Self::Runtime | Self::Static)
    }

    /// True if the strategy is a synthesis problem (generates a term from
    /// a specification) rather than a decision problem (checks satisfiability).
    pub fn is_synthesis(&self) -> bool {
        matches!(self, Self::Synthesize)
    }

    /// True if the strategy prefers thorough/robust verification over speed.
    pub fn prefers_thoroughness(&self) -> bool {
        matches!(self, Self::Thorough | Self::Certified)
    }

    /// Recommended timeout multiplier for this strategy.
    ///
    /// Applied to the base timeout configured in the BackendSwitcher.
    /// Fast strategies get shorter timeouts, thorough strategies get longer.
    pub fn timeout_multiplier(&self) -> f64 {
        match self {
            Self::Runtime | Self::Static => 0.0, // no timeout needed
            Self::Fast => 0.3,       // 30% of base
            Self::Formal => 1.0,     // base
            Self::Thorough => 2.0,   // 2x base
            Self::Certified => 3.0,  // 3x base (two solvers)
            Self::Synthesize => 5.0, // 5x base (synthesis is hard)
        }
    }
}

impl std::fmt::Display for VerifyStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// AST attribute extraction
// ============================================================================

/// Extract the verify strategy from a Verum AST attribute list.
///
/// Scans for `@verify(...)` attributes and parses their argument. Returns:
/// - `Some(strategy)` if a `@verify(...)` attribute was found with a
///   recognized argument.
/// - `None` if no `@verify` attribute is present OR the argument is
///   unrecognized (caller should emit a diagnostic).
///
/// This is the primary entry point used by the compilation pipeline to
/// convert AST attributes into a concrete dispatch strategy.
///
/// ## Example usage (from compiler)
///
/// ```rust,ignore
/// use verum_smt::verify_strategy::{extract_from_attributes, VerifyStrategy};
///
/// match extract_from_attributes(&func.attributes) {
///     Some(strategy) => {
///         if strategy.requires_smt() {
///             let result = switcher.solve_with_strategy(&assertions, &strategy);
///             // ... handle result ...
///         }
///     }
///     None => {
///         // Use the compiler's default verification mode.
///     }
/// }
/// ```
pub fn extract_from_attributes(
    attributes: &verum_common::List<verum_ast::attr::Attribute>,
) -> Option<VerifyStrategy> {
    for attr in attributes.iter() {
        if attr.name.as_str() != "verify" {
            continue;
        }
        let args = attr.args.as_ref()?;
        for arg in args.iter() {
            if let Some(strat) = strategy_from_expr(arg) {
                return Some(strat);
            }
        }
    }
    None
}

/// Try to parse a VerifyStrategy from a single AST expression.
///
/// Recognizes:
/// - `ExprKind::Path` with a single identifier: `@verify(formal)`, `@verify(z3)`, etc.
/// - `ExprKind::Literal(Text(...))` for quoted forms: `@verify("portfolio")`.
fn strategy_from_expr(expr: &verum_ast::Expr) -> Option<VerifyStrategy> {
    use verum_ast::{ExprKind, LiteralKind};
    use verum_ast::ty::PathSegment;

    match &expr.kind {
        ExprKind::Path(path) => {
            if let Some(PathSegment::Name(ident)) = path.segments.last() {
                return VerifyStrategy::from_attribute_value(ident.name.as_str());
            }
            None
        }
        ExprKind::Literal(lit) => {
            if let LiteralKind::Text(text_lit) = &lit.kind {
                return VerifyStrategy::from_attribute_value(text_lit.as_str());
            }
            None
        }
        ExprKind::Paren(inner) => strategy_from_expr(inner),
        _ => None,
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
            VerifyStrategy::from_attribute_value("thorough"),
            Some(VerifyStrategy::Thorough)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("certified"),
            Some(VerifyStrategy::Certified)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("synthesize"),
            Some(VerifyStrategy::Synthesize)
        );
    }

    #[test]
    fn parses_aliases() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("proof"),
            Some(VerifyStrategy::Formal)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("reliable"),
            Some(VerifyStrategy::Thorough)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("robust"),
            Some(VerifyStrategy::Thorough)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("cross_validate"),
            Some(VerifyStrategy::Certified)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("fast"),
            Some(VerifyStrategy::Fast)
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("FORMAL"),
            Some(VerifyStrategy::Formal)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("Thorough"),
            Some(VerifyStrategy::Thorough)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("CERTIFIED"),
            Some(VerifyStrategy::Certified)
        );
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(VerifyStrategy::from_attribute_value("unknown"), None);
        assert_eq!(VerifyStrategy::from_attribute_value(""), None);
    }

    #[test]
    fn backend_choice_mapping() {
        // Non-SMT strategies → None.
        assert_eq!(VerifyStrategy::Runtime.to_backend_choice(), None);
        assert_eq!(VerifyStrategy::Static.to_backend_choice(), None);
        // Formal and its variants → capability routing.
        assert_eq!(
            VerifyStrategy::Formal.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
        assert_eq!(
            VerifyStrategy::Fast.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
        assert_eq!(
            VerifyStrategy::Certified.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
        assert_eq!(
            VerifyStrategy::Synthesize.to_backend_choice(),
            Some(BackendChoice::Capability)
        );
        // Thorough → portfolio.
        assert_eq!(
            VerifyStrategy::Thorough.to_backend_choice(),
            Some(BackendChoice::Portfolio)
        );
    }

    #[test]
    fn cross_validation_flag() {
        assert!(!VerifyStrategy::Formal.requires_cross_validation());
        assert!(!VerifyStrategy::Thorough.requires_cross_validation());
        assert!(VerifyStrategy::Certified.requires_cross_validation());
    }

    #[test]
    fn smt_requirement() {
        assert!(!VerifyStrategy::Runtime.requires_smt());
        assert!(!VerifyStrategy::Static.requires_smt());
        assert!(VerifyStrategy::Formal.requires_smt());
        assert!(VerifyStrategy::Thorough.requires_smt());
        assert!(VerifyStrategy::Certified.requires_smt());
    }

    #[test]
    fn synthesis_detection() {
        assert!(VerifyStrategy::Synthesize.is_synthesis());
        assert!(!VerifyStrategy::Formal.is_synthesis());
        assert!(!VerifyStrategy::Certified.is_synthesis());
    }

    #[test]
    fn thoroughness_preference() {
        assert!(VerifyStrategy::Thorough.prefers_thoroughness());
        assert!(VerifyStrategy::Certified.prefers_thoroughness());
        assert!(!VerifyStrategy::Fast.prefers_thoroughness());
        assert!(!VerifyStrategy::Formal.prefers_thoroughness());
    }

    #[test]
    fn timeout_multipliers_monotonic() {
        // Fast < Formal < Thorough < Certified < Synthesize
        assert!(VerifyStrategy::Fast.timeout_multiplier() < VerifyStrategy::Formal.timeout_multiplier());
        assert!(VerifyStrategy::Formal.timeout_multiplier() < VerifyStrategy::Thorough.timeout_multiplier());
        assert!(VerifyStrategy::Thorough.timeout_multiplier() < VerifyStrategy::Certified.timeout_multiplier());
        assert!(VerifyStrategy::Certified.timeout_multiplier() < VerifyStrategy::Synthesize.timeout_multiplier());
        // Runtime/Static have no timeout.
        assert_eq!(VerifyStrategy::Runtime.timeout_multiplier(), 0.0);
    }

    #[test]
    fn roundtrip_via_display() {
        for strategy in [
            VerifyStrategy::Runtime,
            VerifyStrategy::Static,
            VerifyStrategy::Formal,
            VerifyStrategy::Fast,
            VerifyStrategy::Thorough,
            VerifyStrategy::Certified,
            VerifyStrategy::Synthesize,
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

    // ========================================================================
    // Tests for attribute extraction from AST
    // ========================================================================

    use verum_ast::{Expr, ExprKind, Literal, LiteralKind, Span};
    use verum_ast::literal::StringLit;
    use verum_ast::ty::{Path, PathSegment};
    use verum_ast::attr::Attribute;
    use verum_common::{List, Text};

    fn make_path_expr(name: &str) -> Expr {
        let ident = verum_ast::Ident {
            name: Text::from(name),
            span: Span::default(),
        };
        let path = Path::new(
            List::from(vec![PathSegment::Name(ident)]),
            Span::default(),
        );
        Expr::new(ExprKind::Path(path), Span::default())
    }

    fn make_text_expr(value: &str) -> Expr {
        Expr::new(
            ExprKind::Literal(Literal {
                kind: LiteralKind::Text(StringLit::Regular(Text::from(value))),
                span: Span::default(),
            }),
            Span::default(),
        )
    }

    fn make_attr(name: &str, arg: Expr) -> Attribute {
        use verum_common::Maybe;
        Attribute::new(
            Text::from(name),
            Maybe::Some(List::from(vec![arg])),
            Span::default(),
        )
    }

    #[test]
    fn extract_from_path_identifier() {
        let attr = make_attr("verify", make_path_expr("formal"));
        let attrs = List::from(vec![attr]);
        assert_eq!(extract_from_attributes(&attrs), Some(VerifyStrategy::Formal));
    }

    #[test]
    fn extract_thorough_strategy() {
        let attr = make_attr("verify", make_path_expr("thorough"));
        let attrs = List::from(vec![attr]);
        assert_eq!(
            extract_from_attributes(&attrs),
            Some(VerifyStrategy::Thorough)
        );
    }

    #[test]
    fn extract_from_text_literal() {
        let attr = make_attr("verify", make_text_expr("cross_validate"));
        let attrs = List::from(vec![attr]);
        assert_eq!(
            extract_from_attributes(&attrs),
            Some(VerifyStrategy::Certified)
        );
    }

    #[test]
    fn extract_ignores_unrelated_attributes() {
        let attr = make_attr("derive", make_path_expr("Debug"));
        let attrs = List::from(vec![attr]);
        assert_eq!(extract_from_attributes(&attrs), None);
    }

    #[test]
    fn extract_returns_first_valid_match() {
        // If there are multiple @verify attributes (unusual), first wins.
        let attr1 = make_attr("verify", make_path_expr("fast"));
        let attr2 = make_attr("verify", make_path_expr("thorough"));
        let attrs = List::from(vec![attr1, attr2]);
        assert_eq!(extract_from_attributes(&attrs), Some(VerifyStrategy::Fast));
    }

    #[test]
    fn extract_rejects_solver_specific_attribute_values() {
        // Solver backend names (z3, cvc5) are NOT user-facing.
        let attr = make_attr("verify", make_path_expr("z3"));
        let attrs = List::from(vec![attr]);
        assert_eq!(extract_from_attributes(&attrs), None);

        let attr = make_attr("verify", make_path_expr("cvc5"));
        let attrs = List::from(vec![attr]);
        assert_eq!(extract_from_attributes(&attrs), None);
    }

    #[test]
    fn extract_returns_none_for_unknown_value() {
        let attr = make_attr("verify", make_path_expr("bogus"));
        let attrs = List::from(vec![attr]);
        assert_eq!(extract_from_attributes(&attrs), None);
    }

    #[test]
    fn extract_from_empty_attributes() {
        let attrs: List<Attribute> = List::new();
        assert_eq!(extract_from_attributes(&attrs), None);
    }
}
