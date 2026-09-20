//! The registry of `@name(...)` forms in EXPRESSION position — one authority.
//!
//! # Why this lives in `verum_ast`
//!
//! Three places answer "is this `@name(...)` known, and what may it be
//! handed":
//!
//!   * `verum_fast_parser` — `@name(...)` in expression position;
//!   * `verum_types` — the typing of a meta-call;
//!   * `verum_vbc` codegen — the lowering of one.
//!
//! A FOURTH, `verum_fast_parser::attr_validation`, answers a DIFFERENT
//! question — `@name` on a DECLARATION, where the name is an attribute and
//! the interesting property is which targets it may attach to. Names overlap
//! (`@asm`, `@size_of`, `@with_params`, `@llvm_only`, `@vbc`, `@const`,
//! `@file`, `@line`, `@column`), but an attribute and a meta-call are not the
//! same construct and folding the two rosters together would assert they are.
//!
//! The three above used to carry private lists, and the lists disagreed. Measured
//! 2026-09-16, on the four probes of the row this module closes plus a
//! sweep of every name the parser admitted:
//!
//! | class                                     | count | behaviour |
//! |-------------------------------------------|-------|-----------|
//! | admitted and lowered                      | 20    | works |
//! | admitted by the parser, no lowering        | 31    | silent `nil` |
//! | typed by the checker, unknown to the parser| 3     | `nil` + E0410 |
//! | lowered by codegen, unknown to the parser  | 1     | works + E0410 |
//! | a user's declared `meta` macro              | all   | silent `nil` |
//! | a name that exists nowhere                  | all   | silent `nil` + E0410 |
//!
//! `nil` is indistinguishable from a legitimate answer, so every row below
//! the first was a program that ran and answered nothing. The cost is on
//! record: `core/io/file.vr` built its `fstat(2)` buffer with `@zeroed()`,
//! which typed as `Unit` — a zero-byte object handed to an FFI.
//!
//! `verum_ast` is the only crate all four judges already depend on, so the
//! table lives here and each of them reads it rather than restating it.
//!
//! # What a row asserts
//!
//! A row is a CONTRACT, not a permission: the name, how many arguments it
//! takes, what each argument must be, and whether the compiler can actually
//! lower it. `Status::Unimplemented` is a first-class answer — a name the
//! language reserves and the compiler cannot yet produce a value for must
//! be REFUSED by name, because the alternative is the `nil` above.
//!
//! Adding a lowering means flipping one row to `Implemented`; the gate
//! `check_meta_function_names.py` and the specs under
//! `vcs/specs/L0-critical/meta/` read the same table.

use verum_common::Text;

/// How many arguments a meta-function accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// Exactly this many.
    Exact(usize),
    /// Between `min` and `max` inclusive.
    Range(usize, usize),
    /// At least this many, no upper bound.
    AtLeast(usize),
}

impl Arity {
    /// Does `n` satisfy this contract?
    pub fn admits(self, n: usize) -> bool {
        match self {
            Arity::Exact(k) => n == k,
            Arity::Range(lo, hi) => n >= lo && n <= hi,
            Arity::AtLeast(k) => n >= k,
        }
    }

    /// How to spell the expectation in a diagnostic.
    pub fn describe(self) -> Text {
        match self {
            Arity::Exact(0) => Text::from("no arguments"),
            Arity::Exact(1) => Text::from("exactly 1 argument"),
            Arity::Exact(k) => Text::from(format!("exactly {k} arguments")),
            Arity::Range(lo, hi) => Text::from(format!("between {lo} and {hi} arguments")),
            Arity::AtLeast(1) => Text::from("at least 1 argument"),
            Arity::AtLeast(k) => Text::from(format!("at least {k} arguments")),
        }
    }
}

/// What an argument position must hold.
///
/// This is a SYNTACTIC expectation where it can be checked syntactically
/// (`TextLiteral`, `Ident`), and a coarse semantic one otherwise. It is
/// deliberately coarse: a meta-function argument is checked before ordinary
/// unification has run, so a fine-grained demand here would report on
/// unresolved type variables rather than on the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    /// Any expression; no constraint beyond arity.
    Any,
    /// `Int` or `Float` — the numeric meta-functions.
    Numeric,
    /// A `Float`-valued expression (integers coerce).
    Float,
    /// A literal `Text` — checked syntactically, so `@intrinsic(name)` with
    /// a bare identifier is caught at the call rather than inside lowering.
    TextLiteral,
    /// A bare identifier (an opcode name, a cfg key).
    Ident,
    /// A type name / type expression.
    TypeName,
    /// A `@cfg` predicate; validated by `crate::cfg::parse_cfg_predicate`.
    CfgPredicate,
}

/// Can the compiler lower this name to a value?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `verum_vbc` codegen has an arm for it; the call produces a value.
    Implemented,
    /// The language reserves the name and no lowering exists. A call must
    /// be REFUSED, naming the name — never typed and lowered to `nil`.
    Unimplemented,
}

/// One row of the registry.
#[derive(Debug, Clone, Copy)]
pub struct MetaFn {
    /// The name as written after `@`.
    pub name: &'static str,
    /// How many arguments it takes.
    pub arity: Arity,
    /// Positional argument expectations. Shorter than the accepted arity
    /// means the LAST entry repeats for the remaining positions; empty
    /// means every position is `ArgKind::Any`.
    pub args: &'static [ArgKind],
    /// Whether a call can be lowered.
    pub status: Status,
    /// Whether the grammar's `meta_function_name` production lists it.
    /// Names outside it reach expression position through `meta_call`,
    /// which admits any path — see `grammar/verum.ebnf` §2.16.
    pub in_grammar_list: bool,
}

impl MetaFn {
    /// The expectation for argument `i`, honouring the repeat rule.
    pub fn arg_kind(&self, i: usize) -> ArgKind {
        if self.args.is_empty() {
            return ArgKind::Any;
        }
        *self.args.get(i).unwrap_or_else(|| {
            self.args
                .last()
                .expect("args is non-empty in this branch")
        })
    }
}

/// The open namespace: `@builtin_*` carries its semantics and its return
/// type at the stdlib DECLARATION site rather than in a table here, so the
/// prefix is admitted whole. All four judges must encode this — the parser
/// did not until T1352, and nine correct `@builtin_*` calls in
/// `core/math/hott.vr` were reported as unknown.
pub const BUILTIN_PREFIX: &str = "builtin_";

/// Is `name` in the open `@builtin_*` namespace?
pub fn is_builtin_namespace(name: &str) -> bool {
    name.starts_with(BUILTIN_PREFIX)
}

use ArgKind::*;
use Arity::*;
use Status::*;

/// Shorthand for a row.
const fn row(
    name: &'static str,
    arity: Arity,
    args: &'static [ArgKind],
    status: Status,
    in_grammar_list: bool,
) -> MetaFn {
    MetaFn {
        name,
        arity,
        args,
        status,
        in_grammar_list,
    }
}

/// Every `@name` the compiler recognises in expression position.
///
/// Ordered by family, not alphabetically, so that a family's contract can
/// be read as a unit. `in_grammar_list` marks the twenty-one names the
/// grammar's `meta_function_name` production enumerates
/// (`grammar/verum.ebnf`); the rest are `meta_call` paths the compiler
/// implements, and the flag is what lets a diagnostic say which of the two
/// a reader is looking at.
pub static REGISTRY: &[MetaFn] = &[
    // ---- compile-time evaluation -------------------------------------
    row("const", AtLeast(1), &[Any], Implemented, true),
    // ---- compile-time diagnostics ------------------------------------
    row("error", Exact(1), &[TextLiteral], Implemented, true),
    row("warning", Exact(1), &[TextLiteral], Implemented, true),
    // ---- token manipulation ------------------------------------------
    row("stringify", Exact(1), &[Any], Implemented, true),
    // `@concat()` is the empty Text, not a mistake — the identity of the
    // fold. `L0-critical/builtin-syntax/meta_functions_edge_cases.vr`
    // spells it out under "Empty concat", and a contract that refused it
    // would be the check being wrong about correct code.
    row("concat", AtLeast(0), &[Any], Implemented, true),
    // ---- configuration -----------------------------------------------
    row("cfg", Exact(1), &[CfgPredicate], Implemented, true),
    // ---- source location ----------------------------------------------
    row("file", Exact(0), &[], Implemented, true),
    row("line", Exact(0), &[], Implemented, true),
    row("column", Exact(0), &[], Implemented, true),
    row("module", Exact(0), &[], Implemented, true),
    row("function", Exact(0), &[], Implemented, true),
    // ---- compile-time reflection --------------------------------------
    // Section 15 of the type-system notes reserves these; lowering needs
    // the checker's view of the argument threaded into codegen, which does
    // not exist. Refused by name rather than answered with `nil`.
    row("type_name", Exact(1), &[TypeName], Unimplemented, true),
    row("type_fields", Exact(1), &[TypeName], Unimplemented, true),
    row("type_of", Exact(1), &[Any], Unimplemented, true),
    row("fields_of", Exact(1), &[TypeName], Unimplemented, true),
    row("variants_of", Exact(1), &[TypeName], Unimplemented, true),
    row("field_access", Exact(2), &[Any, TextLiteral], Unimplemented, true),
    row("is_struct", Exact(1), &[TypeName], Unimplemented, true),
    row("is_enum", Exact(1), &[TypeName], Unimplemented, true),
    row("is_tuple", Exact(1), &[TypeName], Unimplemented, true),
    row("implements", Exact(2), &[TypeName, TypeName], Unimplemented, true),
    // `@size_of(T)` / `@align_of(T)` route to the SAME compile-time constant
    // the `@intrinsic("size_of", T)` spelling uses, so the two cannot
    // disagree. That constant is a PLACEHOLDER 8 for every type — a defect of
    // its own, recorded in the debt register — but a wrong number a reader can
    // see beats the `nil` these answered before, and `core/runtime/config.vr`
    // multiplies an allocation count by it.
    row("size_of", Exact(1), &[TypeName], Implemented, false),
    row("align_of", Exact(1), &[TypeName], Implemented, false),
    row("type_id", Exact(1), &[TypeName], Unimplemented, false),
    // ---- escape hatches into the backends -----------------------------
    row("intrinsic", AtLeast(1), &[TextLiteral, Any], Implemented, false),
    row("vbc", AtLeast(1), &[Ident, Any], Implemented, false),
    row("vbc_raw", AtLeast(1), &[Ident, Any], Unimplemented, false),
    row("mlir", AtLeast(1), &[TextLiteral, Any], Unimplemented, false),
    row("mlir_typed", AtLeast(1), &[TextLiteral, Any], Unimplemented, false),
    // `@asm("lfence")` is a STATEMENT with no value: 84 sites in `core/`
    // spell memory barriers and CPU hints with it. At Tier 0 the interpreter
    // cannot execute an instruction and does not need to — it is sequential —
    // so the lowering is "emit nothing, evaluate to unit". Tier 1 is where the
    // instruction has to appear and does not yet; that is named in the debt
    // register rather than hidden behind a `nil`.
    row("asm", AtLeast(1), &[TextLiteral, Any], Implemented, false),
    row("llvm", AtLeast(1), &[TextLiteral, Any], Implemented, false),
    // `@llvm_only(reason = "…")` is an ATTRIBUTE in every one of its 96
    // `core/` occurrences — first token on its line, attached to a
    // declaration — so `attr_validation.rs` judges those, not this table. The
    // row exists because the name is also grammatical in expression position,
    // where it means the same thing and lowers the same way as `@llvm`.
    row("llvm_only", AtLeast(1), &[TextLiteral, Any], Implemented, false),
    row("with_params", AtLeast(1), &[Any], Unimplemented, false),
    // ---- runtime introspection ----------------------------------------
    row("get_tag", Exact(1), &[Any], Implemented, false),
    row("has_gpu", Exact(0), &[], Unimplemented, false),
    // ---- numeric ------------------------------------------------------
    row("abs", Exact(1), &[Numeric], Implemented, false),
    row("sqrt", Exact(1), &[Float], Implemented, false),
    row("sin", Exact(1), &[Float], Implemented, false),
    row("cos", Exact(1), &[Float], Implemented, false),
    row("tan", Exact(1), &[Float], Implemented, false),
    row("log", Exact(1), &[Float], Implemented, false),
    row("exp", Exact(1), &[Float], Implemented, false),
    row("floor", Exact(1), &[Float], Implemented, false),
    row("ceil", Exact(1), &[Float], Implemented, false),
    row("round", Exact(1), &[Float], Implemented, false),
    row("min", Exact(2), &[Numeric, Numeric], Implemented, false),
    row("max", Exact(2), &[Numeric, Numeric], Implemented, false),
    row("clamp", Exact(3), &[Numeric, Numeric, Numeric], Implemented, false),
    row("pow", Exact(2), &[Numeric, Numeric], Implemented, false),
    // ---- error handling and async -------------------------------------
    row("catch", Exact(1), &[Any], Implemented, false),
    row("catch_cbgr_violation", Exact(1), &[Any], Implemented, false),
    row("block_on", Exact(1), &[Any], Implemented, false),
    // `Range(1, 2)` because BOTH spellings are live and the lowering
    // reads only the first argument: `@timeout(expr)` as a nursery
    // modifier (`nursery @timeout(100.ms) { … }`,
    // `L0-critical/vbc/async/004_nursery.vr`) and the two-argument call.
    // An `Exact(2)` here would have been a contract the code generator
    // does not keep — it moves `args[0]` to the destination and ignores
    // the rest.
    row("timeout", Range(1, 2), &[Any, Any], Implemented, false),
    // ---- memory --------------------------------------------------------
    row("forget", Exact(1), &[Any], Implemented, false),
    row("ref_eq", Exact(2), &[Any, Any], Implemented, false),
    row("get_generation", Exact(1), &[Any], Implemented, false),
    row("get_stored_generation", Exact(1), &[Any], Implemented, false),
    // ---- collections ----------------------------------------------------
    row("unwrap", Exact(1), &[Any], Implemented, false),
    row("list_with_capacity", Range(0, 1), &[Numeric], Implemented, false),
    row("byte_list_with_capacity", Range(0, 1), &[Numeric], Implemented, false),
];

/// The row for `name`, if the compiler recognises it.
///
/// `@builtin_*` is deliberately NOT answered here: it is an open namespace
/// with no arity contract to state, and callers test it with
/// [`is_builtin_namespace`] so the two cases stay visibly distinct.
pub fn lookup(name: &str) -> Option<&'static MetaFn> {
    REGISTRY.iter().find(|m| m.name == name)
}

/// Is `name` a form the compiler recognises at all (whether or not it can
/// lower it)?
pub fn is_recognised(name: &str) -> bool {
    is_builtin_namespace(name) || lookup(name).is_some()
}

/// Levenshtein distance, for "did you mean".
fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let (m, n) = (a_chars.len(), b_chars.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur = vec![0usize; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = usize::from(a_chars[i - 1] != b_chars[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
}

/// The closest recognised name to `unknown`, for a "did you mean" hint.
///
/// Prefix containment is tried only after edit distance fails, so
/// `@sqr` suggests `@sqrt` (distance 1) rather than the first row whose
/// name happens to start with `sq`.
pub fn similar(unknown: &str) -> Option<&'static str> {
    const MAX_DISTANCE: usize = 3;
    let mut best: Option<&'static str> = None;
    let mut best_distance = usize::MAX;
    for m in REGISTRY {
        let d = edit_distance(unknown, m.name);
        if d < best_distance && d <= MAX_DISTANCE {
            best_distance = d;
            best = Some(m.name);
        }
    }
    if best.is_none() {
        for m in REGISTRY {
            if m.name.starts_with(unknown) || unknown.starts_with(m.name) {
                return Some(m.name);
            }
        }
    }
    best
}

/// A short, stable list of names for a diagnostic's fallback hint.
///
/// The grammar's own enumeration, so a reader who follows the hint into
/// `grammar/verum.ebnf` finds the same names in the same order.
pub fn grammar_listed_names() -> impl Iterator<Item = &'static str> {
    REGISTRY
        .iter()
        .filter(|m| m.in_grammar_list)
        .map(|m| m.name)
}
