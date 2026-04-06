//! Value formatting for display in the playbook.
//!
//! Converts VBC `Value` instances to human-readable string representations
//! with appropriate formatting for different types.

use verum_common::Text;
use verum_vbc::value::Value;

/// Display options for value formatting.
#[derive(Debug, Clone)]
pub struct ValueDisplayOptions {
    /// Maximum string length before truncation.
    pub max_string_length: usize,
    /// Maximum collection elements to show.
    pub max_collection_elements: usize,
    /// Maximum depth for nested structures.
    pub max_depth: usize,
    /// Whether to show type annotations.
    pub show_types: bool,
    /// Number format: "decimal", "hex", "binary".
    pub number_format: NumberFormat,
    /// Float precision (decimal places).
    pub float_precision: usize,
    /// Whether to show memory addresses for pointers.
    pub show_addresses: bool,
}

/// Number format options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumberFormat {
    #[default]
    Decimal,
    Hex,
    Binary,
    Scientific,
}

impl Default for ValueDisplayOptions {
    fn default() -> Self {
        Self {
            max_string_length: 1000,
            max_collection_elements: 50,
            max_depth: 5,
            show_types: true,
            number_format: NumberFormat::Decimal,
            float_precision: 6,
            show_addresses: false,
        }
    }
}

impl ValueDisplayOptions {
    /// Creates options suitable for a compact single-line display.
    pub fn compact() -> Self {
        Self {
            max_string_length: 80,
            max_collection_elements: 5,
            max_depth: 2,
            show_types: false,
            number_format: NumberFormat::Decimal,
            float_precision: 4,
            show_addresses: false,
        }
    }

    /// Creates options suitable for verbose debug output.
    pub fn verbose() -> Self {
        Self {
            max_string_length: 10000,
            max_collection_elements: 1000,
            max_depth: 10,
            show_types: true,
            number_format: NumberFormat::Decimal,
            float_precision: 15,
            show_addresses: true,
        }
    }
}

/// Formats a value for display.
///
/// Returns the formatted string representation.
pub fn format_value(value: &Value, options: &ValueDisplayOptions) -> Text {
    format_value_recursive(value, options, 0)
}

/// Formats a value with its type information.
///
/// Returns (display_string, type_string).
pub fn format_value_with_type(
    value: &Value,
    type_hint: &Text,
    options: &ValueDisplayOptions,
) -> (Text, Text) {
    let display = format_value(value, options);
    let type_info = infer_type_from_value(value, type_hint);
    (display, type_info)
}

/// Recursive value formatter with depth tracking.
fn format_value_recursive(value: &Value, options: &ValueDisplayOptions, depth: usize) -> Text {
    if depth > options.max_depth {
        return Text::from("...");
    }

    // Check value type using Value methods
    if value.is_float() {
        return format_float(value.as_f64(), options);
    }

    if value.is_int() {
        return format_int(value.as_i64(), options);
    }

    if value.is_bool() {
        return Text::from(if value.as_bool() { "true" } else { "false" });
    }

    if value.is_unit() {
        return Text::from("()");
    }

    if value.is_nil() {
        return Text::from("nil");
    }

    if value.is_small_string() {
        let s = value.as_small_string();
        return format_string(s.as_str(), options);
    }

    if value.is_type_ref() {
        let type_id = value.as_type_id();
        return Text::from(format!("<type:{}>", type_id.0).as_str());
    }

    if value.is_func_ref() {
        let func_id = value.as_func_id();
        return Text::from(format!("<fn:{}>", func_id.0).as_str());
    }

    if value.is_ptr() {
        // Could be an object reference or other pointer type
        if options.show_addresses {
            return Text::from(format!("<ptr:{:?}>", value.as_ptr::<()>()).as_str());
        } else {
            return Text::from("<object>");
        }
    }

    // For complex types we'd need more context
    // For now, return a generic representation
    Text::from(format!("<value:{:#x}>", value.to_bits()).as_str())
}

/// Formats an integer value.
fn format_int(value: i64, options: &ValueDisplayOptions) -> Text {
    let s = match options.number_format {
        NumberFormat::Decimal => format!("{}", value),
        NumberFormat::Hex => format!("0x{:x}", value),
        NumberFormat::Binary => format!("0b{:b}", value),
        NumberFormat::Scientific => format!("{:e}", value as f64),
    };
    Text::from(s.as_str())
}

/// Formats a floating-point value.
fn format_float(value: f64, options: &ValueDisplayOptions) -> Text {
    let s = if value.is_nan() {
        "NaN".to_string()
    } else if value.is_infinite() {
        if value.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    } else {
        match options.number_format {
            NumberFormat::Scientific => format!("{:.*e}", options.float_precision, value),
            _ => {
                // Use default precision formatting
                let s = format!("{:.*}", options.float_precision, value);
                // Trim trailing zeros but keep at least one decimal place
                let trimmed = s.trim_end_matches('0');
                if trimmed.ends_with('.') {
                    format!("{}0", trimmed)
                } else {
                    trimmed.to_string()
                }
            }
        }
    };
    Text::from(s.as_str())
}

/// Formats a string value.
fn format_string(value: &str, options: &ValueDisplayOptions) -> Text {
    if value.len() > options.max_string_length {
        let truncated = &value[..options.max_string_length];
        Text::from(format!("\"{}...\"", escape_string(truncated)).as_str())
    } else {
        Text::from(format!("\"{}\"", escape_string(value)).as_str())
    }
}

/// Escapes special characters in a string.
fn escape_string(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            c if c.is_control() => result.push_str(&format!("\\u{{{:04x}}}", c as u32)),
            c => result.push(c),
        }
    }
    result
}

/// Infers type information from a value.
fn infer_type_from_value(value: &Value, type_hint: &Text) -> Text {
    // If we have a type hint and it's not generic, use it
    if !type_hint.as_str().starts_with('<') && !type_hint.is_empty() {
        return type_hint.clone();
    }

    // Otherwise infer from the value
    if value.is_float() {
        Text::from("Float")
    } else if value.is_int() {
        Text::from("Int")
    } else if value.is_bool() {
        Text::from("Bool")
    } else if value.is_unit() {
        Text::from("()")
    } else if value.is_nil() {
        Text::from("Nil")
    } else if value.is_small_string() {
        Text::from("Text")
    } else if value.is_type_ref() {
        Text::from("Type")
    } else if value.is_func_ref() {
        Text::from("Function")
    } else if value.is_ptr() {
        Text::from("Object")
    } else {
        Text::from("<unknown>")
    }
}

/// Formats a list of values as a collection.
pub fn format_collection(
    values: &[Value],
    element_type: &str,
    options: &ValueDisplayOptions,
) -> Text {
    let len = values.len();
    let show_count = len.min(options.max_collection_elements);

    let mut parts: Vec<String> = values[..show_count]
        .iter()
        .map(|v| format_value(v, options).to_string())
        .collect();

    if len > show_count {
        parts.push(format!("... ({} more)", len - show_count));
    }

    if options.show_types {
        Text::from(format!("[{}]: List<{}>", parts.join(", "), element_type).as_str())
    } else {
        Text::from(format!("[{}]", parts.join(", ")).as_str())
    }
}

/// Formats a map/dictionary of key-value pairs.
pub fn format_map(
    entries: &[(Value, Value)],
    key_type: &str,
    value_type: &str,
    options: &ValueDisplayOptions,
) -> Text {
    let len = entries.len();
    let show_count = len.min(options.max_collection_elements);

    let mut parts: Vec<String> = entries[..show_count]
        .iter()
        .map(|(k, v)| {
            format!(
                "{}: {}",
                format_value(k, options),
                format_value(v, options)
            )
        })
        .collect();

    if len > show_count {
        parts.push(format!("... ({} more)", len - show_count));
    }

    if options.show_types {
        Text::from(format!("{{ {} }}: Map<{}, {}>", parts.join(", "), key_type, value_type).as_str())
    } else {
        Text::from(format!("{{ {} }}", parts.join(", ")).as_str())
    }
}

/// Formats a struct/record with named fields.
pub fn format_struct(
    type_name: &str,
    fields: &[(&str, Value)],
    options: &ValueDisplayOptions,
) -> Text {
    let field_strs: Vec<String> = fields
        .iter()
        .map(|(name, value)| format!("{}: {}", name, format_value(value, options)))
        .collect();

    Text::from(format!("{} {{ {} }}", type_name, field_strs.join(", ")).as_str())
}

/// Formats an enum/variant value.
pub fn format_variant(
    variant_name: &str,
    payload: Option<&Value>,
    options: &ValueDisplayOptions,
) -> Text {
    match payload {
        Some(value) => {
            Text::from(format!("{}({})", variant_name, format_value(value, options)).as_str())
        }
        None => Text::from(variant_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_int() {
        let options = ValueDisplayOptions::default();

        assert_eq!(format_int(42, &options).as_str(), "42");
        assert_eq!(format_int(-123, &options).as_str(), "-123");
        assert_eq!(format_int(0, &options).as_str(), "0");
    }

    #[test]
    fn test_format_int_hex() {
        let mut options = ValueDisplayOptions::default();
        options.number_format = NumberFormat::Hex;

        assert_eq!(format_int(255, &options).as_str(), "0xff");
        assert_eq!(format_int(0, &options).as_str(), "0x0");
    }

    #[test]
    fn test_format_float() {
        let options = ValueDisplayOptions::default();

        let result = format_float(3.14159, &options);
        assert!(result.as_str().starts_with("3.14"));

        assert_eq!(format_float(f64::NAN, &options).as_str(), "NaN");
        assert_eq!(format_float(f64::INFINITY, &options).as_str(), "Infinity");
        assert_eq!(format_float(f64::NEG_INFINITY, &options).as_str(), "-Infinity");
    }

    #[test]
    fn test_format_string() {
        let options = ValueDisplayOptions::default();

        assert_eq!(format_string("hello", &options).as_str(), "\"hello\"");
        assert_eq!(format_string("with\nnewline", &options).as_str(), "\"with\\nnewline\"");
        assert_eq!(format_string("with\ttab", &options).as_str(), "\"with\\ttab\"");
    }

    #[test]
    fn test_format_string_truncation() {
        let mut options = ValueDisplayOptions::default();
        options.max_string_length = 5;

        let result = format_string("hello world", &options);
        assert!(result.as_str().contains("..."));
    }

    #[test]
    fn test_format_value_basic() {
        let options = ValueDisplayOptions::default();

        assert_eq!(format_value(&Value::from_i64(42), &options).as_str(), "42");
        assert_eq!(format_value(&Value::from_bool(true), &options).as_str(), "true");
        assert_eq!(format_value(&Value::unit(), &options).as_str(), "()");
    }

    #[test]
    fn test_escape_string() {
        assert_eq!(escape_string("hello"), "hello");
        assert_eq!(escape_string("line\nbreak"), "line\\nbreak");
        assert_eq!(escape_string("tab\there"), "tab\\there");
        assert_eq!(escape_string("quote\"here"), "quote\\\"here");
    }
}
