//! Literal value nodes in the AST.
//!
//! This module defines all literal types supported by Verum, including
//! integers, floats, strings, characters, and booleans.

use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};
use std::fmt;
use verum_common::Text;

/// A literal value in the source code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Literal {
    pub kind: LiteralKind,
    pub span: Span,
}

impl Literal {
    pub fn new(kind: LiteralKind, span: Span) -> Self {
        Self { kind, span }
    }

    pub fn int(value: i128, span: Span) -> Self {
        Self::new(
            LiteralKind::Int(IntLit {
                value,
                suffix: None,
            }),
            span,
        )
    }

    pub fn float(value: f64, span: Span) -> Self {
        Self::new(
            LiteralKind::Float(FloatLit {
                value,
                suffix: None,
            }),
            span,
        )
    }

    pub fn string(value: Text, span: Span) -> Self {
        Self::new(LiteralKind::Text(StringLit::Regular(value)), span)
    }

    pub fn char(value: char, span: Span) -> Self {
        Self::new(LiteralKind::Char(value), span)
    }

    pub fn byte_char(value: u8, span: Span) -> Self {
        Self::new(LiteralKind::ByteChar(value), span)
    }

    pub fn byte_string(value: Vec<u8>, span: Span) -> Self {
        Self::new(LiteralKind::ByteString(value), span)
    }

    pub fn bool(value: bool, span: Span) -> Self {
        Self::new(LiteralKind::Bool(value), span)
    }

    pub fn tagged(tag: Text, content: Text, span: Span) -> Self {
        Self::new(LiteralKind::Tagged { tag, content }, span)
    }

    pub fn interpolated_string(prefix: Text, content: Text, span: Span) -> Self {
        Self::new(
            LiteralKind::InterpolatedString(InterpolatedStringLit::new(prefix, content)),
            span,
        )
    }

    pub fn contract(content: Text, span: Span) -> Self {
        Self::new(LiteralKind::Contract(content), span)
    }

    pub fn context_adaptive(kind: ContextAdaptiveKind, raw: Text, span: Span) -> Self {
        Self::new(
            LiteralKind::ContextAdaptive(ContextAdaptiveLit::new(kind, raw)),
            span,
        )
    }

    pub fn hex_adaptive(value: u64, raw: Text, span: Span) -> Self {
        Self::context_adaptive(ContextAdaptiveKind::Hex(value), raw, span)
    }

    pub fn composite(tag: Text, content: Text, delimiter: CompositeDelimiter, span: Span) -> Self {
        Self::new(
            LiteralKind::Composite(CompositeLiteral::new(tag, content, delimiter)),
            span,
        )
    }
}

impl Literal {
    /// Returns true if this is a contract literal (contract#"...").
    pub fn is_contract(&self) -> bool {
        matches!(self.kind, LiteralKind::Contract(_))
    }
}

impl Spanned for Literal {
    fn span(&self) -> Span {
        self.span
    }
}

/// The kind of literal value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LiteralKind {
    /// Integer literal (arbitrary precision)
    Int(IntLit),
    /// Floating-point literal (IEEE 754 double)
    Float(FloatLit),
    /// Text literal
    Text(StringLit),
    /// Character literal
    Char(char),
    /// Byte character literal (b'x')
    ByteChar(u8),
    /// Byte string literal (b"hello")
    ByteString(Vec<u8>),
    /// Boolean literal
    Bool(bool),
    /// Tagged literal: d#"2025-11-05", sql#"SELECT...", rx#"pattern"
    Tagged { tag: Text, content: Text },
    /// Interpolated string literal (e.g., `sql"SELECT * FROM users WHERE id = {id}"`)
    /// Prefixed strings desugar to safe function calls that prevent injection attacks:
    /// sql -> SQL.query, html -> HTML.escape, uri -> URI.encode, json -> JSON.encode,
    /// xml -> XML.escape, gql -> GraphQL.query. The prefix `f` is a regular format string.
    InterpolatedString(InterpolatedStringLit),
    /// Contract literal: contract#"it > 0", contract#"requires x > 0"
    Contract(Text),
    /// Composite literal: mat#"[[1, 2], [3, 4]]", vec#"<1, 2, 3>", chem#"H2O", etc.
    /// Domain-specific structured data with tagged delimiters. Each tag has a registered
    /// meta-system handler that validates and transforms the content at compile time.
    /// Supported types: mat (Matrix), vec (Vector), chem (Molecule), music (Melody),
    /// interval (Interval). Custom tags can be registered via @tagged_literal.
    Composite(CompositeLiteral),
    /// Context-adaptive literal: #FF5733 (adapts to expected type via FromHexLiteral protocol)
    /// The interpretation depends on the expected type context at the use site -- the same
    /// literal `#FF5733` can be Color, u32, ByteArray, etc. Resolution uses the meta-system's
    /// type-driven literal protocol: types implement `FromHexLiteral` (or similar) to opt in.
    ContextAdaptive(ContextAdaptiveLit),
}

/// An integer literal with optional type or unit suffix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntLit {
    /// The integer value (arbitrary precision)
    pub value: i128,
    /// Optional type suffix (e.g., "i32", "u64") or unit suffix (e.g., "km", "deg")
    pub suffix: Option<IntSuffix>,
}

impl IntLit {
    pub fn new(value: i128) -> Self {
        Self {
            value,
            suffix: None,
        }
    }

    pub fn with_suffix(value: i128, suffix: IntSuffix) -> Self {
        Self {
            value,
            suffix: Some(suffix),
        }
    }
}

/// Integer literal suffix indicating the specific type or unit of measure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntSuffix {
    // Signed integers
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    // Unsigned integers
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
    // Custom unit suffix for units of measure (e.g., "km", "deg", "ms")
    Custom(Text),
}

impl IntSuffix {
    pub fn as_str(&self) -> Text {
        match self {
            IntSuffix::I8 => "i8".into(),
            IntSuffix::I16 => "i16".into(),
            IntSuffix::I32 => "i32".into(),
            IntSuffix::I64 => "i64".into(),
            IntSuffix::I128 => "i128".into(),
            IntSuffix::Isize => "isize".into(),
            IntSuffix::U8 => "u8".into(),
            IntSuffix::U16 => "u16".into(),
            IntSuffix::U32 => "u32".into(),
            IntSuffix::U64 => "u64".into(),
            IntSuffix::U128 => "u128".into(),
            IntSuffix::Usize => "usize".into(),
            IntSuffix::Custom(s) => s.clone(),
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, IntSuffix::Custom(_))
    }
}

impl fmt::Display for IntSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A floating-point literal with optional type or unit suffix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloatLit {
    /// The floating-point value
    pub value: f64,
    /// Optional type suffix (e.g., "f32", "f64") or unit suffix (e.g., "m", "kg")
    pub suffix: Option<FloatSuffix>,
}

impl FloatLit {
    pub fn new(value: f64) -> Self {
        Self {
            value,
            suffix: None,
        }
    }

    pub fn with_suffix(value: f64, suffix: FloatSuffix) -> Self {
        Self {
            value,
            suffix: Some(suffix),
        }
    }
}

/// Floating-point literal suffix indicating the specific type or unit of measure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FloatSuffix {
    F32,
    F64,
    // Custom unit suffix for units of measure (e.g., "m", "kg", "s")
    Custom(Text),
}

impl FloatSuffix {
    pub fn as_str(&self) -> Text {
        match self {
            FloatSuffix::F32 => "f32".into(),
            FloatSuffix::F64 => "f64".into(),
            FloatSuffix::Custom(s) => s.clone(),
        }
    }

    pub fn is_custom(&self) -> bool {
        matches!(self, FloatSuffix::Custom(_))
    }
}

impl fmt::Display for FloatSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A string literal with different representations.
///
/// # Simplified Literal Architecture (v6.0)
///
/// Verum uses a simplified approach to string literals:
/// - `"..."` - Regular strings with escape processing (`\n`, `\t`, etc.)
/// - `"""..."""` - Triple-quoted strings: raw AND multiline (no escape processing)
///
/// The old `r#"..."#` syntax has been removed. Use `"""..."""` for raw strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StringLit {
    /// Regular string with escape sequences processed: `"hello\nworld"`
    Regular(Text),
    /// Multi-line raw string (triple-quoted): `"""raw content\n preserved"""`
    ///
    /// Triple-quoted strings are BOTH raw (no escape processing) AND multiline.
    /// This is the unified approach: `"""...""" = raw = multiline`.
    MultiLine(Text),
}

impl StringLit {
    pub fn as_str(&self) -> &str {
        match self {
            StringLit::Regular(s) | StringLit::MultiLine(s) => s.as_str(),
        }
    }

    pub fn into_string(self) -> Text {
        match self {
            StringLit::Regular(s) | StringLit::MultiLine(s) => s,
        }
    }

    /// Check if this string literal is raw (no escape processing).
    /// `MultiLine` (triple-quoted) strings are always raw.
    pub fn is_raw(&self) -> bool {
        matches!(self, StringLit::MultiLine(_))
    }

    /// Check if this string literal is multiline (triple-quoted).
    pub fn is_multiline(&self) -> bool {
        matches!(self, StringLit::MultiLine(_))
    }
}

impl fmt::Display for StringLit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StringLit::Regular(s) => write!(f, "\"{}\"", s),
            StringLit::MultiLine(s) => write!(f, "\"\"\"{}\"\"\"", s),
        }
    }
}

/// An interpolated string literal with a safe prefix.
///
/// Examples:
/// - `sql"SELECT * FROM users WHERE id = {user_id}"`
/// - `html"<div>{content}</div>"`
/// - `uri"https://example.com/{path}"`
/// - `json"{\"name\": \"{name}\"}"`
/// - `xml"<tag>{value}</tag>"`
/// - `gql"query { user(id: {id}) { name } }"`
/// - `f"Hello {name}"` (regular format string, not a safe prefix)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpolatedStringLit {
    /// The prefix before the string (e.g., "sql", "html", "uri", "json", "xml", "gql", "f")
    pub prefix: Text,
    /// The template content with interpolations (e.g., "SELECT * FROM users WHERE id = {user_id}")
    /// Note: Content includes the raw interpolation markers {expr}
    pub content: Text,
}

impl InterpolatedStringLit {
    pub fn new(prefix: Text, content: Text) -> Self {
        Self { prefix, content }
    }

    pub fn is_safe_interpolation(&self) -> bool {
        matches!(
            self.prefix.as_str(),
            "sql" | "html" | "uri" | "json" | "xml" | "gql"
        )
    }

    pub fn desugaring_target(&self) -> Option<&'static str> {
        match self.prefix.as_str() {
            "sql" => Some("SQL.query"),
            "html" => Some("HTML.escape"),
            "uri" => Some("URI.encode"),
            "json" => Some("JSON.encode"),
            "xml" => Some("XML.escape"),
            "gql" => Some("GraphQL.query"),
            _ => None,
        }
    }
}

impl fmt::Display for InterpolatedStringLit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\"{}\"", self.prefix, self.content)
    }
}

/// Composite literal - domain-specific structured data with tagged delimiters.
///
/// Examples:
/// - `mat#"[[1, 2], [3, 4]]"` → Matrix<2, 2, i32>
/// - `vec#"<1, 2, 3>"` → Vector3<f64>
/// - `chem#"H2O"` → Molecule
/// - `music#"C4 D4 E4 F4"` → Melody
/// - `interval#"[0, 100)"` → Interval<f64>
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositeLiteral {
    /// The tag identifying the composite type (e.g., "mat", "vec", "chem", "music", "interval")
    pub tag: Text,
    /// The content of the composite literal, without delimiters
    pub content: Text,
    /// The delimiter style used: quotes, parens, brackets, or braces
    pub delimiter: CompositeDelimiter,
}

impl CompositeLiteral {
    pub fn new(tag: Text, content: Text, delimiter: CompositeDelimiter) -> Self {
        Self {
            tag,
            content,
            delimiter,
        }
    }

    /// Check if this is a recognized composite literal type
    pub fn is_recognized(&self) -> bool {
        matches!(
            self.tag.as_str(),
            "mat" | "vec" | "chem" | "music" | "interval"
        )
    }

    /// Get the type of composite literal
    pub fn composite_type(&self) -> Option<CompositeType> {
        match self.tag.as_str() {
            "mat" => Some(CompositeType::Matrix),
            "vec" => Some(CompositeType::Vector),
            "chem" => Some(CompositeType::Chemistry),
            "music" => Some(CompositeType::Music),
            "interval" => Some(CompositeType::Interval),
            _ => None,
        }
    }

    /// Validate the content syntax for this composite type
    pub fn validate(&self) -> Result<(), Text> {
        match self.composite_type() {
            Some(CompositeType::Matrix) => validate_matrix_content(self.content.as_str()),
            Some(CompositeType::Vector) => validate_vector_content(self.content.as_str()),
            Some(CompositeType::Chemistry) => validate_chemistry_content(self.content.as_str()),
            Some(CompositeType::Music) => validate_music_content(self.content.as_str()),
            Some(CompositeType::Interval) => validate_interval_content(self.content.as_str()),
            None => Err(Text::from(format!(
                "Unknown composite literal type: {}",
                self.tag
            ))),
        }
    }
}

impl fmt::Display for CompositeLiteral {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}#{}",
            self.tag,
            self.delimiter.wrap(self.content.as_str())
        )
    }
}

/// Delimiter style for composite literals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompositeDelimiter {
    /// Double quotes: "..."
    Quote,
    /// Triple quotes for multiline: """..."""
    TripleQuote,
    /// Parentheses: (...)
    Paren,
    /// Square brackets: [...]
    Bracket,
    /// Curly braces: {...}
    Brace,
}

impl CompositeDelimiter {
    /// Wrap content with appropriate delimiters
    pub fn wrap(self, content: &str) -> Text {
        match self {
            CompositeDelimiter::Quote => format!("\"{}\"", content).into(),
            CompositeDelimiter::TripleQuote => format!("\"\"\"{}\"\"\"", content).into(),
            CompositeDelimiter::Paren => format!("({})", content).into(),
            CompositeDelimiter::Bracket => format!("[{}]", content).into(),
            CompositeDelimiter::Brace => format!("{{{}}}", content).into(),
        }
    }
}

/// Types of composite literals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompositeType {
    /// Matrix: mat#"[[1, 2], [3, 4]]"
    Matrix,
    /// Vector: vec#"<1, 2, 3>"
    Vector,
    /// Chemical formula: chem#"H2O"
    Chemistry,
    /// Musical notation: music#"C4 D4 E4 F4"
    Music,
    /// Interval: interval#"[0, 100)"
    Interval,
}

// Validation functions for each composite type

/// Validate matrix content: [[1, 2], [3, 4]]
fn validate_matrix_content(content: &str) -> Result<(), Text> {
    let trimmed = content.trim();

    // Matrix must have double brackets: [[...]]
    // e.g., mat#"[[1, 2], [3, 4]]"
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return Err(Text::from("Matrix must have format [[...]]"));
    }

    // Check that there are nested brackets (at least 2 open and 2 close)
    let open_count = trimmed.matches('[').count();
    let close_count = trimmed.matches(']').count();

    if open_count < 2 || close_count < 2 {
        return Err(Text::from(
            "Matrix must have format [[...]] with nested brackets",
        ));
    }

    if open_count != close_count {
        return Err(Text::from("Unmatched brackets in matrix"));
    }

    Ok(())
}

/// Validate vector content: <1, 2, 3> or [1, 2, 3]
fn validate_vector_content(content: &str) -> Result<(), Text> {
    let trimmed = content.trim();

    // Must have at least one comma or be empty
    if !trimmed.contains(',') && !trimmed.is_empty() {
        // Single element is allowed
    }

    // Check for balanced delimiters if present
    let angle_open = trimmed.matches('<').count();
    let angle_close = trimmed.matches('>').count();
    let bracket_open = trimmed.matches('[').count();
    let bracket_close = trimmed.matches(']').count();

    if angle_open != angle_close {
        return Err(Text::from("Unmatched angle brackets in vector"));
    }

    if bracket_open != bracket_close {
        return Err(Text::from("Unmatched square brackets in vector"));
    }

    Ok(())
}

/// Validate chemistry content: H2O, C6H12O6
fn validate_chemistry_content(content: &str) -> Result<(), Text> {
    let trimmed = content.trim();

    // Must contain at least one element letter
    if !trimmed
        .chars()
        .any(|c| c.is_ascii_uppercase() || c.is_ascii_lowercase())
    {
        return Err(Text::from(
            "Chemical formula must contain at least one element",
        ));
    }

    // Check for valid characters: letters and digits
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c.is_whitespace())
    {
        return Err(Text::from("Chemical formula contains invalid characters"));
    }

    Ok(())
}

/// Validate music content: C4 D4 E4 or Cmaj7
fn validate_music_content(content: &str) -> Result<(), Text> {
    let trimmed = content.trim();

    // Must contain at least one valid note letter
    if !trimmed
        .to_uppercase()
        .chars()
        .any(|c| matches!(c, 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G'))
    {
        return Err(Text::from(
            "Music notation must contain at least one note letter",
        ));
    }

    // Check for valid characters: notes, sharps, flats, octaves, chord qualities
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | 'b' | '-' | '+' | ' ' | '/'))
    {
        return Err(Text::from("Music notation contains invalid characters"));
    }

    Ok(())
}

/// Validate interval content: [0, 100) or [0..100]
/// NOTE: The parser strips outer delimiters, so content for interval#"[0, 100]"
/// will be "0, 100" (without brackets). The brackets are encoded in the delimiter field.
fn validate_interval_content(content: &str) -> Result<(), Text> {
    let trimmed = content.trim();

    // Must contain separator (comma or ..)
    if !trimmed.contains(',') && !trimmed.contains("..") {
        return Err(Text::from("Interval must contain separator (comma or ..)"));
    }

    // Content should contain at least two parts separated by comma or ..
    // The brackets are in the delimiter, not in the content
    let parts: Vec<&str> = if trimmed.contains("..") {
        trimmed.split("..").collect()
    } else {
        trimmed.split(',').collect()
    };

    if parts.len() < 2 {
        return Err(Text::from("Interval must have start and end values"));
    }

    Ok(())
}

/// Context-adaptive literal that changes interpretation based on expected type.
///
/// Examples:
/// - `#FF5733` as CssColor → CssColor::from_hex(0xFF5733)
/// - `#FF5733` as RgbColor → RgbColor { r: 255, g: 87, b: 51 }
/// - `#FF5733` as u32 → 0xFF5733
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextAdaptiveLit {
    /// The kind of context-adaptive literal
    pub kind: ContextAdaptiveKind,
    /// Raw text representation for error reporting
    pub raw: Text,
}

impl ContextAdaptiveLit {
    pub fn new(kind: ContextAdaptiveKind, raw: Text) -> Self {
        Self { kind, raw }
    }

    pub fn hex(value: u64, raw: Text) -> Self {
        Self::new(ContextAdaptiveKind::Hex(value), raw)
    }

    pub fn numeric(value: Text, raw: Text) -> Self {
        Self::new(ContextAdaptiveKind::Numeric(value), raw)
    }

    pub fn identifier(ident: Text, raw: Text) -> Self {
        Self::new(ContextAdaptiveKind::Identifier(ident), raw)
    }
}

/// The kind of context-adaptive literal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContextAdaptiveKind {
    /// Hex literal: #FF5733 → 0xFF5733
    /// Can be interpreted as Color, u32, ByteArray, etc. depending on context
    Hex(u64),

    /// Numeric literal that adapts to context
    /// e.g., `100` can be Int, Float, Distance<Meters>, Duration<Seconds>, etc.
    Numeric(Text),

    /// Identifier-style adaptive literal: @username, $variable
    /// Context determines interpretation (CSS selector, social media handle, etc.)
    Identifier(Text),
}

/// Base for integer literals (decimal, hexadecimal, binary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntBase {
    /// Decimal (base 10)
    Decimal,
    /// Hexadecimal (base 16, 0x prefix)
    Hexadecimal,
    /// Binary (base 2, 0b prefix)
    Binary,
}

impl IntBase {
    pub fn radix(&self) -> u32 {
        match self {
            IntBase::Decimal => 10,
            IntBase::Hexadecimal => 16,
            IntBase::Binary => 2,
        }
    }
}
