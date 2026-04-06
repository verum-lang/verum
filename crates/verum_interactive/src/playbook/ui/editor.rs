//! Full-featured editor widget for editing cell content
//!
//! Features:
//! - Syntax highlighting for Verum code
//! - Text selection with mouse/keyboard
//! - Copy/paste support
//! - Full-screen toggle
//! - LSP integration for error highlighting

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Widget};

/// Selection range in the editor
#[derive(Debug, Clone, Copy, Default)]
pub struct Selection {
    /// Start position (line, column)
    pub start: (usize, usize),
    /// End position (line, column)
    pub end: (usize, usize),
}

impl Selection {
    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Normalize selection so start <= end
    pub fn normalize(&self) -> Selection {
        let (start, end) = if self.start.0 < self.end.0
            || (self.start.0 == self.end.0 && self.start.1 <= self.end.1) {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        };
        Selection { start, end }
    }
}

/// Editor state for managing text editing.
///
/// **Cursor convention**: `cursor.1` is a **char index** (number of Unicode
/// scalar values from the start of the line), NOT a byte offset. All String
/// operations convert to byte offsets via [`char_to_byte`] before slicing.
#[derive(Debug, Clone)]
pub struct EditorState {
    /// Text content as lines
    pub lines: Vec<String>,
    /// Cursor position (line, char_column) — char-based, not bytes
    pub cursor: (usize, usize),
    /// Selection (if any)
    pub selection: Option<Selection>,
    /// Scroll offset (line)
    pub scroll_offset: usize,
    /// Whether the editor is in full-screen mode
    pub fullscreen: bool,
    /// Clipboard content
    clipboard: String,
    /// Undo history
    undo_stack: Vec<EditorSnapshot>,
    /// Redo history
    redo_stack: Vec<EditorSnapshot>,
}

#[derive(Debug, Clone)]
struct EditorSnapshot {
    lines: Vec<String>,
    cursor: (usize, usize),
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorState {
    /// Create a new editor state
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            selection: None,
            scroll_offset: 0,
            fullscreen: false,
            clipboard: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Set content from a string
    pub fn set_content(&mut self, content: &str) {
        self.lines = content.lines().map(String::from).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor = (0, 0);
        self.selection = None;
        self.scroll_offset = 0;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Get content as a string
    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Get current line
    pub fn current_line(&self) -> &str {
        self.lines.get(self.cursor.0).map(|s| s.as_str()).unwrap_or("")
    }

    /// Convert char index to byte offset in a string.
    fn char_to_byte(s: &str, char_col: usize) -> usize {
        s.char_indices()
            .nth(char_col)
            .map(|(i, _)| i)
            .unwrap_or(s.len())
    }

    /// Number of chars in a string (display width for monospace).
    fn char_len(s: &str) -> usize {
        s.chars().count()
    }

    /// Get current line mutably
    fn current_line_mut(&mut self) -> &mut String {
        while self.cursor.0 >= self.lines.len() {
            self.lines.push(String::new());
        }
        &mut self.lines[self.cursor.0]
    }

    /// Save state for undo
    fn save_undo(&mut self) {
        self.undo_stack.push(EditorSnapshot {
            lines: self.lines.clone(),
            cursor: self.cursor,
        });
        self.redo_stack.clear();
        // Limit undo history
        if self.undo_stack.len() > 100 {
            self.undo_stack.remove(0);
        }
    }

    /// Undo last change
    pub fn undo(&mut self) -> bool {
        if let Some(snapshot) = self.undo_stack.pop() {
            self.redo_stack.push(EditorSnapshot {
                lines: self.lines.clone(),
                cursor: self.cursor,
            });
            self.lines = snapshot.lines;
            self.cursor = snapshot.cursor;
            self.selection = None;
            true
        } else {
            false
        }
    }

    /// Redo last undone change
    pub fn redo(&mut self) -> bool {
        if let Some(snapshot) = self.redo_stack.pop() {
            self.undo_stack.push(EditorSnapshot {
                lines: self.lines.clone(),
                cursor: self.cursor,
            });
            self.lines = snapshot.lines;
            self.cursor = snapshot.cursor;
            self.selection = None;
            true
        } else {
            false
        }
    }

    /// Insert a character at cursor (char-based cursor)
    pub fn insert_char(&mut self, c: char) {
        self.save_undo();
        self.delete_selection();

        let (row, char_col) = self.cursor;

        // Ensure line exists
        while row >= self.lines.len() {
            self.lines.push(String::new());
        }

        let byte_pos = Self::char_to_byte(&self.lines[row], char_col);

        if c == '\n' {
            let rest = self.lines[row][byte_pos..].to_string();
            self.lines[row].truncate(byte_pos);
            self.cursor.0 += 1;
            self.cursor.1 = 0;
            self.lines.insert(self.cursor.0, rest);
        } else {
            self.lines[row].insert(byte_pos, c);
            self.cursor.1 += 1;
        }
    }

    /// Insert a string at cursor (char-based)
    pub fn insert_str(&mut self, s: &str) {
        self.save_undo();
        self.delete_selection();

        for c in s.chars() {
            let (row, char_col) = self.cursor;

            while row >= self.lines.len() {
                self.lines.push(String::new());
            }

            if c == '\n' {
                let byte_pos = Self::char_to_byte(&self.lines[row], char_col);
                let rest = self.lines[row][byte_pos..].to_string();
                self.lines[row].truncate(byte_pos);
                self.cursor.0 += 1;
                self.cursor.1 = 0;
                self.lines.insert(self.cursor.0, rest);
            } else {
                let byte_pos = Self::char_to_byte(&self.lines[row], char_col);
                self.lines[row].insert(byte_pos, c);
                self.cursor.1 += 1;
            }
        }
    }

    /// Delete character before cursor (backspace)
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }

        self.save_undo();

        let (row, char_col) = self.cursor;

        if char_col > 0 {
            let byte_start = Self::char_to_byte(&self.lines[row], char_col - 1);
            let byte_end = Self::char_to_byte(&self.lines[row], char_col);
            self.lines[row].drain(byte_start..byte_end);
            self.cursor.1 -= 1;
        } else if row > 0 {
            let current = self.lines.remove(row);
            self.cursor.0 -= 1;
            self.cursor.1 = Self::char_len(&self.lines[self.cursor.0]);
            self.lines[self.cursor.0].push_str(&current);
        }
    }

    /// Delete character at cursor
    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }

        self.save_undo();

        let (row, char_col) = self.cursor;
        let line_chars = self.lines.get(row).map(|l| Self::char_len(l)).unwrap_or(0);

        if char_col < line_chars {
            let byte_start = Self::char_to_byte(&self.lines[row], char_col);
            let byte_end = Self::char_to_byte(&self.lines[row], char_col + 1);
            self.lines[row].drain(byte_start..byte_end);
        } else if row < self.lines.len() - 1 {
            let next = self.lines.remove(row + 1);
            self.lines[row].push_str(&next);
        }
    }

    /// Delete selected text if any (char-based selection)
    fn delete_selection(&mut self) -> bool {
        if let Some(sel) = self.selection.take() {
            if sel.is_empty() {
                return false;
            }

            self.save_undo();
            let sel = sel.normalize();

            if sel.start.0 == sel.end.0 {
                // Same line
                let byte_start = Self::char_to_byte(&self.lines[sel.start.0], sel.start.1);
                let byte_end = Self::char_to_byte(&self.lines[sel.start.0], sel.end.1);
                self.lines[sel.start.0].drain(byte_start..byte_end);
            } else {
                // Multiple lines
                let byte_start = Self::char_to_byte(&self.lines[sel.start.0], sel.start.1);
                let byte_end = Self::char_to_byte(&self.lines[sel.end.0], sel.end.1);

                let new_line = format!(
                    "{}{}",
                    &self.lines[sel.start.0][..byte_start],
                    &self.lines[sel.end.0][byte_end..]
                );

                for _ in sel.start.0..=sel.end.0 {
                    if sel.start.0 < self.lines.len() {
                        self.lines.remove(sel.start.0);
                    }
                }

                self.lines.insert(sel.start.0, new_line);
            }

            self.cursor = sel.start;
            true
        } else {
            false
        }
    }

    /// Move cursor left (one char)
    pub fn move_left(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);

        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
        } else if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = Self::char_len(&self.lines[self.cursor.0]);
        }

        self.end_selection(with_selection);
    }

    /// Move cursor right (one char)
    pub fn move_right(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);

        let line_chars = Self::char_len(self.current_line());
        if self.cursor.1 < line_chars {
            self.cursor.1 += 1;
        } else if self.cursor.0 < self.lines.len() - 1 {
            self.cursor.0 += 1;
            self.cursor.1 = 0;
        }

        self.end_selection(with_selection);
    }

    /// Move cursor up (clamp to line length in chars)
    pub fn move_up(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);

        if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.cursor.1.min(Self::char_len(&self.lines[self.cursor.0]));
        }

        self.end_selection(with_selection);
    }

    /// Move cursor down (clamp to line length in chars)
    pub fn move_down(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);

        if self.cursor.0 < self.lines.len() - 1 {
            self.cursor.0 += 1;
            self.cursor.1 = self.cursor.1.min(Self::char_len(&self.lines[self.cursor.0]));
        }

        self.end_selection(with_selection);
    }

    /// Move cursor to start of line
    pub fn move_home(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);
        self.cursor.1 = 0;
        self.end_selection(with_selection);
    }

    /// Move cursor to end of line (char count)
    pub fn move_end(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);
        self.cursor.1 = Self::char_len(self.current_line());
        self.end_selection(with_selection);
    }

    /// Move cursor to start of text
    pub fn move_to_start(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);
        self.cursor = (0, 0);
        self.end_selection(with_selection);
    }

    /// Move cursor to end of text
    pub fn move_to_end(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);
        self.cursor.0 = self.lines.len().saturating_sub(1);
        self.cursor.1 = Self::char_len(self.current_line());
        self.end_selection(with_selection);
    }

    /// Begin a selection before cursor movement.
    /// If with_selection and no active selection, starts one at the current cursor.
    /// If not with_selection, clears the selection.
    fn begin_selection(&mut self, with_selection: bool) {
        if with_selection {
            if self.selection.is_none() {
                self.selection = Some(Selection {
                    start: self.cursor,
                    end: self.cursor,
                });
            }
        } else {
            self.selection = None;
        }
    }

    /// End a selection after cursor movement.
    /// Updates the selection end to the current cursor position.
    fn end_selection(&mut self, with_selection: bool) {
        if with_selection
            && let Some(ref mut sel) = self.selection {
                sel.end = self.cursor;
            }
    }

    /// Start selection at current position
    pub fn start_selection(&mut self) {
        self.selection = Some(Selection {
            start: self.cursor,
            end: self.cursor,
        });
    }

    /// Select all text
    pub fn select_all(&mut self) {
        self.selection = Some(Selection {
            start: (0, 0),
            end: (
                self.lines.len().saturating_sub(1),
                self.lines.last().map(|l| Self::char_len(l)).unwrap_or(0),
            ),
        });
    }

    /// Get selected text (char-based selection)
    pub fn selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?.normalize();
        if sel.is_empty() {
            return None;
        }

        let mut result = String::new();

        if sel.start.0 == sel.end.0 {
            let line = &self.lines[sel.start.0];
            let bs = Self::char_to_byte(line, sel.start.1);
            let be = Self::char_to_byte(line, sel.end.1);
            result.push_str(&line[bs..be]);
        } else {
            // First line
            if let Some(line) = self.lines.get(sel.start.0) {
                let bs = Self::char_to_byte(line, sel.start.1);
                result.push_str(&line[bs..]);
            }

            // Middle lines
            for i in (sel.start.0 + 1)..sel.end.0 {
                result.push('\n');
                if let Some(line) = self.lines.get(i) {
                    result.push_str(line);
                }
            }

            // Last line
            if sel.end.0 > sel.start.0 {
                result.push('\n');
                if let Some(line) = self.lines.get(sel.end.0) {
                    let be = Self::char_to_byte(line, sel.end.1);
                    result.push_str(&line[..be]);
                }
            }
        }

        Some(result)
    }

    /// Copy selected text to clipboard
    pub fn copy(&mut self) {
        if let Some(text) = self.selected_text() {
            self.clipboard = text.clone();
            system_clipboard_write(&text);
        }
    }

    /// Cut selected text to clipboard
    pub fn cut(&mut self) {
        self.copy();
        self.delete_selection();
    }

    /// Paste from system clipboard (falls back to internal)
    pub fn paste(&mut self) {
        let text = system_clipboard_read()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.clipboard.clone());
        if !text.is_empty() {
            self.insert_str(&text);
        }
    }

    /// Toggle fullscreen mode
    pub fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
    }

    /// Ensure cursor is visible
    pub fn ensure_cursor_visible(&mut self, visible_lines: usize) {
        if self.cursor.0 < self.scroll_offset {
            self.scroll_offset = self.cursor.0;
        } else if self.cursor.0 >= self.scroll_offset + visible_lines {
            self.scroll_offset = self.cursor.0 - visible_lines + 1;
        }
    }

    /// Move cursor one word left (char-based).
    pub fn move_word_left(&mut self, select: bool) {
        self.begin_selection(select);
        let (row, char_col) = self.cursor;
        if char_col == 0 {
            if row > 0 {
                self.cursor.0 -= 1;
                self.cursor.1 = Self::char_len(&self.lines[self.cursor.0]);
            }
            self.end_selection(select);
            return;
        }
        let line = self.current_line().to_string();
        let chars: Vec<char> = line.chars().collect();
        let mut i = char_col;
        while i > 0 && chars[i - 1].is_whitespace() { i -= 1; }
        while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') { i -= 1; }
        self.cursor.1 = i;
        self.end_selection(select);
    }

    /// Move cursor one word right (char-based).
    pub fn move_word_right(&mut self, select: bool) {
        self.begin_selection(select);
        let (row, char_col) = self.cursor;
        let line = self.current_line().to_string();
        let chars: Vec<char> = line.chars().collect();
        let len = chars.len();
        if char_col >= len {
            if row < self.lines.len() - 1 { self.cursor.0 += 1; self.cursor.1 = 0; }
            self.end_selection(select);
            return;
        }
        let mut i = char_col;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
        while i < len && chars[i].is_whitespace() { i += 1; }
        self.cursor.1 = i;
        self.end_selection(select);
    }

    /// Insert a tab (4 spaces with alignment).
    pub fn insert_tab(&mut self) {
        let col = self.cursor.1;
        let spaces = 4 - (col % 4);
        for _ in 0..spaces { self.insert_char(' '); }
    }

    /// Scroll up
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// Scroll down
    pub fn scroll_down(&mut self, lines: usize) {
        self.scroll_offset = (self.scroll_offset + lines).min(self.lines.len().saturating_sub(1));
    }
}

/// Diagnostic from LSP for error highlighting
#[derive(Debug, Clone)]
pub struct EditorDiagnostic {
    /// Line number (0-indexed)
    pub line: usize,
    /// Column start (0-indexed)
    pub col_start: usize,
    /// Column end
    pub col_end: usize,
    /// Message
    pub message: String,
    /// Severity (error, warning, info, hint)
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

impl DiagnosticSeverity {
    pub fn style(&self) -> Style {
        match self {
            DiagnosticSeverity::Error => Style::default().fg(Color::Red).add_modifier(Modifier::UNDERLINED),
            DiagnosticSeverity::Warning => Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED),
            DiagnosticSeverity::Info => Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED),
            DiagnosticSeverity::Hint => Style::default().fg(Color::Gray).add_modifier(Modifier::UNDERLINED),
        }
    }
}

/// Widget for editing cell content
pub struct EditorWidget<'a> {
    state: &'a EditorState,
    show_line_numbers: bool,
    diagnostics: &'a [EditorDiagnostic],
    title: String,
}

impl<'a> EditorWidget<'a> {
    pub fn new(state: &'a EditorState) -> Self {
        Self {
            state,
            show_line_numbers: true,
            diagnostics: &[],
            title: "Editor".to_string(),
        }
    }

    pub fn line_numbers(mut self, show: bool) -> Self {
        self.show_line_numbers = show;
        self
    }

    pub fn diagnostics(mut self, diags: &'a [EditorDiagnostic]) -> Self {
        self.diagnostics = diags;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Apply syntax highlighting to Verum code
    fn highlight_verum(source: &str) -> Vec<Span<'_>> {
        let mut spans = Vec::new();
        // Keywords per grammar/verum.ebnf (all categories)
        let keywords = [
            "fn", "let", "is", "type", "where", "using",
            "if", "else", "match", "return", "for", "while", "loop", "break", "continue",
            "async", "await", "spawn", "defer", "errdefer", "try", "yield", "throws", "select", "nursery",
            "pub", "mut", "const", "unsafe", "pure", "ffi",
            "module", "mount", "implement", "context", "protocol", "extends",
            "self", "super", "cog", "static", "meta", "provide", "finally", "recover",
            "invariant", "decreases", "stream", "tensor", "affine", "linear",
            "public", "internal", "protected", "ensures", "requires", "result", "some",
            "theorem", "lemma", "axiom", "corollary", "proof", "calc",
            "have", "show", "suffices", "obtain", "by", "qed",
            "induction", "cases", "contradiction", "forall", "exists",
        ];
        let types = [
            "Int", "Float", "Bool", "Char", "Text",
            "List", "Map", "Set", "Maybe", "Heap", "Shared",
            "Deque", "Channel", "Mutex", "Task", "Result", "Tensor", "Future", "Duration",
        ];

        let mut chars = source.char_indices().peekable();
        let mut current_start = 0;

        while let Some((i, c)) = chars.next() {
            // Check for identifiers/keywords
            if c.is_alphabetic() || c == '_' {
                let start = i;
                let mut end = i + c.len_utf8();

                while let Some(&(next_i, next_c)) = chars.peek() {
                    if next_c.is_alphanumeric() || next_c == '_' {
                        end = next_i + next_c.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }

                // Add any text before this identifier
                if start > current_start {
                    spans.push(Span::raw(&source[current_start..start]));
                }

                let word = &source[start..end];
                let style = if keywords.contains(&word) {
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
                } else if types.contains(&word) {
                    Style::default().fg(Color::Cyan)
                } else if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::White)
                };

                spans.push(Span::styled(word, style));
                current_start = end;
            }
            // Check for strings
            else if c == '"' {
                let start = i;
                let mut end = i + 1;
                let mut escaped = false;

                while let Some(&(next_i, next_c)) = chars.peek() {
                    end = next_i + next_c.len_utf8();
                    chars.next();

                    if escaped {
                        escaped = false;
                    } else if next_c == '\\' {
                        escaped = true;
                    } else if next_c == '"' {
                        break;
                    }
                }

                if start > current_start {
                    spans.push(Span::raw(&source[current_start..start]));
                }

                spans.push(Span::styled(&source[start..end], Style::default().fg(Color::Yellow)));
                current_start = end;
            }
            // Check for comments
            else if c == '/' {
                if let Some(&(_, '/')) = chars.peek() {
                    let start = i;
                    // Consume until end of line
                    while let Some(&(_, next_c)) = chars.peek() {
                        if next_c == '\n' {
                            break;
                        }
                        chars.next();
                    }
                    let end = chars.peek().map(|(i, _)| *i).unwrap_or(source.len());

                    if start > current_start {
                        spans.push(Span::raw(&source[current_start..start]));
                    }

                    spans.push(Span::styled(&source[start..end], Style::default().fg(Color::DarkGray)));
                    current_start = end;
                }
            }
            // Check for numbers
            else if c.is_ascii_digit() {
                let start = i;
                let mut end = i + 1;

                while let Some(&(next_i, next_c)) = chars.peek() {
                    if next_c.is_ascii_digit() || next_c == '.' || next_c == '_' {
                        end = next_i + next_c.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }

                if start > current_start {
                    spans.push(Span::raw(&source[current_start..start]));
                }

                spans.push(Span::styled(&source[start..end], Style::default().fg(Color::LightBlue)));
                current_start = end;
            }
        }

        // Add remaining text
        if current_start < source.len() {
            spans.push(Span::raw(&source[current_start..]));
        }

        if spans.is_empty() {
            spans.push(Span::raw(source));
        }

        spans
    }
}

impl<'a> Widget for EditorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.state.fullscreen {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };

        let title = if self.state.fullscreen {
            format!("{} [FULLSCREEN - Ctrl+F to exit]", self.title)
        } else {
            self.title.to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title);

        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 || inner.width == 0 {
            return;
        }

        // Calculate line number width
        let line_num_width = if self.show_line_numbers {
            let max_line = self.state.lines.len();
            (max_line.to_string().len() + 1) as u16
        } else {
            0
        };

        let text_width = inner.width.saturating_sub(line_num_width + 1);
        let visible_lines = inner.height as usize;

        // Render visible lines
        for (i, line_idx) in (self.state.scroll_offset..(self.state.scroll_offset + visible_lines)).enumerate() {
            let y = inner.y + i as u16;

            if line_idx >= self.state.lines.len() {
                // Render ~ for lines past end of file
                if self.show_line_numbers {
                    buf.set_string(
                        inner.x,
                        y,
                        "~",
                        Style::default().fg(Color::DarkGray),
                    );
                }
                continue;
            }

            // Render line number
            if self.show_line_numbers {
                let num_str = format!("{:>width$} ", line_idx + 1, width = line_num_width as usize - 1);
                let num_style = if line_idx == self.state.cursor.0 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                buf.set_string(inner.x, y, &num_str, num_style);
            }

            // Get line content
            let line = &self.state.lines[line_idx];
            let text_x = inner.x + line_num_width;

            // Apply syntax highlighting
            let spans = Self::highlight_verum(line);
            let mut col = 0u16;

            for span in spans {
                let content = span.content.as_ref();
                for c in content.chars() {
                    if col >= text_width {
                        break;
                    }

                    // Check if this position is selected (col is now char index)
                    let char_idx = col as usize;
                    let is_selected = self.state.selection.as_ref().is_some_and(|sel| {
                        let sel = sel.normalize();
                        let pos = (line_idx, char_idx);
                        (sel.start.0 < pos.0 || (sel.start.0 == pos.0 && sel.start.1 <= pos.1))
                            && (pos.0 < sel.end.0 || (pos.0 == sel.end.0 && pos.1 < sel.end.1))
                    });

                    let mut style = span.style;
                    if is_selected {
                        style = style.bg(Color::Blue);
                    }

                    // Check for diagnostics (col_start/col_end are byte-based in diagnostics,
                    // but we compare with char_idx for display purposes)
                    for diag in self.diagnostics {
                        if diag.line == line_idx && char_idx >= diag.col_start && char_idx < diag.col_end {
                            style = style.patch(diag.severity.style());
                        }
                    }

                    buf.set_string(text_x + col, y, c.to_string(), style);
                    col += 1;
                }
            }

            // Render cursor
            if line_idx == self.state.cursor.0 {
                let cursor_col = self.state.cursor.1 as u16;
                if cursor_col <= text_width {
                    let cursor_x = text_x + cursor_col;
                    // Get the cell at cursor position and invert its style
                    if let Some(cell) = buf.cell_mut((cursor_x, y)) {
                        cell.set_style(Style::default().bg(Color::White).fg(Color::Black));
                    }
                }
            }
        }

        // Render scrollbar if needed
        if self.state.lines.len() > visible_lines {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            let mut scrollbar_state = ScrollbarState::new(self.state.lines.len())
                .position(self.state.scroll_offset);

            scrollbar.render(
                Rect {
                    x: area.x + area.width - 1,
                    y: area.y + 1,
                    width: 1,
                    height: area.height - 2,
                },
                buf,
                &mut scrollbar_state,
            );
        }
    }
}

// ==================== System Clipboard ====================

/// Write text to the OS clipboard (macOS pbcopy, Linux xclip/xsel, Wayland wl-copy).
fn system_clipboard_write(text: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let cmd = if cfg!(target_os = "macos") {
        Some(("pbcopy", &[] as &[&str]))
    } else if cfg!(target_os = "linux") {
        // Try wl-copy first (Wayland), fall back to xclip (X11)
        if Command::new("wl-copy").arg("--version").output().is_ok() {
            Some(("wl-copy", &[] as &[&str]))
        } else {
            Some(("xclip", &["-selection", "clipboard"] as &[&str]))
        }
    } else {
        None
    };

    if let Some((program, args)) = cmd
        && let Ok(mut child) = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
}

/// Read text from the OS clipboard.
fn system_clipboard_read() -> Option<String> {
    use std::process::Command;

    let output = if cfg!(target_os = "macos") {
        Command::new("pbpaste").output().ok()
    } else if cfg!(target_os = "linux") {
        Command::new("wl-paste")
            .arg("--no-newline")
            .output()
            .ok()
            .or_else(|| {
                Command::new("xclip")
                    .args(["-selection", "clipboard", "-o"])
                    .output()
                    .ok()
            })
    } else {
        None
    };

    output
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}
