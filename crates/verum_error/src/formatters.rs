//! Error Output Formatters
//!
//! Provides multiple output formats for error contexts, enabling integration
//! with various logging, monitoring, and debugging systems.
//!
//! # Supported Formats
//!
//! - **PlainText**: Human-readable text format (default)
//! - **Json**: Compact JSON for log aggregation
//! - **JsonPretty**: Pretty-printed JSON for debugging
//! - **Yaml**: Human-readable YAML format
//! - **Logfmt**: Key-value format for structured logging
//!
//! # Examples
//!
//! ```rust,ignore
//! use verum_error::formatters::OutputFormat;
//!
//! let formatted = error
//!     .format_with(OutputFormat::Json);
//!
//! // Output: {"error":"...", "context":["..."], "structured":{...}}
//! ```

use crate::context::ContextError;
use crate::structured_context::ContextValue;
use std::fmt;
use verum_common::{List, Map, Text};

/// Output format for error contexts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Plain text format (human-readable)
    PlainText,
    /// Compact JSON format (single line)
    Json,
    /// Pretty-printed JSON with indentation
    JsonPretty,
    /// YAML format (human-readable structured)
    Yaml,
    /// Logfmt format (key=value pairs)
    Logfmt,
}

impl OutputFormat {
    /// Get all available formats
    pub fn all() -> &'static [OutputFormat] {
        &[
            OutputFormat::PlainText,
            OutputFormat::Json,
            OutputFormat::JsonPretty,
            OutputFormat::Yaml,
            OutputFormat::Logfmt,
        ]
    }

    /// Get the format name as a string
    pub fn name(&self) -> &'static str {
        match self {
            OutputFormat::PlainText => "plain",
            OutputFormat::Json => "json",
            OutputFormat::JsonPretty => "json-pretty",
            OutputFormat::Yaml => "yaml",
            OutputFormat::Logfmt => "logfmt",
        }
    }

    /// Parse a format from a string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "plain" | "plaintext" | "text" => Some(OutputFormat::PlainText),
            "json" => Some(OutputFormat::Json),
            "json-pretty" | "jsonpretty" | "pretty" => Some(OutputFormat::JsonPretty),
            "yaml" | "yml" => Some(OutputFormat::Yaml),
            "logfmt" | "log" => Some(OutputFormat::Logfmt),
            _ => None,
        }
    }
}

/// Trait for formatting errors with structured contexts
pub trait FormatError {
    /// Format the error with the specified output format
    fn format_with(&self, format: OutputFormat) -> Text;

    /// Format as plain text (default)
    fn format_plain_text(&self) -> Text;

    /// Format as JSON
    fn format_json(&self, pretty: bool) -> Text;

    /// Format as YAML
    fn format_yaml(&self) -> Text;

    /// Format as Logfmt
    fn format_logfmt(&self) -> Text;
}

impl<E: fmt::Display> FormatError for ContextError<E> {
    fn format_with(&self, format: OutputFormat) -> Text {
        match format {
            OutputFormat::PlainText => self.format_plain_text(),
            OutputFormat::Json => self.format_json(false),
            OutputFormat::JsonPretty => self.format_json(true),
            OutputFormat::Yaml => self.format_yaml(),
            OutputFormat::Logfmt => self.format_logfmt(),
        }
    }

    fn format_plain_text(&self) -> Text {
        let mut output = String::new();

        // Error message
        output.push_str("Error: ");
        output.push_str(&self.error().to_string());
        output.push('\n');

        // Text contexts
        let contexts = self.context_chain();
        if !contexts.is_empty() {
            output.push_str("\nContext:\n");
            for ctx in contexts {
                output.push_str("  - ");
                output.push_str(ctx.as_str());
                output.push('\n');
            }
        }

        // Structured contexts
        let structured = self.structured_contexts();
        if !structured.is_empty() {
            output.push_str("\nStructured Data:\n");
            let mut keys: Vec<&Text> = structured.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(value) = structured.get(key) {
                    output.push_str("  ");
                    output.push_str(key);
                    output.push_str(": ");
                    output.push_str(value.to_display_string().as_str());
                    output.push('\n');
                }
            }
        }

        output.into()
    }

    fn format_json(&self, pretty: bool) -> Text {
        if pretty {
            self.format_json_pretty()
        } else {
            self.format_json_compact()
        }
    }

    fn format_yaml(&self) -> Text {
        let mut output = String::new();

        // Error message
        output.push_str("error: ");
        let error_msg = self.error().to_string();
        if error_msg.contains('\n') || error_msg.contains(':') {
            output.push_str(&format!("\"{}\"", error_msg.replace('"', "\\\"")));
        } else {
            output.push_str(&error_msg);
        }
        output.push('\n');

        // Text contexts
        let contexts = self.context_chain();
        if !contexts.is_empty() {
            output.push_str("context:\n");
            for ctx in contexts {
                output.push_str("  - ");
                if ctx.contains("\n") || ctx.contains(":") {
                    output.push_str(&format!("\"{}\"", ctx.as_str().replace('"', "\\\"")));
                } else {
                    output.push_str(ctx.as_str());
                }
                output.push('\n');
            }
        }

        // Structured contexts
        let structured = self.structured_contexts();
        if !structured.is_empty() {
            output.push_str("structured:\n");
            let mut keys: Vec<&Text> = structured.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(value) = structured.get(key) {
                    output.push_str("  ");
                    output.push_str(key);
                    output.push_str(": ");
                    let value_yaml = value.to_yaml(2);
                    output.push_str(value_yaml.as_str());
                    output.push('\n');
                }
            }
        }

        output.into()
    }

    fn format_logfmt(&self) -> Text {
        let mut parts: Vec<String> = Vec::new();

        // Error message
        let error_msg = self.error().to_string();
        if error_msg.contains(' ') || error_msg.contains('=') {
            parts.push(format!("error=\"{}\"", error_msg.replace('"', "\\\"")));
        } else {
            parts.push(format!("error={}", error_msg));
        }

        // Text contexts (as a JSON array)
        let contexts = self.context_chain();
        if !contexts.is_empty() {
            let contexts_json: Vec<String> = contexts
                .iter()
                .map(|c| format!("\"{}\"", c.as_str().replace('"', "\\\"")))
                .collect();
            parts.push(format!("context=[{}]", contexts_json.join(",")));
        }

        // Structured contexts (as key=value pairs)
        let structured = self.structured_contexts();
        if !structured.is_empty() {
            let mut keys: Vec<&Text> = structured.keys().collect();
            keys.sort();
            for key in keys {
                if let Some(value) = structured.get(key) {
                    let value_str = value.to_logfmt();
                    parts.push(format!("{}={}", key, value_str.as_str()));
                }
            }
        }

        parts.join(" ").into()
    }
}

// Helper methods for ContextError

impl<E: fmt::Display> ContextError<E> {
    /// Format as compact JSON (internal helper)
    fn format_json_compact(&self) -> Text {
        let mut parts: Vec<String> = Vec::new();

        // Error message
        let error_msg = self.error().to_string();
        let error_json = ContextValue::Text(error_msg.into()).to_json(false);
        parts.push(format!("\"error\":{}", error_json.as_str()));

        // Text contexts
        let contexts = self.context_chain();
        if !contexts.is_empty() {
            let contexts_json: Vec<String> = contexts
                .iter()
                .map(|c| ContextValue::Text((*c).clone()).to_json(false).into_string())
                .collect();
            parts.push(format!("\"context\":[{}]", contexts_json.join(",")));
        }

        // Structured contexts
        let structured = self.structured_contexts();
        if !structured.is_empty() {
            let structured_json = ContextValue::Map(structured.clone()).to_json(false);
            parts.push(format!("\"structured\":{}", structured_json.as_str()));
        }

        format!("{{{}}}", parts.join(",")).into()
    }

    /// Format as pretty JSON (internal helper)
    fn format_json_pretty(&self) -> Text {
        let mut output = String::from("{\n");

        // Error message
        let error_msg = self.error().to_string();
        let error_json = ContextValue::Text(error_msg.into()).to_json(false);
        output.push_str(&format!("  \"error\": {}", error_json.as_str()));

        // Text contexts
        let contexts = self.context_chain();
        if !contexts.is_empty() {
            output.push_str(",\n  \"context\": [\n");
            for (i, ctx) in contexts.iter().enumerate() {
                let ctx_json = ContextValue::Text((*ctx).clone()).to_json(false);
                output.push_str(&format!("    {}", ctx_json.as_str()));
                if i < contexts.len() - 1 {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str("  ]");
        }

        // Structured contexts
        let structured = self.structured_contexts();
        if !structured.is_empty() {
            output.push_str(",\n  \"structured\": ");
            let structured_json = ContextValue::Map(structured.clone()).to_json(true);
            // Indent the structured JSON
            let indented = structured_json
                .as_str()
                .lines()
                .enumerate()
                .map(|(i, line)| {
                    if i == 0 {
                        line.to_string()
                    } else {
                        format!("  {}", line)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            output.push_str(&indented);
        }

        output.push_str("\n}");
        output.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ErrorContext;
    use crate::structured_context::ToContextValue;
    use crate::{ErrorKind, VerumError};

    fn create_test_error() -> ContextError<VerumError> {
        let error = VerumError::new("Connection timeout", ErrorKind::Network);
        let mut ctx_error = ContextError::new(error, "Failed to connect");

        // Add structured contexts
        ctx_error = ctx_error.add_structured("user_id", 12345);
        ctx_error = ctx_error.add_structured("retry_count", 3);
        ctx_error = ctx_error.add_structured("timeout_ms", 5000);

        ctx_error
    }

    #[test]
    fn test_output_format_name() {
        assert_eq!(OutputFormat::PlainText.name(), "plain");
        assert_eq!(OutputFormat::Json.name(), "json");
        assert_eq!(OutputFormat::JsonPretty.name(), "json-pretty");
        assert_eq!(OutputFormat::Yaml.name(), "yaml");
        assert_eq!(OutputFormat::Logfmt.name(), "logfmt");
    }

    #[test]
    fn test_output_format_from_str() {
        assert_eq!(
            OutputFormat::from_str("plain"),
            Some(OutputFormat::PlainText)
        );
        assert_eq!(OutputFormat::from_str("json"), Some(OutputFormat::Json));
        assert_eq!(OutputFormat::from_str("yaml"), Some(OutputFormat::Yaml));
        assert_eq!(OutputFormat::from_str("invalid"), None);
    }

    #[test]
    fn test_format_plain_text() {
        let error = create_test_error();
        let output = error.format_plain_text();

        // VerumError Display format is "[{kind}] {message}"
        assert!(output.contains("Error: [Network] Connection timeout"));
        assert!(output.contains("Context:"));
        assert!(output.contains("Failed to connect"));
        assert!(output.contains("Structured Data:"));
        assert!(output.contains("user_id: 12345"));
    }

    #[test]
    fn test_format_json_compact() {
        let error = create_test_error();
        let output = error.format_json(false);

        assert!(output.contains("\"error\":"));
        assert!(output.contains("\"context\":"));
        assert!(output.contains("\"structured\":"));
        assert!(output.contains("\"user_id\":12345"));
        assert!(!output.contains("\n")); // Should be compact
    }

    #[test]
    fn test_format_json_pretty() {
        let error = create_test_error();
        let output = error.format_json(true);

        assert!(output.contains("\"error\":"));
        assert!(output.contains("\"context\":"));
        assert!(output.contains("\"structured\":"));
        assert!(output.contains("\n")); // Should have newlines
        assert!(output.contains("  ")); // Should have indentation
    }

    #[test]
    fn test_format_yaml() {
        let error = create_test_error();
        let output = error.format_yaml();

        assert!(output.contains("error:"));
        assert!(output.contains("context:"));
        assert!(output.contains("structured:"));
        assert!(output.contains("user_id: 12345"));
    }

    #[test]
    fn test_format_logfmt() {
        let error = create_test_error();
        let output = error.format_logfmt();

        assert!(output.contains("error="));
        assert!(output.contains("user_id=12345"));
        assert!(output.contains("retry_count=3"));
        assert!(output.contains("timeout_ms=5000"));
    }

    #[test]
    fn test_format_with_dispatcher() {
        let error = create_test_error();

        // Test all formats through the format_with dispatcher
        for format in OutputFormat::all() {
            let output = error.format_with(*format);
            assert!(
                !output.is_empty(),
                "Format {:?} produced empty output",
                format
            );

            // Each format should contain the error message
            assert!(
                output.contains("Connection timeout") || output.contains("error"),
                "Format {:?} missing error message",
                format
            );
        }
    }

    #[test]
    fn test_empty_structured_contexts() {
        let error = VerumError::new("Simple error", ErrorKind::Other);
        let ctx_error = ContextError::new(error, "Context message");

        let json = ctx_error.format_json(false);
        let yaml = ctx_error.format_yaml();

        // Should not include structured section when empty
        assert!(!json.contains("\"structured\":"));
        assert!(!yaml.contains("structured:"));
    }

    #[test]
    fn test_special_characters_in_error_message() {
        let error = VerumError::new("Error with \"quotes\" and\nnewlines", ErrorKind::Other);
        let ctx_error = ContextError::new(error, "Context");

        let json = ctx_error.format_json(false);
        let yaml = ctx_error.format_yaml();

        // JSON should escape special characters
        assert!(json.contains("\\\""));
        assert!(json.contains("\\n"));

        // YAML should quote the string
        assert!(yaml.contains("\""));
    }

    #[test]
    fn test_nested_structured_values() {
        let error = VerumError::new("Error", ErrorKind::Other);
        let mut ctx_error = ContextError::new(error, "Context");

        // Add nested map
        let mut nested = Map::new();
        nested.insert(
            Text::from("inner_key"),
            ContextValue::Text(Text::from("inner_value")),
        );
        ctx_error = ctx_error.add_structured("nested", ContextValue::Map(nested));

        let json = ctx_error.format_json(true);
        let yaml = ctx_error.format_yaml();

        assert!(json.contains("\"nested\":"));
        assert!(json.contains("\"inner_key\""));
        assert!(yaml.contains("nested:"));
        assert!(yaml.contains("inner_key:"));
    }

    #[test]
    fn test_list_in_structured_context() {
        let error = VerumError::new("Error", ErrorKind::Other);
        let mut ctx_error = ContextError::new(error, "Context");

        let list: List<i32> = vec![1, 2, 3].into();
        ctx_error = ctx_error.add_structured("numbers", list);

        let json = ctx_error.format_json(false);
        assert!(json.contains("[1,2,3]"));
    }
}
