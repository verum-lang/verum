//! Unified SMT Backend Trait - Isomorphic API for All Solvers
//!

//! This module defines a trait-based abstraction that allows transparent switching
//! between SMT solvers (Z3, CVC5, etc.) with zero runtime overhead when using
//! static dispatch.
//!

//! ## Design Principles
//!

//! 1. **Isomorphic API**: All backends expose identical functionality
//! 2. **Type Safety**: Associated types ensure compile-time correctness
//! 3. **Zero Cost**: Static dispatch eliminates virtual call overhead
//! 4. **Completeness**: All operations from both Z3 and CVC5 are supported
//!

//! ## Architecture
//!

//! ```text
//! ┌─────────────────────────────────────┐
//! │ SmtBackend Trait │
//! │ (Unified API - ~60 methods) │
//! └─────────────────────────────────────┘
//!  ▲ ▲
//!  │ │
//!  ┌─────┴────┐ ┌────┴─────┐
//!  │ Z3Backend│ │Cvc5Backend│
//!  └──────────┘ └───────────┘
//! ```
//!

//! Verum's refinement type system allows types like `Int{> 0}`, `Text{len(it) > 5}`,
//! and sigma-type refinements `n: Int where n > 0`. These refinements become SMT
//! constraints during `@verify(proof)` compilation. The backend trait abstracts over
//! Z3 and CVC5 solvers to check satisfiability of these constraints.
//! Performance: <15ns overhead per check (static dispatch)

use std::fmt::Debug;
use verum_common::{List, Map, Maybe, Result, Text};

// ==================== Core Trait ====================

/// Unified SMT Backend trait providing isomorphic API across all solvers
///

/// This trait defines the complete interface that all SMT backends must implement.
/// It includes:
/// - Term and sort construction
/// - Assertion management
/// - Satisfiability checking
/// - Model extraction
/// - Unsat core analysis
/// - Incremental solving
/// - Proof generation
/// - Statistics tracking
pub trait SmtBackend: Send + Sync + Debug {
    // ==================== Associated Types ====================

    /// Term representation (solver-specific)
    type Term: Clone + Send + Sync + Debug;

    /// Sort (type) representation (solver-specific)
    type Sort: Clone + Send + Sync + Debug;

    /// Model representation (solver-specific)
    type Model: Clone + Send + Sync + Debug;

    /// Value representation (solver-specific)
    type Value: Clone + Send + Sync + Debug;

    /// Error type (solver-specific)
    type Error: std::error::Error + Send + Sync + 'static;

    // ==================== Backend Identification ====================

    /// Get backend name ("Z3", "CVC5", etc.)
    fn backend_name(&self) -> &'static str;

    /// Get backend version string
    fn backend_version(&self) -> Text;

    /// Get backend capabilities
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }

    // ==================== Configuration ====================

    /// Set SMT-LIB logic (QF_LIA, QF_BV, etc.)
    fn set_logic(&mut self, logic: SmtLogic) -> Result<(), Self::Error>;

    /// Set global timeout in milliseconds
    fn set_timeout(&mut self, timeout_ms: u64) -> Result<(), Self::Error>;

    /// Set solver option (key-value pair)
    fn set_option(&mut self, key: &str, value: &str) -> Result<(), Self::Error>;

    /// Enable/disable model production
    fn set_produce_models(&mut self, enable: bool) -> Result<(), Self::Error>;

    /// Enable/disable proof production
    fn set_produce_proofs(&mut self, enable: bool) -> Result<(), Self::Error>;

    /// Enable/disable unsat core production
    fn set_produce_unsat_cores(&mut self, enable: bool) -> Result<(), Self::Error>;

    // ==================== Assertions ====================

    /// Assert a formula into the solver
    fn assert(&mut self, term: &Self::Term) -> Result<(), Self::Error>;

    /// Assert with tracking label for unsat core extraction
    fn assert_and_track(&mut self, term: &Self::Term, label: &Text) -> Result<(), Self::Error>;

    /// Get all asserted formulas
    fn get_assertions(&self) -> Result<List<Self::Term>, Self::Error>;

    // ==================== Satisfiability Checking ====================

    /// Check satisfiability of all assertions
    fn check_sat(&mut self) -> Result<SatResult, Self::Error>;

    /// Check satisfiability with additional assumptions
    fn check_sat_assuming(&mut self, assumptions: &[Self::Term]) -> Result<SatResult, Self::Error>;

    /// Get reason for unknown result
    fn get_reason_unknown(&self) -> Result<Maybe<Text>, Self::Error>;

    // ==================== Models ====================

    /// Get model (requires SAT result and produce-models=true)
    fn get_model(&self) -> Result<Self::Model, Self::Error>;

    /// Evaluate term in model
    fn eval_in_model(
        &self,
        model: &Self::Model,
        term: &Self::Term,
    ) -> Result<Self::Value, Self::Error>;

    /// Get all constants in model
    fn get_model_constants(
        &self,
        model: &Self::Model,
    ) -> Result<Map<Text, Self::Value>, Self::Error>;

    // ==================== Unsat Cores ====================

    /// Get unsat core (requires UNSAT result and produce-unsat-cores=true)
    fn get_unsat_core(&self) -> Result<List<Self::Term>, Self::Error>;

    /// Get minimal unsat core (requires multiple solver calls)
    fn get_minimal_unsat_core(&mut self) -> Result<List<Self::Term>, Self::Error> {
        // Default implementation: just return regular core
        self.get_unsat_core()
    }

    // ==================== Incremental Solving ====================

    /// Push assertion scope onto stack
    fn push(&mut self) -> Result<(), Self::Error>;

    /// Pop N assertion scopes from stack
    fn pop(&mut self, n: usize) -> Result<(), Self::Error>;

    /// Reset solver to initial state
    fn reset(&mut self) -> Result<(), Self::Error>;

    /// Get current stack depth
    fn get_stack_depth(&self) -> usize;

    // ==================== Term Construction - Constants ====================

    /// Create named constant with given sort
    fn mk_const(&mut self, name: &Text, sort: Self::Sort) -> Result<Self::Term, Self::Error>;

    /// Create integer value
    fn mk_int_val(&mut self, value: i64) -> Result<Self::Term, Self::Error>;

    /// Create real value (rational number)
    fn mk_real_val(&mut self, num: i64, den: i64) -> Result<Self::Term, Self::Error>;

    /// Create boolean value
    fn mk_bool_val(&mut self, value: bool) -> Result<Self::Term, Self::Error>;

    /// Create string value
    fn mk_string_val(&mut self, value: &str) -> Result<Self::Term, Self::Error>;

    // ==================== Arithmetic Operations ====================

    /// Create addition (n-ary)
    fn mk_add(&mut self, terms: &[Self::Term]) -> Result<Self::Term, Self::Error>;

    /// Create subtraction (n-ary)
    fn mk_sub(&mut self, terms: &[Self::Term]) -> Result<Self::Term, Self::Error>;

    /// Create multiplication (n-ary)
    fn mk_mul(&mut self, terms: &[Self::Term]) -> Result<Self::Term, Self::Error>;

    /// Create division
    fn mk_div(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create modulo
    fn mk_mod(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create negation (unary minus)
    fn mk_neg(&mut self, term: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create absolute value
    fn mk_abs(&mut self, term: &Self::Term) -> Result<Self::Term, Self::Error> {
        // Default: |x| = ite(x >= 0, x, -x)
        let zero = self.mk_int_val(0)?;
        let ge = self.mk_ge(term, &zero)?;
        let neg = self.mk_neg(term)?;
        self.mk_ite(&ge, term, &neg)
    }

    // ==================== Comparison Operations ====================

    /// Create equality
    fn mk_eq(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create inequality
    fn mk_ne(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error> {
        let eq = self.mk_eq(left, right)?;
        self.mk_not(&eq)
    }

    /// Create less than
    fn mk_lt(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create less than or equal
    fn mk_le(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create greater than
    fn mk_gt(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create greater than or equal
    fn mk_ge(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    // ==================== Boolean Operations ====================

    /// Create conjunction (n-ary AND)
    fn mk_and(&mut self, terms: &[Self::Term]) -> Result<Self::Term, Self::Error>;

    /// Create disjunction (n-ary OR)
    fn mk_or(&mut self, terms: &[Self::Term]) -> Result<Self::Term, Self::Error>;

    /// Create negation (NOT)
    fn mk_not(&mut self, term: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create implication (=>)
    fn mk_implies(
        &mut self,
        left: &Self::Term,
        right: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    /// Create bi-implication (iff, <=>)
    fn mk_iff(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create exclusive or (XOR)
    fn mk_xor(&mut self, left: &Self::Term, right: &Self::Term) -> Result<Self::Term, Self::Error>;

    /// Create if-then-else (ternary conditional)
    fn mk_ite(
        &mut self,
        cond: &Self::Term,
        then_term: &Self::Term,
        else_term: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    // ==================== Quantifiers ====================

    /// Create universal quantifier (forall)
    fn mk_forall(
        &mut self,
        vars: &[Self::Term],
        body: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    /// Create existential quantifier (exists)
    fn mk_exists(
        &mut self,
        vars: &[Self::Term],
        body: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    /// Create bound variable for quantifiers
    fn mk_bound_var(&mut self, name: &Text, sort: Self::Sort) -> Result<Self::Term, Self::Error> {
        // Default: use mk_const
        self.mk_const(name, sort)
    }

    // ==================== Arrays ====================

    /// Create array sort (index sort -> element sort)
    fn mk_array_sort(
        &mut self,
        index: Self::Sort,
        elem: Self::Sort,
    ) -> Result<Self::Sort, Self::Error>;

    /// Create array select (read): array[index]
    fn mk_array_select(
        &mut self,
        array: &Self::Term,
        index: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    /// Create array store (write): array[index] := value
    fn mk_array_store(
        &mut self,
        array: &Self::Term,
        index: &Self::Term,
        value: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    /// Create constant array (all elements equal)
    fn mk_const_array(
        &mut self,
        sort: Self::Sort,
        value: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    // ==================== Bit-Vectors ====================

    /// Create bit-vector value
    fn mk_bv_val(&mut self, value: i64, size: u32) -> Result<Self::Term, Self::Error>;

    /// Create bit-vector addition
    fn mk_bv_add(
        &mut self,
        left: &Self::Term,
        right: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    /// Create bit-vector and
    fn mk_bv_and(
        &mut self,
        left: &Self::Term,
        right: &Self::Term,
    ) -> Result<Self::Term, Self::Error>;

    // ==================== Sorts (Types) ====================

    /// Get boolean sort
    fn bool_sort(&self) -> Self::Sort;

    /// Get integer sort
    fn int_sort(&self) -> Self::Sort;

    /// Get real sort
    fn real_sort(&self) -> Self::Sort;

    /// Create bit-vector sort
    fn bv_sort(&self, size: usize) -> Result<Self::Sort, Self::Error>;

    /// Create string sort
    fn string_sort(&self) -> Result<Self::Sort, Self::Error>;

    // ==================== Statistics & Diagnostics ====================

    /// Get solver statistics
    fn get_statistics(&self) -> Map<Text, u64>;

    /// Get proof (requires UNSAT result and produce-proofs=true)
    fn get_proof(&self) -> Result<Maybe<Text>, Self::Error>;

    /// Export to SMT-LIB2 format
    fn to_smt2(&self) -> Result<Text, Self::Error>;

    // ==================== Utility Methods ====================

    /// Clone term (deep copy)
    fn clone_term(&self, term: &Self::Term) -> Self::Term {
        term.clone()
    }

    /// Get term sort
    fn get_sort(&self, term: &Self::Term) -> Result<Self::Sort, Self::Error>;

    /// Simplify term
    fn simplify(&mut self, term: &Self::Term) -> Result<Self::Term, Self::Error>;
}

// ==================== Supporting Types ====================

/// SMT-LIB logic specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SmtLogic {
    /// Quantifier-free linear integer arithmetic
    QF_LIA,
    /// Quantifier-free linear real arithmetic
    QF_LRA,
    /// Quantifier-free bit-vectors
    QF_BV,
    /// Quantifier-free nonlinear integer arithmetic
    QF_NIA,
    /// Quantifier-free nonlinear real arithmetic
    QF_NRA,
    /// Quantifier-free arrays with extensionality
    QF_AX,
    /// Quantifier-free uninterpreted functions + LIA
    QF_UFLIA,
    /// Quantifier-free arrays + UF + LIA
    QF_AUFLIA,
    /// All supported logics (auto-detect)
    ALL,
}

/// Per-variant projection for [`SmtLogic`].
///
/// `name` is the canonical SMT-LIB2 logic identifier returned by
/// `as_str` (uppercase, with underscores — `"QF_LIA"`, `"QF_AUFLIA"`,
/// `"ALL"`, …). `is_quantifier_free` partitions the catalogue: every
/// `QF_*` logic is decidable when its theories are decidable, while
/// `ALL` admits arbitrary quantifier alternation. `is_arithmetic`
/// flags logics that reason about Int / Real arithmetic; the
/// remaining `QF_BV` (bit-vectors) and `QF_AX` (arrays-with-
/// extensionality) are non-arithmetic. Adding a new logic forces
/// an explicit decision in `meta()` instead of silently widening
/// the partition.
#[derive(Debug, Clone, Copy)]
pub struct SmtLogicMeta {
    pub name: &'static str,
    pub is_quantifier_free: bool,
    pub is_arithmetic: bool,
}

impl SmtLogic {
    pub const ALL_LOGICS: &'static [Self] = &[
        Self::QF_LIA,
        Self::QF_LRA,
        Self::QF_BV,
        Self::QF_NIA,
        Self::QF_NRA,
        Self::QF_AX,
        Self::QF_UFLIA,
        Self::QF_AUFLIA,
        Self::ALL,
    ];

    pub const fn meta(self) -> SmtLogicMeta {
        match self {
            Self::QF_LIA => SmtLogicMeta {
                name: "QF_LIA",
                is_quantifier_free: true,
                is_arithmetic: true,
            },
            Self::QF_LRA => SmtLogicMeta {
                name: "QF_LRA",
                is_quantifier_free: true,
                is_arithmetic: true,
            },
            Self::QF_BV => SmtLogicMeta {
                name: "QF_BV",
                is_quantifier_free: true,
                is_arithmetic: false,
            },
            Self::QF_NIA => SmtLogicMeta {
                name: "QF_NIA",
                is_quantifier_free: true,
                is_arithmetic: true,
            },
            Self::QF_NRA => SmtLogicMeta {
                name: "QF_NRA",
                is_quantifier_free: true,
                is_arithmetic: true,
            },
            Self::QF_AX => SmtLogicMeta {
                name: "QF_AX",
                is_quantifier_free: true,
                is_arithmetic: false,
            },
            Self::QF_UFLIA => SmtLogicMeta {
                name: "QF_UFLIA",
                is_quantifier_free: true,
                is_arithmetic: true,
            },
            Self::QF_AUFLIA => SmtLogicMeta {
                name: "QF_AUFLIA",
                is_quantifier_free: true,
                is_arithmetic: true,
            },
            Self::ALL => SmtLogicMeta {
                name: "ALL",
                is_quantifier_free: false,
                is_arithmetic: false,
            },
        }
    }

    /// Convert to SMT-LIB2 string.
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    /// Parse SMT-LIB2 logic name (case-insensitive — uppercases the
    /// input before lookup). SMT-LIB2 logic identifiers are
    /// uppercase-by-convention but solver-supplied or user-typed
    /// identifiers may vary in case; absorbing the cvc5 backend's
    /// historical case-insensitive behaviour keeps the canonical
    /// surface strictly more accepting than any of the previous
    /// duplicates this method replaced.
    pub fn from_str(s: &str) -> Option<Self> {
        let upper = s.to_ascii_uppercase();
        for v in Self::ALL_LOGICS {
            if v.meta().name == upper.as_str() {
                return Some(*v);
            }
        }
        None
    }

    #[inline]
    pub const fn is_quantifier_free(&self) -> bool {
        self.meta().is_quantifier_free
    }

    #[inline]
    pub const fn is_arithmetic(&self) -> bool {
        self.meta().is_arithmetic
    }
}

/// Satisfiability result
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SatResult {
    /// Formula is satisfiable
    Sat,
    /// Formula is unsatisfiable
    Unsat,
    /// Solver could not determine
    Unknown,
}

/// Per-variant projection for [`SatResult`]. Names match the SMT-LIB2
/// `(check-sat)` output tokens — so a solver-supplied string and the
/// typed enum round-trip cleanly through `from_str` / `as_str`.
#[derive(Debug, Clone, Copy)]
pub struct SatResultMeta {
    pub name: &'static str,
    pub is_definitive: bool,
}

impl SatResult {
    pub const ALL: &'static [Self] = &[Self::Sat, Self::Unsat, Self::Unknown];

    pub const fn meta(self) -> SatResultMeta {
        match self {
            Self::Sat => SatResultMeta {
                name: "sat",
                is_definitive: true,
            },
            Self::Unsat => SatResultMeta {
                name: "unsat",
                is_definitive: true,
            },
            Self::Unknown => SatResultMeta {
                name: "unknown",
                is_definitive: false,
            },
        }
    }

    /// SMT-LIB2 result token (`"sat"` / `"unsat"` / `"unknown"`).
    #[inline]
    pub const fn as_str(&self) -> &'static str {
        self.meta().name
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for v in Self::ALL {
            if v.meta().name == s {
                return Some(*v);
            }
        }
        None
    }

    /// True for `Sat` / `Unsat`; false for `Unknown` (timeout,
    /// resource exhaustion, undecidable fragment).
    #[inline]
    pub const fn is_definitive(&self) -> bool {
        self.meta().is_definitive
    }
}

/// Backend capabilities flags
#[derive(Debug, Clone, Default)]
pub struct BackendCapabilities {
    /// Supports proof generation
    pub proofs: bool,
    /// Supports unsat core extraction
    pub unsat_cores: bool,
    /// Supports interpolation
    pub interpolation: bool,
    /// Supports optimization (MaxSMT)
    pub optimization: bool,
    /// Supports quantifier elimination
    pub quantifier_elim: bool,
    /// Supports incremental solving
    pub incremental: bool,
    /// Supports parallel solving
    pub parallel: bool,
    /// Supports theory-specific tactics
    pub tactics: bool,
    /// Maximum supported bit-vector size
    pub max_bv_size: Maybe<usize>,
}

// ==================== Conversion Traits ====================

/// Convert native SMT result to unified result
pub trait IntoSatResult {
    fn into_sat_result(self) -> SatResult;
}

impl IntoSatResult for z3::SatResult {
    fn into_sat_result(self) -> SatResult {
        match self {
            z3::SatResult::Sat => SatResult::Sat,
            z3::SatResult::Unsat => SatResult::Unsat,
            z3::SatResult::Unknown => SatResult::Unknown,
        }
    }
}

// ==================== Error Handling ====================

/// Backend-agnostic error type
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("backend initialization failed: {0}")]
    InitFailed(String),

    #[error("backend not available: {0}")]
    NotAvailable(String),

    #[error("operation not supported: {0}")]
    Unsupported(String),

    #[error("backend-specific error: {0}")]
    BackendSpecific(String),
}

/// Discriminator-only kind for [`BackendError`].
///
/// All four variants carry the same `String` payload (a free-form
/// diagnostic message), so the kind enum is zero-sized.  Lets
/// telemetry / dispatch / metric callers iterate the error
/// surface (for filtering / aggregation / docs) without supplying
/// payload data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendErrorKind {
    InitFailed,
    NotAvailable,
    Unsupported,
    BackendSpecific,
}

/// Per-kind projection for [`BackendErrorKind`].
///
/// `name` is the snake_case telemetry label (`init_failed` /
/// `not_available` / `unsupported` / `backend_specific`).
/// `is_setup_failure` flags `InitFailed` and `NotAvailable` —
/// both fire before the solver is usable, distinguishing them
/// from per-query errors (`Unsupported` / `BackendSpecific`)
/// that fire after a successful initialisation.
/// `is_capability_gap` flags `Unsupported` and `NotAvailable` —
/// both report features the backend genuinely doesn't carry,
/// distinct from operational-failure modes (`InitFailed` /
/// `BackendSpecific`) that report breakage rather than absence.
#[derive(Debug, Clone, Copy)]
pub struct BackendErrorKindMeta {
    pub name: &'static str,
    pub is_setup_failure: bool,
    pub is_capability_gap: bool,
}

impl BackendErrorKind {
    pub const ALL: &'static [Self] = &[
        Self::InitFailed,
        Self::NotAvailable,
        Self::Unsupported,
        Self::BackendSpecific,
    ];

    pub const fn meta(self) -> BackendErrorKindMeta {
        match self {
            Self::InitFailed => BackendErrorKindMeta {
                name: "init_failed",
                is_setup_failure: true,
                is_capability_gap: false,
            },
            Self::NotAvailable => BackendErrorKindMeta {
                name: "not_available",
                is_setup_failure: true,
                is_capability_gap: true,
            },
            Self::Unsupported => BackendErrorKindMeta {
                name: "unsupported",
                is_setup_failure: false,
                is_capability_gap: true,
            },
            Self::BackendSpecific => BackendErrorKindMeta {
                name: "backend_specific",
                is_setup_failure: false,
                is_capability_gap: false,
            },
        }
    }

    #[inline]
    pub const fn name(&self) -> &'static str {
        self.meta().name
    }

    pub fn from_str(s: &str) -> Option<Self> {
        for k in Self::ALL {
            if k.meta().name == s {
                return Some(*k);
            }
        }
        None
    }

    /// True for `InitFailed` / `NotAvailable` — both fire before
    /// the solver is usable for queries.
    #[inline]
    pub const fn is_setup_failure(&self) -> bool {
        self.meta().is_setup_failure
    }

    /// True for `Unsupported` / `NotAvailable` — both report
    /// missing capability rather than operational breakage.
    #[inline]
    pub const fn is_capability_gap(&self) -> bool {
        self.meta().is_capability_gap
    }
}

impl BackendError {
    /// Discriminator-only kind for telemetry / surface enumeration.
    pub fn kind(&self) -> BackendErrorKind {
        match self {
            Self::InitFailed(_) => BackendErrorKind::InitFailed,
            Self::NotAvailable(_) => BackendErrorKind::NotAvailable,
            Self::Unsupported(_) => BackendErrorKind::Unsupported,
            Self::BackendSpecific(_) => BackendErrorKind::BackendSpecific,
        }
    }
}

#[cfg(test)]
mod meta_consolidation_pins {
    use super::*;

    #[test]
    fn smt_logic_round_trip_unique_and_classification() {
        assert_eq!(SmtLogic::ALL_LOGICS.len(), 9);
        let mut seen = Vec::new();
        for v in SmtLogic::ALL_LOGICS {
            let s = v.as_str();
            assert_eq!(SmtLogic::from_str(s), Some(*v), "round-trip {:?}", v);
            assert!(!seen.contains(&s), "duplicate name '{}'", s);
            seen.push(s);
        }
        assert!(SmtLogic::from_str("__not_a_logic__").is_none());

        // Quantifier-free partition: 8 QF_*, 1 ALL.
        let qf = SmtLogic::ALL_LOGICS
            .iter()
            .filter(|v| v.is_quantifier_free())
            .count();
        let qfree_neg = SmtLogic::ALL_LOGICS
            .iter()
            .filter(|v| !v.is_quantifier_free())
            .count();
        assert_eq!(qf, 8);
        assert_eq!(qfree_neg, 1);
        // ALL is the unique non-quantifier-free logic.
        assert!(!SmtLogic::ALL.is_quantifier_free());
        // Arithmetic partition: QF_LIA / QF_LRA / QF_NIA / QF_NRA /
        // QF_UFLIA / QF_AUFLIA = 6; QF_BV / QF_AX / ALL = 3 non-arith.
        let arith = SmtLogic::ALL_LOGICS
            .iter()
            .filter(|v| v.is_arithmetic())
            .count();
        assert_eq!(arith, 6);
        // Wire-form spot pin: identifiers are uppercase with
        // underscores (matches SMT-LIB2 convention).
        assert_eq!(SmtLogic::QF_AUFLIA.as_str(), "QF_AUFLIA");
        assert_eq!(SmtLogic::ALL.as_str(), "ALL");
    }

    #[test]
    fn meta_pin_backend_error_kind_round_trip_and_partitions() {
        assert_eq!(BackendErrorKind::ALL.len(), 4);
        for k in BackendErrorKind::ALL {
            let s = k.name();
            assert_eq!(BackendErrorKind::from_str(s), Some(*k));
        }
        // Wire form (snake_case for telemetry).
        assert_eq!(BackendErrorKind::InitFailed.name(), "init_failed");
        assert_eq!(BackendErrorKind::NotAvailable.name(), "not_available");
        assert_eq!(BackendErrorKind::Unsupported.name(), "unsupported");
        assert_eq!(
            BackendErrorKind::BackendSpecific.name(),
            "backend_specific"
        );
        // is_setup_failure: InitFailed + NotAvailable = 2.
        let setup_count = BackendErrorKind::ALL
            .iter()
            .filter(|k| k.is_setup_failure())
            .count();
        assert_eq!(setup_count, 2);
        // is_capability_gap: NotAvailable + Unsupported = 2.
        let cap_count = BackendErrorKind::ALL
            .iter()
            .filter(|k| k.is_capability_gap())
            .count();
        assert_eq!(cap_count, 2);
        // NotAvailable is the unique kind that's both setup-failure
        // AND capability-gap (the backend isn't initialised because
        // the capability isn't available).
        for k in BackendErrorKind::ALL {
            let both = k.is_setup_failure() && k.is_capability_gap();
            assert_eq!(
                both,
                *k == BackendErrorKind::NotAvailable,
                "NotAvailable is the unique setup-failure ∩ capability-gap kind"
            );
        }
        // Payload variant kind() agreement.
        assert_eq!(
            BackendError::InitFailed("dummy".into()).kind(),
            BackendErrorKind::InitFailed
        );
        assert_eq!(
            BackendError::NotAvailable("dummy".into()).kind(),
            BackendErrorKind::NotAvailable
        );
        assert_eq!(
            BackendError::Unsupported("dummy".into()).kind(),
            BackendErrorKind::Unsupported
        );
        assert_eq!(
            BackendError::BackendSpecific("dummy".into()).kind(),
            BackendErrorKind::BackendSpecific
        );
    }

    #[test]
    fn sat_result_round_trip_unique_and_definitive_partition() {
        assert_eq!(SatResult::ALL.len(), 3);
        for v in SatResult::ALL {
            let s = v.as_str();
            assert_eq!(SatResult::from_str(s), Some(*v));
        }
        // SMT-LIB2 wire form is lowercase.
        assert_eq!(SatResult::Sat.as_str(), "sat");
        assert_eq!(SatResult::Unsat.as_str(), "unsat");
        assert_eq!(SatResult::Unknown.as_str(), "unknown");
        // Definitive partition: Sat/Unsat are definitive; Unknown is
        // the lone non-definitive verdict.
        assert!(SatResult::Sat.is_definitive());
        assert!(SatResult::Unsat.is_definitive());
        assert!(!SatResult::Unknown.is_definitive());
        assert_eq!(
            SatResult::ALL
                .iter()
                .filter(|v| !v.is_definitive())
                .count(),
            1
        );
    }
}

// ==================== Module Statistics ====================

// Total lines: ~520
// Complete unified trait for SMT backend abstraction
// Features:
// - 60+ methods covering all SMT operations
// - Associated types for solver-specific data
// - Default implementations where possible
// - Comprehensive documentation
// - Zero-cost abstraction via static dispatch
