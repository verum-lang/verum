//! Formatting utilities for Verum
//!
//! This module provides centralized formatting functions to eliminate duplication
//! across the codebase. All formatting operations should use these utilities.
//!
//! # Architecture
//!
//! ```text
//! verum_common::formatting (foundation)
//!   ├─ List formatting (format_list, format_list_with, format_list_pretty)
//!   ├─ Collection formatting (format_map, format_set)
//!   ├─ Cycle formatting (format_cycle, format_dependency_chain)
//!   ├─ Code formatting (format_code_block, format_inline_code, format_function_signature)
//!   ├─ Text utilities (truncate_with_ellipsis, indent, wrap_text)
//!   └─ Builders (ListFormatter, TableFormatter)
//! ```
//!
//! # Usage
//!
//! ```rust
//! use verum_common::formatting::{format_list, format_cycle, format_code_block};
//!
//! // Format a simple list
//! let items = vec!["a", "b", "c"];
//! assert_eq!(format_list(&items, ", "), "a, b, c");
//!
//! // Format a dependency cycle
//! let cycle = vec!["mod_a", "mod_b", "mod_c"];
//! assert_eq!(format_cycle(&cycle), "mod_a -> mod_b -> mod_c -> mod_a");
//!
//! // Format a code block with indentation
//! let code = "fn main() {\n    println!(\"Hello\");\n}";
//! let formatted = format_code_block(code, 4);
//! ```
//!
//! Centralized formatting eliminates duplication across crates and ensures
//! consistent output formatting for diagnostics, error messages, and debug output.

use crate::{List, Map, Maybe, OrderedMap, OrderedSet, Set, Text};
use std::fmt::Display;
use std::hash::Hash;

// ================================================================================================
// List Formatting
// ================================================================================================

/// Format a list of items with a custom separator
///
/// # Examples
/// ```
/// use verum_common::formatting::format_list;
///
/// let items = vec!["apple", "banana", "cherry"];
/// assert_eq!(format_list(&items, ", "), "apple, banana, cherry");
/// assert_eq!(format_list(&items, " | "), "apple | banana | cherry");
///
/// let empty: Vec<&str> = vec![];
/// assert_eq!(format_list(&empty, ", "), "");
/// ```
pub fn format_list<T: Display>(items: &[T], separator: &str) -> Text {
    if items.is_empty() {
        return Text::new();
    }

    Text::from(
        items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .join(separator),
    )
}

/// Format a list of items with a custom formatter function
///
/// # Examples
/// ```
/// use verum_common::formatting::format_list_with;
/// use verum_common::Text;
///
/// let items = vec![1, 2, 3];
/// let formatted = format_list_with(&items, ", ", |x| Text::from(format!("#{}", x)));
/// assert_eq!(formatted, "#1, #2, #3");
/// ```
pub fn format_list_with<T, F>(items: &[T], separator: &str, formatter: F) -> Text
where
    F: Fn(&T) -> Text,
{
    if items.is_empty() {
        return Text::new();
    }

    Text::from(
        items
            .iter()
            .map(|x| formatter(x).into_string())
            .collect::<Vec<_>>()
            .join(separator),
    )
}

/// Format a list with Oxford comma (a, b, and c)
///
/// # Examples
/// ```
/// use verum_common::formatting::format_list_pretty;
///
/// assert_eq!(format_list_pretty(&vec!["a"]), "a");
/// assert_eq!(format_list_pretty(&vec!["a", "b"]), "a and b");
/// assert_eq!(format_list_pretty(&vec!["a", "b", "c"]), "a, b, and c");
/// assert_eq!(format_list_pretty(&vec!["a", "b", "c", "d"]), "a, b, c, and d");
/// ```
pub fn format_list_pretty<T: Display>(items: &[T]) -> Text {
    match items.len() {
        0 => Text::new(),
        1 => Text::from(items[0].to_string()),
        2 => Text::from(format!("{} and {}", items[0], items[1])),
        _ => {
            let mut result = Text::new();
            for (i, item) in items.iter().enumerate() {
                if i > 0 && i < items.len() - 1 {
                    result.push_str(", ");
                } else if i == items.len() - 1 {
                    result.push_str(", and ");
                }
                result.push_str(&item.to_string());
            }
            result
        }
    }
}

/// Format a list with brackets
///
/// # Examples
/// ```
/// use verum_common::formatting::format_list_bracketed;
///
/// let items = vec!["a", "b", "c"];
/// assert_eq!(format_list_bracketed(&items, ", "), "[a, b, c]");
///
/// let empty: Vec<&str> = vec![];
/// assert_eq!(format_list_bracketed(&empty, ", "), "[]");
/// ```
pub fn format_list_bracketed<T: Display>(items: &[T], separator: &str) -> Text {
    if items.is_empty() {
        return Text::from("[]");
    }
    Text::from(format!("[{}]", format_list(items, separator)))
}

// ================================================================================================
// Collection Formatting
// ================================================================================================

/// Format a Map for display
///
/// # Examples
/// ```
/// use verum_common::formatting::format_map;
/// use verum_common::Map;
///
/// let mut map = Map::new();
/// map.insert("key1", "value1");
/// map.insert("key2", "value2");
/// // Output format: {key1: value1, key2: value2} (order may vary)
/// ```
pub fn format_map<K, V>(map: &Map<K, V>) -> Text
where
    K: Display + Eq + Hash,
    V: Display,
{
    if map.is_empty() {
        return Text::from("{}");
    }

    let mut pairs: List<Text> = map
        .iter()
        .map(|(k, v)| Text::from(format!("{}: {}", k, v)))
        .collect();
    pairs.sort();

    Text::from(format!("{{{}}}", pairs.join(", ")))
}

/// Format an OrderedMap for display
///
/// # Examples
/// ```
/// use verum_common::formatting::format_ordered_map;
/// use verum_common::OrderedMap;
///
/// let mut map = OrderedMap::new();
/// map.insert(1, "one");
/// map.insert(2, "two");
/// // Output: {1: one, 2: two} (ordered by key)
/// ```
pub fn format_ordered_map<K, V>(map: &OrderedMap<K, V>) -> Text
where
    K: Display + Ord,
    V: Display,
{
    if map.is_empty() {
        return Text::from("{}");
    }

    let pairs: List<Text> = map
        .iter()
        .map(|(k, v)| Text::from(format!("{}: {}", k, v)))
        .collect();

    Text::from(format!("{{{}}}", pairs.join(", ")))
}

/// Format a Set for display
///
/// # Examples
/// ```
/// use verum_common::formatting::format_set;
/// use verum_common::Set;
///
/// let mut set = Set::new();
/// set.insert("a");
/// set.insert("b");
/// // Output format: {a, b} (order may vary)
/// ```
pub fn format_set<T>(set: &Set<T>) -> Text
where
    T: Display + Eq + Hash,
{
    if set.is_empty() {
        return Text::from("{}");
    }

    let mut items: List<Text> = set.iter().map(|item| Text::from(item.to_string())).collect();
    items.sort();

    Text::from(format!("{{{}}}", items.join(", ")))
}

/// Format an OrderedSet for display
///
/// # Examples
/// ```
/// use verum_common::formatting::format_ordered_set;
/// use verum_common::OrderedSet;
///
/// let mut set = OrderedSet::new();
/// set.insert(1);
/// set.insert(2);
/// // Output: {1, 2} (ordered)
/// ```
pub fn format_ordered_set<T>(set: &OrderedSet<T>) -> Text
where
    T: Display + Ord,
{
    if set.is_empty() {
        return Text::from("{}");
    }

    let items: List<Text> = set.iter().map(|item| Text::from(item.to_string())).collect();

    Text::from(format!("{{{}}}", items.join(", ")))
}

// ================================================================================================
// Cycle Formatting
// ================================================================================================

/// Format a dependency cycle with arrows
///
/// # Examples
/// ```
/// use verum_common::formatting::format_cycle;
///
/// let cycle = vec!["mod_a", "mod_b", "mod_c"];
/// assert_eq!(format_cycle(&cycle), "mod_a -> mod_b -> mod_c -> mod_a");
///
/// let empty: Vec<&str> = vec![];
/// assert_eq!(format_cycle(&empty), "[]");
/// ```
pub fn format_cycle<T: Display>(cycle: &[T]) -> Text {
    if cycle.is_empty() {
        return Text::from("[]");
    }

    let mut result = format_list(cycle, " -> ");
    if !result.is_empty() {
        // Close the cycle by repeating the first element
        result.push_str(" -> ");
        result.push_str(&cycle[0].to_string());
    }
    result
}

/// Format a dependency chain (no cycle closure)
///
/// # Examples
/// ```
/// use verum_common::formatting::format_dependency_chain;
///
/// let chain = vec!["step1", "step2", "step3"];
/// assert_eq!(format_dependency_chain(&chain), "step1 -> step2 -> step3");
/// ```
pub fn format_dependency_chain<T: Display>(chain: &[T]) -> Text {
    format_list(chain, " -> ")
}

// ================================================================================================
// Code Formatting
// ================================================================================================

/// Format a code block with indentation
///
/// # Examples
/// ```
/// use verum_common::formatting::format_code_block;
///
/// let code = "fn main() {\n    println!(\"Hello\");\n}";
/// let formatted = format_code_block(code, 4);
/// // Each line will be indented by 4 spaces
/// ```
pub fn format_code_block(code: &str, indent_spaces: usize) -> Text {
    let indent_str = " ".repeat(indent_spaces);
    Text::from(
        code.lines()
            .map(|line| format!("{}{}", indent_str, line))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Format inline code with backticks
///
/// # Examples
/// ```
/// use verum_common::formatting::format_inline_code;
///
/// assert_eq!(format_inline_code("x + 1"), "`x + 1`");
/// ```
pub fn format_inline_code(code: &str) -> Text {
    Text::from(format!("`{}`", code))
}

/// Format a function signature
///
/// # Examples
/// ```
/// use verum_common::formatting::format_function_signature;
///
/// let params = vec![verum_common::Text::from("x: Int"), verum_common::Text::from("y: Int")];
/// assert_eq!(
///     format_function_signature("add", &params, Some("Int")),
///     "fn add(x: Int, y: Int) -> Int"
/// );
///
/// assert_eq!(
///     format_function_signature("print", &[], None),
///     "fn print()"
/// );
/// ```
pub fn format_function_signature(name: &str, params: &[Text], return_type: Maybe<&str>) -> Text {
    let params_str = params
        .iter()
        .map(|p| p.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    match return_type {
        Some(ret) => Text::from(format!("fn {}({}) -> {}", name, params_str, ret)),
        None => Text::from(format!("fn {}({})", name, params_str)),
    }
}

/// Format a type signature with generics
///
/// # Examples
/// ```
/// use verum_common::formatting::format_type_signature;
///
/// assert_eq!(
///     format_type_signature("List", &["T"]),
///     "List<T>"
/// );
///
/// assert_eq!(
///     format_type_signature("Map", &["K", "V"]),
///     "Map<K, V>"
/// );
/// ```
pub fn format_type_signature(name: &str, generics: &[&str]) -> Text {
    if generics.is_empty() {
        Text::from(name)
    } else {
        Text::from(format!("{}<{}>", name, generics.join(", ")))
    }
}

// ================================================================================================
// Text Utilities
// ================================================================================================

/// Truncate text with ellipsis if it exceeds max length
///
/// # Examples
/// ```
/// use verum_common::formatting::truncate_with_ellipsis;
///
/// assert_eq!(truncate_with_ellipsis("hello world", 20), "hello world");
/// assert_eq!(truncate_with_ellipsis("hello world", 8), "hello...");
/// assert_eq!(truncate_with_ellipsis("hello", 8), "hello");
/// ```
pub fn truncate_with_ellipsis(text: &str, max_len: usize) -> Text {
    if text.len() <= max_len {
        Text::from(text)
    } else {
        let truncate_at = max_len.saturating_sub(3);
        Text::from(format!("{}...", &text[..truncate_at]))
    }
}

/// Indent each line of text by a number of spaces
///
/// # Examples
/// ```
/// use verum_common::formatting::indent;
///
/// let text = "line1\nline2\nline3";
/// let indented = indent(text, 4);
/// assert_eq!(indented, "    line1\n    line2\n    line3");
/// ```
pub fn indent(text: &str, spaces: usize) -> Text {
    let indent_str = " ".repeat(spaces);
    Text::from(
        text.lines()
            .map(|line| format!("{}{}", indent_str, line))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Wrap text to a specified width
///
/// # Examples
/// ```
/// use verum_common::formatting::wrap_text;
///
/// let text = "The quick brown fox jumps over the lazy dog";
/// let wrapped = wrap_text(text, 20);
/// // Lines will be wrapped at ~20 characters
/// ```
pub fn wrap_text(text: &str, width: usize) -> Text {
    let mut result = Text::new();
    let mut current_line = Text::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line.push_str(word);
        } else if current_line.len() + word.len() < width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&current_line);
            current_line.clear();
            current_line.push_str(word);
        }
    }

    if !current_line.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&current_line);
    }

    result
}

/// Join text lines with a separator
///
/// # Examples
/// ```
/// use verum_common::formatting::join_lines;
///
/// let lines = vec!["line1", "line2", "line3"];
/// assert_eq!(join_lines(&lines, "\n"), "line1\nline2\nline3");
/// assert_eq!(join_lines(&lines, "; "), "line1; line2; line3");
/// ```
pub fn join_lines<T: AsRef<str>>(lines: &[T], separator: &str) -> Text {
    Text::from(
        lines
            .iter()
            .map(|l| l.as_ref())
            .collect::<Vec<_>>()
            .join(separator),
    )
}

// ================================================================================================
// Formatting Builders
// ================================================================================================

/// Alignment for table columns
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
    Center,
}

/// Builder for formatting lists with custom options
#[derive(Debug, Clone)]
pub struct ListFormatter {
    separator: Text,
    prefix: Text,
    suffix: Text,
    oxford_comma: bool,
    bracket_style: Maybe<BracketStyle>,
}

/// Bracket styles for list formatting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BracketStyle {
    Square, // [...]
    Curly,  // {...}
    Paren,  // (...)
    Angle,  // <...>
}

impl BracketStyle {
    fn opening(&self) -> &'static str {
        match self {
            BracketStyle::Square => "[",
            BracketStyle::Curly => "{",
            BracketStyle::Paren => "(",
            BracketStyle::Angle => "<",
        }
    }

    fn closing(&self) -> &'static str {
        match self {
            BracketStyle::Square => "]",
            BracketStyle::Curly => "}",
            BracketStyle::Paren => ")",
            BracketStyle::Angle => ">",
        }
    }
}

impl Default for ListFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl ListFormatter {
    /// Create a new list formatter with default settings
    pub fn new() -> Self {
        Self {
            separator: Text::from(", "),
            prefix: Text::new(),
            suffix: Text::new(),
            oxford_comma: false,
            bracket_style: None,
        }
    }

    /// Set the separator between items
    pub fn separator(mut self, sep: impl Into<Text>) -> Self {
        self.separator = sep.into();
        self
    }

    /// Set a prefix for the entire list
    pub fn prefix(mut self, prefix: impl Into<Text>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Set a suffix for the entire list
    pub fn suffix(mut self, suffix: impl Into<Text>) -> Self {
        self.suffix = suffix.into();
        self
    }

    /// Enable Oxford comma for 3+ items
    pub fn oxford_comma(mut self, enable: bool) -> Self {
        self.oxford_comma = enable;
        self
    }

    /// Set bracket style
    pub fn brackets(mut self, style: BracketStyle) -> Self {
        self.bracket_style = Some(style);
        self
    }

    /// Format a list of items
    pub fn format<T: Display>(&self, items: &[T]) -> Text {
        let content = if self.oxford_comma && items.len() >= 3 {
            format_list_pretty(items)
        } else {
            format_list(items, &self.separator)
        };

        let mut result = self.prefix.clone();

        if let Some(style) = self.bracket_style {
            result.push_str(style.opening());
        }

        result.push_str(&content);

        if let Some(style) = self.bracket_style {
            result.push_str(style.closing());
        }

        result.push_str(&self.suffix);

        result
    }
}

/// Builder for formatting tables
#[derive(Debug, Clone)]
pub struct TableFormatter {
    headers: List<Text>,
    rows: List<List<Text>>,
    alignments: List<Alignment>,
    border: bool,
}

impl Default for TableFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl TableFormatter {
    /// Create a new table formatter
    pub fn new() -> Self {
        Self {
            headers: List::new(),
            rows: List::new(),
            alignments: List::new(),
            border: true,
        }
    }

    /// Set table headers
    pub fn headers(mut self, headers: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        self.headers = headers.into_iter().map(|h| h.into()).collect();
        self.alignments = List::from(vec![Alignment::Left; self.headers.len()]);
        self
    }

    /// Add a row to the table
    pub fn row(mut self, cells: impl IntoIterator<Item = impl Into<Text>>) -> Self {
        self.rows
            .push(cells.into_iter().map(|c| c.into()).collect());
        self
    }

    /// Set column alignments
    pub fn alignments(mut self, alignments: impl IntoIterator<Item = Alignment>) -> Self {
        self.alignments = alignments.into_iter().collect();
        self
    }

    /// Enable or disable borders
    pub fn border(mut self, enabled: bool) -> Self {
        self.border = enabled;
        self
    }

    /// Format the table
    pub fn format(&self) -> Text {
        if self.headers.is_empty() {
            return Text::new();
        }

        // Calculate column widths
        let mut widths: List<usize> = self.headers.iter().map(|h| h.len()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.len());
                }
            }
        }

        let mut result = Text::new();

        // Format header
        if self.border {
            result.push_str(&self.format_row(&self.headers, &widths));
            result.push('\n');
            result.push_str(&self.format_separator(&widths));
        } else {
            result.push_str(&self.format_row(&self.headers, &widths));
        }

        // Format rows
        for row in &self.rows {
            result.push('\n');
            result.push_str(&self.format_row(row, &widths));
        }

        result
    }

    fn format_row(&self, cells: &[Text], widths: &[usize]) -> Text {
        let formatted: List<Text> = cells
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let width = widths.get(i).copied().unwrap_or(0);
                let align = self.alignments.get(i).copied().unwrap_or(Alignment::Left);
                self.align_cell(cell, width, align)
            })
            .collect();

        if self.border {
            Text::from(format!("| {} |", formatted.join(" | ")))
        } else {
            formatted.join("  ")
        }
    }

    fn format_separator(&self, widths: &[usize]) -> Text {
        let separators: List<Text> = widths
            .iter()
            .map(|w| Text::from("-".repeat(*w)))
            .collect();
        Text::from(format!("|{}-|", separators.join("-|-")))
    }

    fn align_cell(&self, cell: &str, width: usize, align: Alignment) -> Text {
        let padding = width.saturating_sub(cell.len());
        match align {
            Alignment::Left => Text::from(format!("{}{}", cell, " ".repeat(padding))),
            Alignment::Right => Text::from(format!("{}{}", " ".repeat(padding), cell)),
            Alignment::Center => {
                let left_pad = padding / 2;
                let right_pad = padding - left_pad;
                Text::from(format!(
                    "{}{}{}",
                    " ".repeat(left_pad),
                    cell,
                    " ".repeat(right_pad)
                ))
            }
        }
    }
}

// ================================================================================================
// Legacy Compatibility (for error.rs)
// ================================================================================================

/// Format a list of items (AsRef<str> version for backward compatibility)
///
/// This function maintains compatibility with existing code in error.rs
pub fn format_list_str<T: AsRef<str>>(items: &[T]) -> Text {
    if items.is_empty() {
        return Text::new();
    }

    Text::from(
        items
            .iter()
            .map(|item| item.as_ref())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Format a cycle (AsRef<str> version for backward compatibility)
///
/// This function maintains compatibility with existing code in error.rs
pub fn format_cycle_str<T: AsRef<str>>(items: &[T]) -> Text {
    if items.is_empty() {
        return Text::from("[]");
    }

    let mut result = items
        .iter()
        .map(|item| item.as_ref())
        .collect::<Vec<_>>()
        .join(" -> ");

    if !result.is_empty() {
        result.push_str(" -> ");
        result.push_str(items[0].as_ref());
    }

    Text::from(result)
}
