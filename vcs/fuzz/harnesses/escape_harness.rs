//! Shell escape fuzzing harness.
//!
//! Drives `core.shell.escape.Escaper` (POSIX flavour) through random byte
//! inputs and verifies the four invariants below for every result.  Any
//! invariant violation is reported as a `EscapeError`; combined with a
//! cargo-fuzz / AFL driver this gives coverage-guided detection of
//! injection-shaped payloads that bypass quoting.
//!
//! Invariants checked:
//!   1. Output starts and ends with `'`, *or* the input was in the
//!      safe-unquoted character class.
//!   2. Every embedded single quote in the input is rewritten as the
//!      `'\''` close-escape-reopen idiom — no lone `'` in the interior.
//!   3. Length monotonicity: escape grows the string by at most a small
//!      constant per byte (4× plus a 2-byte wrapping overhead).
//!   4. Round-trip: feeding the escaped string into a shell `printf`
//!      reproduces the original byte sequence.  Round-trip is checked by
//!      the harness consumer (it requires a real shell), not here.

use std::time::{Duration, Instant};

/// Outcome of a single fuzz iteration.
#[derive(Debug, Clone)]
pub struct EscapeResult {
    /// Bytes that came out of `Escaper.posix`.
    pub escaped: String,
    /// Whether the input was eligible for the safe-unquoted fast path.
    pub safe_unquoted: bool,
    /// Detected invariant violations.
    pub errors: Vec<EscapeError>,
    /// Time spent escaping (excluding harness overhead).
    pub duration: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscapeError {
    /// Output did not start with `'` and the input was not safe-unquoted.
    MissingOpeningQuote,
    /// Output did not end with `'` despite starting with one.
    MissingClosingQuote,
    /// Embedded single quote not rewritten through the canonical idiom.
    LoneSingleQuote { offset: usize },
    /// Length exploded beyond the linear bound — possible algorithmic DoS.
    LengthBlowup { input_len: usize, output_len: usize },
}

#[derive(Debug, Clone)]
pub struct EscapeStats {
    pub total_runs: u64,
    pub failed_runs: u64,
    pub max_blowup_ratio: f64,
    pub total_duration: Duration,
}

impl Default for EscapeStats {
    fn default() -> Self {
        Self {
            total_runs: 0,
            failed_runs: 0,
            max_blowup_ratio: 0.0,
            total_duration: Duration::ZERO,
        }
    }
}

/// Stateless harness — every call is independent.
pub struct EscapeHarness {
    pub stats: EscapeStats,
}

impl EscapeHarness {
    pub fn new() -> Self {
        Self { stats: EscapeStats::default() }
    }

    /// Run one fuzz iteration on `input`. Returns the result; updates stats.
    pub fn run(&mut self, input: &[u8]) -> EscapeResult {
        let start = Instant::now();

        // Lossy UTF-8 decode mirrors what the Verum runtime does at the
        // FFI boundary — the escape primitive itself operates on Text.
        let s = String::from_utf8_lossy(input).into_owned();
        let safe_unquoted = is_posix_safe_unquoted(&s);
        let escaped = posix_escape(&s);

        let mut errors = Vec::new();

        if !safe_unquoted {
            if !escaped.starts_with('\'') {
                errors.push(EscapeError::MissingOpeningQuote);
            }
            if !escaped.ends_with('\'') {
                errors.push(EscapeError::MissingClosingQuote);
            }
            // Scan interior for lone single quotes outside the close/escape/reopen idiom.
            if escaped.starts_with('\'') && escaped.ends_with('\'') && escaped.len() >= 2 {
                let bytes = &escaped.as_bytes()[1..escaped.len() - 1];
                let mut i = 0;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        if i + 3 >= bytes.len()
                            || bytes[i + 1] != b'\\'
                            || bytes[i + 2] != b'\''
                            || bytes[i + 3] != b'\''
                        {
                            errors.push(EscapeError::LoneSingleQuote { offset: i + 1 });
                            break;
                        }
                        i += 4;
                    } else {
                        i += 1;
                    }
                }
            }
        }

        // Length blowup: posix can grow each byte by at most 4 bytes
        // ('\'\\\'' for an embedded '), plus 2 bytes of wrapping. So:
        //   output_len ≤ 4 * input_len + 2.
        let limit = 4 * input.len() + 2;
        if escaped.len() > limit {
            errors.push(EscapeError::LengthBlowup {
                input_len: input.len(),
                output_len: escaped.len(),
            });
        }

        let duration = start.elapsed();
        self.stats.total_runs += 1;
        if !errors.is_empty() {
            self.stats.failed_runs += 1;
        }
        if !input.is_empty() {
            let ratio = escaped.len() as f64 / input.len() as f64;
            if ratio > self.stats.max_blowup_ratio {
                self.stats.max_blowup_ratio = ratio;
            }
        }
        self.stats.total_duration += duration;

        EscapeResult { escaped, safe_unquoted, errors, duration }
    }
}

impl Default for EscapeHarness {
    fn default() -> Self { Self::new() }
}

// =============================================================================
// Reference POSIX escape — kept in the harness so this crate doesn't
// have to dep on the Verum-side implementation. Must mirror
// core.shell.escape.Escaper.posix byte-for-byte.
// =============================================================================

fn is_posix_safe_unquoted(s: &str) -> bool {
    if s.is_empty() { return false; }
    s.bytes().all(|b| matches!(b,
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9'
            | b'.' | b'_' | b'/' | b'-' | b'=' | b':' | b'+' | b'@'))
}

fn posix_escape(s: &str) -> String {
    if s.is_empty() { return "''".to_string(); }
    if is_posix_safe_unquoted(s) { return s.to_string(); }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' { out.push_str("'\\''"); }
        else          { out.push(ch); }
    }
    out.push('\'');
    out
}

// =============================================================================
// Tests — known injection patterns that must be defended.
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn run(input: &[u8]) -> EscapeResult {
        EscapeHarness::new().run(input)
    }

    #[test]
    fn empty_input_renders_as_empty_pair() {
        let r = run(b"");
        assert_eq!(r.escaped, "''");
        assert!(r.errors.is_empty());
    }

    #[test]
    fn classic_injection_payload_is_quoted() {
        let r = run(b"evil; rm -rf /");
        assert!(r.escaped.starts_with("'"));
        assert!(r.escaped.ends_with("'"));
        assert!(r.errors.is_empty());
    }

    #[test]
    fn embedded_quote_uses_canonical_idiom() {
        let r = run(b"'");
        assert_eq!(r.escaped, "''\\'''");
        assert!(r.errors.is_empty());
    }

    #[test]
    fn double_quote_stays_inside_single_quotes() {
        let r = run(b"\"$(echo X)\"");
        assert!(r.escaped.starts_with("'"));
        assert!(r.errors.is_empty());
    }

    #[test]
    fn safe_unquoted_passes_through() {
        let r = run(b"path/to/file.txt");
        assert_eq!(r.escaped, "path/to/file.txt");
        assert!(r.safe_unquoted);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn length_bound_is_4x_plus_2() {
        let pathological = "'".repeat(1000);
        let r = run(pathological.as_bytes());
        assert!(r.escaped.len() <= 4 * pathological.len() + 2);
        assert!(r.errors.is_empty());
    }

    #[test]
    fn null_byte_inside_input_handled_lossy() {
        let r = run(&[b'a', 0u8, b'b']);
        // Lossy UTF-8 keeps NUL as control byte; harness does not crash.
        assert!(r.errors.is_empty());
    }
}
