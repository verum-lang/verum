//! CVC5 SMT Backend - Optional Alternative to Z3
//!
//! This module provides CVC5 integration as an alternative to Z3 for SMT solving.
//! CVC5 support is **optional** and requires the CVC5 library to be installed.
//!
//! ## Feature Flags
//!
//! - `cvc5`: Enables the CVC5 backend API (types and trait implementations)
//! - `cvc5-ffi`: Enables actual FFI bindings to libcvc5 (requires CVC5 installed)
//!
//! ## Usage
//!
//! When `cvc5` feature is enabled but `cvc5-ffi` is not:
//! - All types are available for API compatibility
//! - `Cvc5Backend::new()` returns `Err(Cvc5Error::NotAvailable)`
//! - Use Z3 as the primary solver (always available)
//!
//! When both `cvc5` and `cvc5-ffi` features are enabled:
//! - Full CVC5 functionality is available
//! - Requires libcvc5.so/dylib to be installed and accessible
//!
//! ## Installing CVC5
//!
//! CVC5 can be installed from:
//! - <https://cvc5.github.io/downloads.html>
//! - Package managers: `apt install cvc5`, `brew install cvc5`
//!
//! ## Example
//!
//! ```rust,no_run
//! use verum_smt::cvc5_backend::{Cvc5Backend, Cvc5Config, Cvc5Error};
//!
//! let config = Cvc5Config::default();
//! match Cvc5Backend::new(config) {
//!     Ok(backend) => {
//!         // Use CVC5 for SMT solving
//!     }
//!     Err(Cvc5Error::NotAvailable(msg)) => {
//!         // CVC5 not installed - fall back to Z3
//!         eprintln!("CVC5 not available: {}", msg);
//!     }
//!     Err(e) => {
//!         eprintln!("CVC5 initialization error: {}", e);
//!     }
//! }
//! ```
//!
//! CVC5 backend for refinement type verification. CVC5 excels at string theory and
//! nonlinear arithmetic. Refinement predicates are translated to SMT-LIB2 and checked
//! for satisfiability. Supports all five refinement binding forms.

// `CStr`, `CString`, `Instant` are retained via `#[allow(unused_imports)]`
// for future cvc5 FFI plumbing — the backend is half-wired today.
#[allow(unused_imports)]
use std::ffi::{CStr, CString};
#[allow(unused_imports)]
use std::time::Instant;

use verum_common::{List, Map, Maybe, Text};

// ==================== FFI Bindings ====================
// These bindings are only active when cvc5-ffi feature is enabled.
// When disabled, the Cvc5Backend::new() returns NotAvailable error.

#[cfg(feature = "cvc5-ffi")]
#[allow(non_camel_case_types)]
mod cvc5_sys {
    use std::os::raw::{c_char, c_int, c_uint, c_void};

    // Opaque type pointers
    pub type cvc5_tm = *mut c_void;
    pub type cvc5_solver = *mut c_void;
    pub type cvc5_sort = *mut c_void;
    pub type cvc5_term = *mut c_void;
    pub type cvc5_result = *mut c_void;

    // Result enum
    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Cvc5Result {
        SAT = 0,
        UNSAT = 1,
        UNKNOWN = 2,
    }

    // FFI bindings to libcvc5
    // These require libcvc5.so/dylib to be installed
    unsafe extern "C" {
        // TermManager (replaces Context in newer CVC5)
        pub fn cvc5_tm_new() -> cvc5_tm;
        pub fn cvc5_tm_delete(tm: cvc5_tm);

        // Solver
        pub fn cvc5_solver_new(tm: cvc5_tm) -> cvc5_solver;
        pub fn cvc5_solver_delete(solver: cvc5_solver);
        pub fn cvc5_solver_set_logic(solver: cvc5_solver, logic: *const c_char);
        pub fn cvc5_solver_set_option(
            solver: cvc5_solver,
            option: *const c_char,
            value: *const c_char,
        );

        // Assertions
        pub fn cvc5_solver_assert_formula(solver: cvc5_solver, term: cvc5_term);
        pub fn cvc5_solver_check_sat(solver: cvc5_solver) -> Cvc5Result;
        pub fn cvc5_solver_check_sat_assuming(
            solver: cvc5_solver,
            assumptions: *const cvc5_term,
            n: c_uint,
        ) -> Cvc5Result;

        // Models
        pub fn cvc5_solver_get_value(solver: cvc5_solver, term: cvc5_term) -> cvc5_term;
        pub fn cvc5_solver_get_model_domain_elements(
            solver: cvc5_solver,
            sort: cvc5_sort,
            size: *mut c_uint,
        ) -> *mut cvc5_term;

        // Unsat cores
        pub fn cvc5_solver_get_unsat_core(solver: cvc5_solver, size: *mut c_uint)
        -> *mut cvc5_term;
        pub fn cvc5_solver_get_unsat_core_lemmas(
            solver: cvc5_solver,
            size: *mut c_uint,
        ) -> *mut cvc5_term;

        // Incremental
        pub fn cvc5_solver_push(solver: cvc5_solver, levels: c_uint);
        pub fn cvc5_solver_pop(solver: cvc5_solver, levels: c_uint);

        // Sorts
        pub fn cvc5_tm_mk_boolean_sort(tm: cvc5_tm) -> cvc5_sort;
        pub fn cvc5_tm_mk_integer_sort(tm: cvc5_tm) -> cvc5_sort;
        pub fn cvc5_tm_mk_real_sort(tm: cvc5_tm) -> cvc5_sort;
        pub fn cvc5_tm_mk_bv_sort(tm: cvc5_tm, size: c_uint) -> cvc5_sort;
        pub fn cvc5_tm_mk_array_sort(tm: cvc5_tm, index: cvc5_sort, elem: cvc5_sort) -> cvc5_sort;

        // Constants
        pub fn cvc5_tm_mk_const(tm: cvc5_tm, sort: cvc5_sort, name: *const c_char) -> cvc5_term;
        pub fn cvc5_tm_mk_boolean(tm: cvc5_tm, val: bool) -> cvc5_term;
        pub fn cvc5_tm_mk_integer_int64(tm: cvc5_tm, val: i64) -> cvc5_term;
        pub fn cvc5_tm_mk_real_from_int(tm: cvc5_tm, num: i64, den: i64) -> cvc5_term;

        // Operations
        pub fn cvc5_tm_mk_term(
            tm: cvc5_tm,
            kind: c_int,
            args: *const cvc5_term,
            n: c_uint,
        ) -> cvc5_term;

        // Quantifiers
        pub fn cvc5_tm_mk_var(tm: cvc5_tm, sort: cvc5_sort, name: *const c_char) -> cvc5_term;

        // Term inspection
        pub fn cvc5_term_to_string(term: cvc5_term) -> *const c_char;
        pub fn cvc5_term_get_kind(term: cvc5_term) -> c_int;
        pub fn cvc5_term_is_int_value(term: cvc5_term) -> bool;
        pub fn cvc5_term_get_int_value(term: cvc5_term) -> i64;
        pub fn cvc5_term_is_real_value(term: cvc5_term) -> bool;
        pub fn cvc5_term_get_real_value_num(term: cvc5_term) -> i64;
        pub fn cvc5_term_get_real_value_den(term: cvc5_term) -> i64;
        pub fn cvc5_term_is_bool_value(term: cvc5_term) -> bool;
        pub fn cvc5_term_get_bool_value(term: cvc5_term) -> bool;

        // Proofs
        pub fn cvc5_solver_get_proof(solver: cvc5_solver) -> *const c_char;

        // Statistics
        pub fn cvc5_solver_get_statistics(solver: cvc5_solver) -> *const c_char;
    }

    // Term kinds (subset - CVC5 has ~200 kinds)
    pub const CVC5_KIND_AND: c_int = 0;
    pub const CVC5_KIND_OR: c_int = 1;
    pub const CVC5_KIND_NOT: c_int = 2;
    pub const CVC5_KIND_IMPLIES: c_int = 3;
    pub const CVC5_KIND_EQUAL: c_int = 4;
    pub const CVC5_KIND_LT: c_int = 5;
    pub const CVC5_KIND_LEQ: c_int = 6;
    pub const CVC5_KIND_GT: c_int = 7;
    pub const CVC5_KIND_GEQ: c_int = 8;
    pub const CVC5_KIND_ADD: c_int = 9;
    pub const CVC5_KIND_SUB: c_int = 10;
    pub const CVC5_KIND_MULT: c_int = 11;
    pub const CVC5_KIND_DIV: c_int = 12;
    pub const CVC5_KIND_MOD: c_int = 13;
    pub const CVC5_KIND_ITE: c_int = 14;
    pub const CVC5_KIND_FORALL: c_int = 15;
    pub const CVC5_KIND_EXISTS: c_int = 16;
    pub const CVC5_KIND_SELECT: c_int = 17;
    pub const CVC5_KIND_STORE: c_int = 18;
}

// When cvc5-ffi is not enabled, provide placeholder types.
// The Cvc5Result enum and CVC5_KIND_* constants mirror the real cvc5-sys
// bindings so downstream code that `match`es on them compiles in both
// stub and FFI modes. They are legitimately dead in stub mode — the
// SmtBackendSwitcher routes all goals to Z3 then — and become live only
// when `--features cvc5-ffi` links real libcvc5.
#[cfg(not(feature = "cvc5-ffi"))]
#[allow(non_camel_case_types)]
#[allow(dead_code)]
mod cvc5_sys {
    use std::os::raw::{c_int, c_void};

    pub type cvc5_tm = *mut c_void;
    pub type cvc5_solver = *mut c_void;
    pub type cvc5_sort = *mut c_void;
    pub type cvc5_term = *mut c_void;

    #[repr(C)]
    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Cvc5Result {
        SAT = 0,
        UNSAT = 1,
        UNKNOWN = 2,
    }

    // Term kinds (needed for API compatibility)
    pub const CVC5_KIND_AND: c_int = 0;
    pub const CVC5_KIND_OR: c_int = 1;
    pub const CVC5_KIND_NOT: c_int = 2;
    pub const CVC5_KIND_IMPLIES: c_int = 3;
    pub const CVC5_KIND_EQUAL: c_int = 4;
    pub const CVC5_KIND_LT: c_int = 5;
    pub const CVC5_KIND_LEQ: c_int = 6;
    pub const CVC5_KIND_GT: c_int = 7;
    pub const CVC5_KIND_GEQ: c_int = 8;
    pub const CVC5_KIND_ADD: c_int = 9;
    pub const CVC5_KIND_SUB: c_int = 10;
    pub const CVC5_KIND_MULT: c_int = 11;
    pub const CVC5_KIND_DIV: c_int = 12;
    pub const CVC5_KIND_MOD: c_int = 13;
    pub const CVC5_KIND_ITE: c_int = 14;
    pub const CVC5_KIND_FORALL: c_int = 15;
    pub const CVC5_KIND_EXISTS: c_int = 16;
    pub const CVC5_KIND_SELECT: c_int = 17;
    pub const CVC5_KIND_STORE: c_int = 18;
}

// ==================== Core Configuration ====================

/// CVC5 configuration with full feature support
#[derive(Debug, Clone)]
pub struct Cvc5Config {
    /// SMT-LIB logic (QF_LIA, QF_LRA, QF_BV, QF_NRA, etc.)
    pub logic: SmtLogic,
    /// Global timeout in milliseconds
    pub timeout_ms: Maybe<u64>,
    /// Enable incremental solving
    pub incremental: bool,
    /// Produce models for SAT results
    pub produce_models: bool,
    /// Produce proofs for UNSAT results
    pub produce_proofs: bool,
    /// Produce unsat cores
    pub produce_unsat_cores: bool,
    /// Enable preprocessing
    pub preprocessing: bool,
    /// Quantifier instantiation strategy
    pub quantifier_mode: QuantifierMode,
    /// Random seed for reproducibility
    pub random_seed: Maybe<u32>,
    /// Verbosity level (0-5)
    pub verbosity: u32,
}

impl Default for Cvc5Config {
    fn default() -> Self {
        Self {
            logic: SmtLogic::ALL,
            timeout_ms: Maybe::Some(30000), // 30s default
            incremental: true,
            produce_models: true,
            produce_proofs: true,
            produce_unsat_cores: true,
            preprocessing: true,
            quantifier_mode: QuantifierMode::Auto,
            random_seed: Maybe::None,
            verbosity: 0,
        }
    }
}

/// SMT-LIB logic specifications
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Quantifier-free uninterpreted functions with linear integer arithmetic
    QF_UFLIA,
    /// Quantifier-free arrays, uninterpreted functions, linear integer arithmetic
    QF_AUFLIA,
    /// All supported logics (auto-detect)
    ALL,
}

impl SmtLogic {
    /// Return the SMT-LIB 2 logic name string (e.g., `"QF_LIA"`).
    ///
    /// These strings are passed directly to CVC5's `set-logic` command.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::QF_LIA => "QF_LIA",
            Self::QF_LRA => "QF_LRA",
            Self::QF_BV => "QF_BV",
            Self::QF_NIA => "QF_NIA",
            Self::QF_NRA => "QF_NRA",
            Self::QF_AX => "QF_AX",
            Self::QF_UFLIA => "QF_UFLIA",
            Self::QF_AUFLIA => "QF_AUFLIA",
            Self::ALL => "ALL",
        }
    }
}

/// Quantifier instantiation modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantifierMode {
    /// Automatic selection
    Auto,
    /// No quantifier instantiation
    None,
    /// E-matching with patterns
    EMatching,
    /// Counterexample-guided quantifier instantiation
    CEGQI,
    /// Model-based quantifier instantiation
    MBQI,
}

// ==================== Error Types ====================

/// CVC5 error types
#[derive(Debug, thiserror::Error)]
pub enum Cvc5Error {
    /// CVC5 is not available (library not installed or feature not enabled)
    #[error("CVC5 not available: {0}")]
    NotAvailable(String),

    #[error("initialization failed: {0}")]
    InitializationFailed(String),

    #[error("configuration error: {0}")]
    ConfigurationError(String),

    #[error("term construction error: {0}")]
    TermConstructionError(String),

    #[error("model error: {0}")]
    ModelError(String),

    #[error("stack underflow")]
    StackUnderflow,

    #[error("timeout after {0}ms")]
    Timeout(u64),

    #[error("unsupported operation: {0}")]
    Unsupported(String),
}

// ==================== Core Backend ====================

/// CVC5 SMT Backend
///
/// This backend provides CVC5 integration for SMT solving.
/// It requires the `cvc5-ffi` feature to be enabled for actual functionality.
///
/// When `cvc5-ffi` is not enabled, `Cvc5Backend::new()` returns
/// `Err(Cvc5Error::NotAvailable)` and the struct fields are unused; they
/// become live as soon as cvc5-ffi is linked. The `dead_code` allow is
/// scoped narrowly here rather than crate-wide so unrelated dead code
/// keeps surfacing as warnings.
#[allow(dead_code)]
pub struct Cvc5Backend {
    /// Term manager (context)
    tm: cvc5_sys::cvc5_tm,
    /// Solver instance
    solver: cvc5_sys::cvc5_solver,
    /// Configuration
    config: Cvc5Config,
    /// Term cache for reuse
    term_cache: Map<Text, Cvc5Term>,
    /// Sort cache
    sort_cache: Map<Text, Cvc5Sort>,
    /// Named assertions for unsat core
    named_assertions: Map<Text, Cvc5Term>,
    /// Assertion stack depth
    assertion_stack_depth: usize,
    /// Statistics
    stats: Cvc5Stats,
}

// Implementation when cvc5-ffi feature IS enabled
#[cfg(feature = "cvc5-ffi")]
impl Cvc5Backend {
    /// Create new CVC5 backend with configuration
    ///
    /// Returns `Err(Cvc5Error::NotAvailable)` if CVC5 library is not installed.
    pub fn new(config: Cvc5Config) -> Result<Self, Cvc5Error> {
        // SAFETY: FFI calls to CVC5 C library functions.
        // - cvc5_tm_new creates a new term manager with proper initialization
        // - cvc5_solver_new creates a new solver instance bound to the term manager
        // - We check for null pointers and clean up (cvc5_tm_delete) on failure
        // - All returned pointers are owned by this struct and cleaned up in Drop
        unsafe {
            // Create term manager
            let tm = cvc5_sys::cvc5_tm_new();
            if tm.is_null() {
                return Err(Cvc5Error::InitializationFailed(
                    "Failed to create term manager - is libcvc5 installed?".to_string(),
                ));
            }

            // Create solver
            let solver = cvc5_sys::cvc5_solver_new(tm);
            if solver.is_null() {
                cvc5_sys::cvc5_tm_delete(tm);
                return Err(Cvc5Error::InitializationFailed(
                    "Failed to create solver".to_string(),
                ));
            }

            // Configure solver
            let mut backend = Self {
                tm,
                solver,
                config: config.clone(),
                term_cache: Map::new(),
                sort_cache: Map::new(),
                named_assertions: Map::new(),
                assertion_stack_depth: 0,
                stats: Cvc5Stats::default(),
            };

            backend.apply_configuration()?;
            Ok(backend)
        }
    }

    /// Apply configuration to solver
    fn apply_configuration(&mut self) -> Result<(), Cvc5Error> {
        // SAFETY: FFI calls to configure the CVC5 solver.
        // - All CString::new().unwrap().as_ptr() calls create valid null-terminated C strings
        // - Solver pointer is valid (checked in new())
        // - Option names and values are valid UTF-8 strings
        // - CVC5 API expects null-terminated C strings, which CString provides
        unsafe {
            // Set logic
            let logic_cstr = CString::new(self.config.logic.as_str())
                .map_err(|e| Cvc5Error::ConfigurationError(e.to_string()))?;
            cvc5_sys::cvc5_solver_set_logic(self.solver, logic_cstr.as_ptr());

            // Set timeout
            if let Maybe::Some(timeout) = self.config.timeout_ms {
                let timeout_str = CString::new(timeout.to_string())
                    .map_err(|e| Cvc5Error::ConfigurationError(e.to_string()))?;
                cvc5_sys::cvc5_solver_set_option(
                    self.solver,
                    CString::new("tlimit-per").unwrap().as_ptr(),
                    timeout_str.as_ptr(),
                );
            }

            // Set incremental mode
            let incremental_str = if self.config.incremental {
                "true"
            } else {
                "false"
            };
            cvc5_sys::cvc5_solver_set_option(
                self.solver,
                CString::new("incremental").unwrap().as_ptr(),
                CString::new(incremental_str).unwrap().as_ptr(),
            );

            // Set model production
            let models_str = if self.config.produce_models {
                "true"
            } else {
                "false"
            };
            cvc5_sys::cvc5_solver_set_option(
                self.solver,
                CString::new("produce-models").unwrap().as_ptr(),
                CString::new(models_str).unwrap().as_ptr(),
            );

            // Set proof production
            let proofs_str = if self.config.produce_proofs {
                "true"
            } else {
                "false"
            };
            cvc5_sys::cvc5_solver_set_option(
                self.solver,
                CString::new("produce-proofs").unwrap().as_ptr(),
                CString::new(proofs_str).unwrap().as_ptr(),
            );

            // Set unsat core production
            let cores_str = if self.config.produce_unsat_cores {
                "true"
            } else {
                "false"
            };
            cvc5_sys::cvc5_solver_set_option(
                self.solver,
                CString::new("produce-unsat-cores").unwrap().as_ptr(),
                CString::new(cores_str).unwrap().as_ptr(),
            );

            // Set random seed if provided
            if let Maybe::Some(seed) = self.config.random_seed {
                let seed_str = CString::new(seed.to_string())
                    .map_err(|e| Cvc5Error::ConfigurationError(e.to_string()))?;
                cvc5_sys::cvc5_solver_set_option(
                    self.solver,
                    CString::new("seed").unwrap().as_ptr(),
                    seed_str.as_ptr(),
                );
            }

            Ok(())
        }
    }

    // ==================== Assertions ====================

    /// Assert a formula into the solver
    pub fn assert(&mut self, term: &Cvc5Term) -> Result<(), Cvc5Error> {
        // SAFETY: FFI call to assert a formula.
        // - self.solver is a valid pointer (checked in new())
        // - term.raw is a valid CVC5 term pointer created by this backend
        // - The term is compatible with the solver's term manager
        unsafe {
            cvc5_sys::cvc5_solver_assert_formula(self.solver, term.raw);
            self.stats.total_assertions += 1;
            Ok(())
        }
    }

    /// Assert with tracking label for unsat core extraction
    pub fn assert_and_track(&mut self, term: &Cvc5Term, label: &Text) -> Result<(), Cvc5Error> {
        // Track assertion for unsat core
        self.named_assertions.insert(label.clone(), term.clone());
        self.assert(term)
    }

    // ==================== Checking ====================

    /// Check satisfiability
    pub fn check_sat(&mut self) -> Result<SatResult, Cvc5Error> {
        let start = Instant::now();
        self.stats.total_checks += 1;

        // SAFETY: FFI call to check satisfiability.
        // - self.solver is a valid pointer
        // - cvc5_solver_check_sat returns a valid Cvc5Result enum value
        // - No memory is allocated or freed in this call
        unsafe {
            let result = cvc5_sys::cvc5_solver_check_sat(self.solver);
            let elapsed = start.elapsed();
            self.stats.total_time_ms += elapsed.as_millis() as u64;

            match result {
                cvc5_sys::Cvc5Result::SAT => {
                    self.stats.sat_count += 1;
                    Ok(SatResult::Sat)
                }
                cvc5_sys::Cvc5Result::UNSAT => {
                    self.stats.unsat_count += 1;
                    Ok(SatResult::Unsat)
                }
                cvc5_sys::Cvc5Result::UNKNOWN => {
                    self.stats.unknown_count += 1;
                    Ok(SatResult::Unknown)
                }
            }
        }
    }

    /// Check satisfiability with assumptions
    pub fn check_sat_assuming(&mut self, assumptions: &[Cvc5Term]) -> Result<SatResult, Cvc5Error> {
        let start = Instant::now();
        self.stats.total_checks += 1;

        // SAFETY: FFI call to check satisfiability with assumptions.
        // - self.solver is a valid pointer
        // - raw_terms is a valid array of term pointers created by this backend
        // - raw_terms.as_ptr() points to valid memory for at least raw_terms.len() elements
        // - The length is passed correctly as u32
        // - The Vec stays alive for the duration of the FFI call
        unsafe {
            let raw_terms: Vec<cvc5_sys::cvc5_term> = assumptions.iter().map(|t| t.raw).collect();
            let result = cvc5_sys::cvc5_solver_check_sat_assuming(
                self.solver,
                raw_terms.as_ptr(),
                raw_terms.len() as u32,
            );
            let elapsed = start.elapsed();
            self.stats.total_time_ms += elapsed.as_millis() as u64;

            match result {
                cvc5_sys::Cvc5Result::SAT => {
                    self.stats.sat_count += 1;
                    Ok(SatResult::Sat)
                }
                cvc5_sys::Cvc5Result::UNSAT => {
                    self.stats.unsat_count += 1;
                    Ok(SatResult::Unsat)
                }
                cvc5_sys::Cvc5Result::UNKNOWN => {
                    self.stats.unknown_count += 1;
                    Ok(SatResult::Unknown)
                }
            }
        }
    }

    // ==================== Models ====================

    /// Get model (requires SAT result and produce-models=true)
    pub fn get_model(&self) -> Result<Cvc5Model, Cvc5Error> {
        Ok(Cvc5Model {
            solver: self.solver,
            tm: self.tm,
        })
    }

    /// Evaluate term in current model
    pub fn eval(&self, term: &Cvc5Term) -> Result<Cvc5Value, Cvc5Error> {
        // SAFETY: FFI call to get term value from model.
        // - self.solver is a valid pointer with a current model (caller must ensure SAT result)
        // - term.raw is a valid CVC5 term pointer
        // - We check for null pointer return value before using
        // - term_to_value() safely converts the term to a Verum value
        unsafe {
            let value_term = cvc5_sys::cvc5_solver_get_value(self.solver, term.raw);
            if value_term.is_null() {
                return Err(Cvc5Error::ModelError("Failed to evaluate term".to_string()));
            }

            Self::term_to_value(value_term)
        }
    }

    /// Convert CVC5 term to Verum value
    // SAFETY: Caller must provide a valid, non-null CVC5 term pointer.
    // - All cvc5_term_is_*_value and cvc5_term_get_*_value calls require valid term pointer
    // - cvc5_term_to_string may return null, which we check
    // - CStr::from_ptr requires a valid null-terminated C string, provided by CVC5
    // - The C string lifetime is managed by CVC5 and valid for this call
    unsafe fn term_to_value(term: cvc5_sys::cvc5_term) -> Result<Cvc5Value, Cvc5Error> {
        if cvc5_sys::cvc5_term_is_bool_value(term) {
            let val = cvc5_sys::cvc5_term_get_bool_value(term);
            Ok(Cvc5Value::Bool(val))
        } else if cvc5_sys::cvc5_term_is_int_value(term) {
            let val = cvc5_sys::cvc5_term_get_int_value(term);
            Ok(Cvc5Value::Int(val))
        } else if cvc5_sys::cvc5_term_is_real_value(term) {
            let num = cvc5_sys::cvc5_term_get_real_value_num(term);
            let den = cvc5_sys::cvc5_term_get_real_value_den(term);
            Ok(Cvc5Value::Real(num, den))
        } else {
            // Generic term representation
            let c_str = cvc5_sys::cvc5_term_to_string(term);
            if c_str.is_null() {
                Ok(Cvc5Value::Unknown)
            } else {
                let rust_str = CStr::from_ptr(c_str).to_string_lossy().into_owned();
                Ok(Cvc5Value::Term(rust_str))
            }
        }
    }

    // ==================== Unsat Cores ====================

    /// Get unsat core (requires UNSAT result and produce-unsat-cores=true)
    pub fn get_unsat_core(&self) -> Result<List<Cvc5Term>, Cvc5Error> {
        // SAFETY: FFI call to get unsat core.
        // - self.solver is a valid pointer with an UNSAT result (caller responsibility)
        // - size is a valid mutable reference to u32
        // - core_ptr, if non-null, points to an array of at least 'size' term pointers
        // - core_ptr.add(i) is safe for i < size
        // - Term pointers are owned by CVC5 and valid for this solver's lifetime
        unsafe {
            let mut size: u32 = 0;
            let core_ptr = cvc5_sys::cvc5_solver_get_unsat_core(self.solver, &mut size);

            if core_ptr.is_null() {
                return Ok(List::new());
            }

            let mut core = List::new();
            for i in 0..size {
                let term_ptr = *core_ptr.add(i as usize);
                core.push(Cvc5Term { raw: term_ptr });
            }

            Ok(core)
        }
    }

    // ==================== Incremental Solving ====================

    /// Push assertion scope
    pub fn push(&mut self) -> Result<(), Cvc5Error> {
        // SAFETY: FFI call to push assertion scope.
        // - self.solver is a valid pointer
        // - Push level (1) is valid
        // - Incremental mode must be enabled (set in apply_configuration)
        unsafe {
            cvc5_sys::cvc5_solver_push(self.solver, 1);
            self.assertion_stack_depth += 1;
            self.stats.push_count += 1;
            Ok(())
        }
    }

    /// Pop assertion scope
    pub fn pop(&mut self, n: usize) -> Result<(), Cvc5Error> {
        if n > self.assertion_stack_depth {
            return Err(Cvc5Error::StackUnderflow);
        }

        // SAFETY: FFI call to pop assertion scope.
        // - self.solver is a valid pointer
        // - n <= assertion_stack_depth (checked above), so pop is valid
        // - Incremental mode must be enabled
        unsafe {
            cvc5_sys::cvc5_solver_pop(self.solver, n as u32);
            self.assertion_stack_depth -= n;
            self.stats.pop_count += 1;
            Ok(())
        }
    }

    // ==================== Term Construction ====================

    /// Create boolean constant
    pub fn mk_const(&mut self, name: &Text, sort: Cvc5Sort) -> Result<Cvc5Term, Cvc5Error> {
        // Check cache
        if let Maybe::Some(term) = self.term_cache.get(name) {
            return Ok(term.clone());
        }

        // SAFETY: FFI call to create a constant term.
        // - self.tm is a valid term manager pointer
        // - sort.raw is a valid sort pointer from this term manager
        // - name_cstr.as_ptr() provides a valid null-terminated C string
        // - The CString stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            let name_cstr = CString::new(name.as_str())
                .map_err(|e| Cvc5Error::TermConstructionError(e.to_string()))?;
            let term_ptr = cvc5_sys::cvc5_tm_mk_const(self.tm, sort.raw, name_cstr.as_ptr());

            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create constant".to_string(),
                ));
            }

            let term = Cvc5Term { raw: term_ptr };
            self.term_cache.insert(name.clone(), term.clone());
            Ok(term)
        }
    }

    /// Create integer value
    pub fn mk_int_val(&mut self, value: i64) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create integer value term.
        // - self.tm is a valid term manager pointer
        // - value is a valid i64 integer
        // - We check for null pointer return value
        unsafe {
            let term_ptr = cvc5_sys::cvc5_tm_mk_integer_int64(self.tm, value);
            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create int value".to_string(),
                ));
            }
            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    /// Create real value (rational)
    pub fn mk_real_val(&mut self, num: i64, den: i64) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create rational value term.
        // - self.tm is a valid term manager pointer
        // - num and den are valid i64 integers
        // - Caller must ensure den != 0 for valid rational
        // - We check for null pointer return value
        unsafe {
            let term_ptr = cvc5_sys::cvc5_tm_mk_real_from_int(self.tm, num, den);
            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create real value".to_string(),
                ));
            }
            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    /// Create boolean value
    pub fn mk_bool_val(&mut self, value: bool) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create boolean value term.
        // - self.tm is a valid term manager pointer
        // - value is a valid bool
        // - We check for null pointer return value
        unsafe {
            let term_ptr = cvc5_sys::cvc5_tm_mk_boolean(self.tm, value);
            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create bool value".to_string(),
                ));
            }
            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    // ==================== Operations ====================

    /// Create addition
    pub fn mk_add(&mut self, args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_nary_op(cvc5_sys::CVC5_KIND_ADD, args)
    }

    /// Create subtraction
    pub fn mk_sub(&mut self, args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_nary_op(cvc5_sys::CVC5_KIND_SUB, args)
    }

    /// Create multiplication
    pub fn mk_mul(&mut self, args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_nary_op(cvc5_sys::CVC5_KIND_MULT, args)
    }

    /// Create division
    pub fn mk_div(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_DIV, left, right)
    }

    /// Create equality
    pub fn mk_eq(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_EQUAL, left, right)
    }

    /// Create less than
    pub fn mk_lt(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_LT, left, right)
    }

    /// Create less than or equal
    pub fn mk_le(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_LEQ, left, right)
    }

    /// Create greater than
    pub fn mk_gt(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_GT, left, right)
    }

    /// Create greater than or equal
    pub fn mk_ge(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_GEQ, left, right)
    }

    /// Create conjunction
    pub fn mk_and(&mut self, args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_nary_op(cvc5_sys::CVC5_KIND_AND, args)
    }

    /// Create disjunction
    pub fn mk_or(&mut self, args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_nary_op(cvc5_sys::CVC5_KIND_OR, args)
    }

    /// Create negation
    pub fn mk_not(&mut self, term: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_unary_op(cvc5_sys::CVC5_KIND_NOT, term)
    }

    /// Create implication
    pub fn mk_implies(&mut self, left: &Cvc5Term, right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_IMPLIES, left, right)
    }

    // ==================== Quantifiers ====================

    /// Create forall quantifier
    pub fn mk_forall(&mut self, vars: &[Cvc5Term], body: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create forall quantifier.
        // - self.tm is a valid term manager pointer
        // - All terms in vars and body are valid term pointers from this manager
        // - raw_args.as_ptr() points to valid memory for raw_args.len() elements
        // - The Vec stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            // Create bound variable list + body
            let mut all_args = vars.to_vec();
            all_args.push(body.clone());

            let raw_args: Vec<cvc5_sys::cvc5_term> = all_args.iter().map(|t| t.raw).collect();
            let term_ptr = cvc5_sys::cvc5_tm_mk_term(
                self.tm,
                cvc5_sys::CVC5_KIND_FORALL,
                raw_args.as_ptr(),
                raw_args.len() as u32,
            );

            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create forall".to_string(),
                ));
            }

            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    /// Create exists quantifier
    pub fn mk_exists(&mut self, vars: &[Cvc5Term], body: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create exists quantifier.
        // - self.tm is a valid term manager pointer
        // - All terms in vars and body are valid term pointers from this manager
        // - raw_args.as_ptr() points to valid memory for raw_args.len() elements
        // - The Vec stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            let mut all_args = vars.to_vec();
            all_args.push(body.clone());

            let raw_args: Vec<cvc5_sys::cvc5_term> = all_args.iter().map(|t| t.raw).collect();
            let term_ptr = cvc5_sys::cvc5_tm_mk_term(
                self.tm,
                cvc5_sys::CVC5_KIND_EXISTS,
                raw_args.as_ptr(),
                raw_args.len() as u32,
            );

            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create exists".to_string(),
                ));
            }

            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    // ==================== Arrays ====================

    /// Create array select (read)
    pub fn mk_array_select(
        &mut self,
        array: &Cvc5Term,
        index: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        self.mk_binary_op(cvc5_sys::CVC5_KIND_SELECT, array, index)
    }

    /// Create array store (write)
    pub fn mk_array_store(
        &mut self,
        array: &Cvc5Term,
        index: &Cvc5Term,
        value: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create array store term.
        // - self.tm is a valid term manager pointer
        // - array, index, and value are valid term pointers from this manager
        // - args.as_ptr() points to valid memory for 3 term pointers
        // - The Vec stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            let args = vec![array.raw, index.raw, value.raw];
            let term_ptr = cvc5_sys::cvc5_tm_mk_term(
                self.tm,
                cvc5_sys::CVC5_KIND_STORE,
                args.as_ptr(),
                args.len() as u32,
            );

            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create store".to_string(),
                ));
            }

            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    // ==================== Sorts ====================

    /// Get boolean sort
    pub fn bool_sort(&mut self) -> Cvc5Sort {
        if let Maybe::Some(sort) = self.sort_cache.get(&Text::from("Bool")) {
            return sort.clone();
        }

        // SAFETY: FFI call to create boolean sort.
        // - self.tm is a valid term manager pointer
        // - cvc5_tm_mk_boolean_sort always returns a valid sort pointer
        unsafe {
            let sort = Cvc5Sort {
                raw: cvc5_sys::cvc5_tm_mk_boolean_sort(self.tm),
            };
            self.sort_cache.insert(Text::from("Bool"), sort.clone());
            sort
        }
    }

    /// Get integer sort
    pub fn int_sort(&mut self) -> Cvc5Sort {
        if let Maybe::Some(sort) = self.sort_cache.get(&Text::from("Int")) {
            return sort.clone();
        }

        // SAFETY: FFI call to create integer sort.
        // - self.tm is a valid term manager pointer
        // - cvc5_tm_mk_integer_sort always returns a valid sort pointer
        unsafe {
            let sort = Cvc5Sort {
                raw: cvc5_sys::cvc5_tm_mk_integer_sort(self.tm),
            };
            self.sort_cache.insert(Text::from("Int"), sort.clone());
            sort
        }
    }

    /// Get real sort
    pub fn real_sort(&mut self) -> Cvc5Sort {
        if let Maybe::Some(sort) = self.sort_cache.get(&Text::from("Real")) {
            return sort.clone();
        }

        // SAFETY: FFI call to create real sort.
        // - self.tm is a valid term manager pointer
        // - cvc5_tm_mk_real_sort always returns a valid sort pointer
        unsafe {
            let sort = Cvc5Sort {
                raw: cvc5_sys::cvc5_tm_mk_real_sort(self.tm),
            };
            self.sort_cache.insert(Text::from("Real"), sort.clone());
            sort
        }
    }

    /// Create bit-vector sort
    pub fn bv_sort(&mut self, size: u32) -> Cvc5Sort {
        let key = Text::from(format!("BV{}", size));
        if let Maybe::Some(sort) = self.sort_cache.get(&key) {
            return sort.clone();
        }

        // SAFETY: FFI call to create bit-vector sort.
        // - self.tm is a valid term manager pointer
        // - size is a valid u32 (typically > 0 for meaningful bit-vectors)
        // - cvc5_tm_mk_bv_sort returns a valid sort pointer
        unsafe {
            let sort = Cvc5Sort {
                raw: cvc5_sys::cvc5_tm_mk_bv_sort(self.tm, size),
            };
            self.sort_cache.insert(key, sort.clone());
            sort
        }
    }

    /// Create array sort
    pub fn array_sort(&mut self, index: Cvc5Sort, elem: Cvc5Sort) -> Cvc5Sort {
        // SAFETY: FFI call to create array sort.
        // - self.tm is a valid term manager pointer
        // - index.raw and elem.raw are valid sort pointers from this manager
        // - cvc5_tm_mk_array_sort returns a valid sort pointer
        unsafe {
            Cvc5Sort {
                raw: cvc5_sys::cvc5_tm_mk_array_sort(self.tm, index.raw, elem.raw),
            }
        }
    }

    // ==================== Helpers ====================

    fn mk_unary_op(&mut self, kind: i32, arg: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create unary operation term.
        // - self.tm is a valid term manager pointer
        // - arg.raw is a valid term pointer from this manager
        // - args.as_ptr() points to valid memory for 1 term pointer
        // - The Vec stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            let args = vec![arg.raw];
            let term_ptr = cvc5_sys::cvc5_tm_mk_term(self.tm, kind, args.as_ptr(), 1);
            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create unary op".to_string(),
                ));
            }
            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    fn mk_binary_op(
        &mut self,
        kind: i32,
        left: &Cvc5Term,
        right: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        // SAFETY: FFI call to create binary operation term.
        // - self.tm is a valid term manager pointer
        // - left.raw and right.raw are valid term pointers from this manager
        // - args.as_ptr() points to valid memory for 2 term pointers
        // - The Vec stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            let args = vec![left.raw, right.raw];
            let term_ptr = cvc5_sys::cvc5_tm_mk_term(self.tm, kind, args.as_ptr(), 2);
            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create binary op".to_string(),
                ));
            }
            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    fn mk_nary_op(&mut self, kind: i32, args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        if args.is_empty() {
            return Err(Cvc5Error::TermConstructionError(
                "Cannot create n-ary op with no arguments".to_string(),
            ));
        }

        // SAFETY: FFI call to create n-ary operation term.
        // - self.tm is a valid term manager pointer
        // - All terms in args are valid term pointers from this manager
        // - raw_args.as_ptr() points to valid memory for raw_args.len() elements
        // - The Vec stays alive for the FFI call duration
        // - We check for null pointer return value
        unsafe {
            let raw_args: Vec<cvc5_sys::cvc5_term> = args.iter().map(|t| t.raw).collect();
            let term_ptr =
                cvc5_sys::cvc5_tm_mk_term(self.tm, kind, raw_args.as_ptr(), raw_args.len() as u32);

            if term_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create n-ary op".to_string(),
                ));
            }

            Ok(Cvc5Term { raw: term_ptr })
        }
    }

    // ==================== Statistics ====================

    /// Get solver statistics
    pub fn get_stats(&self) -> &Cvc5Stats {
        &self.stats
    }

    /// Get proof (requires UNSAT result and produce-proofs=true)
    pub fn get_proof(&self) -> Result<Maybe<String>, Cvc5Error> {
        // SAFETY: FFI call to get proof.
        // - self.solver is a valid pointer with an UNSAT result (caller responsibility)
        // - proof_ptr, if non-null, points to a valid null-terminated C string
        // - The C string is owned by CVC5 and valid for this solver's lifetime
        // - CStr::from_ptr requires valid null-terminated string, which CVC5 provides
        unsafe {
            let proof_ptr = cvc5_sys::cvc5_solver_get_proof(self.solver);
            if proof_ptr.is_null() {
                return Ok(Maybe::None);
            }

            let c_str = CStr::from_ptr(proof_ptr);
            let proof_str = c_str.to_string_lossy().into_owned();
            Ok(Maybe::Some(proof_str))
        }
    }

    /// Get reason for unknown result
    pub fn get_reason_unknown(&self) -> Result<String, Cvc5Error> {
        // CVC5 doesn't have a standard get-reason-unknown API in the C bindings
        // This would require querying solver options or statistics
        Ok("Unknown reason".to_string())
    }

    /// Assert formula from Verum expression
    pub fn assert_formula_from_expr(
        &mut self,
        expr: &verum_ast::expr::Expr,
    ) -> Result<(), Cvc5Error> {
        let term = self.convert_expr_to_term(expr)?;
        self.assert(&term)
    }

    /// Convert Verum expression to CVC5 term
    fn convert_expr_to_term(
        &mut self,
        expr: &verum_ast::expr::Expr,
    ) -> Result<Cvc5Term, Cvc5Error> {
        use verum_ast::expr::{BinOp, ExprKind, UnOp};
        use verum_ast::literal::LiteralKind;

        match &expr.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Bool(b) => self.mk_bool_val(*b),
                LiteralKind::Int(i) => {
                    // IntLit has a value field with i128
                    let val = i.value as i64;
                    self.mk_int_val(val)
                }
                LiteralKind::Float(f) => {
                    // FloatLit has a value field with f64
                    // Convert float to rational (simplified)
                    // Simple conversion: multiply by 1000 to preserve 3 decimal places
                    let num = (f.value * 1000.0) as i64;
                    self.mk_real_val(num, 1000)
                }
                _ => Err(Cvc5Error::Unsupported(
                    "Unsupported literal type".to_string(),
                )),
            },

            ExprKind::Path(path) => {
                // Extract variable name from path
                let name: String =
                    if let Maybe::Some(ident) = crate::option_to_maybe(path.as_ident()) {
                        ident.as_str().to_string()
                    } else {
                        path.segments
                            .iter()
                            .filter_map(|seg| match seg {
                                verum_ast::ty::PathSegment::Name(id) => Some(id.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(".")
                    };

                // Create boolean variable (default)
                let bool_sort = self.get_bool_sort()?;
                self.mk_const(&Text::from(name), bool_sort)
            }

            ExprKind::Binary { op, left, right } => {
                let left_term = self.convert_expr_to_term(left)?;
                let right_term = self.convert_expr_to_term(right)?;

                match op {
                    BinOp::Add => self.mk_add(&[left_term, right_term]),
                    BinOp::Sub => self.mk_sub(&[left_term, right_term]),
                    BinOp::Mul => self.mk_mul(&[left_term, right_term]),
                    BinOp::Div => self.mk_div(&left_term, &right_term),
                    BinOp::Eq => self.mk_eq(&left_term, &right_term),
                    BinOp::Ne => {
                        let eq = self.mk_eq(&left_term, &right_term)?;
                        self.mk_not(&eq)
                    }
                    BinOp::Lt => self.mk_lt(&left_term, &right_term),
                    BinOp::Le => self.mk_le(&left_term, &right_term),
                    BinOp::Gt => self.mk_gt(&left_term, &right_term),
                    BinOp::Ge => self.mk_ge(&left_term, &right_term),
                    BinOp::And => self.mk_and(&[left_term, right_term]),
                    BinOp::Or => self.mk_or(&[left_term, right_term]),
                    _ => Err(Cvc5Error::Unsupported(format!("Binary operator {:?}", op))),
                }
            }

            ExprKind::Unary { op, expr: operand } => {
                let operand_term = self.convert_expr_to_term(operand)?;

                match op {
                    UnOp::Not => self.mk_not(&operand_term),
                    UnOp::Neg => {
                        let zero = self.mk_int_val(0)?;
                        self.mk_sub(&[zero, operand_term])
                    }
                    _ => Err(Cvc5Error::Unsupported(format!("Unary operator {:?}", op))),
                }
            }

            ExprKind::Attenuate { context, .. } => {
                // Attenuate expressions restrict capability, recursively process the context
                // For SMT translation, capability attenuation is a compile-time concept
                self.convert_expr_to_term(context)
            }

            _ => Err(Cvc5Error::Unsupported(format!(
                "Expression kind {:?}",
                expr.kind
            ))),
        }
    }

    /// Get boolean sort
    fn get_bool_sort(&mut self) -> Result<Cvc5Sort, Cvc5Error> {
        if let Maybe::Some(sort) = self.sort_cache.get(&Text::from("Bool")) {
            return Ok(sort.clone());
        }

        // SAFETY: FFI call to create boolean sort.
        // - self.tm is a valid term manager pointer
        // - We check for null pointer return value
        unsafe {
            let sort_ptr = cvc5_sys::cvc5_tm_mk_boolean_sort(self.tm);
            if sort_ptr.is_null() {
                return Err(Cvc5Error::TermConstructionError(
                    "Failed to create boolean sort".to_string(),
                ));
            }

            let sort = Cvc5Sort { raw: sort_ptr };
            self.sort_cache.insert(Text::from("Bool"), sort.clone());
            Ok(sort)
        }
    }
}

#[cfg(feature = "cvc5-ffi")]
impl Drop for Cvc5Backend {
    fn drop(&mut self) {
        // SAFETY: FFI calls to clean up CVC5 resources.
        // - We check for null before deletion
        // - self.solver and self.tm are valid pointers created in new()
        // - CVC5 delete functions handle cleanup of all associated resources
        // - Delete order: solver first, then term manager (proper dependency order)
        unsafe {
            if !self.solver.is_null() {
                cvc5_sys::cvc5_solver_delete(self.solver);
            }
            if !self.tm.is_null() {
                cvc5_sys::cvc5_tm_delete(self.tm);
            }
        }
    }
}

// Implementation when cvc5-ffi feature is NOT enabled
#[cfg(not(feature = "cvc5-ffi"))]
impl Cvc5Backend {
    /// Create new CVC5 backend
    ///
    /// When the `cvc5-ffi` feature is not enabled, this always returns
    /// `Err(Cvc5Error::NotAvailable)`. Use Z3 as the primary solver.
    ///
    /// To enable CVC5 support:
    /// 1. Install CVC5: <https://cvc5.github.io/downloads.html>
    /// 2. Enable the feature: `cargo build --features cvc5-ffi`
    pub fn new(_config: Cvc5Config) -> Result<Self, Cvc5Error> {
        Err(Cvc5Error::NotAvailable(
            "CVC5 backend requires the 'cvc5-ffi' feature to be enabled. \
             This feature requires libcvc5 to be installed on your system. \
             Install CVC5 from https://cvc5.github.io/downloads.html and \
             rebuild with: cargo build --features cvc5-ffi"
                .to_string(),
        ))
    }

    // Stub implementations for API compatibility
    // These will never be called since new() always returns Err

    pub fn assert(&mut self, _term: &Cvc5Term) -> Result<(), Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn assert_and_track(&mut self, _term: &Cvc5Term, _label: &Text) -> Result<(), Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn check_sat(&mut self) -> Result<SatResult, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn check_sat_assuming(
        &mut self,
        _assumptions: &[Cvc5Term],
    ) -> Result<SatResult, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn get_model(&self) -> Result<Cvc5Model, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn eval(&self, _term: &Cvc5Term) -> Result<Cvc5Value, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn get_unsat_core(&self) -> Result<List<Cvc5Term>, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn push(&mut self) -> Result<(), Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn pop(&mut self, _n: usize) -> Result<(), Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_const(&mut self, _name: &Text, _sort: Cvc5Sort) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_int_val(&mut self, _value: i64) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_real_val(&mut self, _num: i64, _den: i64) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_bool_val(&mut self, _value: bool) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_add(&mut self, _args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_sub(&mut self, _args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_mul(&mut self, _args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_div(&mut self, _left: &Cvc5Term, _right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_eq(&mut self, _left: &Cvc5Term, _right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_lt(&mut self, _left: &Cvc5Term, _right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_le(&mut self, _left: &Cvc5Term, _right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_gt(&mut self, _left: &Cvc5Term, _right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_ge(&mut self, _left: &Cvc5Term, _right: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_and(&mut self, _args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_or(&mut self, _args: &[Cvc5Term]) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_not(&mut self, _term: &Cvc5Term) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_implies(
        &mut self,
        _left: &Cvc5Term,
        _right: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_forall(
        &mut self,
        _vars: &[Cvc5Term],
        _body: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_exists(
        &mut self,
        _vars: &[Cvc5Term],
        _body: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_array_select(
        &mut self,
        _array: &Cvc5Term,
        _index: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn mk_array_store(
        &mut self,
        _array: &Cvc5Term,
        _index: &Cvc5Term,
        _value: &Cvc5Term,
    ) -> Result<Cvc5Term, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn bool_sort(&mut self) -> Cvc5Sort {
        Cvc5Sort {
            raw: std::ptr::null_mut(),
        }
    }

    pub fn int_sort(&mut self) -> Cvc5Sort {
        Cvc5Sort {
            raw: std::ptr::null_mut(),
        }
    }

    pub fn real_sort(&mut self) -> Cvc5Sort {
        Cvc5Sort {
            raw: std::ptr::null_mut(),
        }
    }

    pub fn bv_sort(&mut self, _size: u32) -> Cvc5Sort {
        Cvc5Sort {
            raw: std::ptr::null_mut(),
        }
    }

    pub fn array_sort(&mut self, _index: Cvc5Sort, _elem: Cvc5Sort) -> Cvc5Sort {
        Cvc5Sort {
            raw: std::ptr::null_mut(),
        }
    }

    pub fn get_stats(&self) -> &Cvc5Stats {
        &self.stats
    }

    pub fn get_proof(&self) -> Result<Maybe<String>, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn get_reason_unknown(&self) -> Result<String, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn assert_formula_from_expr(
        &mut self,
        _expr: &verum_ast::expr::Expr,
    ) -> Result<(), Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }
}

// ==================== Supporting Types ====================

/// CVC5 term wrapper
#[derive(Clone)]
pub struct Cvc5Term {
    raw: cvc5_sys::cvc5_term,
}

impl std::fmt::Debug for Cvc5Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cvc5Term({:p})", self.raw)
    }
}

/// CVC5 sort wrapper. Field is dead without cvc5-ffi (stub returns
/// Err(NotAvailable) before any Cvc5Sort is constructed).
#[derive(Clone)]
#[allow(dead_code)]
pub struct Cvc5Sort {
    raw: cvc5_sys::cvc5_sort,
}

/// SAT result
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SatResult {
    Sat,
    Unsat,
    Unknown,
}

/// Type alias for consistency with backend_switcher.rs
pub type Cvc5SatResult = SatResult;

/// CVC5 value representation
#[derive(Debug, Clone)]
pub enum Cvc5Value {
    Bool(bool),
    Int(i64),
    Real(i64, i64), // numerator, denominator
    Term(String),   // Generic term representation
    Unknown,
}

/// Model extractor. Fields are dead without cvc5-ffi.
#[allow(dead_code)]
pub struct Cvc5Model {
    solver: cvc5_sys::cvc5_solver,
    tm: cvc5_sys::cvc5_tm,
}

impl std::fmt::Debug for Cvc5Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cvc5Model({:p})", self.solver)
    }
}

#[cfg(feature = "cvc5-ffi")]
impl Cvc5Model {
    /// Evaluate term in model
    pub fn eval(&self, term: &Cvc5Term) -> Result<Cvc5Value, Cvc5Error> {
        // SAFETY: FFI call to get term value from model.
        // - self.solver is a valid solver pointer with a current model
        // - term.raw is a valid CVC5 term pointer
        // - We check for null pointer return value
        // - term_to_value() safely converts the term to a Verum value
        unsafe {
            let value_term = cvc5_sys::cvc5_solver_get_value(self.solver, term.raw);
            if value_term.is_null() {
                return Err(Cvc5Error::ModelError("Failed to evaluate term".to_string()));
            }

            Cvc5Backend::term_to_value(value_term)
        }
    }

    /// Get value by variable name
    pub fn get_value(&self, var: &Text) -> Result<Cvc5Value, Cvc5Error> {
        let var_cstr = std::ffi::CString::new(var.as_str())
            .map_err(|e| Cvc5Error::ModelError(format!("Invalid variable name: {}", e)))?;

        // SAFETY: FFI call to create a Boolean constant and get its value
        // - solver is a valid pointer (checked in construction)
        // - var_cstr is a valid null-terminated C string
        unsafe {
            // Get the Boolean sort (assuming Boolean for simplicity)
            let bool_sort = cvc5_sys::cvc5_tm_mk_boolean_sort(self.tm);
            if bool_sort.is_null() {
                return Err(Cvc5Error::ModelError("Failed to create sort".to_string()));
            }

            // Create a constant with the variable name
            let term = cvc5_sys::cvc5_tm_mk_const(self.tm, bool_sort, var_cstr.as_ptr());
            if term.is_null() {
                return Err(Cvc5Error::ModelError(format!(
                    "Variable '{}' not found in model",
                    var
                )));
            }

            // Get the value from the model
            let value_term = cvc5_sys::cvc5_solver_get_value(self.solver, term);
            if value_term.is_null() {
                return Err(Cvc5Error::ModelError(format!(
                    "Failed to get value for variable '{}'",
                    var
                )));
            }

            Cvc5Backend::term_to_value(value_term)
        }
    }

    /// Convert model to map
    pub fn to_map(&self) -> Result<Map<Text, Cvc5Value>, Cvc5Error> {
        // NOTE: Full model extraction requires access to the Cvc5Backend's term cache
        // or a list of declared symbols. Since Cvc5Model only has solver pointer,
        // we provide a limited implementation.
        tracing::warn!(
            target: "verum_smt::cvc5",
            "to_map() called on Cvc5Model without term cache access; use Cvc5Backend::get_model_map()"
        );

        Ok(Map::new())
    }
}

#[cfg(not(feature = "cvc5-ffi"))]
impl Cvc5Model {
    pub fn eval(&self, _term: &Cvc5Term) -> Result<Cvc5Value, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn get_value(&self, _var: &Text) -> Result<Cvc5Value, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }

    pub fn to_map(&self) -> Result<Map<Text, Cvc5Value>, Cvc5Error> {
        Err(Cvc5Error::NotAvailable("CVC5 not available".to_string()))
    }
}

/// Statistics
#[derive(Debug, Clone, Default)]
pub struct Cvc5Stats {
    pub total_checks: usize,
    pub sat_count: usize,
    pub unsat_count: usize,
    pub unknown_count: usize,
    pub total_time_ms: u64,
    pub total_assertions: usize,
    pub push_count: usize,
    pub pop_count: usize,
}

// ==================== Public API ====================

/// Create CVC5 backend with default configuration
///
/// Returns `Err(Cvc5Error::NotAvailable)` if CVC5 is not installed or
/// the `cvc5-ffi` feature is not enabled.
pub fn create_cvc5_backend() -> Result<Cvc5Backend, Cvc5Error> {
    Cvc5Backend::new(Cvc5Config::default())
}

/// Create CVC5 backend for specific logic
///
/// Returns `Err(Cvc5Error::NotAvailable)` if CVC5 is not installed or
/// the `cvc5-ffi` feature is not enabled.
pub fn create_cvc5_backend_for_logic(logic: SmtLogic) -> Result<Cvc5Backend, Cvc5Error> {
    let config = Cvc5Config {
        logic,
        ..Default::default()
    };
    Cvc5Backend::new(config)
}

/// Check if CVC5 backend is available
///
/// Returns `true` if the `cvc5-ffi` feature is enabled, `false` otherwise.
pub fn is_cvc5_available() -> bool {
    cfg!(feature = "cvc5-ffi")
}
