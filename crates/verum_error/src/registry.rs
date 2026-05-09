//! Structured error-code registry for the Verum error system (#63).
//!
//! Maps numeric error codes (E0xx–E9xx) to their category, severity,
//! and a short description.  This is the single source of truth for
//! what each error code means at the runtime level; the richer
//! diagnostics (examples, fix suggestions) live in `verum_diagnostics`.
//!
//! # Error code ranges
//!
//! | Range | Category | Examples |
//! |-------|----------|---------|
//! | E0xx  | Parse    | E001 unexpected token, E002 unterminated string |
//! | E1xx  | Name resolution | E100 undefined variable, E101 undefined type |
//! | E2xx  | Module   | E200 import not found, E201 circular import |
//! | E3xx  | Memory / Lifetime | E310 use-after-move, E312 lifetime error |
//! | E4xx  | Type system | E400 type mismatch, E401 invalid cast |
//! | E5xx  | Verification | E500 contract violated, E501 SMT timeout |
//! | E6xx  | Context system | E600 context not provided, E601 context conflict |
//! | E7xx  | Async | E700 future cancelled, E701 async boundary |
//! | E8xx  | FFI | E800 unsafe FFI violation, E801 ABI mismatch |
//! | E9xx  | Internal | E900 ICE, E901 compiler assertion failed |
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

        // ── E2xx: Module ──────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E200", numeric: 200, category: ErrorCategory::Module, description: "import not found" },
        ErrorCodeEntry { code: "E201", numeric: 201, category: ErrorCategory::Module, description: "circular import" },
        ErrorCodeEntry { code: "E202", numeric: 202, category: ErrorCategory::Module, description: "private item imported" },
        ErrorCodeEntry { code: "E203", numeric: 203, category: ErrorCategory::Module, description: "module not found" },

        // ── E3xx: Memory / Lifetime ───────────────────────────────────────────
        ErrorCodeEntry { code: "E310", numeric: 310, category: ErrorCategory::Memory, description: "use after move" },
        ErrorCodeEntry { code: "E311", numeric: 311, category: ErrorCategory::Memory, description: "double move" },
        ErrorCodeEntry { code: "E312", numeric: 312, category: ErrorCategory::Memory, description: "lifetime error" },
        ErrorCodeEntry { code: "E313", numeric: 313, category: ErrorCategory::Memory, description: "dangling reference" },
        ErrorCodeEntry { code: "E314", numeric: 314, category: ErrorCategory::Memory, description: "borrow conflict" },

        // ── E4xx: Type system ─────────────────────────────────────────────────
        ErrorCodeEntry { code: "E400", numeric: 400, category: ErrorCategory::Type, description: "type mismatch" },
        ErrorCodeEntry { code: "E401", numeric: 401, category: ErrorCategory::Type, description: "invalid cast" },
        ErrorCodeEntry { code: "E402", numeric: 402, category: ErrorCategory::Type, description: "Send bound not satisfied" },
        ErrorCodeEntry { code: "E403", numeric: 403, category: ErrorCategory::Type, description: "Sync bound not satisfied" },
        ErrorCodeEntry { code: "E404", numeric: 404, category: ErrorCategory::Type, description: "missing protocol implementation" },
        ErrorCodeEntry { code: "E405", numeric: 405, category: ErrorCategory::Type, description: "protocol method not implemented" },
        ErrorCodeEntry { code: "E406", numeric: 406, category: ErrorCategory::Type, description: "type inference failure" },
        ErrorCodeEntry { code: "E407", numeric: 407, category: ErrorCategory::Type, description: "recursive type without indirection" },

        // ── E5xx: Verification ────────────────────────────────────────────────
        ErrorCodeEntry { code: "E500", numeric: 500, category: ErrorCategory::Verification, description: "contract violated" },
        ErrorCodeEntry { code: "E501", numeric: 501, category: ErrorCategory::Verification, description: "SMT solver timeout" },
        ErrorCodeEntry { code: "E502", numeric: 502, category: ErrorCategory::Verification, description: "refinement predicate false" },
        ErrorCodeEntry { code: "E503", numeric: 503, category: ErrorCategory::Verification, description: "precondition not satisfied" },
        ErrorCodeEntry { code: "E504", numeric: 504, category: ErrorCategory::Verification, description: "postcondition not established" },

        // ── E6xx: Context system ──────────────────────────────────────────────
        ErrorCodeEntry { code: "E600", numeric: 600, category: ErrorCategory::Context, description: "context not provided" },
        ErrorCodeEntry { code: "E601", numeric: 601, category: ErrorCategory::Context, description: "context conflict" },
        ErrorCodeEntry { code: "E602", numeric: 602, category: ErrorCategory::Context, description: "context cycle" },

        // ── E7xx: Async ───────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E700", numeric: 700, category: ErrorCategory::Async, description: "future cancelled unexpectedly" },
        ErrorCodeEntry { code: "E701", numeric: 701, category: ErrorCategory::Async, description: "async boundary violation" },
        ErrorCodeEntry { code: "E702", numeric: 702, category: ErrorCategory::Async, description: "task join error" },

        // ── E8xx: FFI ─────────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E800", numeric: 800, category: ErrorCategory::Ffi, description: "unsafe FFI violation" },
        ErrorCodeEntry { code: "E801", numeric: 801, category: ErrorCategory::Ffi, description: "ABI mismatch" },
        ErrorCodeEntry { code: "E802", numeric: 802, category: ErrorCategory::Ffi, description: "null pointer dereference in FFI" },

        // ── E9xx: Internal ────────────────────────────────────────────────────
        ErrorCodeEntry { code: "E900", numeric: 900, category: ErrorCategory::Internal, description: "internal compiler error" },
        ErrorCodeEntry { code: "E901", numeric: 901, category: ErrorCategory::Internal, description: "compiler assertion failed" },
        ErrorCodeEntry { code: "E902", numeric: 902, category: ErrorCategory::Internal, description: "unexpected compiler state" },
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
