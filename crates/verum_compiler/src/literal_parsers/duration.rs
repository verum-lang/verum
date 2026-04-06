//! Duration literal parser
//!
//! Parses and validates duration strings at compile-time.
//!
//! # Formats
//! - Simple: `5s`, `100ms`, `1h`, `30m`
//! - Combined: `1h30m`, `2h30m45s`, `1d12h`
//! - ISO 8601: `P1DT2H30M` (partial support)
//!
//! # Units
//! - `ns` - nanoseconds
//! - `us` or `μs` - microseconds
//! - `ms` - milliseconds
//! - `s` - seconds
//! - `m` - minutes
//! - `h` - hours
//! - `d` - days
//! - `w` - weeks
//!
//! # Example
//! ```verum
//! let timeout = duration#"5s"
//! let long_timeout = duration#"1h30m"
//! ```
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.

use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

/// Conversion factors to nanoseconds
const NS_PER_US: u64 = 1_000;
const NS_PER_MS: u64 = 1_000_000;
const NS_PER_S: u64 = 1_000_000_000;
const NS_PER_M: u64 = 60 * NS_PER_S;
const NS_PER_H: u64 = 60 * NS_PER_M;
const NS_PER_D: u64 = 24 * NS_PER_H;
const NS_PER_W: u64 = 7 * NS_PER_D;

/// Parse a duration string at compile-time
///
/// # Arguments
/// * `content` - The duration string to parse
/// * `span` - Source location for error reporting
///
/// # Returns
/// The duration in nanoseconds
///
/// # Errors
/// Returns a diagnostic if the duration format is invalid
pub fn parse_duration(
    content: &str,
    span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> Result<u64, Diagnostic> {
    let trimmed = content.trim();

    if trimmed.is_empty() {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("Empty duration literal")
            .help("Examples: 5s, 100ms, 1h30m")
            .build());
    }

    // Check for ISO 8601 format
    if trimmed.starts_with('P') {
        return parse_iso8601_duration(trimmed, span);
    }

    // Parse as simple format: 1h30m45s
    parse_simple_duration(trimmed, span)
}

/// Parse simple duration format: 1h30m45s
fn parse_simple_duration(input: &str, span: Span) -> Result<u64, Diagnostic> {
    let mut total_ns: u64 = 0;
    let mut current_num = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            current_num.push(c);
        } else {
            // Found a unit
            if current_num.is_empty() {
                return Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!(
                        "Invalid duration format: expected number before unit '{}'",
                        c
                    ))
                    .help("Examples: 5s, 100ms, 1h30m")
                    .build());
            }

            // Collect full unit (for multi-char units like ms, us, ns)
            let mut unit = String::new();
            unit.push(c);

            // Check for multi-character units
            while let Some(&next_c) = chars.peek() {
                if next_c.is_alphabetic() || next_c == 'μ' {
                    unit.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            // Parse the number
            let value: f64 = current_num.parse().map_err(|_| {
                DiagnosticBuilder::new(Severity::Error)
                    .message(format!("Invalid number in duration: '{}'", current_num))
                    .build()
            })?;

            // Convert to nanoseconds
            let ns = value_to_ns(value, &unit, span)?;

            // Add to total, checking for overflow
            total_ns = total_ns.checked_add(ns).ok_or_else(|| {
                DiagnosticBuilder::new(Severity::Error)
                    .message("Duration overflow: value exceeds maximum")
                    .build()
            })?;

            current_num.clear();
        }
    }

    // Handle trailing number without unit (assume seconds for compatibility)
    if !current_num.is_empty() {
        let value: f64 = current_num.parse().map_err(|_| {
            DiagnosticBuilder::new(Severity::Error)
                .message(format!("Invalid number in duration: '{}'", current_num))
                .build()
        })?;

        // Default to seconds if no unit specified
        let ns = value_to_ns(value, "s", span)?;
        total_ns = total_ns.checked_add(ns).ok_or_else(|| {
            DiagnosticBuilder::new(Severity::Error)
                .message("Duration overflow: value exceeds maximum")
                .build()
        })?;
    }

    if total_ns == 0 && !input.chars().all(|c| c == '0' || c == 's') {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("Invalid duration format")
            .help("Examples: 5s, 100ms, 1h30m")
            .build());
    }

    Ok(total_ns)
}

/// Convert a value with unit to nanoseconds
fn value_to_ns(value: f64, unit: &str, _span: Span) -> Result<u64, Diagnostic> {
    let multiplier = match unit.to_lowercase().as_str() {
        "ns" | "nano" | "nanos" | "nanosecond" | "nanoseconds" => 1,
        "us" | "μs" | "micro" | "micros" | "microsecond" | "microseconds" => NS_PER_US,
        "ms" | "milli" | "millis" | "millisecond" | "milliseconds" => NS_PER_MS,
        "s" | "sec" | "secs" | "second" | "seconds" => NS_PER_S,
        "m" | "min" | "mins" | "minute" | "minutes" => NS_PER_M,
        "h" | "hr" | "hrs" | "hour" | "hours" => NS_PER_H,
        "d" | "day" | "days" => NS_PER_D,
        "w" | "week" | "weeks" => NS_PER_W,
        _ => {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!("Unknown duration unit: '{}'", unit))
                .help("Valid units: ns, us/μs, ms, s, m, h, d, w")
                .build());
        }
    };

    // Convert to nanoseconds
    let ns = value * (multiplier as f64);

    if ns < 0.0 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("Negative duration not allowed")
            .build());
    }

    if ns > (u64::MAX as f64) {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("Duration overflow: value exceeds maximum")
            .build());
    }

    Ok(ns as u64)
}

/// Parse ISO 8601 duration format: P[n]Y[n]M[n]DT[n]H[n]M[n]S
fn parse_iso8601_duration(input: &str, _span: Span) -> Result<u64, Diagnostic> {
    if !input.starts_with('P') {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("ISO 8601 duration must start with 'P'")
            .help("Example: P1DT2H30M")
            .build());
    }

    let mut total_ns: u64 = 0;
    let mut current_num = String::new();
    let mut in_time_part = false;

    for c in input[1..].chars() {
        if c.is_ascii_digit() || c == '.' {
            current_num.push(c);
        } else if c == 'T' {
            // Switch to time portion
            in_time_part = true;
        } else {
            if current_num.is_empty() {
                continue; // Skip empty designators
            }

            let value: f64 = current_num.parse().map_err(|_| {
                DiagnosticBuilder::new(Severity::Error)
                    .message(format!(
                        "Invalid number in ISO 8601 duration: '{}'",
                        current_num
                    ))
                    .build()
            })?;

            let ns = match c {
                'Y' => {
                    // Years - approximate as 365.25 days
                    (value * 365.25 * NS_PER_D as f64) as u64
                }
                'M' if !in_time_part => {
                    // Months - approximate as 30.44 days
                    (value * 30.44 * NS_PER_D as f64) as u64
                }
                'W' => (value * NS_PER_W as f64) as u64,
                'D' => (value * NS_PER_D as f64) as u64,
                'H' => (value * NS_PER_H as f64) as u64,
                'M' if in_time_part => (value * NS_PER_M as f64) as u64,
                'S' => (value * NS_PER_S as f64) as u64,
                _ => {
                    return Err(DiagnosticBuilder::new(Severity::Error)
                        .message(format!("Unknown ISO 8601 duration designator: '{}'", c))
                        .help("Valid: Y (years), M (months/minutes), W (weeks), D (days), H (hours), S (seconds)")
                        .build());
                }
            };

            total_ns = total_ns.checked_add(ns).ok_or_else(|| {
                DiagnosticBuilder::new(Severity::Error)
                    .message("Duration overflow: value exceeds maximum")
                    .build()
            })?;

            current_num.clear();
        }
    }

    Ok(total_ns)
}

/// Format duration in human-readable form
pub fn format_duration(ns: u64) -> Text {
    if ns == 0 {
        return Text::from("0s");
    }

    let mut result = String::new();
    let mut remaining = ns;

    // Weeks
    if remaining >= NS_PER_W {
        let weeks = remaining / NS_PER_W;
        result.push_str(&format!("{}w", weeks));
        remaining %= NS_PER_W;
    }

    // Days
    if remaining >= NS_PER_D {
        let days = remaining / NS_PER_D;
        result.push_str(&format!("{}d", days));
        remaining %= NS_PER_D;
    }

    // Hours
    if remaining >= NS_PER_H {
        let hours = remaining / NS_PER_H;
        result.push_str(&format!("{}h", hours));
        remaining %= NS_PER_H;
    }

    // Minutes
    if remaining >= NS_PER_M {
        let mins = remaining / NS_PER_M;
        result.push_str(&format!("{}m", mins));
        remaining %= NS_PER_M;
    }

    // Seconds
    if remaining >= NS_PER_S {
        let secs = remaining / NS_PER_S;
        result.push_str(&format!("{}s", secs));
        remaining %= NS_PER_S;
    }

    // Milliseconds
    if remaining >= NS_PER_MS {
        let ms = remaining / NS_PER_MS;
        result.push_str(&format!("{}ms", ms));
        remaining %= NS_PER_MS;
    }

    // Microseconds
    if remaining >= NS_PER_US {
        let us = remaining / NS_PER_US;
        result.push_str(&format!("{}us", us));
        remaining %= NS_PER_US;
    }

    // Nanoseconds
    if remaining > 0 {
        result.push_str(&format!("{}ns", remaining));
    }

    Text::from(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_seconds() {
        let result = parse_duration("5s", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 5 * NS_PER_S);
    }

    #[test]
    fn test_simple_milliseconds() {
        let result = parse_duration("100ms", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 100 * NS_PER_MS);
    }

    #[test]
    fn test_simple_hours() {
        let result = parse_duration("2h", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2 * NS_PER_H);
    }

    #[test]
    fn test_combined_hours_minutes() {
        let result = parse_duration("1h30m", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), NS_PER_H + 30 * NS_PER_M);
    }

    #[test]
    fn test_combined_hours_minutes_seconds() {
        let result = parse_duration("2h30m45s", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            2 * NS_PER_H + 30 * NS_PER_M + 45 * NS_PER_S
        );
    }

    #[test]
    fn test_fractional() {
        let result = parse_duration("1.5s", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), (1.5 * NS_PER_S as f64) as u64);
    }

    #[test]
    fn test_iso8601_basic() {
        let result = parse_duration("P1D", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), NS_PER_D);
    }

    #[test]
    fn test_iso8601_time() {
        let result = parse_duration("PT2H30M", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2 * NS_PER_H + 30 * NS_PER_M);
    }

    #[test]
    fn test_iso8601_full() {
        let result = parse_duration("P1DT2H30M45S", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            NS_PER_D + 2 * NS_PER_H + 30 * NS_PER_M + 45 * NS_PER_S
        );
    }

    #[test]
    fn test_microseconds() {
        let result = parse_duration("500us", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 500 * NS_PER_US);
    }

    #[test]
    fn test_nanoseconds() {
        let result = parse_duration("1000ns", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 1000);
    }

    #[test]
    fn test_weeks() {
        let result = parse_duration("2w", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 2 * NS_PER_W);
    }

    #[test]
    fn test_days() {
        let result = parse_duration("3d", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 3 * NS_PER_D);
    }

    #[test]
    fn test_empty_error() {
        let result = parse_duration("", Span::default(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_unit_error() {
        let result = parse_duration("5x", Span::default(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_format_duration_simple() {
        assert_eq!(format_duration(5 * NS_PER_S).as_str(), "5s");
    }

    #[test]
    fn test_format_duration_combined() {
        assert_eq!(
            format_duration(NS_PER_H + 30 * NS_PER_M).as_str(),
            "1h30m"
        );
    }

    #[test]
    fn test_format_duration_zero() {
        assert_eq!(format_duration(0).as_str(), "0s");
    }
}
