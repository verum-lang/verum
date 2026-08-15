//! Numeric coercions for the enveloped dispatch handlers — and the one place
//! that knows how often the interpreter answers with a zero it invented.
//!
//! # What these do, and the part that is a defect
//!
//! `arith_extended`, `math_extended` and `char_extended` each read their
//! operands through a coercion that accepts a `Value` and produces an `f64` /
//! `i64` / `char`.  Two of the three branches are legitimate: a float read as
//! a float, and an int widened to a float, are coercions the language
//! defines.
//!
//! The third branch is not.  When the operand is neither — a Unit, a nil, a
//! pointer, a string, a bool — the coercion returns **zero**, and the
//! arithmetic proceeds on a number the program never produced.  There is no
//! diagnostic, no trace, and the result is a plausible one: `0`, or whatever
//! the operation yields for zero.  A caller cannot tell it from a real
//! computation, and neither can a test whose oracle is the exit code.
//!
//! Measured 2026-08-15: **233 call sites** across the three handler files
//! reach these coercions, and the three files each carried a
//! byte-identical private copy of them (same md5), so the behaviour could
//! not be changed, counted, or reasoned about in one place.
//!
//! # Why this module exists rather than a fourth copy
//!
//! Three things follow from having ONE authority:
//!
//! * **It can be counted.**  [`substitutions`] reports how many times the
//!   interpreter invented a zero during a run.  Any number above nought is a
//!   list of places where a program got an answer instead of an error.
//! * **It can be located.**  `#[track_caller]` means the trace names the
//!   handler line that asked, with no argument threaded through 233 call
//!   sites.
//! * **It can be made strict.**  `VERUM_STRICT_VALUES=1` turns the invented
//!   zero into a hard stop, the same way `VERUM_STRICT_MONO=1` elevates a
//!   silent monomorphisation gap.  That is what makes the class *findable*:
//!   run a corpus under it and the survivors are the honest programs.
//!
//! Default behaviour is unchanged — zero is still returned, silently, unless
//! a variable is set.  A cost this wide is not something to flip blind; the
//! counter comes first, and the count decides.
//!
//! # Instruments
//!
//! | variable | effect |
//! |---|---|
//! | `VERUM_TRACE_LENIENT` | one line per invented value: what was asked for, what arrived, which handler line |
//! | `VERUM_STRICT_VALUES` | the invented value becomes a hard stop naming the same three things |

use std::panic::Location;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::value::Value;

/// How many times a coercion has invented a value this process.
static SUBSTITUTIONS: AtomicU64 = AtomicU64::new(0);

/// How many times a coercion ran at all.
static COERCIONS: AtomicU64 = AtomicU64::new(0);

/// `(coercions, substitutions)` so far.
///
/// Both numbers, never just the second one.  A report of "0 invented values"
/// is worth nothing without the first: zero out of zero means the handlers
/// were never reached and the instrument never ran, which reads exactly like
/// zero out of a million and means the opposite.  Measured 2026-08-15 across
/// 200 conformance specs, the substitution count was 0 — a figure that only
/// became evidence once the coercion count proved the path was live.
pub fn counters() -> (u64, u64) {
    (
        COERCIONS.load(Ordering::Relaxed),
        SUBSTITUTIONS.load(Ordering::Relaxed),
    )
}

/// Count of invented values so far.  Zero means every operand the enveloped
/// handlers read was a value the program actually produced — but see
/// [`counters`] before reading a zero as good news.
pub fn substitutions() -> u64 {
    SUBSTITUTIONS.load(Ordering::Relaxed)
}

/// Print `(coercions, substitutions)` at process exit under
/// `VERUM_TRACE_LENIENT`, so a sweep can read one line per run instead of
/// grepping for events that may legitimately never occur.
pub fn report_at_exit() {
    if std::env::var_os("VERUM_TRACE_LENIENT").is_none() {
        return;
    }
    let (total, invented) = counters();
    eprintln!("[lenient] coercions={total} invented={invented}");
}

/// A human-readable name for what actually arrived, so a trace line says
/// `wanted f64, got Unit` rather than printing a NaN-boxed bit pattern.
fn kind_of(v: &Value) -> &'static str {
    if v.is_nil() {
        "nil"
    } else if v.is_unit() {
        "Unit"
    } else if v.is_bool() {
        "Bool"
    } else if v.is_ptr() {
        "pointer"
    } else if v.is_cbgr_regref() {
        "CBGR ref"
    } else if v.is_tagged() {
        "tagged"
    } else {
        "unknown"
    }
}

#[cold]
#[track_caller]
fn invented(wanted: &'static str, got: Value) {
    SUBSTITUTIONS.fetch_add(1, Ordering::Relaxed);
    if std::env::var_os("VERUM_TRACE_LENIENT").is_some()
        || std::env::var_os("VERUM_STRICT_VALUES").is_some()
    {
        let at = Location::caller();
        let line = format!(
            "[lenient] wanted {}, got {} — substituted a zero, asked at {}:{}",
            wanted,
            kind_of(&got),
            at.file(),
            at.line()
        );
        if std::env::var_os("VERUM_STRICT_VALUES").is_some() {
            panic!(
                "{line}\nVERUM_STRICT_VALUES is set: an operand that is not a \
                 number is an error, not a zero."
            );
        }
        eprintln!("{line}");
    }
}

/// The operand as an `f64`.  Floats pass through, ints widen — both are
/// defined coercions.  Anything else is [`invented`].
#[inline]
#[track_caller]
pub(crate) fn f64_or_zero(v: Value) -> f64 {
    COERCIONS.fetch_add(1, Ordering::Relaxed);
    if v.is_float() {
        v.as_f64()
    } else if v.is_int() {
        v.as_i64() as f64
    } else {
        invented("f64", v);
        0.0
    }
}

/// The operand as an `i64`.  Ints pass through, floats truncate — the
/// truncation is lossy but defined.  Anything else is [`invented`].
#[inline]
#[track_caller]
pub(crate) fn i64_or_zero(v: Value) -> i64 {
    COERCIONS.fetch_add(1, Ordering::Relaxed);
    if v.is_int() {
        v.as_i64()
    } else if v.is_float() {
        v.as_f64() as i64
    } else {
        invented("i64", v);
        0
    }
}

/// The operand as a `char`, via its scalar value.  A code point outside the
/// Unicode scalar range is as much an invented answer as a non-number is, so
/// it takes the same path rather than silently becoming NUL.
#[inline]
#[track_caller]
pub(crate) fn char_or_nul(v: Value) -> char {
    let code = i64_or_zero(v);
    match u32::try_from(code).ok().and_then(char::from_u32) {
        Some(c) => c,
        None => {
            invented("char", v);
            '\0'
        }
    }
}
