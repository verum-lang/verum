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
        // Codes the PARSER prints, enumerated from `ErrorCode::ALL`
        // in `verum_fast_parser` rather than grepped for: the gate
        // could not see them, so 138 codes a user could meet had no
        // entry, and the six that DID collide described something
        // else entirely — `E001` read "unexpected token" here while
        // the compiler printed it for an unterminated character
        // literal (T0973).
        ErrorCodeEntry { code: "E010", numeric: 16, category: ErrorCategory::Parse, description: "missing semicolon" },
        ErrorCodeEntry { code: "E011", numeric: 17, category: ErrorCategory::Parse, description: "unclosed attribute or stmt unclosed block" },
        ErrorCodeEntry { code: "E012", numeric: 18, category: ErrorCategory::Parse, description: "invalid attribute args or stmt unclosed call" },
        ErrorCodeEntry { code: "E013", numeric: 19, category: ErrorCategory::Parse, description: "missing attribute name" },
        ErrorCodeEntry { code: "E014", numeric: 20, category: ErrorCategory::Parse, description: "invalid nested attribute" },
        ErrorCodeEntry { code: "E015", numeric: 21, category: ErrorCategory::Parse, description: "invalid empty cfg attribute" },
        ErrorCodeEntry { code: "E016", numeric: 22, category: ErrorCategory::Parse, description: "invalid empty requires clause" },
        ErrorCodeEntry { code: "E017", numeric: 23, category: ErrorCategory::Parse, description: "invalid empty ensures clause" },
        ErrorCodeEntry { code: "E018", numeric: 24, category: ErrorCategory::Parse, description: "unexpected token" },
        ErrorCodeEntry { code: "E019", numeric: 25, category: ErrorCategory::Parse, description: "missing block after control flow" },
        ErrorCodeEntry { code: "E020", numeric: 32, category: ErrorCategory::Parse, description: "invalid theorem declaration" },
        ErrorCodeEntry { code: "E021", numeric: 33, category: ErrorCategory::Parse, description: "missing theorem name" },
        ErrorCodeEntry { code: "E022", numeric: 34, category: ErrorCategory::Parse, description: "invalid lemma declaration" },
        ErrorCodeEntry { code: "E023", numeric: 35, category: ErrorCategory::Parse, description: "unclosed forall quantifier" },
        ErrorCodeEntry { code: "E024", numeric: 36, category: ErrorCategory::Parse, description: "unclosed exists quantifier" },
        ErrorCodeEntry { code: "E025", numeric: 37, category: ErrorCategory::Parse, description: "invalid proof keyword" },
        ErrorCodeEntry { code: "E026", numeric: 38, category: ErrorCategory::Parse, description: "invalid assert expression" },
        ErrorCodeEntry { code: "E027", numeric: 39, category: ErrorCategory::Parse, description: "invalid assume expression" },
        ErrorCodeEntry { code: "E028", numeric: 40, category: ErrorCategory::Parse, description: "malformed tactic" },
        ErrorCodeEntry { code: "E029", numeric: 41, category: ErrorCategory::Parse, description: "proof block not terminated" },
        ErrorCodeEntry { code: "E030", numeric: 48, category: ErrorCategory::Parse, description: "missing function name" },
        ErrorCodeEntry { code: "E031", numeric: 49, category: ErrorCategory::Parse, description: "missing function parameter list" },
        ErrorCodeEntry { code: "E032", numeric: 50, category: ErrorCategory::Parse, description: "missing function body" },
        ErrorCodeEntry { code: "E033", numeric: 51, category: ErrorCategory::Parse, description: "invalid function visibility" },
        ErrorCodeEntry { code: "E034", numeric: 52, category: ErrorCategory::Parse, description: "duplicate function modifier" },
        ErrorCodeEntry { code: "E035", numeric: 53, category: ErrorCategory::Parse, description: "invalid function parameter" },
        ErrorCodeEntry { code: "E036", numeric: 54, category: ErrorCategory::Parse, description: "missing parameter type" },
        ErrorCodeEntry { code: "E037", numeric: 55, category: ErrorCategory::Parse, description: "invalid return type syntax" },
        ErrorCodeEntry { code: "E038", numeric: 56, category: ErrorCategory::Parse, description: "invalid where clause syntax" },
        ErrorCodeEntry { code: "E039", numeric: 57, category: ErrorCategory::Parse, description: "invalid using clause syntax" },
        ErrorCodeEntry { code: "E040", numeric: 64, category: ErrorCategory::Parse, description: "invalid throws clause or let missing pattern" },
        ErrorCodeEntry { code: "E041", numeric: 65, category: ErrorCategory::Parse, description: "missing generic close or let missing value" },
        ErrorCodeEntry { code: "E042", numeric: 66, category: ErrorCategory::Parse, description: "empty generic params or let missing equals" },
        ErrorCodeEntry { code: "E043", numeric: 67, category: ErrorCategory::Parse, description: "missing type name" },
        ErrorCodeEntry { code: "E044", numeric: 68, category: ErrorCategory::Parse, description: "missing type is or provide invalid" },
        ErrorCodeEntry { code: "E045", numeric: 69, category: ErrorCategory::Parse, description: "missing type body" },
        ErrorCodeEntry { code: "E046", numeric: 70, category: ErrorCategory::Parse, description: "invalid record field or assignment invalid" },
        ErrorCodeEntry { code: "E047", numeric: 71, category: ErrorCategory::Parse, description: "missing field type" },
        ErrorCodeEntry { code: "E048", numeric: 72, category: ErrorCategory::Parse, description: "invalid variant syntax" },
        ErrorCodeEntry { code: "E049", numeric: 73, category: ErrorCategory::Parse, description: "duplicate field name" },
        ErrorCodeEntry { code: "E050", numeric: 80, category: ErrorCategory::Parse, description: "invalid generic constraint" },
        ErrorCodeEntry { code: "E051", numeric: 81, category: ErrorCategory::Parse, description: "missing protocol opening brace" },
        ErrorCodeEntry { code: "E052", numeric: 82, category: ErrorCategory::Parse, description: "invalid protocol method" },
        ErrorCodeEntry { code: "E053", numeric: 83, category: ErrorCategory::Parse, description: "invalid refinement syntax" },
        ErrorCodeEntry { code: "E054", numeric: 84, category: ErrorCategory::Parse, description: "missing impl type" },
        ErrorCodeEntry { code: "E055", numeric: 85, category: ErrorCategory::Parse, description: "missing 'for' in trait impl" },
        ErrorCodeEntry { code: "E056", numeric: 86, category: ErrorCategory::Parse, description: "invalid impl method" },
        ErrorCodeEntry { code: "E057", numeric: 87, category: ErrorCategory::Parse, description: "missing impl opening brace" },
        ErrorCodeEntry { code: "E058", numeric: 88, category: ErrorCategory::Parse, description: "missing context name" },
        ErrorCodeEntry { code: "E059", numeric: 89, category: ErrorCategory::Parse, description: "missing context body" },
        ErrorCodeEntry { code: "E060", numeric: 96, category: ErrorCategory::Parse, description: "invalid context method" },
        ErrorCodeEntry { code: "E061", numeric: 97, category: ErrorCategory::Parse, description: "missing module name" },
        ErrorCodeEntry { code: "E062", numeric: 98, category: ErrorCategory::Parse, description: "missing module opening brace" },
        ErrorCodeEntry { code: "E063", numeric: 99, category: ErrorCategory::Parse, description: "invalid link syntax" },
        ErrorCodeEntry { code: "E064", numeric: 100, category: ErrorCategory::Parse, description: "invalid pub use syntax" },
        ErrorCodeEntry { code: "E065", numeric: 101, category: ErrorCategory::Parse, description: "missing const type" },
        ErrorCodeEntry { code: "E066", numeric: 102, category: ErrorCategory::Parse, description: "missing const value" },
        ErrorCodeEntry { code: "E067", numeric: 103, category: ErrorCategory::Parse, description: "missing static type" },
        ErrorCodeEntry { code: "E068", numeric: 104, category: ErrorCategory::Parse, description: "invalid const/static expression" },
        ErrorCodeEntry { code: "E069", numeric: 105, category: ErrorCategory::Parse, description: "duplicate generic parameter name" },
        ErrorCodeEntry { code: "E070", numeric: 112, category: ErrorCategory::Parse, description: "unclosed array type" },
        ErrorCodeEntry { code: "E071", numeric: 113, category: ErrorCategory::Parse, description: "array missing size or pattern invalid identifier" },
        ErrorCodeEntry { code: "E072", numeric: 114, category: ErrorCategory::Parse, description: "array negative size or pattern invalid rest" },
        ErrorCodeEntry { code: "E073", numeric: 115, category: ErrorCategory::Parse, description: "array double semicolon or pattern invalid mut" },
        ErrorCodeEntry { code: "E074", numeric: 116, category: ErrorCategory::Parse, description: "array missing element or pattern empty tuple" },
        ErrorCodeEntry { code: "E075", numeric: 117, category: ErrorCategory::Parse, description: "unclosed capability or pattern invalid active args" },
        ErrorCodeEntry { code: "E076", numeric: 118, category: ErrorCategory::Parse, description: "empty capability or pattern invalid field" },
        ErrorCodeEntry { code: "E077", numeric: 119, category: ErrorCategory::Parse, description: "capability no with or pattern duplicate field" },
        ErrorCodeEntry { code: "E078", numeric: 120, category: ErrorCategory::Parse, description: "unclosed refinement or pattern nested or" },
        ErrorCodeEntry { code: "E079", numeric: 121, category: ErrorCategory::Parse, description: "refinement no base or pattern or binding" },
        ErrorCodeEntry { code: "E080", numeric: 128, category: ErrorCategory::Parse, description: "invalid int suffix or pattern invalid type" },
        ErrorCodeEntry { code: "E081", numeric: 129, category: ErrorCategory::Parse, description: "unclosed constraint generic or pattern invalid slice" },
        ErrorCodeEntry { code: "E082", numeric: 130, category: ErrorCategory::Parse, description: "empty generic args or pattern invalid unicode" },
        ErrorCodeEntry { code: "E083", numeric: 131, category: ErrorCategory::Parse, description: "double comma capability or pattern invalid variant args" },
        ErrorCodeEntry { code: "E084", numeric: 132, category: ErrorCategory::Parse, description: "trailing comma capability or pattern invalid and" },
        ErrorCodeEntry { code: "E085", numeric: 133, category: ErrorCategory::Parse, description: "double angle bracket or pattern trailing pipe" },
        ErrorCodeEntry { code: "E086", numeric: 134, category: ErrorCategory::Parse, description: "double ampersand ref or pattern invalid guard" },
        ErrorCodeEntry { code: "E087", numeric: 135, category: ErrorCategory::Parse, description: "ref without type or pattern invalid match arm" },
        ErrorCodeEntry { code: "E088", numeric: 136, category: ErrorCategory::Parse, description: "double checked ref or pattern invalid let" },
        ErrorCodeEntry { code: "E089", numeric: 137, category: ErrorCategory::Parse, description: "conflicting ref modifiers or pattern empty or" },
        ErrorCodeEntry { code: "E090", numeric: 144, category: ErrorCategory::Parse, description: "rank-2 function missing parameter list" },
        ErrorCodeEntry { code: "E091", numeric: 145, category: ErrorCategory::Parse, description: "unclosed function parameter list" },
        ErrorCodeEntry { code: "E092", numeric: 146, category: ErrorCategory::Parse, description: "function type missing return type" },
        ErrorCodeEntry { code: "E093", numeric: 147, category: ErrorCategory::Parse, description: "wrong arrow operator (=> instead of ->)" },
        ErrorCodeEntry { code: "E094", numeric: 148, category: ErrorCategory::Parse, description: "unclosed throws clause in function type" },
        ErrorCodeEntry { code: "E095", numeric: 149, category: ErrorCategory::Parse, description: "using clause without context list" },
        ErrorCodeEntry { code: "E096", numeric: 150, category: ErrorCategory::Parse, description: "async keyword in wrong position" },
        ErrorCodeEntry { code: "E097", numeric: 151, category: ErrorCategory::Parse, description: "unclosed tuple type" },
        ErrorCodeEntry { code: "E098", numeric: 152, category: ErrorCategory::Parse, description: "single element tuple invalid" },
        ErrorCodeEntry { code: "E099", numeric: 153, category: ErrorCategory::Parse, description: "unit type with content" },
        ErrorCodeEntry { code: "E0A0", numeric: 160, category: ErrorCategory::Parse, description: "throw without expression" },
        ErrorCodeEntry { code: "E0A1", numeric: 161, category: ErrorCategory::Parse, description: "finally clause without block" },
        ErrorCodeEntry { code: "E0A2", numeric: 162, category: ErrorCategory::Parse, description: "recover with malformed closure" },
        ErrorCodeEntry { code: "E0A3", numeric: 163, category: ErrorCategory::Parse, description: "invalid async block" },
        ErrorCodeEntry { code: "E0A4", numeric: 164, category: ErrorCategory::Parse, description: "invalid await expr" },
        ErrorCodeEntry { code: "E0A5", numeric: 165, category: ErrorCategory::Parse, description: "invalid select arm" },
        ErrorCodeEntry { code: "E0A6", numeric: 166, category: ErrorCategory::Parse, description: "invalid spawn expr" },
        ErrorCodeEntry { code: "E0A7", numeric: 167, category: ErrorCategory::Parse, description: "missing channel op" },
        ErrorCodeEntry { code: "E0A8", numeric: 168, category: ErrorCategory::Parse, description: "unclosed select" },
        ErrorCodeEntry { code: "E0A9", numeric: 169, category: ErrorCategory::Parse, description: "invalid break" },
        ErrorCodeEntry { code: "E0AA", numeric: 170, category: ErrorCategory::Parse, description: "invalid continue" },
        ErrorCodeEntry { code: "E0AB", numeric: 171, category: ErrorCategory::Parse, description: "invalid return" },
        ErrorCodeEntry { code: "E0AC", numeric: 172, category: ErrorCategory::Parse, description: "invalid yield" },
        ErrorCodeEntry { code: "E0B0", numeric: 176, category: ErrorCategory::Parse, description: "generic type args unclosed angle" },
        ErrorCodeEntry { code: "E0B1", numeric: 177, category: ErrorCategory::Parse, description: "turbofish missing type" },
        ErrorCodeEntry { code: "E0B2", numeric: 178, category: ErrorCategory::Parse, description: "tuple index invalid literal" },
        ErrorCodeEntry { code: "E0B3", numeric: 179, category: ErrorCategory::Parse, description: "invalid field access" },
        ErrorCodeEntry { code: "E0B4", numeric: 180, category: ErrorCategory::Parse, description: "invalid method call" },
        ErrorCodeEntry { code: "E0B5", numeric: 181, category: ErrorCategory::Parse, description: "invalid index expr" },
        ErrorCodeEntry { code: "E0B6", numeric: 182, category: ErrorCategory::Parse, description: "invalid call args" },
        ErrorCodeEntry { code: "E0B7", numeric: 183, category: ErrorCategory::Parse, description: "invalid closure" },
        ErrorCodeEntry { code: "E0B8", numeric: 184, category: ErrorCategory::Parse, description: "invalid match" },
        ErrorCodeEntry { code: "E0B9", numeric: 185, category: ErrorCategory::Parse, description: "invalid if" },
        ErrorCodeEntry { code: "E0BA", numeric: 186, category: ErrorCategory::Parse, description: "invalid for" },
        ErrorCodeEntry { code: "E0BB", numeric: 187, category: ErrorCategory::Parse, description: "invalid while" },
        ErrorCodeEntry { code: "E0BC", numeric: 188, category: ErrorCategory::Parse, description: "invalid loop" },
        ErrorCodeEntry { code: "E0BD", numeric: 189, category: ErrorCategory::Parse, description: "invalid range" },
        ErrorCodeEntry { code: "E0BE", numeric: 190, category: ErrorCategory::Parse, description: "invalid binary op" },
        ErrorCodeEntry { code: "E0BF", numeric: 191, category: ErrorCategory::Parse, description: "invalid unary op" },
        ErrorCodeEntry { code: "E0C0", numeric: 192, category: ErrorCategory::Parse, description: "tagged literal missing string" },
        ErrorCodeEntry { code: "E0C1", numeric: 193, category: ErrorCategory::Parse, description: "typeof expression without argument" },
        ErrorCodeEntry { code: "E0C2", numeric: 194, category: ErrorCategory::Parse, description: "forall missing dot before body" },
        ErrorCodeEntry { code: "E0C3", numeric: 195, category: ErrorCategory::Parse, description: "exists missing dot" },
        ErrorCodeEntry { code: "E0C4", numeric: 196, category: ErrorCategory::Parse, description: "invalid comprehension" },
        ErrorCodeEntry { code: "E0C5", numeric: 197, category: ErrorCategory::Parse, description: "invalid pipeline" },
        ErrorCodeEntry { code: "E0C6", numeric: 198, category: ErrorCategory::Parse, description: "invalid try expr" },
        ErrorCodeEntry { code: "E0C7", numeric: 199, category: ErrorCategory::Parse, description: "invalid defer" },
        ErrorCodeEntry { code: "E0C8", numeric: 200, category: ErrorCategory::Parse, description: "invalid provide" },
        ErrorCodeEntry { code: "E0C9", numeric: 201, category: ErrorCategory::Parse, description: "invalid let pattern" },
        ErrorCodeEntry { code: "E0D0", numeric: 208, category: ErrorCategory::Parse, description: "trailing separator" },
        ErrorCodeEntry { code: "E0D1", numeric: 209, category: ErrorCategory::Parse, description: "empty construct" },
        ErrorCodeEntry { code: "E0D2", numeric: 210, category: ErrorCategory::Parse, description: "duplicate clause" },
        ErrorCodeEntry { code: "E0D3", numeric: 211, category: ErrorCategory::Parse, description: "invalid splice" },
        ErrorCodeEntry { code: "E0D4", numeric: 212, category: ErrorCategory::Parse, description: "missing block expr" },
        ErrorCodeEntry { code: "E0D5", numeric: 213, category: ErrorCategory::Parse, description: "empty shape params" },
        ErrorCodeEntry { code: "E0E0", numeric: 224, category: ErrorCategory::Parse, description: "rust keyword used" },
        ErrorCodeEntry { code: "E0E1", numeric: 225, category: ErrorCategory::Parse, description: "rust type used" },
        ErrorCodeEntry { code: "E0E2", numeric: 226, category: ErrorCategory::Parse, description: "rust macro syntax" },
        ErrorCodeEntry { code: "E001", numeric: 1, category: ErrorCategory::Parse, description: "unterminated character literal" },
        ErrorCodeEntry { code: "E002", numeric: 2, category: ErrorCategory::Parse, description: "invalid escape sequence" },
        ErrorCodeEntry { code: "E003", numeric: 3, category: ErrorCategory::Parse, description: "invalid number literal" },
        ErrorCodeEntry { code: "E004", numeric: 4, category: ErrorCategory::Parse, description: "empty character literal" },
        ErrorCodeEntry { code: "E005", numeric: 5, category: ErrorCategory::Parse, description: "invalid interpolation syntax" },
        ErrorCodeEntry { code: "E006", numeric: 6, category: ErrorCategory::Parse, description: "unknown token/character" },
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
        ErrorCodeEntry { code: "E613", numeric: 613, category: ErrorCategory::Context, description: "context used but not declared in the function signature" },
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
        // Attribute subsystem (`verum_types::attr`), found the moment
        // the scan learned the enum-to-literal spelling (T0973).
        ErrorCodeEntry { code: "E0400", numeric: 1024, category: ErrorCategory::Type, description: "unknown attribute" },
        ErrorCodeEntry { code: "E0402", numeric: 1026, category: ErrorCategory::Type, description: "invalid attribute arguments" },
        ErrorCodeEntry { code: "E0403", numeric: 1027, category: ErrorCategory::Type, description: "duplicate attribute" },
        ErrorCodeEntry { code: "E0404", numeric: 1028, category: ErrorCategory::Type, description: "conflicting attributes" },
        ErrorCodeEntry { code: "E0405", numeric: 1029, category: ErrorCategory::Type, description: "attribute requirement not met" },
        ErrorCodeEntry { code: "E0406", numeric: 1030, category: ErrorCategory::Type, description: "attribute needs a feature gate" },
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
