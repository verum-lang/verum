//! ANSI color support for rich diagnostics.
//!
//! Provides a clean abstraction over terminal colors with support for:
//! - Theme-based color schemes
//! - Automatic color detection and disabling
//! - Unicode vs ASCII fallback for glyphs

use std::env;
use std::io::{self, IsTerminal};

/// Color scheme for diagnostic rendering
#[derive(Debug, Clone)]
pub struct ColorScheme {
    pub error_code: Color,
    pub severity_error: Color,
    pub severity_warning: Color,
    pub severity_note: Color,
    pub severity_help: Color,
    pub file_path: Color,
    pub line_number: Color,
    pub source_code: Color,
    pub caret: Color,
    pub underline_primary: Color,
    pub underline_secondary: Color,
    pub gutter: Color,
    pub border: Color,
    pub suggestion_add: Color,
    pub suggestion_remove: Color,
}

impl ColorScheme {
    /// Default color scheme with full ANSI colors
    pub fn default_colors() -> Self {
        Self {
            error_code: Color::Red,
            severity_error: Color::Red,
            severity_warning: Color::Yellow,
            severity_note: Color::Cyan,
            severity_help: Color::Green,
            file_path: Color::Cyan,
            line_number: Color::Blue,
            source_code: Color::White,
            caret: Color::Red,
            underline_primary: Color::Red,
            underline_secondary: Color::Blue,
            gutter: Color::Blue,
            border: Color::Blue,
            suggestion_add: Color::Green,
            suggestion_remove: Color::Red,
        }
    }

    /// No-color scheme (all colors disabled)
    pub fn no_color() -> Self {
        Self {
            error_code: Color::None,
            severity_error: Color::None,
            severity_warning: Color::None,
            severity_note: Color::None,
            severity_help: Color::None,
            file_path: Color::None,
            line_number: Color::None,
            source_code: Color::None,
            caret: Color::None,
            underline_primary: Color::None,
            underline_secondary: Color::None,
            gutter: Color::None,
            border: Color::None,
            suggestion_add: Color::None,
            suggestion_remove: Color::None,
        }
    }

    /// Auto-detect whether colors should be enabled
    pub fn auto() -> Self {
        if Self::should_use_color() {
            Self::default_colors()
        } else {
            Self::no_color()
        }
    }

    /// Check if colors should be enabled based on environment
    fn should_use_color() -> bool {
        // Check NO_COLOR environment variable (https://no-color.org/)
        if env::var("NO_COLOR").is_ok() {
            return false;
        }

        // Check CLICOLOR_FORCE
        if env::var("CLICOLOR_FORCE").is_ok() {
            return true;
        }

        // Check if stdout is a terminal
        if !io::stdout().is_terminal() {
            return false;
        }

        // Check TERM variable
        if let Ok(term) = env::var("TERM")
            && term == "dumb"
        {
            return false;
        }

        true
    }

    /// Wrap text with the appropriate color
    pub fn colorize(&self, text: &str, color: &Color) -> String {
        color.wrap(text)
    }
}

/// ANSI color codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Red,
    Green,
    Blue,
    Yellow,
    Cyan,
    Magenta,
    White,
    Black,
    BrightRed,
    BrightGreen,
    BrightBlue,
    BrightYellow,
    BrightCyan,
    BrightMagenta,
    BrightWhite,
    None,
}

impl Color {
    /// Wrap text with ANSI color codes
    pub fn wrap(&self, text: &str) -> String {
        match self {
            Color::Red => format!("\x1b[31;1m{}\x1b[0m", text),
            Color::Green => format!("\x1b[32;1m{}\x1b[0m", text),
            Color::Blue => format!("\x1b[34;1m{}\x1b[0m", text),
            Color::Yellow => format!("\x1b[33;1m{}\x1b[0m", text),
            Color::Cyan => format!("\x1b[36;1m{}\x1b[0m", text),
            Color::Magenta => format!("\x1b[35;1m{}\x1b[0m", text),
            Color::White => format!("\x1b[37;1m{}\x1b[0m", text),
            Color::Black => format!("\x1b[30;1m{}\x1b[0m", text),
            Color::BrightRed => format!("\x1b[91;1m{}\x1b[0m", text),
            Color::BrightGreen => format!("\x1b[92;1m{}\x1b[0m", text),
            Color::BrightBlue => format!("\x1b[94;1m{}\x1b[0m", text),
            Color::BrightYellow => format!("\x1b[93;1m{}\x1b[0m", text),
            Color::BrightCyan => format!("\x1b[96;1m{}\x1b[0m", text),
            Color::BrightMagenta => format!("\x1b[95;1m{}\x1b[0m", text),
            Color::BrightWhite => format!("\x1b[97;1m{}\x1b[0m", text),
            Color::None => text.to_string(),
        }
    }

    /// Wrap text with dim (faint) style
    pub fn wrap_dim(&self, text: &str) -> String {
        match self {
            Color::None => text.to_string(),
            _ => {
                let colored = self.wrap(text);
                // Insert dim code after the color code
                colored.replace("\x1b[", "\x1b[2;")
            }
        }
    }
}

/// Glyph set for rendering (Unicode vs ASCII)
#[derive(Debug, Clone)]
pub struct GlyphSet {
    pub horizontal_line: &'static str,
    pub vertical_line: &'static str,
    pub top_left_corner: &'static str,
    pub top_right_corner: &'static str,
    pub bottom_left_corner: &'static str,
    pub bottom_right_corner: &'static str,
    pub vertical_right: &'static str,
    pub left_bracket: &'static str,
    pub right_bracket: &'static str,
    pub underline_char: &'static str,
    pub arrow_right: &'static str,
    pub bullet: &'static str,
}

impl GlyphSet {
    /// Unicode glyphs (default for modern terminals)
    pub fn unicode() -> Self {
        Self {
            horizontal_line: "─",
            vertical_line: "│",
            top_left_corner: "┌",
            top_right_corner: "┐",
            bottom_left_corner: "└",
            bottom_right_corner: "┘",
            vertical_right: "├",
            left_bracket: "┤",
            right_bracket: "├",
            underline_char: "─",
            arrow_right: "→",
            bullet: "•",
        }
    }

    /// ASCII glyphs (fallback for limited terminals)
    pub fn ascii() -> Self {
        Self {
            horizontal_line: "-",
            vertical_line: "|",
            top_left_corner: "+",
            top_right_corner: "+",
            bottom_left_corner: "+",
            bottom_right_corner: "+",
            vertical_right: "+",
            left_bracket: "+",
            right_bracket: "+",
            underline_char: "^",
            arrow_right: "->",
            bullet: "*",
        }
    }

    /// Auto-detect based on environment
    pub fn auto() -> Self {
        if Self::should_use_unicode() {
            Self::unicode()
        } else {
            Self::ascii()
        }
    }

    /// Check if Unicode glyphs should be used
    fn should_use_unicode() -> bool {
        // Check environment variable
        if let Ok(val) = env::var("VERUM_ASCII")
            && (val == "1" || val.to_lowercase() == "true")
        {
            return false;
        }

        // Check LC_ALL, LC_CTYPE, LANG for UTF-8
        for var in &["LC_ALL", "LC_CTYPE", "LANG"] {
            if let Ok(val) = env::var(var)
                && (val.to_uppercase().contains("UTF-8") || val.to_uppercase().contains("UTF8"))
            {
                return true;
            }
        }

        // Default to Unicode on most modern systems
        true
    }
}

/// Style combination for text
#[derive(Debug, Clone)]
pub struct Style {
    pub color: Color,
    pub bold: bool,
    pub dim: bool,
    pub underline: bool,
}

impl Style {
    pub fn new(color: Color) -> Self {
        Self {
            color,
            bold: false,
            dim: false,
            underline: false,
        }
    }

    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    pub fn apply(&self, text: &str) -> String {
        if self.color == Color::None {
            return text.to_string();
        }

        let mut codes = Vec::new();

        // Add color code
        let color_code = match self.color {
            Color::Red => "31",
            Color::Green => "32",
            Color::Blue => "34",
            Color::Yellow => "33",
            Color::Cyan => "36",
            Color::Magenta => "35",
            Color::White => "37",
            Color::Black => "30",
            Color::BrightRed => "91",
            Color::BrightGreen => "92",
            Color::BrightBlue => "94",
            Color::BrightYellow => "93",
            Color::BrightCyan => "96",
            Color::BrightMagenta => "95",
            Color::BrightWhite => "97",
            Color::None => return text.to_string(),
        };
        codes.push(color_code.to_string());

        if self.bold {
            codes.push("1".to_string());
        }
        if self.dim {
            codes.push("2".to_string());
        }
        if self.underline {
            codes.push("4".to_string());
        }

        format!("\x1b[{}m{}\x1b[0m", codes.join(";"), text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_wrap() {
        let red = Color::Red;
        let text = "error";
        let wrapped = red.wrap(text);
        assert!(wrapped.contains("error"));
        assert!(wrapped.starts_with("\x1b["));
        assert!(wrapped.ends_with("\x1b[0m"));
    }

    #[test]
    fn test_no_color() {
        let none = Color::None;
        let text = "no color";
        let wrapped = none.wrap(text);
        assert_eq!(wrapped, text);
    }

    #[test]
    fn test_color_scheme_no_color() {
        let scheme = ColorScheme::no_color();
        let text = "test";
        let result = scheme.colorize(text, &scheme.error_code);
        assert_eq!(result, text);
    }

    #[test]
    fn test_glyph_set_unicode() {
        let glyphs = GlyphSet::unicode();
        assert_eq!(glyphs.horizontal_line, "─");
        assert_eq!(glyphs.vertical_line, "│");
        assert_eq!(glyphs.arrow_right, "→");
    }

    #[test]
    fn test_glyph_set_ascii() {
        let glyphs = GlyphSet::ascii();
        assert_eq!(glyphs.horizontal_line, "-");
        assert_eq!(glyphs.vertical_line, "|");
        assert_eq!(glyphs.arrow_right, "->");
    }

    #[test]
    fn test_style_application() {
        let style = Style::new(Color::Red).bold().underline();
        let result = style.apply("test");
        assert!(result.contains("test"));
        // Should contain both bold (1) and underline (4) codes
        assert!(result.contains("\x1b["));
    }

    #[test]
    fn test_style_with_no_color() {
        let style = Style::new(Color::None).bold();
        let result = style.apply("test");
        assert_eq!(result, "test");
    }
}
