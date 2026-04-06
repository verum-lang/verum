//! UUID literal parser
//!
//! Parses and validates UUID strings at compile-time.
//!
//! # Format
//! Supports standard UUID formats:
//! - Full: `550e8400-e29b-41d4-a716-446655440000`
//! - No dashes: `550e8400e29b41d4a716446655440000`
//! - Braced: `{550e8400-e29b-41d4-a716-446655440000}`
//! - URN: `urn:uuid:550e8400-e29b-41d4-a716-446655440000`
//!
//! # Example
//! ```verum
//! let id = uuid#"550e8400-e29b-41d4-a716-446655440000"
//! ```
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.

use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

/// A parsed UUID value
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUuid {
    /// Raw bytes of the UUID (16 bytes)
    pub bytes: [u8; 16],
    /// Version of the UUID (1-5 or 0 for nil)
    pub version: u8,
    /// Variant of the UUID
    pub variant: UuidVariant,
}

/// UUID variants per RFC 4122
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UuidVariant {
    /// Reserved, NCS backward compatibility
    Ncs,
    /// The variant specified in RFC 4122
    Rfc4122,
    /// Reserved, Microsoft backward compatibility
    Microsoft,
    /// Reserved for future definition
    Future,
}

/// Parse a UUID string at compile-time
///
/// # Arguments
/// * `content` - The UUID string to parse
/// * `span` - Source location for error reporting
///
/// # Returns
/// The validated UUID string (normalized to standard format)
///
/// # Errors
/// Returns a diagnostic if the UUID format is invalid
pub fn parse_uuid(
    content: &str,
    _span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> Result<String, Diagnostic> {
    let normalized = normalize_uuid(content)?;

    // Validate structure
    if normalized.len() != 36 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message(format!(
                "Invalid UUID format: expected 36 characters after normalization, got {}",
                normalized.len()
            ))
            .help("UUID format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx")
            .build());
    }

    // Check format: 8-4-4-4-12
    let parts: Vec<&str> = normalized.split('-').collect();
    if parts.len() != 5 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("Invalid UUID format: incorrect number of sections")
            .help("UUID should have 5 sections: 8-4-4-4-12 hex characters")
            .build());
    }

    let expected_lengths = [8, 4, 4, 4, 12];
    for (i, (part, expected_len)) in parts.iter().zip(expected_lengths.iter()).enumerate() {
        if part.len() != *expected_len {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Invalid UUID format: section {} has {} characters, expected {}",
                    i + 1,
                    part.len(),
                    expected_len
                ))
                .build());
        }

        // Validate hex characters
        if !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Invalid UUID format: section {} contains non-hexadecimal characters",
                    i + 1
                ))
                .build());
        }
    }

    // Parse to bytes to validate fully
    let bytes = parse_uuid_bytes(&normalized)?;

    // Extract and validate version (bits 12-15 of time_hi_and_version)
    let version = (bytes[6] >> 4) & 0x0f;
    if version > 5 && version != 0 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message(format!("Invalid UUID version: {} (expected 0-5)", version))
            .help("UUID versions: 1 (time-based), 2 (DCE security), 3 (MD5), 4 (random), 5 (SHA-1)")
            .build());
    }

    // Extract and validate variant
    let _variant = match (bytes[8] >> 6) & 0x03 {
        0b00 | 0b01 => UuidVariant::Ncs,
        0b10 => UuidVariant::Rfc4122,
        0b11 => {
            if (bytes[8] >> 5) & 0x01 == 0 {
                UuidVariant::Microsoft
            } else {
                UuidVariant::Future
            }
        }
        _ => unreachable!(),
    };

    Ok(normalized)
}

/// Normalize various UUID formats to standard format
fn normalize_uuid(input: &str) -> Result<String, Diagnostic> {
    let trimmed = input.trim();

    // Remove common prefixes
    let content = if trimmed.starts_with("urn:uuid:") {
        &trimmed[9..]
    } else if trimmed.starts_with('{') && trimmed.ends_with('}') {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };

    // Handle no-dash format
    if content.len() == 32 && !content.contains('-') {
        // Insert dashes
        let mut normalized = String::with_capacity(36);
        normalized.push_str(&content[0..8]);
        normalized.push('-');
        normalized.push_str(&content[8..12]);
        normalized.push('-');
        normalized.push_str(&content[12..16]);
        normalized.push('-');
        normalized.push_str(&content[16..20]);
        normalized.push('-');
        normalized.push_str(&content[20..32]);
        return Ok(normalized.to_lowercase());
    }

    // Already in standard format
    if content.len() == 36 && content.matches('-').count() == 4 {
        return Ok(content.to_lowercase());
    }

    Err(DiagnosticBuilder::new(Severity::Error)
        .message(format!(
            "Invalid UUID format: expected 32 hex chars (no dashes) or 36 chars (with dashes), got {}",
            content.len()
        ))
        .help("Examples: 550e8400-e29b-41d4-a716-446655440000 or 550e8400e29b41d4a716446655440000")
        .build())
}

/// Parse UUID string to raw bytes
fn parse_uuid_bytes(normalized: &str) -> Result<[u8; 16], Diagnostic> {
    let hex_str: String = normalized.chars().filter(|c| *c != '-').collect();

    if hex_str.len() != 32 {
        return Err(DiagnosticBuilder::new(Severity::Error)
            .message("Internal error: normalized UUID should have 32 hex characters")
            .build());
    }

    let mut bytes = [0u8; 16];
    for i in 0..16 {
        let hex_byte = &hex_str[i * 2..i * 2 + 2];
        bytes[i] = u8::from_str_radix(hex_byte, 16).map_err(|_| {
            DiagnosticBuilder::new(Severity::Error)
                .message(format!("Invalid hex byte: {}", hex_byte))
                .build()
        })?;
    }

    Ok(bytes)
}

/// Generate a nil UUID
pub fn nil_uuid() -> Text {
    Text::from("00000000-0000-0000-0000-000000000000")
}

/// Check if a UUID is nil (all zeros)
pub fn is_nil(uuid: &str) -> bool {
    uuid.chars().filter(|c| *c != '-').all(|c| c == '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_uuid_standard_format() {
        let result = parse_uuid(
            "550e8400-e29b-41d4-a716-446655440000",
            Span::default(),
            None,
        );
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().as_str(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_valid_uuid_no_dashes() {
        let result = parse_uuid("550e8400e29b41d4a716446655440000", Span::default(), None);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().as_str(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_valid_uuid_braced() {
        let result = parse_uuid(
            "{550e8400-e29b-41d4-a716-446655440000}",
            Span::default(),
            None,
        );
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().as_str(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_valid_uuid_urn() {
        let result = parse_uuid(
            "urn:uuid:550e8400-e29b-41d4-a716-446655440000",
            Span::default(),
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_valid_uuid_uppercase() {
        let result = parse_uuid(
            "550E8400-E29B-41D4-A716-446655440000",
            Span::default(),
            None,
        );
        assert!(result.is_ok());
        // Should be normalized to lowercase
        assert_eq!(
            result.unwrap().as_str(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_nil_uuid() {
        let result = parse_uuid(
            "00000000-0000-0000-0000-000000000000",
            Span::default(),
            None,
        );
        assert!(result.is_ok());
        assert!(is_nil(result.unwrap().as_str()));
    }

    #[test]
    fn test_invalid_uuid_too_short() {
        let result = parse_uuid("550e8400-e29b-41d4-a716", Span::default(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_uuid_bad_chars() {
        let result = parse_uuid(
            "550e8400-e29b-41d4-a716-44665544000g",
            Span::default(),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_uuid_wrong_sections() {
        let result = parse_uuid("550e8400-e29b41d4-a716-446655440000", Span::default(), None);
        assert!(result.is_err());
    }
}
