//! XML literal parser
//!
//! Tagged text literal parser: handles `tag#"content"` compile-time parsing
//! and validation. Tags are registered via @tagged_literal attribute.
//!
//! Parses and validates XML literals:
//! - xml#"<root><item>value</item></root>"

use quick_xml::Reader;
use verum_ast::Span;
use verum_common::Text;
use verum_diagnostics::{Diagnostic, DiagnosticBuilder};

use crate::literal_registry::ParsedLiteral;

/// Parse XML literal at compile-time
///
/// Semantic literal: `xml#"<root>...</root>"` is compile-time validated XML.
/// The tagged literal syntax `tag#"content"` desugars to a meta-system call
/// that parses and validates content at compile-time, producing type XmlDocument.
///
/// # Arguments
/// - `content`: The XML string
/// - `span`: Source location for error reporting
///
/// # Returns
/// Validated XML on success
pub fn parse_xml(
    content: &Text,
    _span: Span,
    _source_file: Option<&verum_ast::SourceFile>,
) -> std::result::Result<ParsedLiteral, Diagnostic> {
    let s = content.as_str().trim();

    if s.is_empty() {
        return Err(DiagnosticBuilder::error()
            .message("XML cannot be empty")
            .build());
    }

    // Validate XML syntax using quick-xml
    let mut reader = Reader::from_str(s);
    reader.config_mut().check_end_names = true;

    let mut buf: Vec<u8> = Vec::new();
    let mut tag_stack: Vec<Vec<u8>> = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Eof) => break,
            Ok(quick_xml::events::Event::Start(e)) => {
                // Push opening tag name onto stack
                tag_stack.push(e.name().as_ref().to_vec());
            }
            Ok(quick_xml::events::Event::End(e)) => {
                // Pop and verify matching tag
                if let Some(expected) = tag_stack.pop() {
                    if expected != e.name().as_ref() {
                        return Err(DiagnosticBuilder::error()
                            .message(format!(
                                "Mismatched closing tag: expected </{}>, found </{}>",
                                String::from_utf8_lossy(&expected),
                                String::from_utf8_lossy(e.name().as_ref())
                            ))
                            .build());
                    }
                } else {
                    return Err(DiagnosticBuilder::error()
                        .message(format!(
                            "Unexpected closing tag: </{}>",
                            String::from_utf8_lossy(e.name().as_ref())
                        ))
                        .build());
                }
            }
            Ok(quick_xml::events::Event::Empty(_)) => {
                // Self-closing tags don't affect the stack
            }
            Err(e) => {
                return Err(DiagnosticBuilder::error()
                    .message(format!("Invalid XML: {}", e))
                    .build());
            }
            _ => {}
        }
        buf.clear();
    }

    // Check for unclosed tags
    if !tag_stack.is_empty() {
        let unclosed: Vec<String> = tag_stack
            .iter()
            .map(|t| String::from_utf8_lossy(t).to_string())
            .collect();
        return Err(DiagnosticBuilder::error()
            .message(format!("Unclosed XML tags: {}", unclosed.join(", ")))
            .build());
    }

    Ok(ParsedLiteral::Xml(content.clone()))
}
