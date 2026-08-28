//! Structured error-code registry for the Verum error system (#63).
//!
//! Maps error codes to their category and a short description.  This is
//! the single source of truth for what each code means, and it is now
//! ENFORCED as one: `tests/registry_covers_every_emitted_code.rs` fails
//! the build if the compiler can print a code that is missing here.  The
//! richer diagnostics (worked examples, fix suggestions) live in
//! `verum_diagnostics`, and `verum explain` falls back to the
//! description here for every code that has no long-form entry yet.
//!
//! Until that gate existed the claim above was aspirational: measured on
//! 2026-08-28, forty-one of the codes the compiler could print had no
//! entry, thirty-six type diagnostics carried no code at all, and one
//! number (`E1001`) meant "use-after-free" in one crate and "stage
//! mismatch" in another.  An incomplete table is worse than none,
//! because the next author picks a number by reading what looks free.
//!
//! # Error code ranges
//!
//! | Range | Category | Examples |
//! |-------|----------|---------|
//! | E0xx  | Parse    | E001 unexpected token, E002 unterminated string |
//! | E1xx  | Name resolution | E100 undefined variable, E102 arity mismatch |
//! | E2xx  | Module   | E200 import not found, E201 circular import |
//! | E3xx  | Memory / Lifetime | E305 uninitialized value, E310 use-after-move |
//! | E4xx  | Type system | E400 type mismatch, E401 invalid cast |
//! | E5xx  | Verification | E500 contract violated, E501 SMT timeout |
//! | E6xx  | Context system | E600 context not provided, E603 context mismatch |
//! | E8xx  | FFI | E800 unsafe FFI violation, E803 inline-asm operand |
//! | E9xx  | Internal | E900 ICE, E901 compiler assertion failed |
//! | four-digit | Lint / phase | E0900–E0906, E1000–E1005 denied lints |
//!
//! E7xx (Async) is reserved and currently emitted by nothing.
//!
//! # What this registry does NOT cover
//!
//! Three other numberings exist in the workspace and none of them agrees
//! with this one.  They are recorded here so the next person does not
//! rediscover them one at a time:
//!
//! * `verum_fast_parser/src/error.rs` numbers parse errors E010–E099,
//!   in parallel with the E0xx band above.
//! * `verum_diagnostics/src/explanations.rs` is keyed by rustc-style
//!   four-digit codes (E0101–E0501).
//! * `VerumError::code_prefix()` maps a category to a two-digit PREFIX
//!   on a completely different scheme — memory to "E01", type to "E02",
//!   verification to "E03" — contradicting the bands above outright.
//!
//! Unifying them is a language-surface decision, not a repair, so it is
//! deliberately out of scope here.
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_error::registry::{lookup, ErrorCodeEntry, ErrorCategory};
//!
//! let entry = lookup("E400").expect("known error code");
//! println!("{}: {}", entry.code, entry.description);
//! ```

use once_cell::sync::Lazy;
use std::collections::HashMap;

/// Category of an error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCategory {
    /// Lexer / parser errors (E0xx).
    Parse,
    /// Name-resolution errors — undefined variables, types, functions (E1xx).
    NameResolution,
    /// Module-system errors — import failures, circular imports (E2xx).
    Module,
    /// Memory and lifetime errors — use-after-move, borrow conflicts (E3xx).
    Memory,
    /// Type-system errors — type mismatch, invalid cast (E4xx).
    Type,
    /// Formal-verification errors — SMT timeout, contract violation (E5xx).
    Verification,
    /// Context-system errors — missing / conflicting DI contexts (E6xx).
    Context,
    /// Async-runtime errors — cancelled futures, task join failures (E7xx).
    Async,
    /// FFI errors — ABI mismatch, null-pointer dereference (E8xx).
    Ffi,
    /// Internal compiler errors — ICE, assertion failures (E9xx).
    Internal,
    /// Compiler lints that were escalated to errors (E09xx / E10xx).
    ///
    /// These carry FOUR digits, and that is not decoration: the lint
    /// tables in `verum_compiler` pair every lint with both a warning
    /// code (`W1000`) and an error code for when the lint is denied, and
    /// they were numbered in their own band. Registering them here is
    /// what stops the two bands from drifting into each other again —
    /// `E1001` already meant "stage mismatch" to the lint table and
    /// "use-after-free" to `verum_cbgr`'s diagnostics at the same time.
    Lint,
}

impl ErrorCategory {
    /// Short ASCII label used in diagnostic output.
    pub fn label(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::NameResolution => "name",
            Self::Module => "module",
            Self::Memory => "memory",
            Self::Type => "type",
            Self::Verification => "verify",
            Self::Context => "context",
            Self::Async => "async",
            Self::Ffi => "ffi",
            Self::Internal => "internal",
            Self::Lint => "lint",
        }
    }
}

/// A single entry in the error-code registry.
#[derive(Debug, Clone)]
pub struct ErrorCodeEntry {
    /// The error code string, e.g. "E400".
    pub code: &'static str,
    /// Numeric value of the code (400 for E400).
    pub numeric: u16,
    /// High-level category.
    pub category: ErrorCategory,
    /// One-line description of the error.
    pub description: &'static str,
}

impl ErrorCodeEntry {
    /// Returns the numeric prefix (E0xx → 0, E1xx → 1, …, E9xx → 9).
    pub fn range_prefix(&self) -> u8 {
        (self.numeric / 100) as u8
    }
}

/// The global error-code registry.
///
/// Keyed by the code string (e.g., "E400").  Access via [`lookup`].
pub static REGISTRY: Lazy<HashMap<&'static str, ErrorCodeEntry>> = Lazy::new(|| {
    let entries: &[ErrorCodeEntry] = &[
        // ── E0xx: Parse ──────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E001", numeric: 1,   category: ErrorCategory::Parse, description: "unexpected token" },
        ErrorCodeEntry { code: "E002", numeric: 2,   category: ErrorCategory::Parse, description: "unterminated string literal" },
        ErrorCodeEntry { code: "E003", numeric: 3,   category: ErrorCategory::Parse, description: "invalid escape sequence" },
        ErrorCodeEntry { code: "E004", numeric: 4,   category: ErrorCategory::Parse, description: "missing closing delimiter" },
        ErrorCodeEntry { code: "E005", numeric: 5,   category: ErrorCategory::Parse, description: "expected expression" },
        ErrorCodeEntry { code: "E006", numeric: 6,   category: ErrorCategory::Parse, description: "invalid integer literal" },
        ErrorCodeEntry { code: "E007", numeric: 7,   category: ErrorCategory::Parse, description: "invalid float literal" },

        // ── E1xx: Name resolution ─────────────────────────────────────────────
        ErrorCodeEntry { code: "E100", numeric: 100, category: ErrorCategory::NameResolution, description: "undefined variable" },
        ErrorCodeEntry { code: "E101", numeric: 101, category: ErrorCategory::NameResolution, description: "undefined type" },
        ErrorCodeEntry { code: "E102", numeric: 102, category: ErrorCategory::NameResolution, description: "undefined function" },
        ErrorCodeEntry { code: "E103", numeric: 103, category: ErrorCategory::NameResolution, description: "field not found on type" },
        ErrorCodeEntry { code: "E104", numeric: 104, category: ErrorCategory::NameResolution, description: "duplicate definition" },
        ErrorCodeEntry { code: "E105", numeric: 105, category: ErrorCategory::NameResolution, description: "ambiguous name" },
        ErrorCodeEntry { code: "E106", numeric: 106, category: ErrorCategory::NameResolution, description: "unresolved type placeholder" },

        // ── E2xx: Module ──────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E200", numeric: 200, category: ErrorCategory::Module, description: "import not found" },
        ErrorCodeEntry { code: "E201", numeric: 201, category: ErrorCategory::Module, description: "circular import" },
        ErrorCodeEntry { code: "E202", numeric: 202, category: ErrorCategory::Module, description: "private item imported" },
        ErrorCodeEntry { code: "E203", numeric: 203, category: ErrorCategory::Module, description: "module not found" },
        ErrorCodeEntry { code: "E204", numeric: 204, category: ErrorCategory::Module, description: "circular constant dependency" },

        // ── E3xx: Memory / Lifetime ───────────────────────────────────────────
        ErrorCodeEntry { code: "E302", numeric: 302, category: ErrorCategory::Memory, description: "affine value used in a loop" },
        ErrorCodeEntry { code: "E303", numeric: 303, category: ErrorCategory::Memory, description: "linear value not consumed exactly once" },
        ErrorCodeEntry { code: "E304", numeric: 304, category: ErrorCategory::Memory, description: "affine value used more than once" },
        ErrorCodeEntry { code: "E305", numeric: 305, category: ErrorCategory::Memory, description: "use of an uninitialized or partially initialized value" },
        ErrorCodeEntry { code: "E306", numeric: 306, category: ErrorCategory::Memory, description: "capability violation" },
        ErrorCodeEntry { code: "E310", numeric: 310, category: ErrorCategory::Memory, description: "use after move" },
        ErrorCodeEntry { code: "E311", numeric: 311, category: ErrorCategory::Memory, description: "double move" },
        ErrorCodeEntry { code: "E312", numeric: 312, category: ErrorCategory::Memory, description: "lifetime error" },
        ErrorCodeEntry { code: "E313", numeric: 313, category: ErrorCategory::Memory, description: "dangling reference" },
        ErrorCodeEntry { code: "E314", numeric: 314, category: ErrorCategory::Memory, description: "borrow conflict" },
        ErrorCodeEntry { code: "E315", numeric: 315, category: ErrorCategory::Memory, description: "use after free" },
        ErrorCodeEntry { code: "E316", numeric: 316, category: ErrorCategory::Memory, description: "double free" },
        ErrorCodeEntry { code: "E317", numeric: 317, category: ErrorCategory::Memory, description: "data race" },
        ErrorCodeEntry { code: "E318", numeric: 318, category: ErrorCategory::Memory, description: "potential deadlock" },
        ErrorCodeEntry { code: "E319", numeric: 319, category: ErrorCategory::Memory, description: "thread-safety violation" },
        ErrorCodeEntry { code: "E320", numeric: 320, category: ErrorCategory::Memory, description: "stack allocation exceeds the safe limit" },
        ErrorCodeEntry { code: "E321", numeric: 321, category: ErrorCategory::Memory, description: "unbounded recursion detected" },
        ErrorCodeEntry { code: "E370", numeric: 370, category: ErrorCategory::Memory, description: "positivity violation in a recursive type" },

        // ── E4xx: Type system ─────────────────────────────────────────────────
        ErrorCodeEntry { code: "E400", numeric: 400, category: ErrorCategory::Type, description: "type mismatch" },
        ErrorCodeEntry { code: "E401", numeric: 401, category: ErrorCategory::Type, description: "invalid cast" },
        ErrorCodeEntry { code: "E402", numeric: 402, category: ErrorCategory::Type, description: "Send bound not satisfied" },
        ErrorCodeEntry { code: "E403", numeric: 403, category: ErrorCategory::Type, description: "Sync bound not satisfied" },
        ErrorCodeEntry { code: "E404", numeric: 404, category: ErrorCategory::Type, description: "missing protocol implementation" },
        ErrorCodeEntry { code: "E405", numeric: 405, category: ErrorCategory::Type, description: "protocol method not implemented" },
        ErrorCodeEntry { code: "E406", numeric: 406, category: ErrorCategory::Type, description: "type inference failure" },
        ErrorCodeEntry { code: "E407", numeric: 407, category: ErrorCategory::Type, description: "recursive type without indirection" },
        ErrorCodeEntry { code: "E408", numeric: 408, category: ErrorCategory::Type, description: "dependent value-argument arity mismatch" },
        ErrorCodeEntry { code: "E409", numeric: 409, category: ErrorCategory::Type, description: "dereference of a non-reference type" },
        ErrorCodeEntry { code: "E410", numeric: 410, category: ErrorCategory::Type, description: "integer literal does not fit its type" },
        ErrorCodeEntry { code: "E411", numeric: 411, category: ErrorCategory::Type, description: "capability cannot be widened" },
        ErrorCodeEntry { code: "E412", numeric: 412, category: ErrorCategory::Type, description: "value is not a function" },
        ErrorCodeEntry { code: "E413", numeric: 413, category: ErrorCategory::Type, description: "const generic parameter mismatch" },
        ErrorCodeEntry { code: "E414", numeric: 414, category: ErrorCategory::Type, description: "ambiguous type: not inferable without more context" },
        ErrorCodeEntry { code: "E415", numeric: 415, category: ErrorCategory::Type, description: "name is not a type" },
        ErrorCodeEntry { code: "E416", numeric: 416, category: ErrorCategory::Type, description: "`?` used outside a function" },
        ErrorCodeEntry { code: "E417", numeric: 417, category: ErrorCategory::Type, description: "cycle among type definitions" },
        ErrorCodeEntry { code: "E418", numeric: 418, category: ErrorCategory::Type, description: "existential type escapes its scope" },
        ErrorCodeEntry { code: "E419", numeric: 419, category: ErrorCategory::Type, description: "existential bound not satisfied" },
        ErrorCodeEntry { code: "E420", numeric: 420, category: ErrorCategory::Type, description: "kind mismatch" },
        ErrorCodeEntry { code: "E421", numeric: 421, category: ErrorCategory::Type, description: "type constructor arity mismatch" },
        ErrorCodeEntry { code: "E422", numeric: 422, category: ErrorCategory::Type, description: "associated type cannot be resolved" },
        ErrorCodeEntry { code: "E423", numeric: 423, category: ErrorCategory::Type, description: "ambiguous associated type" },
        ErrorCodeEntry { code: "E424", numeric: 424, category: ErrorCategory::Type, description: "negative bound violated" },
        ErrorCodeEntry { code: "E425", numeric: 425, category: ErrorCategory::Type, description: "specialization overlap" },
        ErrorCodeEntry { code: "E426", numeric: 426, category: ErrorCategory::Type, description: "higher-kinded bound not satisfied" },

        // ── E5xx: Verification ────────────────────────────────────────────────
        ErrorCodeEntry { code: "E500", numeric: 500, category: ErrorCategory::Verification, description: "contract violated" },
        ErrorCodeEntry { code: "E501", numeric: 501, category: ErrorCategory::Verification, description: "SMT solver timeout" },
        ErrorCodeEntry { code: "E502", numeric: 502, category: ErrorCategory::Verification, description: "refinement predicate false" },
        ErrorCodeEntry { code: "E503", numeric: 503, category: ErrorCategory::Verification, description: "precondition not satisfied" },
        ErrorCodeEntry { code: "E504", numeric: 504, category: ErrorCategory::Verification, description: "postcondition not established" },
        ErrorCodeEntry { code: "E505", numeric: 505, category: ErrorCategory::Verification, description: "corecursive function is non-productive" },
        ErrorCodeEntry { code: "E506", numeric: 506, category: ErrorCategory::Verification, description: "meta argument violates its refinement" },

        // ── E6xx: Context system ──────────────────────────────────────────────
        ErrorCodeEntry { code: "E600", numeric: 600, category: ErrorCategory::Context, description: "context not provided" },
        ErrorCodeEntry { code: "E601", numeric: 601, category: ErrorCategory::Context, description: "context conflict" },
        ErrorCodeEntry { code: "E602", numeric: 602, category: ErrorCategory::Context, description: "context cycle" },
        ErrorCodeEntry { code: "E603", numeric: 603, category: ErrorCategory::Context, description: "context mismatch" },
        ErrorCodeEntry { code: "E604", numeric: 604, category: ErrorCategory::Context, description: "context not allowed here" },
        ErrorCodeEntry { code: "E605", numeric: 605, category: ErrorCategory::Context, description: "undefined context" },
        ErrorCodeEntry { code: "E606", numeric: 606, category: ErrorCategory::Context, description: "context has no such method" },
        ErrorCodeEntry { code: "E607", numeric: 607, category: ErrorCategory::Context, description: "invalid sub-context" },
        ErrorCodeEntry { code: "E608", numeric: 608, category: ErrorCategory::Context, description: "excluded context used" },
        ErrorCodeEntry { code: "E609", numeric: 609, category: ErrorCategory::Context, description: "transitive negative-context violation" },
        ErrorCodeEntry { code: "E610", numeric: 610, category: ErrorCategory::Context, description: "non-context protocol in a `using` clause" },
        ErrorCodeEntry { code: "E611", numeric: 611, category: ErrorCategory::Context, description: "direct negative-context violation" },
        ErrorCodeEntry { code: "E612", numeric: 612, category: ErrorCategory::Context, description: "context alias conflict" },

        // ── E7xx: Async ───────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E700", numeric: 700, category: ErrorCategory::Async, description: "future cancelled unexpectedly" },
        ErrorCodeEntry { code: "E701", numeric: 701, category: ErrorCategory::Async, description: "async boundary violation" },
        ErrorCodeEntry { code: "E702", numeric: 702, category: ErrorCategory::Async, description: "task join error" },

        // ── E8xx: FFI ─────────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E800", numeric: 800, category: ErrorCategory::Ffi, description: "unsafe FFI violation" },
        ErrorCodeEntry { code: "E801", numeric: 801, category: ErrorCategory::Ffi, description: "ABI mismatch" },
        ErrorCodeEntry { code: "E802", numeric: 802, category: ErrorCategory::Ffi, description: "null pointer dereference in FFI" },
        ErrorCodeEntry { code: "E803", numeric: 803, category: ErrorCategory::Ffi, description: "invalid type for an inline-assembly const operand" },
        ErrorCodeEntry { code: "E804", numeric: 804, category: ErrorCategory::Ffi, description: "inline-assembly output operand is not an lvalue" },
        ErrorCodeEntry { code: "E808", numeric: 808, category: ErrorCategory::Ffi, description: "duplicate `provide` for one context" },

        // ── E9xx: Internal ────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E900", numeric: 900, category: ErrorCategory::Internal, description: "internal compiler error" },
        ErrorCodeEntry { code: "E901", numeric: 901, category: ErrorCategory::Internal, description: "compiler assertion failed" },
        ErrorCodeEntry { code: "E902", numeric: 902, category: ErrorCategory::Internal, description: "unexpected compiler state" },

        // ── Four-digit codes ──────────────────────────────────────────────────
        //
        // A SECOND numbering lives alongside the three-digit one above, and
        // registering it here is not an endorsement of the split — it is what
        // makes the split visible and keeps the two from colliding silently.
        // Note that `numeric` cannot separate them: "E0900" and "E900" both
        // parse to 900. The map is keyed by the CODE STRING, so lookup stays
        // exact; do not reintroduce numeric-keyed lookup.
        ErrorCodeEntry { code: "E0203", numeric: 203, category: ErrorCategory::Type, description: "`?` operand error type does not convert to the function's" },
        ErrorCodeEntry { code: "E0205", numeric: 205, category: ErrorCategory::Type, description: "`?` used on a type that cannot carry failure" },
        ErrorCodeEntry { code: "E0307", numeric: 307, category: ErrorCategory::Type, description: "advanced protocol constraint unsatisfied" },
        ErrorCodeEntry { code: "E0308", numeric: 308, category: ErrorCategory::Type, description: "ambiguous specialization" },
        ErrorCodeEntry { code: "E0309", numeric: 309, category: ErrorCategory::Type, description: "protocol coherence violation" },
        ErrorCodeEntry { code: "E0310", numeric: 310, category: ErrorCategory::Type, description: "associated-type projection failure" },
        ErrorCodeEntry { code: "E0311", numeric: 311, category: ErrorCategory::Type, description: "protocol bound not provable" },
        ErrorCodeEntry { code: "E0312", numeric: 312, category: ErrorCategory::Verification, description: "refinement constraint not satisfied (refinement path)" },
        ErrorCodeEntry { code: "E0317", numeric: 317, category: ErrorCategory::Type, description: "unused value that must be handled" },
        ErrorCodeEntry { code: "E0401", numeric: 401, category: ErrorCategory::Module, description: "stdlib bootstrap could not resolve a module surface" },
        ErrorCodeEntry { code: "E0500", numeric: 500, category: ErrorCategory::Lint, description: "denied lint (general)" },
        ErrorCodeEntry { code: "E0700", numeric: 700, category: ErrorCategory::Internal, description: "VBC codegen phase received the wrong input form" },
        ErrorCodeEntry { code: "E0800", numeric: 800, category: ErrorCategory::Internal, description: "VBC monomorphization received the wrong input form" },
        ErrorCodeEntry { code: "E0801", numeric: 801, category: ErrorCategory::Internal, description: "VBC monomorphization failed" },
        ErrorCodeEntry { code: "E0900", numeric: 900, category: ErrorCategory::Lint, description: "denied lint: unstable intrinsic" },
        ErrorCodeEntry { code: "E0901", numeric: 901, category: ErrorCategory::Lint, description: "denied lint: intrinsic argument count" },
        ErrorCodeEntry { code: "E0902", numeric: 902, category: ErrorCategory::Lint, description: "denied lint: intrinsic argument type" },
        ErrorCodeEntry { code: "E0903", numeric: 903, category: ErrorCategory::Lint, description: "denied lint: intrinsic protocol bound" },
        ErrorCodeEntry { code: "E0904", numeric: 904, category: ErrorCategory::Lint, description: "denied lint: intrinsic not available on this platform" },
        ErrorCodeEntry { code: "E0905", numeric: 905, category: ErrorCategory::Lint, description: "denied lint: intrinsic const-eval" },
        ErrorCodeEntry { code: "E0906", numeric: 906, category: ErrorCategory::Lint, description: "denied lint: deprecated intrinsic" },
        ErrorCodeEntry { code: "E1000", numeric: 1000, category: ErrorCategory::Lint, description: "denied lint: unused stage" },
        ErrorCodeEntry { code: "E1001", numeric: 1001, category: ErrorCategory::Lint, description: "denied lint: stage mismatch in a quote expression" },
        ErrorCodeEntry { code: "E1002", numeric: 1002, category: ErrorCategory::Lint, description: "denied lint: cross-stage function call" },
        ErrorCodeEntry { code: "E1003", numeric: 1003, category: ErrorCategory::Lint, description: "denied lint: stage overflow" },
        ErrorCodeEntry { code: "E1004", numeric: 1004, category: ErrorCategory::Lint, description: "denied lint: cyclic stage dependency" },
        ErrorCodeEntry { code: "E1005", numeric: 1005, category: ErrorCategory::Lint, description: "denied lint: invalid stage escape" },
    ];

    let mut map = HashMap::with_capacity(entries.len());
    for entry in entries {
        map.insert(entry.code, entry.clone());
    }
    map
});

/// Look up an error code entry.
///
/// Returns `None` if the code is not in the registry.
pub fn lookup(code: &str) -> Option<&'static ErrorCodeEntry> {
    REGISTRY.get(code)
}

/// Return all entries for a given category, sorted by numeric code.
pub fn by_category(category: ErrorCategory) -> Vec<&'static ErrorCodeEntry> {
    let mut entries: Vec<&ErrorCodeEntry> = REGISTRY
        .values()
        .filter(|e| e.category == category)
        .collect();
    entries.sort_by_key(|e| e.numeric);
    entries
}

/// Total number of registered error codes.
pub fn count() -> usize {
    REGISTRY.len()
}

/// Returns true iff the given string is a known error code.
pub fn is_known(code: &str) -> bool {
    REGISTRY.contains_key(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e400_type_mismatch_is_registered() {
        let entry = lookup("E400").expect("E400 must be registered");
        assert_eq!(entry.code, "E400");
        assert_eq!(entry.category, ErrorCategory::Type);
        assert!(entry.description.contains("type mismatch") || entry.description.contains("mismatch"));
    }

    #[test]
    fn e001_parse_error_is_registered() {
        let entry = lookup("E001").expect("E001 must be registered");
        assert_eq!(entry.category, ErrorCategory::Parse);
    }

    #[test]
    fn e100_name_resolution_is_registered() {
        let entry = lookup("E100").expect("E100 must be registered");
        assert_eq!(entry.category, ErrorCategory::NameResolution);
    }

    #[test]
    fn e500_verification_is_registered() {
        let entry = lookup("E500").expect("E500 must be registered");
        assert_eq!(entry.category, ErrorCategory::Verification);
    }

    #[test]
    fn e900_internal_is_registered() {
        let entry = lookup("E900").expect("E900 must be registered");
        assert_eq!(entry.category, ErrorCategory::Internal);
    }

    #[test]
    fn unknown_code_returns_none() {
        assert!(lookup("E999").is_none());
        assert!(lookup("X000").is_none());
    }

    #[test]
    fn registry_has_at_least_30_entries() {
        assert!(count() >= 30, "registry must have ≥ 30 entries, got {}", count());
    }

    #[test]
    fn all_categories_have_at_least_one_entry() {
        let categories = [
            ErrorCategory::Parse,
            ErrorCategory::NameResolution,
            ErrorCategory::Module,
            ErrorCategory::Memory,
            ErrorCategory::Type,
            ErrorCategory::Verification,
            ErrorCategory::Context,
            ErrorCategory::Async,
            ErrorCategory::Ffi,
            ErrorCategory::Internal,
        ];
        for cat in categories {
            let entries = by_category(cat);
            assert!(!entries.is_empty(), "category '{:?}' must have ≥ 1 entry", cat);
        }
    }

    #[test]
    fn by_category_type_includes_e400() {
        let type_errors = by_category(ErrorCategory::Type);
        assert!(
            type_errors.iter().any(|e| e.code == "E400"),
            "by_category(Type) must include E400"
        );
    }

    #[test]
    fn range_prefix_is_correct() {
        let e400 = lookup("E400").unwrap();
        assert_eq!(e400.range_prefix(), 4);

        let e001 = lookup("E001").unwrap();
        assert_eq!(e001.range_prefix(), 0);

        let e900 = lookup("E900").unwrap();
        assert_eq!(e900.range_prefix(), 9);
    }

    #[test]
    fn is_known_returns_true_for_e312() {
        assert!(is_known("E312"));
    }

    #[test]
    fn is_known_returns_false_for_garbage() {
        assert!(!is_known("GARBAGE"));
        assert!(!is_known(""));
    }

    #[test]
    fn category_labels_are_non_empty() {
        for cat in [
            ErrorCategory::Parse, ErrorCategory::Type, ErrorCategory::Internal,
        ] {
            assert!(!cat.label().is_empty());
        }
    }
}
