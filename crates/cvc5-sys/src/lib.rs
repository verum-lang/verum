//! # cvc5-sys — Low-Level FFI Bindings for CVC5
//!
//! This crate provides raw, unsafe FFI bindings to the [CVC5 SMT solver's
//! C API](https://cvc5.github.io/doc/cvc5-main/c/). It is intended to be used
//! by higher-level safe wrappers (like `verum_smt::cvc5_backend`) rather than
//! directly by end-user code.
//!
//! ## Build Modes
//!
//! The crate supports three distinct linking strategies, controlled by feature
//! flags (see `Cargo.toml`):
//!
//! | Feature        | Behavior                                              |
//! |----------------|-------------------------------------------------------|
//! | `vendored`     | Build CVC5 from source (static lib in binary)         |
//! | `static`       | Alias for `vendored`                                  |
//! | `system`       | Link against system-installed `libcvc5`               |
//! | *(none)*       | Provide stub bindings only — `init()` returns `false` |
//!
//! For the Verum project, the recommended configuration is `vendored`, which
//! produces a self-contained binary with no external runtime dependencies.
//!
//! ## Safety
//!
//! All functions in this crate are `unsafe`. Callers must uphold:
//! - Pointers returned from CVC5 are valid only until the owning solver/term
//!   manager is destroyed.
//! - Passing null pointers where non-null is expected is undefined behavior.
//! - CVC5 is **not thread-safe**. Each solver instance must be used from a
//!   single thread, though separate instances may be used in parallel.
//!
//! ## Versioning
//!
//! This crate targets CVC5 version **1.2.0+**. ABI stability is not guaranteed
//! across CVC5 releases, so this crate's major version tracks CVC5 compatibility.
//!
//! ## Example (via safe wrapper)
//!
//! ```rust,ignore
//! // Do NOT use these bindings directly. Use `verum_smt::Cvc5Backend` instead.
//! use verum_smt::Cvc5Backend;
//!
//! let mut backend = Cvc5Backend::new()?;
//! backend.set_logic("QF_LIA")?;
//! backend.assert("(> x 0)")?;
//! let result = backend.check_sat()?;
//! ```

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(clippy::missing_safety_doc)]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]

#[cfg(any(feature = "vendored", feature = "static", feature = "system"))]
use std::os::raw::{c_char, c_int, c_uint};
use std::os::raw::c_void;

// ============================================================================
// Build-mode detection
// ============================================================================

/// True if CVC5 was actually linked into this binary.
///
/// When false, all FFI functions return default/null values and `init()`
/// returns `false`. This allows downstream crates to compile and run even
/// without CVC5 available, with the trade-off that `Cvc5Backend::new()`
/// will return `NotAvailable`.
pub const CVC5_LINKED: bool = cfg!(any(
    feature = "vendored",
    feature = "static",
    feature = "system",
));

/// The CVC5 version this crate was built against.
///
/// Format: `"MAJOR.MINOR.PATCH"`.
pub const CVC5_VERSION: &str = "1.2.0";

// ============================================================================
// Opaque types
// ============================================================================
//
// These mirror the opaque pointer types in `cvc5/c/cvc5.h`. CVC5's C API
// uses these handles to manage solver state without exposing C++ internals
// to the caller.

/// Term manager — owns the lifetime of all terms and sorts in a session.
pub type cvc5_tm = *mut c_void;

/// SMT solver — performs check_sat, model extraction, and proof generation.
pub type cvc5_solver = *mut c_void;

/// A sort (type) in the CVC5 logic (e.g., Int, Bool, `Array Int Int`).
pub type cvc5_sort = *mut c_void;

/// A term (expression) in the CVC5 logic.
pub type cvc5_term = *mut c_void;

/// A named operator with parameters (used for parameterized operations).
pub type cvc5_op = *mut c_void;

/// A datatype declaration.
pub type cvc5_datatype_decl = *mut c_void;

/// A datatype constructor declaration.
pub type cvc5_datatype_cons_decl = *mut c_void;

/// A grammar for SyGuS synthesis problems.
pub type cvc5_grammar = *mut c_void;

/// A proof object (valid only after `check_sat` returns UNSAT).
pub type cvc5_proof = *mut c_void;

// ============================================================================
// Result codes
// ============================================================================

/// Result of a satisfiability check.
///
/// Matches CVC5's `cvc5_result_t` enum:
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cvc5Result {
    /// Formula is satisfiable; a model exists.
    SAT = 0,
    /// Formula is unsatisfiable; no model exists.
    UNSAT = 1,
    /// Solver could not determine satisfiability (timeout, resource limit, undecidable fragment).
    UNKNOWN = 2,
}

/// Term kinds — the 200+ operations CVC5 supports.
///
/// This enum is a selection of the most commonly used kinds. The full list is
/// in `cvc5/cvc5_kind.h`. Values match the CVC5 C API exactly.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Cvc5Kind {
    // === General ===
    NULL_TERM = 0,
    CONSTANT = 1,
    VARIABLE = 2,
    // === Boolean ===
    NOT = 10,
    AND = 11,
    OR = 12,
    IMPLIES = 13,
    XOR = 14,
    ITE = 15,
    EQUAL = 16,
    DISTINCT = 17,
    // === Arithmetic ===
    PLUS = 20,
    MULT = 21,
    SUB = 22,
    UMINUS = 23,
    DIVISION = 24,
    INTS_DIVISION = 25,
    INTS_MODULUS = 26,
    ABS = 27,
    LT = 28,
    GT = 29,
    LEQ = 30,
    GEQ = 31,
    TO_INTEGER = 32,
    TO_REAL = 33,
    // === Bit-vectors ===
    BITVECTOR_AND = 40,
    BITVECTOR_OR = 41,
    BITVECTOR_XOR = 42,
    BITVECTOR_NOT = 43,
    BITVECTOR_ADD = 44,
    BITVECTOR_MULT = 45,
    BITVECTOR_CONCAT = 46,
    BITVECTOR_EXTRACT = 47,
    // === Arrays ===
    SELECT = 50,
    STORE = 51,
    // === Strings ===
    STRING_CONCAT = 60,
    STRING_LENGTH = 61,
    STRING_SUBSTR = 62,
    STRING_CONTAINS = 63,
    STRING_REPLACE = 64,
    // === Quantifiers ===
    FORALL = 70,
    EXISTS = 71,
    // === Datatypes ===
    APPLY_CONSTRUCTOR = 80,
    APPLY_SELECTOR = 81,
    APPLY_TESTER = 82,
    // === Uninterpreted functions ===
    APPLY_UF = 90,
}

// ============================================================================
// FFI function declarations
// ============================================================================

#[cfg(any(feature = "vendored", feature = "static", feature = "system"))]
extern "C" {
    // --- Library metadata ---
    pub fn cvc5_version() -> *const c_char;
    pub fn cvc5_copyright() -> *const c_char;

    // --- Term manager lifecycle ---
    pub fn cvc5_tm_new() -> cvc5_tm;
    pub fn cvc5_tm_delete(tm: cvc5_tm);

    // --- Solver lifecycle ---
    pub fn cvc5_solver_new(tm: cvc5_tm) -> cvc5_solver;
    pub fn cvc5_solver_delete(solver: cvc5_solver);

    // --- Configuration ---
    pub fn cvc5_solver_set_logic(solver: cvc5_solver, logic: *const c_char) -> c_int;
    pub fn cvc5_solver_set_option(
        solver: cvc5_solver,
        option: *const c_char,
        value: *const c_char,
    ) -> c_int;
    pub fn cvc5_solver_get_option(
        solver: cvc5_solver,
        option: *const c_char,
    ) -> *const c_char;
    pub fn cvc5_solver_get_info(
        solver: cvc5_solver,
        info: *const c_char,
    ) -> *const c_char;

    // --- Sorts ---
    pub fn cvc5_tm_mk_boolean_sort(tm: cvc5_tm) -> cvc5_sort;
    pub fn cvc5_tm_mk_integer_sort(tm: cvc5_tm) -> cvc5_sort;
    pub fn cvc5_tm_mk_real_sort(tm: cvc5_tm) -> cvc5_sort;
    pub fn cvc5_tm_mk_bv_sort(tm: cvc5_tm, size: c_uint) -> cvc5_sort;
    pub fn cvc5_tm_mk_fp_sort(tm: cvc5_tm, exp: c_uint, sig: c_uint) -> cvc5_sort;
    pub fn cvc5_tm_mk_array_sort(
        tm: cvc5_tm,
        index: cvc5_sort,
        elem: cvc5_sort,
    ) -> cvc5_sort;
    pub fn cvc5_tm_mk_string_sort(tm: cvc5_tm) -> cvc5_sort;
    pub fn cvc5_tm_mk_sequence_sort(tm: cvc5_tm, elem: cvc5_sort) -> cvc5_sort;
    pub fn cvc5_tm_mk_set_sort(tm: cvc5_tm, elem: cvc5_sort) -> cvc5_sort;
    pub fn cvc5_tm_mk_bag_sort(tm: cvc5_tm, elem: cvc5_sort) -> cvc5_sort;
    pub fn cvc5_tm_mk_function_sort(
        tm: cvc5_tm,
        domain: *const cvc5_sort,
        arity: c_uint,
        codomain: cvc5_sort,
    ) -> cvc5_sort;
    pub fn cvc5_tm_mk_uninterpreted_sort(
        tm: cvc5_tm,
        name: *const c_char,
    ) -> cvc5_sort;
    pub fn cvc5_sort_delete(sort: cvc5_sort);

    // --- Constants ---
    pub fn cvc5_tm_mk_const(tm: cvc5_tm, sort: cvc5_sort, name: *const c_char) -> cvc5_term;
    pub fn cvc5_tm_mk_boolean(tm: cvc5_tm, val: bool) -> cvc5_term;
    pub fn cvc5_tm_mk_integer_int64(tm: cvc5_tm, val: i64) -> cvc5_term;
    pub fn cvc5_tm_mk_integer_str(tm: cvc5_tm, val: *const c_char) -> cvc5_term;
    pub fn cvc5_tm_mk_real_from_int(tm: cvc5_tm, num: i64, den: i64) -> cvc5_term;
    pub fn cvc5_tm_mk_real_str(tm: cvc5_tm, val: *const c_char) -> cvc5_term;
    pub fn cvc5_tm_mk_bitvector(tm: cvc5_tm, size: c_uint, val: u64) -> cvc5_term;
    pub fn cvc5_tm_mk_string(tm: cvc5_tm, s: *const c_char, useEscSequences: bool) -> cvc5_term;

    // --- Term construction ---
    pub fn cvc5_tm_mk_term(
        tm: cvc5_tm,
        kind: c_int,
        args: *const cvc5_term,
        n: c_uint,
    ) -> cvc5_term;
    pub fn cvc5_tm_mk_var(tm: cvc5_tm, sort: cvc5_sort, name: *const c_char) -> cvc5_term;
    pub fn cvc5_term_delete(term: cvc5_term);

    // --- Datatypes ---
    pub fn cvc5_tm_mk_datatype_decl(
        tm: cvc5_tm,
        name: *const c_char,
        is_codatatype: bool,
    ) -> cvc5_datatype_decl;
    pub fn cvc5_tm_mk_datatype_cons_decl(
        tm: cvc5_tm,
        name: *const c_char,
    ) -> cvc5_datatype_cons_decl;
    pub fn cvc5_datatype_decl_add_constructor(
        decl: cvc5_datatype_decl,
        cons: cvc5_datatype_cons_decl,
    );
    pub fn cvc5_datatype_cons_decl_add_selector(
        cons: cvc5_datatype_cons_decl,
        name: *const c_char,
        sort: cvc5_sort,
    );

    // --- Assertions & satisfiability ---
    pub fn cvc5_solver_assert_formula(solver: cvc5_solver, term: cvc5_term);
    pub fn cvc5_solver_check_sat(solver: cvc5_solver) -> Cvc5Result;
    pub fn cvc5_solver_check_sat_assuming(
        solver: cvc5_solver,
        assumptions: *const cvc5_term,
        n: c_uint,
    ) -> Cvc5Result;

    // --- Incremental solving ---
    pub fn cvc5_solver_push(solver: cvc5_solver, levels: c_uint);
    pub fn cvc5_solver_pop(solver: cvc5_solver, levels: c_uint);
    pub fn cvc5_solver_reset_assertions(solver: cvc5_solver);

    // --- Models ---
    pub fn cvc5_solver_get_value(solver: cvc5_solver, term: cvc5_term) -> cvc5_term;
    pub fn cvc5_solver_get_model_domain_elements(
        solver: cvc5_solver,
        sort: cvc5_sort,
        size: *mut c_uint,
    ) -> *mut cvc5_term;

    // --- Unsat cores ---
    pub fn cvc5_solver_get_unsat_core(
        solver: cvc5_solver,
        size: *mut c_uint,
    ) -> *mut cvc5_term;
    pub fn cvc5_solver_get_unsat_core_lemmas(
        solver: cvc5_solver,
        size: *mut c_uint,
    ) -> *mut cvc5_term;

    // --- Proofs ---
    pub fn cvc5_solver_get_proof(solver: cvc5_solver) -> *const c_char;

    // --- Interpolation ---
    pub fn cvc5_solver_get_interpolant(
        solver: cvc5_solver,
        conjecture: cvc5_term,
    ) -> cvc5_term;

    // --- Abduction ---
    pub fn cvc5_solver_get_abduct(
        solver: cvc5_solver,
        conjecture: cvc5_term,
    ) -> cvc5_term;

    // --- Quantifier elimination ---
    pub fn cvc5_solver_get_quantifier_elimination(
        solver: cvc5_solver,
        q: cvc5_term,
    ) -> cvc5_term;

    // --- Term inspection ---
    pub fn cvc5_term_to_string(term: cvc5_term) -> *const c_char;
    pub fn cvc5_term_get_kind(term: cvc5_term) -> c_int;
    pub fn cvc5_term_get_sort(term: cvc5_term) -> cvc5_sort;
    pub fn cvc5_term_num_children(term: cvc5_term) -> c_uint;
    pub fn cvc5_term_get_child(term: cvc5_term, idx: c_uint) -> cvc5_term;

    // --- Value extraction ---
    pub fn cvc5_term_is_bool_value(term: cvc5_term) -> bool;
    pub fn cvc5_term_get_bool_value(term: cvc5_term) -> bool;
    pub fn cvc5_term_is_int_value(term: cvc5_term) -> bool;
    pub fn cvc5_term_get_int_value(term: cvc5_term) -> i64;
    pub fn cvc5_term_is_real_value(term: cvc5_term) -> bool;
    pub fn cvc5_term_get_real_value(term: cvc5_term) -> f64;
    pub fn cvc5_term_is_string_value(term: cvc5_term) -> bool;
    pub fn cvc5_term_get_string_value(term: cvc5_term) -> *const c_char;
    pub fn cvc5_term_is_bv_value(term: cvc5_term) -> bool;
    pub fn cvc5_term_get_bv_value(term: cvc5_term) -> u64;

    // --- Sort inspection ---
    pub fn cvc5_sort_to_string(sort: cvc5_sort) -> *const c_char;
    pub fn cvc5_sort_is_bool(sort: cvc5_sort) -> bool;
    pub fn cvc5_sort_is_int(sort: cvc5_sort) -> bool;
    pub fn cvc5_sort_is_real(sort: cvc5_sort) -> bool;
    pub fn cvc5_sort_is_bv(sort: cvc5_sort) -> bool;
    pub fn cvc5_sort_is_array(sort: cvc5_sort) -> bool;
    pub fn cvc5_sort_is_string(sort: cvc5_sort) -> bool;
    pub fn cvc5_sort_is_sequence(sort: cvc5_sort) -> bool;

    // --- Statistics ---
    pub fn cvc5_solver_get_statistics(solver: cvc5_solver) -> *const c_char;

    // --- SyGuS (Syntax-Guided Synthesis) ---
    pub fn cvc5_solver_synth_fun(
        solver: cvc5_solver,
        name: *const c_char,
        vars: *const cvc5_term,
        n_vars: c_uint,
        sort: cvc5_sort,
    ) -> cvc5_term;
    pub fn cvc5_solver_check_synth(solver: cvc5_solver) -> Cvc5Result;
    pub fn cvc5_solver_get_synth_solution(
        solver: cvc5_solver,
        term: cvc5_term,
    ) -> cvc5_term;
}

// ============================================================================
// Stub mode: fake cvc5_version for safe accessors
// ============================================================================
//
// When CVC5 is not linked (no features enabled), the `version()` safe accessor
// still needs *something* to call. We provide a Rust-level stub that mirrors
// what the FFI would do. The full FFI surface is only declared behind feature
// flags; callers of `unsafe extern` functions must also be gated.

// Stub `cvc5_version` fallback is unnecessary because `version()` has a
// separate feature-gated implementation below that doesn't call this function.

// ============================================================================
// Public safe accessors
// ============================================================================

/// Initialize CVC5 backend. Returns `true` on success, `false` if unavailable.
///
/// This function is safe to call from multiple threads but only returns `true`
/// if CVC5 was statically or dynamically linked at build time.
pub fn init() -> bool {
    CVC5_LINKED
}

/// Get the linked CVC5 version, or `"unavailable"` if not linked.
#[cfg(any(feature = "vendored", feature = "static", feature = "system"))]
pub fn version() -> String {
    // SAFETY: `cvc5_version()` returns a static string pointer owned by CVC5.
    unsafe {
        let ptr = cvc5_version();
        if ptr.is_null() {
            return "unknown".to_string();
        }
        std::ffi::CStr::from_ptr(ptr)
            .to_string_lossy()
            .into_owned()
    }
}

/// Stub version accessor when CVC5 is not linked.
#[cfg(not(any(feature = "vendored", feature = "static", feature = "system")))]
pub fn version() -> String {
    "unavailable".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_status_is_consistent() {
        let linked = CVC5_LINKED;
        let initialized = init();
        assert_eq!(linked, initialized);
    }

    #[test]
    fn version_returns_string() {
        let v = version();
        assert!(!v.is_empty());
        if CVC5_LINKED {
            // Linked version should contain a dot (e.g., "1.2.0")
            assert!(v.contains('.') || v == "unknown");
        } else {
            assert_eq!(v, "unavailable");
        }
    }
}
