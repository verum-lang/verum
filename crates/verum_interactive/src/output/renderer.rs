//! Output renderer trait and implementations.
//!
//! Provides a unified interface for rendering different value types.

use verum_common::Text;
use verum_vbc::value::Value;

/// Output format for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Plain text format.
    #[default]
    Plain,
    /// ANSI-colored terminal output.
    Ansi,
    /// HTML output for web interfaces.
    Html,
    /// Markdown output.
    Markdown,
}

/// Rendered output with optional rich formatting.
#[derive(Debug, Clone)]
pub struct RenderedOutput {
    /// Plain text content.
    pub text: Text,
    /// Formatted content (with ANSI codes, HTML, etc.).
    pub formatted: Option<Text>,
    /// Type information.
    pub type_info: Text,
    /// Whether this output can be expanded/collapsed.
    pub collapsible: bool,
    /// Preview text for collapsed view.
    pub preview: Option<Text>,
}

impl RenderedOutput {
    /// Creates a simple text output.
    pub fn text(s: impl Into<Text>, type_info: impl Into<Text>) -> Self {
        Self {
            text: s.into(),
            formatted: None,
            type_info: type_info.into(),
            collapsible: false,
            preview: None,
        }
    }

    /// Creates a formatted output.
    pub fn formatted(text: impl Into<Text>, formatted: impl Into<Text>, type_info: impl Into<Text>) -> Self {
        Self {
            text: text.into(),
            formatted: Some(formatted.into()),
            type_info: type_info.into(),
            collapsible: false,
            preview: None,
        }
    }

    /// Creates a collapsible output.
    pub fn collapsible(
        text: impl Into<Text>,
        type_info: impl Into<Text>,
        preview: impl Into<Text>,
    ) -> Self {
        Self {
            text: text.into(),
            formatted: None,
            type_info: type_info.into(),
            collapsible: true,
            preview: Some(preview.into()),
        }
    }

    /// Returns the display text for the given format.
    pub fn display(&self, format: OutputFormat) -> &Text {
        match format {
            OutputFormat::Ansi | OutputFormat::Html | OutputFormat::Markdown => {
                self.formatted.as_ref().unwrap_or(&self.text)
            }
            OutputFormat::Plain => &self.text,
        }
    }
}

/// Trait for rendering values to output.
pub trait OutputRenderer {
    /// Renders a value to output.
    fn render(&self, value: &Value, type_info: &Text, format: OutputFormat) -> RenderedOutput;

    /// Returns true if this renderer can handle the given value.
    fn can_render(&self, value: &Value, type_info: &Text) -> bool;

    /// Returns a priority (higher = more specific renderer).
    fn priority(&self) -> u32 {
        0
    }
}

/// Default renderer for basic types.
pub struct DefaultRenderer;

impl OutputRenderer for DefaultRenderer {
    fn render(&self, value: &Value, type_info: &Text, format: OutputFormat) -> RenderedOutput {
        use crate::execution::{format_value, ValueDisplayOptions};

        let options = ValueDisplayOptions::default();
        let text = format_value(value, &options);

        let formatted = match format {
            OutputFormat::Ansi => Some(colorize_value(value, &text)),
            OutputFormat::Html => Some(html_format_value(value, &text)),
            OutputFormat::Markdown => Some(markdown_format_value(value, &text)),
            OutputFormat::Plain => None,
        };

        RenderedOutput {
            text,
            formatted,
            type_info: type_info.clone(),
            collapsible: false,
            preview: None,
        }
    }

    fn can_render(&self, _value: &Value, _type_info: &Text) -> bool {
        true // Default renderer handles everything
    }

    fn priority(&self) -> u32 {
        0 // Lowest priority
    }
}

/// ANSI color codes for terminal output.
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";

    // Colors
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";

    // Bright colors
    pub const BRIGHT_RED: &str = "\x1b[91m";
    pub const BRIGHT_GREEN: &str = "\x1b[92m";
    pub const BRIGHT_YELLOW: &str = "\x1b[93m";
    pub const BRIGHT_BLUE: &str = "\x1b[94m";
    pub const BRIGHT_MAGENTA: &str = "\x1b[95m";
    pub const BRIGHT_CYAN: &str = "\x1b[96m";
}

/// Colorizes a value for ANSI terminal output.
fn colorize_value(value: &Value, text: &Text) -> Text {
    let color = if value.is_int() || value.is_float() {
        ansi::BRIGHT_CYAN
    } else if value.is_bool() {
        if value.as_bool() {
            ansi::BRIGHT_GREEN
        } else {
            ansi::BRIGHT_RED
        }
    } else if value.is_small_string() {
        ansi::BRIGHT_YELLOW
    } else if value.is_nil() || value.is_unit() {
        ansi::DIM
    } else if value.is_func_ref() {
        ansi::BRIGHT_MAGENTA
    } else if value.is_type_ref() {
        ansi::BRIGHT_BLUE
    } else {
        ansi::RESET
    };

    Text::from(format!("{}{}{}", color, text.as_str(), ansi::RESET).as_str())
}

/// Formats a value for HTML output.
fn html_format_value(value: &Value, text: &Text) -> Text {
    let class = if value.is_int() || value.is_float() {
        "number"
    } else if value.is_bool() {
        "boolean"
    } else if value.is_small_string() {
        "string"
    } else if value.is_nil() || value.is_unit() {
        "null"
    } else if value.is_func_ref() {
        "function"
    } else if value.is_type_ref() {
        "type"
    } else {
        "value"
    };

    Text::from(format!("<span class=\"{}\">{}<!-- raw HTML omitted --></span>", class, html_escape(text.as_str())).as_str())
}

/// Formats a value for Markdown output.
fn markdown_format_value(value: &Value, text: &Text) -> Text {
    if value.is_int() || value.is_float() || value.is_bool() || value.is_nil() {
        Text::from(format!("`{}`", text.as_str()).as_str())
    } else if value.is_small_string() {
        Text::from(format!("\"{}\"", text.as_str()).as_str())
    } else {
        text.clone()
    }
}

/// Escapes HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Registry of output renderers.
pub struct RendererRegistry {
    renderers: Vec<Box<dyn OutputRenderer + Send + Sync>>,
}

impl Default for RendererRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl RendererRegistry {
    /// Creates a new registry with the default renderer.
    pub fn new() -> Self {
        Self {
            renderers: vec![Box::new(DefaultRenderer)],
        }
    }

    /// Registers a custom renderer.
    pub fn register<R: OutputRenderer + Send + Sync + 'static>(&mut self, renderer: R) {
        self.renderers.push(Box::new(renderer));
        // Sort by priority (highest first)
        self.renderers.sort_by_key(|r| std::cmp::Reverse(r.priority()));
    }

    /// Finds the best renderer for a value.
    pub fn find_renderer(&self, value: &Value, type_info: &Text) -> &dyn OutputRenderer {
        for renderer in &self.renderers {
            if renderer.can_render(value, type_info) {
                return renderer.as_ref();
            }
        }
        // Should never happen since DefaultRenderer handles everything
        self.renderers.last().unwrap().as_ref()
    }

    /// Renders a value using the best available renderer.
    pub fn render(&self, value: &Value, type_info: &Text, format: OutputFormat) -> RenderedOutput {
        self.find_renderer(value, type_info).render(value, type_info, format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rendered_output_text() {
        let output = RenderedOutput::text("42", "Int");
        assert_eq!(output.text.as_str(), "42");
        assert_eq!(output.type_info.as_str(), "Int");
        assert!(!output.collapsible);
    }

    #[test]
    fn test_default_renderer() {
        let renderer = DefaultRenderer;
        let value = Value::from_i64(42);
        let type_info = Text::from("Int");

        assert!(renderer.can_render(&value, &type_info));

        let output = renderer.render(&value, &type_info, OutputFormat::Plain);
        assert_eq!(output.text.as_str(), "42");
    }

    #[test]
    fn test_renderer_registry() {
        let registry = RendererRegistry::new();
        let value = Value::from_bool(true);
        let type_info = Text::from("Bool");

        let output = registry.render(&value, &type_info, OutputFormat::Plain);
        assert_eq!(output.text.as_str(), "true");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<div>"), "&lt;div&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape("\"quoted\""), "&quot;quoted&quot;");
    }
}
