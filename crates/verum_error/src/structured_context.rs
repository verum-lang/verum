//! Structured Error Context
//!
//! Provides **structured key-value contexts** for rich error diagnostics.
//! Extends the basic text context system with typed data that can be
//! formatted as JSON, YAML, or Logfmt for logging and monitoring systems.
//!
//! # Core Concept
//!
//! While text contexts provide human-readable breadcrumbs, structured contexts
//! add machine-readable data that helps with:
//! - Automated error analysis
//! - Log aggregation and filtering
//! - Debugging with specific values
//! - Monitoring and alerting
//!
//! # Examples
//!
//! ```rust,ignore
//! use verum_error::structured_context::{ContextValue, ToContextValue};
//!
//! // Add structured data to errors
//! database_query()
//!     .with_structured("user_id", 12345)
//!     .with_structured("operation", "fetch_profile")
//!     .with_structured("timeout_ms", 5000)?;
//!
//! // Output as JSON:
//! // {
//! //   "error": "Connection timeout",
//! //   "structured": {
//! //     "user_id": 12345,
//! //     "operation": "fetch_profile",
//! //     "timeout_ms": 5000
//! //   }
//! // }
//! ```

use std::fmt;
use verum_common::{List, Map, Maybe, Text};

/// A value that can be stored in structured context
///
/// Supports common data types and nested structures for rich error diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub enum ContextValue {
    /// Text/string value
    Text(Text),
    /// Signed integer
    Int(i64),
    /// Unsigned integer
    UInt(u64),
    /// Floating point number
    Float(f64),
    /// Boolean value
    Bool(bool),
    /// List of context values
    List(List<ContextValue>),
    /// Map of context values
    Map(Map<Text, ContextValue>),
    /// Null/None value
    Null,
}

impl ContextValue {
    /// Try to get the value as text
    pub fn as_text(&self) -> Maybe<&Text> {
        match self {
            ContextValue::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Try to get the value as a signed integer
    pub fn as_int(&self) -> Maybe<i64> {
        match self {
            ContextValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get the value as an unsigned integer
    pub fn as_uint(&self) -> Maybe<u64> {
        match self {
            ContextValue::UInt(u) => Some(*u),
            _ => None,
        }
    }

    /// Try to get the value as a float
    pub fn as_float(&self) -> Maybe<f64> {
        match self {
            ContextValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Try to get the value as a boolean
    pub fn as_bool(&self) -> Maybe<bool> {
        match self {
            ContextValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get the value as a list
    pub fn as_list(&self) -> Maybe<&List<ContextValue>> {
        match self {
            ContextValue::List(l) => Some(l),
            _ => None,
        }
    }

    /// Try to get the value as a map
    pub fn as_map(&self) -> Maybe<&Map<Text, ContextValue>> {
        match self {
            ContextValue::Map(m) => Some(m),
            _ => None,
        }
    }

    /// Check if the value is null
    pub fn is_null(&self) -> bool {
        matches!(self, ContextValue::Null)
    }

    /// Convert the value to a JSON string
    ///
    /// # Arguments
    /// * `pretty` - If true, format with indentation and newlines
    pub fn to_json(&self, pretty: bool) -> Text {
        if pretty {
            self.to_json_impl(0)
        } else {
            self.to_json_compact()
        }
    }

    /// Convert to compact JSON (single line)
    fn to_json_compact(&self) -> Text {
        match self {
            ContextValue::Text(s) => {
                // Escape special characters
                let escaped = s
                    .as_str()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t");
                format!("\"{}\"", escaped).into()
            }
            ContextValue::Int(i) => i.to_string().into(),
            ContextValue::UInt(u) => u.to_string().into(),
            ContextValue::Float(f) => {
                // Handle special float values
                if f.is_nan() {
                    "null".to_string().into()
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        "\"Infinity\"".to_string().into()
                    } else {
                        "\"-Infinity\"".to_string().into()
                    }
                } else {
                    f.to_string().into()
                }
            }
            ContextValue::Bool(b) => b.to_string().into(),
            ContextValue::List(items) => {
                let items_json: Vec<String> = items.iter().map(|v| v.to_json_compact().into_string()).collect();
                format!("[{}]", items_json.join(",")).into()
            }
            ContextValue::Map(map) => {
                let mut entries: Vec<String> = Vec::new();
                let mut keys: Vec<&Text> = map.keys().collect();
                keys.sort(); // Deterministic output
                for key in keys {
                    if let Some(value) = map.get(key) {
                        let key_json = ContextValue::Text(key.clone()).to_json_compact();
                        let value_json = value.to_json_compact();
                        entries.push(format!("{}:{}", key_json.as_str(), value_json.as_str()));
                    }
                }
                format!("{{{}}}", entries.join(",")).into()
            }
            ContextValue::Null => "null".to_string().into(),
        }
    }

    /// Convert to pretty JSON with indentation
    fn to_json_impl(&self, indent: usize) -> Text {
        let indent_str = "  ".repeat(indent);
        let next_indent_str = "  ".repeat(indent + 1);

        match self {
            ContextValue::Text(_)
            | ContextValue::Int(_)
            | ContextValue::UInt(_)
            | ContextValue::Float(_)
            | ContextValue::Bool(_)
            | ContextValue::Null => self.to_json_compact(),
            ContextValue::List(items) => {
                if items.is_empty() {
                    "[]".to_string().into()
                } else {
                    let items_json: Vec<String> = items
                        .iter()
                        .map(|v| format!("{}{}", next_indent_str, v.to_json_impl(indent + 1).as_str()))
                        .collect();
                    format!("[\n{}\n{}]", items_json.join(",\n"), indent_str).into()
                }
            }
            ContextValue::Map(map) => {
                if map.is_empty() {
                    "{}".to_string().into()
                } else {
                    let mut entries: Vec<String> = Vec::new();
                    let mut keys: Vec<&Text> = map.keys().collect();
                    keys.sort();
                    for key in keys {
                        if let Some(value) = map.get(key) {
                            let key_json = ContextValue::Text(key.clone()).to_json_compact();
                            let value_json = value.to_json_impl(indent + 1);
                            entries
                                .push(format!("{}{}: {}", next_indent_str, key_json.as_str(), value_json.as_str()));
                        }
                    }
                    format!("{{\n{}\n{}}}", entries.join(",\n"), indent_str).into()
                }
            }
        }
    }

    /// Convert the value to a YAML string
    ///
    /// # Arguments
    /// * `indent` - Current indentation level (use 0 for top-level)
    pub fn to_yaml(&self, indent: usize) -> Text {
        let indent_str = "  ".repeat(indent);

        match self {
            ContextValue::Text(s) => {
                // Quote strings with special characters
                if s.contains("\n") || s.contains(":") || s.contains("#") || s.as_str().starts_with(' ') {
                    let escaped = s.as_str().replace('\\', "\\\\").replace('"', "\\\"");
                    format!("\"{}\"", escaped).into()
                } else {
                    s.clone()
                }
            }
            ContextValue::Int(i) => i.to_string().into(),
            ContextValue::UInt(u) => u.to_string().into(),
            ContextValue::Float(f) => {
                if f.is_nan() {
                    ".nan".to_string().into()
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        ".inf".to_string().into()
                    } else {
                        "-.inf".to_string().into()
                    }
                } else {
                    f.to_string().into()
                }
            }
            ContextValue::Bool(b) => b.to_string().into(),
            ContextValue::List(items) => {
                if items.is_empty() {
                    "[]".to_string().into()
                } else {
                    let mut result = String::new();
                    for item in items {
                        result.push('\n');
                        result.push_str(&indent_str);
                        result.push_str("- ");
                        let item_yaml = item.to_yaml(indent + 1);
                        if matches!(item, ContextValue::Map(_)) {
                            result.push_str(item_yaml.as_str().trim_start());
                        } else {
                            result.push_str(item_yaml.as_str());
                        }
                    }
                    result.into()
                }
            }
            ContextValue::Map(map) => {
                if map.is_empty() {
                    "{}".to_string().into()
                } else {
                    let mut result = String::new();
                    let mut keys: Vec<&Text> = map.keys().collect();
                    keys.sort();
                    for (i, key) in keys.iter().enumerate() {
                        if let Some(value) = map.get(*key) {
                            if i > 0 {
                                result.push('\n');
                                result.push_str(&indent_str);
                            }
                            result.push_str(key);
                            result.push_str(": ");
                            let value_yaml = value.to_yaml(indent + 1);
                            result.push_str(value_yaml.as_str());
                        }
                    }
                    result.into()
                }
            }
            ContextValue::Null => "null".to_string().into(),
        }
    }

    /// Convert the value to Logfmt format (key=value pairs)
    ///
    /// Logfmt is a format popularized by Heroku that's easy for both humans
    /// and machines to parse. Values with spaces are quoted.
    pub fn to_logfmt(&self) -> Text {
        match self {
            ContextValue::Text(s) => {
                if s.contains(" ") || s.contains("\"") || s.contains("=") {
                    let escaped = s.as_str().replace('\\', "\\\\").replace('"', "\\\"");
                    format!("\"{}\"", escaped).into()
                } else {
                    s.clone()
                }
            }
            ContextValue::Int(i) => i.to_string().into(),
            ContextValue::UInt(u) => u.to_string().into(),
            ContextValue::Float(f) => f.to_string().into(),
            ContextValue::Bool(b) => b.to_string().into(),
            ContextValue::Null => "null".to_string().into(),
            // For complex types, use JSON representation
            ContextValue::List(_) | ContextValue::Map(_) => {
                let json = self.to_json_compact();
                if json.contains(" ") {
                    format!("\"{}\"", json.as_str().replace('"', "\\\"")).into()
                } else {
                    json
                }
            }
        }
    }

    /// Convert to a display-friendly string
    pub fn to_display_string(&self) -> Text {
        match self {
            ContextValue::Text(s) => s.clone(),
            ContextValue::Int(i) => i.to_string().into(),
            ContextValue::UInt(u) => u.to_string().into(),
            ContextValue::Float(f) => f.to_string().into(),
            ContextValue::Bool(b) => b.to_string().into(),
            ContextValue::List(items) => {
                let items_str: Vec<String> = items.iter().map(|v| v.to_display_string().into_string()).collect();
                format!("[{}]", items_str.join(", ")).into()
            }
            ContextValue::Map(m) => {
                let mut entries: Vec<String> = Vec::new();
                let mut keys: Vec<&Text> = m.keys().collect();
                keys.sort();
                for key in keys {
                    if let Some(value) = m.get(key) {
                        entries.push(format!("{}: {}", key, value.to_display_string().as_str()));
                    }
                }
                format!("{{{}}}", entries.join(", ")).into()
            }
            ContextValue::Null => "null".to_string().into(),
        }
    }
}

impl fmt::Display for ContextValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_display_string())
    }
}

/// Trait for types that can be converted to ContextValue
///
/// Implement this trait for custom types that should be usable in
/// structured error contexts.
pub trait ToContextValue {
    /// Convert this value to a ContextValue
    fn to_context_value(&self) -> ContextValue;
}

// Implementations for primitive types

impl ToContextValue for Text {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Text(self.clone())
    }
}

impl ToContextValue for &str {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Text((*self).into())
    }
}

impl ToContextValue for i8 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Int(*self as i64)
    }
}

impl ToContextValue for i16 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Int(*self as i64)
    }
}

impl ToContextValue for i32 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Int(*self as i64)
    }
}

impl ToContextValue for i64 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Int(*self)
    }
}

impl ToContextValue for isize {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Int(*self as i64)
    }
}

impl ToContextValue for u8 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::UInt(*self as u64)
    }
}

impl ToContextValue for u16 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::UInt(*self as u64)
    }
}

impl ToContextValue for u32 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::UInt(*self as u64)
    }
}

impl ToContextValue for u64 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::UInt(*self)
    }
}

impl ToContextValue for usize {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::UInt(*self as u64)
    }
}

impl ToContextValue for f32 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Float(*self as f64)
    }
}

impl ToContextValue for f64 {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Float(*self)
    }
}

impl ToContextValue for bool {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::Bool(*self)
    }
}

// Collection implementations

impl<T: ToContextValue> ToContextValue for List<T> {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::List(self.iter().map(|v| v.to_context_value()).collect())
    }
}

impl<T: ToContextValue> ToContextValue for &[T] {
    fn to_context_value(&self) -> ContextValue {
        ContextValue::List(self.iter().map(|v| v.to_context_value()).collect())
    }
}

impl<K: ToString, V: ToContextValue> ToContextValue for Map<K, V> {
    fn to_context_value(&self) -> ContextValue {
        let mut result = Map::new();
        for (k, v) in self {
            result.insert(k.to_string().into(), v.to_context_value());
        }
        ContextValue::Map(result)
    }
}

// Maybe/Option implementation

impl<T: ToContextValue> ToContextValue for Maybe<T> {
    fn to_context_value(&self) -> ContextValue {
        match self {
            Some(v) => v.to_context_value(),
            None => ContextValue::Null,
        }
    }
}

// Direct implementation for ContextValue

impl ToContextValue for ContextValue {
    fn to_context_value(&self) -> ContextValue {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_value_text() {
        let value = ContextValue::Text(Text::from("hello"));
        assert_eq!(value.as_text(), Some(&Text::from("hello")));
        assert_eq!(value.as_int(), None);
        assert!(!value.is_null());
    }

    #[test]
    fn test_context_value_int() {
        let value = ContextValue::Int(42);
        assert_eq!(value.as_int(), Some(42));
        assert_eq!(value.as_text(), None);
    }

    #[test]
    fn test_context_value_uint() {
        let value = ContextValue::UInt(100);
        assert_eq!(value.as_uint(), Some(100));
        assert_eq!(value.as_int(), None);
    }

    #[test]
    fn test_context_value_float() {
        let value = ContextValue::Float(std::f64::consts::PI);
        assert_eq!(value.as_float(), Some(std::f64::consts::PI));
    }

    #[test]
    fn test_context_value_bool() {
        let value = ContextValue::Bool(true);
        assert_eq!(value.as_bool(), Some(true));
    }

    #[test]
    fn test_context_value_null() {
        let value = ContextValue::Null;
        assert!(value.is_null());
        assert_eq!(value.as_text(), None);
    }

    #[test]
    fn test_json_compact_primitives() {
        assert_eq!(
            ContextValue::Text(Text::from("hello")).to_json(false),
            Text::from("\"hello\"")
        );
        assert_eq!(ContextValue::Int(42).to_json(false), Text::from("42"));
        assert_eq!(ContextValue::UInt(100).to_json(false), Text::from("100"));
        assert_eq!(ContextValue::Float(1.5).to_json(false), Text::from("1.5"));
        assert_eq!(ContextValue::Bool(true).to_json(false), Text::from("true"));
        assert_eq!(ContextValue::Null.to_json(false), Text::from("null"));
    }

    #[test]
    fn test_json_escape_strings() {
        let value = ContextValue::Text(Text::from("hello \"world\"\nnewline"));
        let json = value.to_json(false);
        assert!(json.contains("\\\""));
        assert!(json.contains("\\n"));
    }

    #[test]
    fn test_json_list() {
        let value = ContextValue::List(vec![
            ContextValue::Int(1),
            ContextValue::Int(2),
            ContextValue::Int(3),
        ].into());
        assert_eq!(value.to_json(false), Text::from("[1,2,3]"));
    }

    #[test]
    fn test_json_map() {
        let mut map = Map::new();
        map.insert(Text::from("name"), ContextValue::Text(Text::from("Alice")));
        map.insert(Text::from("age"), ContextValue::Int(30));
        let value = ContextValue::Map(map);
        let json = value.to_json(false);
        // Keys are sorted
        assert!(json.contains("\"age\":30"));
        assert!(json.contains("\"name\":\"Alice\""));
    }

    #[test]
    fn test_yaml_primitives() {
        assert_eq!(ContextValue::Text(Text::from("hello")).to_yaml(0), Text::from("hello"));
        assert_eq!(ContextValue::Int(42).to_yaml(0), Text::from("42"));
        assert_eq!(ContextValue::Bool(true).to_yaml(0), Text::from("true"));
        assert_eq!(ContextValue::Null.to_yaml(0), Text::from("null"));
    }

    #[test]
    fn test_yaml_quote_special_strings() {
        let value = ContextValue::Text(Text::from("hello: world"));
        let yaml = value.to_yaml(0);
        assert!(yaml.as_str().starts_with('"'));
    }

    #[test]
    fn test_logfmt_primitives() {
        assert_eq!(ContextValue::Text(Text::from("hello")).to_logfmt(), Text::from("hello"));
        assert_eq!(ContextValue::Int(42).to_logfmt(), Text::from("42"));
        assert_eq!(ContextValue::Bool(true).to_logfmt(), Text::from("true"));
    }

    #[test]
    fn test_logfmt_quote_spaces() {
        let value = ContextValue::Text(Text::from("hello world"));
        let logfmt = value.to_logfmt();
        assert_eq!(logfmt, Text::from("\"hello world\""));
    }

    #[test]
    fn test_to_context_value_string() {
        let value = "hello".to_context_value();
        assert_eq!(value.as_text(), Some(&Text::from("hello")));
    }

    #[test]
    fn test_to_context_value_int() {
        let value = 42i32.to_context_value();
        assert_eq!(value.as_int(), Some(42));
    }

    #[test]
    fn test_to_context_value_list() {
        let list: List<i32> = vec![1, 2, 3].into();
        let value = list.to_context_value();
        let list_val = value.as_list().unwrap();
        assert_eq!(list_val.len(), 3);
    }

    #[test]
    fn test_to_context_value_maybe_some() {
        let maybe: Maybe<i32> = Some(42);
        let value = maybe.to_context_value();
        assert_eq!(value.as_int(), Some(42));
    }

    #[test]
    fn test_to_context_value_maybe_none() {
        let maybe: Maybe<i32> = None;
        let value = maybe.to_context_value();
        assert!(value.is_null());
    }
}
