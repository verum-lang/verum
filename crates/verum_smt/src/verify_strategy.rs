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
/// The nine-strategy verification ladder (VVA §2.3, §12).
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
    /// No SMT. ν = 1.
    Static,

    /// `@verify(fast)` — single-solver SMT with bounded timeout
    /// (default 100ms). UNKNOWN → conservative accept (warning).
    /// ν = 2.
    Fast,

    /// `@verify(complexity_typed)` — bounded-arithmetic verification
    /// (VVA-8 V0). Polynomial-time obligations discharged through the
    /// V_0 / V_1 / S^1_2 / V_NP / V_PH / IΔ_0 stratum chosen at the
    /// pragma level; CI budget ≤ 30 s; UNKNOWN → conservative accept.
    /// ν = 3 (strictly between `Fast` and `Formal`).
    ComplexityTyped,

    /// `@verify(formal)` — portfolio SMT (Z3 + CVC5) with 5s timeout.
    /// UNKNOWN from any solver → conservative accept. ν = ω.
    Formal,

    /// `@verify(proof)` — user supplies a `proof { … }` tactic
    /// block; kernel rechecks. Unbounded user time but mechanically
    /// checked. ν = ω + 1 (dominates SMT, admits induction).
    Proof,

    /// `@verify(thorough)` — `formal` plus mandatory `decreases`,
    /// `invariant`, `frame` specifications. ≈2× formal cost.
    /// ν = ω · 2.
    Thorough,

    /// `@verify(reliable)` — `thorough` plus Z3 AND CVC5 must both
    /// return UNSAT. Any disagreement → UNKNOWN. ≈2× thorough.
    /// ν = ω · 2 + 1.
    Reliable,

    /// `@verify(certified)` — `reliable` plus certificate
    /// materialisation, kernel re-check, multi-format export.
    /// Any recheck failure → compile error. ≈3× thorough.
    /// ν = ω · 2 + 2.
    Certified,

    /// `@verify(coherent_static)` — α-cert + symbolic ε-claim
    /// (VVA-6 V1 weak coherence). The α-articulation is verified
    /// `certified`-style; the ε-coordinate side is discharged through
    /// the symbolic ε-claim attached at `@enact(epsilon = ...)`. No
    /// runtime monitor. Polynomial; CI budget ≤ 60 s. ν = ω · 2 + 3.
    CoherentStatic,

    /// `@verify(coherent_runtime)` — α-cert + runtime ε-monitor
    /// (VVA-6 V1 hybrid coherence). The α-side is `certified`; the
    /// ε-side is checked at runtime through the monitor wired by
    /// `core.action.coherence_monitor`. Trace-bounded; CI budget
    /// ≤ 5 min. ν = ω · 2 + 4.
    CoherentRuntime,

    /// `@verify(coherent)` — α/ε bidirectional check (VVA-6 V1
    /// strict). Both the α-articulation and the ε-coordinate are
    /// discharged at compile time; the kernel re-checks both
    /// certificates. Single-exponential; CI budget ≤ 30 min.
    /// ν = ω · 2 + 5.
    Coherent,

    /// `@verify(synthesize)` — inverse proof search across
    /// 𝔐 to fill missing lemmas / auxiliary theorems.
    /// Orthogonal to the monotone ladder. ν ≤ ω·3+1.
    Synthesize,
}

/// The Diakrisis ν-invariant ordinal assigned to a verification
/// strategy (VVA §12 Table). Each strategy gets a *distinct* ordinal
/// so the monotone ladder `0 < 1 < 2 < ω < ω+1 < ω·2 < ω·2+1 <
/// ω·2+2 < ω·3+1` is strictly ordered (VVA §2.3 strict-monotonicity
/// claim). Earlier coarse buckets (`FiniteBelowOmega`, `OmegaTwice`)
/// are gone; pattern-match exhaustively against the nine variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NuOrdinal {
    /// ν = 0 — runtime-only.
    Zero,
    /// ν = 1 — `static`: dataflow / CBGR / constant folding.
    FiniteOne,
    /// ν = 2 — `fast`: bounded single-solver SMT.
    FiniteTwo,
    /// ν = 3 — `complexity_typed`: VVA-8 bounded-arithmetic.
    FiniteThree,
    /// ν = ω — `formal`: portfolio SMT.
    Omega,
    /// ν = ω + 1 — `proof`: user tactic; dominates SMT, admits induction.
    OmegaPlusOne,
    /// ν = ω · 2 — `thorough`: invariant / frame / termination obligations.
    OmegaTwice,
    /// ν = ω · 2 + 1 — `reliable`: cross-solver agreement.
    OmegaTwicePlusOne,
    /// ν = ω · 2 + 2 — `certified`: certificate materialisation + recheck + export.
    OmegaTwicePlusTwo,
    /// ν = ω · 2 + 3 — `coherent_static`: VVA-6 weak (α-cert + symbolic ε-claim).
    OmegaTwicePlusThree,
    /// ν = ω · 2 + 4 — `coherent_runtime`: VVA-6 hybrid (α-cert + runtime ε-monitor).
    OmegaTwicePlusFour,
    /// ν = ω · 2 + 5 — `coherent`: VVA-6 strict (α/ε bidirectional check).
    OmegaTwicePlusFive,
    /// ν ≤ ω · 3 + 1 — `synthesize`: inverse search across 𝔐 (orthogonal).
    OmegaThricePlusOne,
}

impl NuOrdinal {
    /// Human-readable rendering of the ordinal.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Zero => "0",
            Self::FiniteOne => "1",
            Self::FiniteTwo => "2",
            Self::FiniteThree => "3",
            Self::Omega => "ω",
            Self::OmegaPlusOne => "ω+1",
            Self::OmegaTwice => "ω·2",
            Self::OmegaTwicePlusOne => "ω·2+1",
            Self::OmegaTwicePlusTwo => "ω·2+2",
            Self::OmegaTwicePlusThree => "ω·2+3",
            Self::OmegaTwicePlusFour => "ω·2+4",
            Self::OmegaTwicePlusFive => "ω·2+5",
            Self::OmegaThricePlusOne => "≤ω·3+1",
        }
    }

    /// Strict total order on the ladder — mirrors the strict-monotone
    /// semantics of VVA §2.3. The `≤` in `≤ω·3+1` means `synthesize`
    /// has an upper bound but its exact ν depends on the synthesised
    /// witness's strategy; callers that care about the orthogonality
    /// should use [`VerifyStrategy::is_synthesis`] explicitly.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::FiniteOne => 1,
            Self::FiniteTwo => 2,
            Self::FiniteThree => 3,
            Self::Omega => 4,
            Self::OmegaPlusOne => 5,
            Self::OmegaTwice => 6,
            Self::OmegaTwicePlusOne => 7,
            Self::OmegaTwicePlusTwo => 8,
            Self::OmegaTwicePlusThree => 9,
            Self::OmegaTwicePlusFour => 10,
            Self::OmegaTwicePlusFive => 11,
            Self::OmegaThricePlusOne => 12,
        }
    }
}

impl std::fmt::Display for NuOrdinal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl VerifyStrategy {
    /// All thirteen strategies in monotone-lift order (`Synthesize`
    /// last, orthogonal). Useful for diagnostics and iteration. Per
    /// VVA-6 V1 + VVA-8 V0 the ladder grew from 9 → 13 entries with
    /// `ComplexityTyped` (ν = 3) inserted between `Fast` and `Formal`,
    /// and the three coherent variants (`CoherentStatic`,
    /// `CoherentRuntime`, `Coherent`) inserted between `Certified`
    /// and `Synthesize` at ν = ω·2+3, ω·2+4, ω·2+5.
    pub const LADDER: [VerifyStrategy; 13] = [
        Self::Runtime,
        Self::Static,
        Self::Fast,
        Self::ComplexityTyped,
        Self::Formal,
        Self::Proof,
        Self::Thorough,
        Self::Reliable,
        Self::Certified,
        Self::CoherentStatic,
        Self::CoherentRuntime,
        Self::Coherent,
        Self::Synthesize,
    ];

    /// Parse a verify-attribute argument string into a strategy.
    ///
    /// Returns `None` for unrecognized values. Case-insensitive match.
    /// Legacy aliases (`quick`/`rapid`, `robust`, `cross_validate`,
    /// `synthesis`/`synth`) are preserved so existing `.vr` sources
    /// keep working; `proof` and `reliable` are now distinct from
    /// `formal` and `thorough` respectively (VVA §12).
    pub fn from_attribute_value(value: &str) -> Option<Self> {
        match value.to_lowercase().as_str() {
            "runtime" => Some(Self::Runtime),
            "static" => Some(Self::Static),
            "fast" | "quick" | "rapid" => Some(Self::Fast),
            "complexity_typed" | "complexity-typed" | "complexitytyped" => {
                Some(Self::ComplexityTyped)
            }
            "formal" => Some(Self::Formal),
            "proof" => Some(Self::Proof),
            "thorough" | "robust" => Some(Self::Thorough),
            "reliable" => Some(Self::Reliable),
            "certified" | "cross_validate" | "cross-validate" | "crossvalidate" => {
                Some(Self::Certified)
            }
            "coherent_static" | "coherent-static" | "coherentstatic" => {
                Some(Self::CoherentStatic)
            }
            "coherent_runtime" | "coherent-runtime" | "coherentruntime" => {
                Some(Self::CoherentRuntime)
            }
            "coherent" => Some(Self::Coherent),
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
            Self::ComplexityTyped => "complexity_typed",
            Self::Formal => "formal",
            Self::Proof => "proof",
            Self::Thorough => "thorough",
            Self::Reliable => "reliable",
            Self::Certified => "certified",
            Self::CoherentStatic => "coherent_static",
            Self::CoherentRuntime => "coherent_runtime",
            Self::Coherent => "coherent",
            Self::Synthesize => "synthesize",
        }
    }

    /// Diakrisis ν-invariant ordinal for this strategy (VVA §12 table).
    /// Strictly monotone in `<` — every strategy gets a distinct ordinal.
    pub fn nu_ordinal(&self) -> NuOrdinal {
        match self {
            Self::Runtime         => NuOrdinal::Zero,
            Self::Static          => NuOrdinal::FiniteOne,
            Self::Fast            => NuOrdinal::FiniteTwo,
            Self::ComplexityTyped => NuOrdinal::FiniteThree,
            Self::Formal          => NuOrdinal::Omega,
            Self::Proof           => NuOrdinal::OmegaPlusOne,
            Self::Thorough        => NuOrdinal::OmegaTwice,
            Self::Reliable        => NuOrdinal::OmegaTwicePlusOne,
            Self::Certified       => NuOrdinal::OmegaTwicePlusTwo,
            Self::CoherentStatic  => NuOrdinal::OmegaTwicePlusThree,
            Self::CoherentRuntime => NuOrdinal::OmegaTwicePlusFour,
            Self::Coherent        => NuOrdinal::OmegaTwicePlusFive,
            Self::Synthesize      => NuOrdinal::OmegaThricePlusOne,
        }
    }

    /// Monotone-lift rank on the verification ladder (VVA §2.3).
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
            Self::ComplexityTyped => 3,
            Self::Formal => 4,
            Self::Proof => 5,
            Self::Thorough => 6,
            Self::Reliable => 7,
            Self::Certified => 8,
            Self::CoherentStatic => 9,
            Self::CoherentRuntime => 10,
            Self::Coherent => 11,
            Self::Synthesize => 12,
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
            // ComplexityTyped: capability routing into the bounded-arithmetic
            // backend stratum; UNKNOWN → conservative accept (warning).
            Self::ComplexityTyped => Some(BackendChoice::Capability),
            // Formal: capability routing — portfolio picks best solver.
            Self::Formal => Some(BackendChoice::Capability),
            // Thorough / Reliable: portfolio mode.
            Self::Thorough | Self::Reliable => Some(BackendChoice::Portfolio),
            // Certified: capability router + cross-validation flag.
            Self::Certified => Some(BackendChoice::Capability),
            // CoherentStatic: discharges α through the certified pipeline;
            // ε is symbolic, no extra backend round.
            Self::CoherentStatic => Some(BackendChoice::Capability),
            // CoherentRuntime: same as CoherentStatic on the SMT side; ε
            // monitor lives at runtime, off the SMT path.
            Self::CoherentRuntime => Some(BackendChoice::Capability),
            // Coherent: bidirectional α/ε check — portfolio because both
            // axes need agreement.
            Self::Coherent => Some(BackendChoice::Portfolio),
            // Synthesize: capability router — synthesis-capable backend.
            Self::Synthesize => Some(BackendChoice::Capability),
        }
    }

    /// True if the strategy requires cross-validation (both primary
    /// and secondary solvers must agree). Applies to `Reliable`,
    /// `Certified` (cross-validation is part of the certified pipeline),
    /// the three `Coherent*` variants (their α-side is `certified`-style),
    /// and `Coherent` strict (which adds an ε-side cross-check).
    pub fn requires_cross_validation(&self) -> bool {
        matches!(
            self,
            Self::Reliable
                | Self::Certified
                | Self::CoherentStatic
                | Self::CoherentRuntime
                | Self::Coherent
        )
    }

    /// True if the strategy must produce a kernel-rechecked
    /// certificate artifact. `Certified` produces the α-cert; the
    /// `Coherent*` variants produce α + ε certificates per VVA-6 V1.
    pub fn requires_certificate(&self) -> bool {
        matches!(
            self,
            Self::Certified
                | Self::CoherentStatic
                | Self::CoherentRuntime
                | Self::Coherent
        )
    }

    /// True if the strategy requires formal SMT infrastructure.
    /// `Runtime`, `Static`, `Proof` all bypass the SMT portfolio.
    pub fn requires_smt(&self) -> bool {
        !matches!(self, Self::Runtime | Self::Static | Self::Proof)
    }

    /// True if the strategy is one of the three VVA-6 V1 coherent
    /// variants (α/ε bidirectional or α + symbolic ε / α + runtime ε).
    pub fn is_coherent(&self) -> bool {
        matches!(
            self,
            Self::CoherentStatic | Self::CoherentRuntime | Self::Coherent
        )
    }

    /// True if the strategy emits a runtime ε-monitor in addition to
    /// compile-time obligations. Applies only to `CoherentRuntime`
    /// among the coherent family.
    pub fn requires_runtime_epsilon_monitor(&self) -> bool {
        matches!(self, Self::CoherentRuntime)
    }

    /// True if the strategy needs a complete compile-time discharge
    /// of the ε-coordinate (no runtime monitor allowed). Applies to
    /// `CoherentStatic` (symbolic ε-claim) and `Coherent` (bidirectional
    /// check); `CoherentRuntime` defers ε to the monitor.
    pub fn requires_static_epsilon(&self) -> bool {
        matches!(self, Self::CoherentStatic | Self::Coherent)
    }

    /// True if the strategy is a synthesis problem rather than a
    /// decision problem.
    pub fn is_synthesis(&self) -> bool {
        matches!(self, Self::Synthesize)
    }

    /// True if the strategy prefers thorough/robust verification
    /// over speed. The `Coherent*` family inherits thoroughness from
    /// their `Certified`-style α-side discharge.
    pub fn prefers_thoroughness(&self) -> bool {
        matches!(
            self,
            Self::Thorough
                | Self::Reliable
                | Self::Certified
                | Self::CoherentStatic
                | Self::CoherentRuntime
                | Self::Coherent
        )
    }

    /// True when the strategy expects a user-supplied `proof { … }`
    /// tactic block (not auto-discharged).
    pub fn requires_tactic_proof(&self) -> bool {
        matches!(self, Self::Proof)
    }

    /// True when the strategy requires explicit frame / invariant /
    /// decreases specifications on every obligation. The `Coherent*`
    /// family inherits this requirement from `Certified`.
    pub fn requires_explicit_specs(&self) -> bool {
        matches!(
            self,
            Self::Thorough
                | Self::Reliable
                | Self::Certified
                | Self::CoherentStatic
                | Self::CoherentRuntime
                | Self::Coherent
        )
    }

    /// Recommended timeout multiplier for this strategy. The base
    /// is `Formal` at 1.0× (5 s). Bounded-arithmetic and the coherent
    /// variants get longer budgets per VVA-6/VVA-8 V0.
    pub fn timeout_multiplier(&self) -> f64 {
        match self {
            Self::Runtime | Self::Static | Self::Proof => 0.0, // no SMT timeout
            Self::Fast => 0.3,             // 30% of base (≤100ms)
            Self::ComplexityTyped => 6.0,  // VVA-8 CI budget ≤ 30 s
            Self::Formal => 1.0,           // base (5 s)
            Self::Thorough => 2.0,         // 2× formal
            Self::Reliable => 3.0,         // two solvers, agreement required
            Self::Certified => 3.0,        // reliable + cert materialisation
            Self::CoherentStatic => 12.0,  // VVA-6 weak — CI budget ≤ 60 s
            Self::CoherentRuntime => 60.0, // VVA-6 hybrid — CI budget ≤ 5 min
            Self::Coherent => 360.0,       // VVA-6 strict — CI budget ≤ 30 min
            Self::Synthesize => 5.0,       // synthesis is hard
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
        // After VVA §12, `proof` and `reliable` are DISTINCT variants,
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

    // ========================================================================
    // VVA-6 V1 — coherent strategy backend wiring
    // ========================================================================

    #[test]
    fn parses_coherent_canonical_forms() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("coherent_static"),
            Some(VerifyStrategy::CoherentStatic)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("coherent_runtime"),
            Some(VerifyStrategy::CoherentRuntime)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("coherent"),
            Some(VerifyStrategy::Coherent)
        );
    }

    #[test]
    fn parses_coherent_aliases_and_case() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("Coherent-Static"),
            Some(VerifyStrategy::CoherentStatic)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("COHERENTRUNTIME"),
            Some(VerifyStrategy::CoherentRuntime)
        );
    }

    #[test]
    fn parses_complexity_typed() {
        assert_eq!(
            VerifyStrategy::from_attribute_value("complexity_typed"),
            Some(VerifyStrategy::ComplexityTyped)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("complexity-typed"),
            Some(VerifyStrategy::ComplexityTyped)
        );
        assert_eq!(
            VerifyStrategy::from_attribute_value("ComplexityTyped"),
            Some(VerifyStrategy::ComplexityTyped)
        );
    }

    #[test]
    fn coherent_predicates() {
        for s in [
            VerifyStrategy::CoherentStatic,
            VerifyStrategy::CoherentRuntime,
            VerifyStrategy::Coherent,
        ] {
            assert!(s.is_coherent(), "is_coherent({:?})", s);
            assert!(s.requires_smt(), "requires_smt({:?})", s);
            assert!(s.requires_certificate(), "requires_certificate({:?})", s);
            assert!(s.requires_cross_validation(), "requires_cross_validation({:?})", s);
            assert!(s.requires_explicit_specs(), "requires_explicit_specs({:?})", s);
            assert!(s.prefers_thoroughness(), "prefers_thoroughness({:?})", s);
            assert!(!s.is_synthesis(), "is_synthesis({:?})", s);
        }
        // Runtime ε-monitor is exclusive to CoherentRuntime.
        assert!(!VerifyStrategy::CoherentStatic.requires_runtime_epsilon_monitor());
        assert!(VerifyStrategy::CoherentRuntime.requires_runtime_epsilon_monitor());
        assert!(!VerifyStrategy::Coherent.requires_runtime_epsilon_monitor());
        // Static-ε is required by CoherentStatic and Coherent (not Runtime).
        assert!(VerifyStrategy::CoherentStatic.requires_static_epsilon());
        assert!(!VerifyStrategy::CoherentRuntime.requires_static_epsilon());
        assert!(VerifyStrategy::Coherent.requires_static_epsilon());
    }

    #[test]
    fn nu_ordinals_strictly_monotone_through_ladder() {
        // Per VVA-6 V1 + VVA-8 V0: the 13-strategy LADDER must keep its
        // strict-monotone ν-invariant. For each adjacent pair, rank is
        // strictly increasing.
        let ranks: Vec<u8> =
            VerifyStrategy::LADDER.iter().map(|s| s.nu_ordinal().rank()).collect();
        for window in ranks.windows(2) {
            assert!(
                window[0] < window[1],
                "non-monotone ν-ordinal step: {} → {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn coherent_timeout_budgets_match_vfe_6_spec() {
        // VVA-6 V1: weak ≤60 s, hybrid ≤5 min, strict ≤30 min.
        // Base is `Formal` at 1.0× (5 s).
        let base_seconds = 5.0;
        let cs = VerifyStrategy::CoherentStatic.timeout_multiplier() * base_seconds;
        let cr = VerifyStrategy::CoherentRuntime.timeout_multiplier() * base_seconds;
        let cc = VerifyStrategy::Coherent.timeout_multiplier() * base_seconds;
        assert!(cs <= 60.0, "CoherentStatic budget exceeds 60 s: {}", cs);
        assert!(cr <= 5.0 * 60.0, "CoherentRuntime budget exceeds 5 min: {}", cr);
        assert!(cc <= 30.0 * 60.0, "Coherent budget exceeds 30 min: {}", cc);
        // Strict order weak < hybrid < strict.
        assert!(cs < cr);
        assert!(cr < cc);
    }

    #[test]
    fn full_ladder_roundtrip_via_display() {
        for strategy in VerifyStrategy::LADDER {
            let s = strategy.as_str();
            let parsed = VerifyStrategy::from_attribute_value(s).unwrap();
            assert_eq!(parsed, strategy, "roundtrip failed for {:?}", strategy);
        }
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
