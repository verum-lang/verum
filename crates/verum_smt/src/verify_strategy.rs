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
/// The nine-strategy verification ladder (VUVA §2.3, §12).
///
/// Each variant is SOUND; they differ in completeness and cost. The
/// ordering forms a monotone lift: a function that passes
/// `@verify(reliable)` also passes `@verify(formal)` / `@verify(fast)`
/// / `@verify(static)` / `@verify(runtime)`. `Synthesize` sits
/// orthogonally above — it's an inverse-search path, not a stricter
/// check. Each strategy is mapped to the Diakrisis ν-invariant via
/// [`VerifyStrategy::nu_ordinal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerifyStrategy {
    /// `@verify(runtime)` — emit runtime assertion; do not discharge
    /// at compile time. ν = 0.
    Runtime,

    /// `@verify(static)` — conservative static analysis (CBGR,
    /// dataflow, constant folding, bounds simplification).
    /// No SMT. ν < ω (finite step count).
    Static,

    /// `@verify(fast)` — single-solver SMT with bounded timeout
    /// (default 100ms). UNKNOWN → conservative accept (warning).
    /// ν < ω.
    Fast,

    /// `@verify(formal)` — portfolio SMT (Z3 + CVC5) with 5s timeout.
    /// UNKNOWN from any solver → conservative accept. ν = ω.
    Formal,

    /// `@verify(proof)` — user supplies a `proof { … }` tactic
    /// block; kernel rechecks. Unbounded user time but mechanically
    /// checked. ν = ω.
    Proof,

    /// `@verify(thorough)` — `formal` plus mandatory `decreases`,
    /// `invariant`, `frame` specifications. ≈2× formal cost.
    /// ν = ω·2.
    Thorough,

    /// `@verify(reliable)` — `thorough` plus Z3 AND CVC5 must both
    /// return UNSAT. Any disagreement → UNKNOWN. ≈2× thorough.
    /// ν = ω·2.
    Reliable,

    /// `@verify(certified)` — `reliable` plus certificate
    /// materialisation, kernel re-check, multi-format export.
    /// Any recheck failure → compile error. ≈3× thorough.
    /// ν = ω·2.
    Certified,

    /// `@verify(synthesize)` — inverse proof search across
    /// 𝔐 to fill missing lemmas / auxiliary theorems.
    /// Orthogonal to the monotone ladder. ν ≤ ω·3+1.
    Synthesize,
}

/// The Diakrisis ν-invariant ordinal assigned to a verification
/// strategy (VUVA §12 Table). Ordinals encoded as a compact enum
/// rather than a full ordinal calculus — the strata coarsely match
/// the first three transfinite levels of ω.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NuOrdinal {
    /// ν = 0 — runtime-only.
    Zero,
    /// ν < ω — finite-step compile-time check.
    FiniteBelowOmega,
    /// ν = ω — first transfinite stratum; full SMT or kernel proof.
    Omega,
    /// ν = ω·2 — multi-strategy / cross-solver / certificate-backed.
    OmegaTwice,
    /// ν ≤ ω·3+1 — inverse search across 𝔐.
    OmegaThricePlusOne,
}

impl NuOrdinal {
    /// Human-readable rendering of the ordinal.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::FiniteBelowOmega => "<ω",
            Self::Omega => "ω",
            Self::OmegaTwice => "ω·2",
            Self::OmegaThricePlusOne => "≤ω·3+1",
        }
    }

    /// Total order on the ladder — mirrors the monotone-lift
    /// semantics of VUVA §2.3. `Synthesize` is treated as distinct
    /// but comparable via its `≤ω·3+1` upper bound for ordering
    /// purposes; callers that care about the orthogonality should
    /// use [`VerifyStrategy::is_synthesis`] explicitly.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::FiniteBelowOmega => 1,
            Self::Omega => 2,
            Self::OmegaTwice => 3,
            Self::OmegaThricePlusOne => 4,
        }
    }
}

impl std::fmt::Display for NuOrdinal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl VerifyStrategy {
    /// All nine strategies in monotone-lift order (`Synthesize` last,
    /// orthogonal). Useful for diagnostics and iteration.
    pub const LADDER: [VerifyStrategy; 9] = [
        Self::Runtime,
        Self::Static,
        Self::Fast,
        Self::Formal,
        Self::Proof,
        Self::Thorough,
        Self::Reliable,
        Self::Certified,
        Self::Synthesize,
    ];

    /// Parse a verify-attribute argument string into a strategy.
    ///
    /// Returns `None` for unrecognized values. Case-insensitive match.
    /// Legacy aliases (`quick`/`rapid`, `robust`, `cross_validate`,
    /// `synthesis`/`synth`) are preserved so existing `.vr` sources
    /// keep working; `proof` and `reliable` are now distinct from
    /// `formal` and `thorough` respectively (VUVA §12).
    pub fn from_attribute_value(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "runtime" => Some(Self::Runtime),
            "static" => Some(Self::Static),
            "fast" | "quick" | "rapid" => Some(Self::Fast),
            "formal" => Some(Self::Formal),
            "proof" => Some(Self::Proof),
            "thorough" | "robust" => Some(Self::Thorough),
            "reliable" => Some(Self::Reliable),
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
            Self::Fast => "fast",
            Self::Formal => "formal",
            Self::Proof => "proof",
            Self::Thorough => "thorough",
            Self::Reliable => "reliable",
            Self::Certified => "certified",
            Self::Synthesize => "synthesize",
        }
    }

    /// Diakrisis ν-invariant ordinal for this strategy (VUVA §12 table).
    pub fn nu_ordinal(&self) -> NuOrdinal {
        match self {
            Self::Runtime => NuOrdinal::Zero,
            Self::Static | Self::Fast => NuOrdinal::FiniteBelowOmega,
            Self::Formal | Self::Proof => NuOrdinal::Omega,
            Self::Thorough | Self::Reliable | Self::Certified => NuOrdinal::OmegaTwice,
            Self::Synthesize => NuOrdinal::OmegaThricePlusOne,
        }
    }

    /// Monotone-lift rank on the verification ladder (VUVA §2.3).
    /// Higher rank ⇒ stricter strategy. A function passing rank `k`
    /// MUST also pass every rank `< k` (the compiler enforces this
    /// by construction — any strategy implies all weaker ones).
    ///
    /// `Synthesize` is ranked at the top of the ordering for
    /// convenience; use [`Self::is_synthesis`] when the orthogonal
    /// semantics matter.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Runtime => 0,
            Self::Static => 1,
            Self::Fast => 2,
            Self::Formal => 3,
            Self::Proof => 4,
            Self::Thorough => 5,
            Self::Reliable => 6,
            Self::Certified => 7,
            Self::Synthesize => 8,
        }
    }

    /// True when `self` is at least as strict as `other`. Used by
    /// the compiler when a module declares a floor strategy and a
    /// function inside it carries a per-function override.
    pub fn at_least(&self, other: &Self) -> bool {
        self.rank() >= other.rank()
    }

    /// Map the strategy to an internal `BackendChoice` for the switcher.
    ///
    /// Returns `None` for strategies that don't require formal proof
    /// infrastructure (`Runtime`, `Static`, `Proof` — the last is
    /// user-supplied and bypasses SMT).
    #[cfg(feature = "cvc5")]
    pub fn to_backend_choice(&self) -> Option<BackendChoice> {
        match self {
            // Runtime / Static are not SMT-backed.
            Self::Runtime | Self::Static => None,
            // Proof is user-supplied tactic; kernel rechecks, no SMT routing.
            Self::Proof => None,
            // Fast: capability routing + stricter timeouts.
            Self::Fast => Some(BackendChoice::Capability),
            // Formal: capability routing — portfolio picks best solver.
            Self::Formal => Some(BackendChoice::Capability),
            // Thorough / Reliable: portfolio mode.
            Self::Thorough | Self::Reliable => Some(BackendChoice::Portfolio),
            // Certified: capability router + cross-validation flag.
            Self::Certified => Some(BackendChoice::Capability),
            // Synthesize: capability router — synthesis-capable backend.
            Self::Synthesize => Some(BackendChoice::Capability),
        }
    }

    /// True if the strategy requires cross-validation (both primary
    /// and secondary solvers must agree). Applies to `Reliable` and
    /// `Certified` — `Reliable` is the minimal cross-validation
    /// level; `Certified` adds certificate export on top.
    pub fn requires_cross_validation(&self) -> bool {
        matches!(self, Self::Reliable | Self::Certified)
    }

    /// True if the strategy must produce a kernel-rechecked
    /// certificate artifact (`@verify(certified)`).
    pub fn requires_certificate(&self) -> bool {
        matches!(self, Self::Certified)
    }

    /// True if the strategy requires formal SMT infrastructure.
    /// `Runtime`, `Static`, `Proof` all bypass the SMT portfolio.
    pub fn requires_smt(&self) -> bool {
        !matches!(self, Self::Runtime | Self::Static | Self::Proof)
    }

    /// True if the strategy is a synthesis problem rather than a
    /// decision problem.
    pub fn is_synthesis(&self) -> bool {
        matches!(self, Self::Synthesize)
    }

    /// True if the strategy prefers thorough/robust verification
    /// over speed.
    pub fn prefers_thoroughness(&self) -> bool {
        matches!(
            self,
            Self::Thorough | Self::Reliable | Self::Certified
        )
    }

    /// True when the strategy expects a user-supplied `proof { … }`
    /// tactic block (not auto-discharged).
    pub fn requires_tactic_proof(&self) -> bool {
        matches!(self, Self::Proof)
    }

    /// True when the strategy requires explicit frame / invariant /
    /// decreases specifications on every obligation.
    pub fn requires_explicit_specs(&self) -> bool {
        matches!(
            self,
            Self::Thorough | Self::Reliable | Self::Certified
        )
    }

    /// Recommended timeout multiplier for this strategy.
    pub fn timeout_multiplier(&self) -> f64 {
        match self {
            Self::Runtime | Self::Static | Self::Proof => 0.0, // no SMT timeout
            Self::Fast => 0.3,       // 30% of base (≤100ms)
            Self::Formal => 1.0,     // base (5s)
            Self::Thorough => 2.0,   // 2× formal
            Self::Reliable => 3.0,   // two solvers, agreement required
            Self::Certified => 3.0,  // reliable + cert materialisation
            Self::Synthesize => 5.0, // synthesis is hard
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
        // After VUVA §12, `proof` and `reliable` are DISTINCT variants,
        // not aliases of `formal` / `thorough`. Only legacy aliases
        // (robust, cross_validate, quick/rapid, synthesis/synth) remain.
        assert_eq!(
            VerifyStrategy::from_attribute_value("proof"),
            Some(VerifyStrategy::Proof)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("reliable"),
            Some(VerifyStrategy::Reliable)
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
