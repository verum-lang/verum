//! Structured data output rendering.
//!
//! Provides rich visualization for records, variants, and collections.

use verum_common::Text;
use verum_vbc::value::Value;

use crate::execution::{format_value, ValueDisplayOptions};
use super::renderer::{OutputFormat, RenderedOutput};

/// Renders a struct/record value.
pub fn render_struct(
    type_name: &str,
    fields: &[(&str, Value)],
    format: OutputFormat,
) -> RenderedOutput {
    let options = ValueDisplayOptions::default();

    // Build field strings
    let field_strs: Vec<String> = fields
        .iter()
        .map(|(name, value)| {
            format!("{}: {}", name, format_value(value, &options))
        })
        .collect();

    let text = if fields.len() <= 3 {
        // Inline format for small structs
        format!("{} {{ {} }}", type_name, field_strs.join(", "))
    } else {
        // Multi-line format for larger structs
        let mut lines = vec![format!("{} {{", type_name)];
        for field_str in &field_strs {
            lines.push(format!("  {},", field_str));
        }
        lines.push("}".to_string());
        lines.join("\n")
    };

    let formatted = match format {
        OutputFormat::Ansi => Some(colorize_struct(&text, type_name)),
        OutputFormat::Html => Some(html_format_struct(&text, type_name, fields)),
        _ => None,
    };

    let preview = format!("{} {{ {} fields }}", type_name, fields.len());

    RenderedOutput {
        text: Text::from(text.as_str()),
        formatted: formatted.map(|s| Text::from(s.as_str())),
        type_info: Text::from(type_name),
        collapsible: fields.len() > 3,
        preview: Some(Text::from(preview.as_str())),
    }
}

/// Renders a variant/enum value.
pub fn render_variant(
    variant_name: &str,
    payload: Option<&Value>,
    format: OutputFormat,
) -> RenderedOutput {
    let options = ValueDisplayOptions::default();

    let text = match payload {
        Some(value) => format!("{}({})", variant_name, format_value(value, &options)),
        None => variant_name.to_string(),
    };

    let formatted = match format {
        OutputFormat::Ansi => Some(colorize_variant(&text, variant_name)),
        OutputFormat::Html => Some(html_format_variant(&text, variant_name)),
        _ => None,
    };

    RenderedOutput {
        text: Text::from(text.as_str()),
        formatted: formatted.map(|s| Text::from(s.as_str())),
        type_info: Text::from(variant_name),
        collapsible: false,
        preview: None,
    }
}

/// Renders a collection (List, Set, etc.).
pub fn render_collection(
    type_name: &str,
    element_type: &str,
    elements: &[Value],
    format: OutputFormat,
) -> RenderedOutput {
    let options = ValueDisplayOptions::default();
    let len = elements.len();
    let max_show = options.max_collection_elements;

    // Build element strings
    let show_count = len.min(max_show);
    let mut element_strs: Vec<String> = elements
        .iter()
        .take(show_count)
        .map(|v| format_value(v, &options).to_string())
        .collect();

    if len > max_show {
        element_strs.push(format!("... ({} more)", len - max_show));
    }

    let text = if len <= 10 {
        // Inline format for small collections
        format!("[{}]", element_strs.join(", "))
    } else {
        // Multi-line format for larger collections
        let mut lines = vec!["[".to_string()];
        for elem_str in &element_strs {
            lines.push(format!("  {},", elem_str));
        }
        lines.push("]".to_string());
        lines.join("\n")
    };

    let type_sig = format!("{}<{}>", type_name, element_type);
    let formatted = match format {
        OutputFormat::Ansi => Some(colorize_collection(&text)),
        OutputFormat::Html => Some(html_format_collection(&text, &type_sig)),
        _ => None,
    };

    let preview = format!("{}<{}>: {} elements", type_name, element_type, len);

    RenderedOutput {
        text: Text::from(text.as_str()),
        formatted: formatted.map(|s| Text::from(s.as_str())),
        type_info: Text::from(type_sig.as_str()),
        collapsible: len > 10,
        preview: Some(Text::from(preview.as_str())),
    }
}

/// Renders a map/dictionary.
pub fn render_map(
    key_type: &str,
    value_type: &str,
    entries: &[(Value, Value)],
    format: OutputFormat,
) -> RenderedOutput {
    let options = ValueDisplayOptions::default();
    let len = entries.len();
    let max_show = options.max_collection_elements;

    // Build entry strings
    let show_count = len.min(max_show);
    let mut entry_strs: Vec<String> = entries
        .iter()
        .take(show_count)
        .map(|(k, v)| {
            format!(
                "{}: {}",
                format_value(k, &options),
                format_value(v, &options)
            )
        })
        .collect();

    if len > max_show {
        entry_strs.push(format!("... ({} more)", len - max_show));
    }

    let text = if len <= 5 {
        // Inline format for small maps
        format!("{{ {} }}", entry_strs.join(", "))
    } else {
        // Multi-line format for larger maps
        let mut lines = vec!["{".to_string()];
        for entry_str in &entry_strs {
            lines.push(format!("  {},", entry_str));
        }
        lines.push("}".to_string());
        lines.join("\n")
    };

    let type_sig = format!("Map<{}, {}>", key_type, value_type);
    let formatted = match format {
        OutputFormat::Ansi => Some(colorize_collection(&text)),
        OutputFormat::Html => Some(html_format_collection(&text, &type_sig)),
        _ => None,
    };

    let preview = format!("Map<{}, {}>: {} entries", key_type, value_type, len);

    RenderedOutput {
        text: Text::from(text.as_str()),
        formatted: formatted.map(|s| Text::from(s.as_str())),
        type_info: Text::from(type_sig.as_str()),
        collapsible: len > 5,
        preview: Some(Text::from(preview.as_str())),
    }
}

/// Renders a tuple value.
pub fn render_tuple(elements: &[Value], format: OutputFormat) -> RenderedOutput {
    let options = ValueDisplayOptions::default();

    let element_strs: Vec<String> = elements
        .iter()
        .map(|v| format_value(v, &options).to_string())
        .collect();

    let text = format!("({})", element_strs.join(", "));

    let type_sig = format!(
        "({})",
        elements
            .iter()
            .map(|_| "_")
            .collect::<Vec<_>>()
            .join(", ")
    );

    let formatted = match format {
        OutputFormat::Ansi => Some(colorize_collection(&text)),
        OutputFormat::Html => Some(html_format_collection(&text, &type_sig)),
        _ => None,
    };

    RenderedOutput {
        text: Text::from(text.as_str()),
        formatted: formatted.map(|s| Text::from(s.as_str())),
        type_info: Text::from(type_sig.as_str()),
        collapsible: false,
        preview: None,
    }
}

// ============================================================================
// ANSI Colorization
// ============================================================================

fn colorize_struct(text: &str, type_name: &str) -> String {
    // Color the type name in magenta
    text.replace(
        type_name,
        &format!("\x1b[35m{}\x1b[0m", type_name),
    )
}

fn colorize_variant(text: &str, variant_name: &str) -> String {
    // Color the variant name in cyan
    text.replace(
        variant_name,
        &format!("\x1b[36m{}\x1b[0m", variant_name),
    )
}

fn colorize_collection(text: &str) -> String {
    // Color brackets in dim
    text.replace('[', "\x1b[2m[\x1b[0m")
        .replace(']', "\x1b[2m]\x1b[0m")
        .replace('{', "\x1b[2m{\x1b[0m")
        .replace('}', "\x1b[2m}\x1b[0m")
}

// ============================================================================
// HTML Formatting
// ============================================================================

fn html_format_struct(text: &str, type_name: &str, fields: &[(&str, Value)]) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        "<pre class=\"struct\" data-type=\"{}\" data-fields=\"{}\">{}<!-- raw HTML omitted --></pre>",
        type_name,
        fields.len(),
        escaped
    )
}

fn html_format_variant(text: &str, variant_name: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        "<span class=\"variant\" data-name=\"{}\">{}<!-- raw HTML omitted --></span>",
        variant_name, escaped
    )
}

fn html_format_collection(text: &str, type_sig: &str) -> String {
    let escaped = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    format!(
        "<pre class=\"collection\" data-type=\"{}\">{}<!-- raw HTML omitted --></pre>",
        type_sig.replace('<', "&lt;").replace('>', "&gt;"),
        escaped
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_struct_small() {
        let fields = vec![
            ("x", Value::from_i64(1)),
            ("y", Value::from_i64(2)),
        ];
        let result = render_struct("Point", &fields, OutputFormat::Plain);

        assert!(result.text.as_str().contains("Point"));
        assert!(result.text.as_str().contains("x: 1"));
        assert!(result.text.as_str().contains("y: 2"));
        assert!(!result.collapsible);
    }

    #[test]
    fn test_render_struct_large() {
        let fields = vec![
            ("a", Value::from_i64(1)),
            ("b", Value::from_i64(2)),
            ("c", Value::from_i64(3)),
            ("d", Value::from_i64(4)),
            ("e", Value::from_i64(5)),
        ];
        let result = render_struct("BigStruct", &fields, OutputFormat::Plain);

        assert!(result.collapsible);
        assert!(result.preview.is_some());
    }

    #[test]
    fn test_render_variant_unit() {
        let result = render_variant("None", None, OutputFormat::Plain);
        assert_eq!(result.text.as_str(), "None");
    }

    #[test]
    fn test_render_variant_with_payload() {
        let result = render_variant("Some", Some(&Value::from_i64(42)), OutputFormat::Plain);
        assert!(result.text.as_str().contains("Some(42)"));
    }

    #[test]
    fn test_render_collection_small() {
        let elements = vec![
            Value::from_i64(1),
            Value::from_i64(2),
            Value::from_i64(3),
        ];
        let result = render_collection("List", "Int", &elements, OutputFormat::Plain);

        assert!(result.text.as_str().contains("[1, 2, 3]"));
        assert!(!result.collapsible);
    }

    #[test]
    fn test_render_tuple() {
        let elements = vec![
            Value::from_i64(1),
            Value::from_bool(true),
        ];
        let result = render_tuple(&elements, OutputFormat::Plain);

        assert!(result.text.as_str().contains("(1, true)"));
    }
}
