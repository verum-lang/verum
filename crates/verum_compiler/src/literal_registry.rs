//! Registry for custom tagged literal handlers
//!
//! Tagged and Compile-Time Literal Protocols:
//! Tagged text literals (§1.4.4) use `tag#"content"` syntax for compile-time
//! parsing and validation. Tags are registered via @tagged_literal attribute on
//! meta functions, which parse the literal content and return typed values
//! (e.g., `d#"2025-01-01"` → Date). Compile-time literal protocols (§1.4.5)
//! extend this with safe interpolation handlers registered via
//! @interpolation_handler, which receive template strings and expression lists,
//! returning injection-safe parameterized output (e.g., `sql"SELECT * WHERE id = {id}"`).
//!
//! This module implements the Revolutionary Literal System v3.0, providing:
//! - Tagged literal registration via @tagged_literal attribute
//! - Compile-time parsing and validation
//! - Runtime validation support
//! - Safe interpolation handlers

use parking_lot::RwLock;
use std::sync::Arc;
use verum_ast::{SourceFile, Span};
use verum_common::{Heap, List, Map, Maybe, Text};
use verum_diagnostics::{Diagnostic, DiagnosticBuilder, Severity};

/// Convert AST span to diagnostic span using source file information
fn convert_span(ast_span: Span, source_file: Option<&SourceFile>) -> verum_diagnostics::Span {
    match source_file {
        Some(file) => {
            // Use SourceFile to convert byte offsets to line/column
            match file.span_to_line_col(ast_span) {
                Some(line_col_span) => line_col_span,
                None => {
                    // Fallback if span doesn't match file (shouldn't happen)
                    verum_diagnostics::Span::new("<unknown>", 1, 1, 1)
                }
            }
        }
        None => {
            // Fallback when source file is not available
            // This can happen in tests or partial compilation
            verum_diagnostics::Span::new("<unknown>", 1, 1, 1)
        }
    }
}

/// Registry for custom tagged literal handlers
///
/// # Thread Safety
/// This registry is thread-safe and can be shared across multiple threads
/// during compilation.
///
/// # Example
/// ```ignore
/// use verum_compiler::literal_registry::LiteralRegistry;
///
/// let mut registry = LiteralRegistry::new();
/// registry.register_builtin_handlers();
/// ```
pub struct LiteralRegistry {
    /// Map: tag -> handler function
    handlers: Arc<RwLock<Map<Text, Heap<TaggedLiteralHandler>>>>,
    /// Map: tag -> expected format description
    formats: Arc<RwLock<Map<Text, Text>>>,
}

/// Handler for a specific tagged literal type
///
/// # Example
/// ```ignore
/// use verum_compiler::literal_registry::TaggedLiteralHandler;
/// use verum_common::Text;
///
/// let handler = TaggedLiteralHandler {
///     tag: Text::from("rx"),
///     handler_fn: Text::from("core.regex.parse_regex"),
///     compile_time: true,
///     runtime: false,
/// };
/// ```
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaggedLiteralHandler {
    /// The tag identifier (e.g., "rx" for rx#"pattern")
    pub tag: Text,
    /// Fully qualified function name for the handler
    pub handler_fn: Text,
    /// Whether this handler supports compile-time parsing
    pub compile_time: bool,
    /// Whether this handler supports runtime validation
    pub runtime: bool,
}

/// Result of parsing a tagged literal at compile-time
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedLiteral {
    /// DateTime parsed to Unix timestamp (seconds since epoch)
    DateTime(i64),
    /// Duration in nanoseconds
    Duration(u64),
    /// Validated regex pattern
    Regex(Text),
    /// Numeric interval with inclusive/exclusive bounds
    Interval {
        start: f64,
        end: f64,
        inclusive_start: bool,
        inclusive_end: bool,
    },
    /// Matrix of floating-point values (row-major order)
    Matrix {
        rows: usize,
        cols: usize,
        data: List<f64>,
    },
    /// Validated URI/URL
    Uri(Text),
    /// Validated email address
    Email(Text),
    /// Validated UUID (normalized to standard format)
    Uuid(String),
    /// JSON value
    Json(Text),
    /// XML document
    Xml(Text),
    /// YAML value
    Yaml(Text),
    /// Generic custom literal (for user-defined handlers)
    Custom { tag: Text, value: Text },
}

impl LiteralRegistry {
    /// Creates a new empty literal registry
    pub fn new() -> Self {
        Self {
            handlers: Arc::new(RwLock::new(Map::new())),
            formats: Arc::new(RwLock::new(Map::new())),
        }
    }

    /// Register a tagged literal handler via @tagged_literal attribute
    ///
    /// # Errors
    /// Returns an error if a handler with the same tag is already registered
    pub fn register_handler(&self, handler: TaggedLiteralHandler) -> std::result::Result<(), Text> {
        let mut handlers = self.handlers.write();
        let tag = handler.tag.clone();

        if handlers.contains_key(&tag) {
            return Err(Text::from(format!(
                "Handler for tag '{}' already registered",
                tag
            )));
        }

        handlers.insert(tag, Heap::new(handler));
        Ok(())
    }

    /// Register a format description for a tag
    pub fn register_format(&self, tag: Text, format: Text) {
        let mut formats = self.formats.write();
        formats.insert(tag, format);
    }

    /// Get a handler by tag
    pub fn get_handler(&self, tag: &Text) -> Maybe<TaggedLiteralHandler> {
        let handlers = self.handlers.read();
        match handlers.get(tag) {
            Some(handler) => Maybe::Some((**handler).clone()),
            None => Maybe::None,
        }
    }

    /// Get format description for a tag
    pub fn get_format(&self, tag: &Text) -> Maybe<Text> {
        let formats = self.formats.read();
        match formats.get(tag) {
            Some(format) => Maybe::Some(format.clone()),
            None => Maybe::None,
        }
    }

    /// Parse tagged literal at compile-time
    ///
    /// Tagged text literals use `tag#"content"` syntax. The registry looks up the
    /// handler for the given tag, invokes it at compile-time, and returns a typed
    /// ParsedLiteral value. If no handler is registered, emits a diagnostic error.
    ///
    /// # Arguments
    /// - `tag`: The literal tag (e.g., "rx", "d", "sql")
    /// - `content`: The literal content
    /// - `span`: Source location for error reporting
    /// - `source_file`: Optional source file for accurate span conversion
    ///
    /// # Returns
    /// Parsed literal on success, or diagnostic on failure
    pub fn parse_literal(
        &self,
        tag: &Text,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        let handler = match self.get_handler(tag) {
            Maybe::Some(h) => h,
            Maybe::None => {
                return Err(DiagnosticBuilder::new(Severity::Error)
                    .message(format!("Unknown tagged literal: {}", tag))
                    .span(convert_span(span, source_file))
                    .build());
            }
        };

        if !handler.compile_time {
            return Err(DiagnosticBuilder::new(Severity::Error)
                .message(format!(
                    "Tag '{}' does not support compile-time parsing",
                    tag
                ))
                .span(convert_span(span, source_file))
                .build());
        }

        // Dispatch to appropriate built-in parser
        match tag.as_str() {
            "d" | "date" | "datetime" => self.parse_datetime(content, span, source_file),
            "duration" | "dur" => self.parse_duration_lit(content, span, source_file),
            "rx" | "regex" => self.parse_regex(content, span, source_file),
            "interval" => self.parse_interval(content, span, source_file),
            "mat" | "matrix" => self.parse_matrix(content, span, source_file),
            "url" | "uri" => self.parse_uri(content, span, source_file),
            "email" => self.parse_email(content, span, source_file),
            "uuid" | "guid" => self.parse_uuid(content, span, source_file),
            "json" => self.parse_json(content, span, source_file),
            "xml" => self.parse_xml(content, span, source_file),
            "yaml" | "yml" => self.parse_yaml(content, span, source_file),
            _ => {
                // Custom handler - return generic custom literal
                Ok(ParsedLiteral::Custom {
                    tag: tag.clone(),
                    value: content.clone(),
                })
            }
        }
    }

    /// Parse UUID literal
    fn parse_uuid(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> Result<ParsedLiteral, Diagnostic> {
        use crate::literal_parsers::parse_uuid;
        let uuid = parse_uuid(content.as_str(), span, source_file)?;
        Ok(ParsedLiteral::Uuid(uuid))
    }

    /// Parse duration literal
    fn parse_duration_lit(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> Result<ParsedLiteral, Diagnostic> {
        use crate::literal_parsers::parse_duration;
        let ns = parse_duration(content.as_str(), span, source_file)?;
        Ok(ParsedLiteral::Duration(ns))
    }

    /// Register all built-in handlers
    ///
    /// Compile-time literal protocols: each tag maps to a meta function that parses
    /// the literal content and returns a typed value. User-defined tags are registered
    /// via @tagged_literal attribute; these built-ins are pre-registered at startup.
    ///
    /// Registers handlers for 11 built-in tagged literal types:
    /// - d#, date#, datetime# - DateTime
    /// - duration#, dur# - Duration
    /// - rx#, regex# - Regex
    /// - interval# - Numeric intervals
    /// - mat#, matrix# - Matrices
    /// - url#, uri# - URIs
    /// - email# - Email addresses
    /// - uuid#, guid# - UUIDs
    /// - json# - JSON
    /// - xml# - XML
    /// - yaml#, yml# - YAML
    pub fn register_builtin_handlers(&self) {
        let handlers = vec![
            // DateTime literals
            TaggedLiteralHandler {
                tag: Text::from("d"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_datetime"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("date"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_datetime"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("datetime"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_datetime"),
                compile_time: true,
                runtime: false,
            },
            // Duration literals
            TaggedLiteralHandler {
                tag: Text::from("duration"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_duration"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("dur"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_duration"),
                compile_time: true,
                runtime: false,
            },
            // Regex literals
            TaggedLiteralHandler {
                tag: Text::from("rx"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_regex"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("regex"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_regex"),
                compile_time: true,
                runtime: false,
            },
            // Interval literals
            TaggedLiteralHandler {
                tag: Text::from("interval"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_interval"),
                compile_time: true,
                runtime: false,
            },
            // Matrix literals
            TaggedLiteralHandler {
                tag: Text::from("mat"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_matrix"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("matrix"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_matrix"),
                compile_time: true,
                runtime: false,
            },
            // URI/URL literals
            TaggedLiteralHandler {
                tag: Text::from("url"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_uri"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("uri"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_uri"),
                compile_time: true,
                runtime: false,
            },
            // Email literals
            TaggedLiteralHandler {
                tag: Text::from("email"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_email"),
                compile_time: true,
                runtime: false,
            },
            // UUID literals
            TaggedLiteralHandler {
                tag: Text::from("uuid"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_uuid"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("guid"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_uuid"),
                compile_time: true,
                runtime: false,
            },
            // JSON literals
            TaggedLiteralHandler {
                tag: Text::from("json"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_json"),
                compile_time: true,
                runtime: false,
            },
            // XML literals
            TaggedLiteralHandler {
                tag: Text::from("xml"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_xml"),
                compile_time: true,
                runtime: false,
            },
            // YAML literals
            TaggedLiteralHandler {
                tag: Text::from("yaml"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_yaml"),
                compile_time: true,
                runtime: false,
            },
            TaggedLiteralHandler {
                tag: Text::from("yml"),
                handler_fn: Text::from("verum_compiler::literal_parsers::parse_yaml"),
                compile_time: true,
                runtime: false,
            },
        ];

        for handler in handlers {
            let _ = self.register_handler(handler);
        }

        // Register format descriptions
        self.register_format(
            Text::from("d"),
            Text::from("ISO 8601 datetime (e.g., '2024-01-15T10:30:00Z')"),
        );
        self.register_format(Text::from("rx"), Text::from("Regular expression pattern"));
        self.register_format(
            Text::from("interval"),
            Text::from("Numeric interval (e.g., '[0, 100)')"),
        );
        self.register_format(
            Text::from("mat"),
            Text::from("Matrix literal (e.g., '[[1, 2], [3, 4]]')"),
        );
        self.register_format(
            Text::from("url"),
            Text::from("URI/URL (e.g., 'https://example.com')"),
        );
        self.register_format(
            Text::from("email"),
            Text::from("Email address (e.g., 'user@example.com')"),
        );
        self.register_format(Text::from("json"), Text::from("JSON value"));
        self.register_format(Text::from("xml"), Text::from("XML document"));
        self.register_format(Text::from("yaml"), Text::from("YAML value"));
    }

    // Built-in parser stubs - actual implementation in literal_parsers module

    fn parse_datetime(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::datetime::parse_datetime(content, span, source_file)
    }

    fn parse_regex(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::regex::parse_regex(content, span, source_file)
    }

    fn parse_interval(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::interval::parse_interval(content, span, source_file)
    }

    fn parse_matrix(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::matrix::parse_matrix(content, span, source_file)
    }

    fn parse_uri(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::uri::parse_uri(content, span, source_file)
    }

    fn parse_email(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::email::parse_email(content, span, source_file)
    }

    fn parse_json(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::json::parse_json(content, span, source_file)
    }

    fn parse_xml(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::xml::parse_xml(content, span, source_file)
    }

    fn parse_yaml(
        &self,
        content: &Text,
        span: Span,
        source_file: Option<&SourceFile>,
    ) -> std::result::Result<ParsedLiteral, Diagnostic> {
        crate::literal_parsers::yaml::parse_yaml(content, span, source_file)
    }
}

impl Default for LiteralRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for LiteralRegistry {
    fn clone(&self) -> Self {
        Self {
            handlers: Arc::clone(&self.handlers),
            formats: Arc::clone(&self.formats),
        }
    }
}
