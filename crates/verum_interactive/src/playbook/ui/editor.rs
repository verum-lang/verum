//! Full-featured editor widget for editing cell content
//!
//! Features:
//! - Syntax highlighting for Verum code
//! - Text selection with mouse/keyboard
//! - Copy/paste support
//! - Full-screen toggle
//! - LSP integration for error highlighting

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Widget,
};

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
            || (self.start.0 == self.end.0 && self.start.1 <= self.end.1)
        {
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
    /// Horizontal scroll offset (chars) — long lines pan instead of
    /// hiding the cursor past the right edge.
    pub h_scroll: usize,
    /// Whether the editor is in full-screen mode
    pub fullscreen: bool,
    /// Clipboard content
    clipboard: String,
    /// Undo history
    undo_stack: Vec<EditorSnapshot>,
    /// Redo history
    redo_stack: Vec<EditorSnapshot>,
    /// What the previous edit was — consecutive same-kind edits
    /// coalesce into ONE undo entry (typing a word is one undo step,
    /// not one per character).
    last_edit: EditKind,
}

/// A LINE-SPAN undo delta — not a full-buffer clone.  Editing a 1 MB
/// buffer must not copy 1 MB per keystroke: every edit operation
/// declares the line span it is about to touch, and the snapshot
/// stores only those lines plus enough bookkeeping to reconstruct the
/// replacement span after the edit (`len_before` — the total line
/// count when the snapshot was taken — pins how many lines the edit
/// left in the span's place).
#[derive(Debug, Clone)]
struct EditorSnapshot {
    /// First line of the touched span.
    first: usize,
    /// The span's lines BEFORE the edit.
    old_lines: Vec<String>,
    /// Total buffer line count when the snapshot was taken.
    len_before: usize,
    cursor: (usize, usize),
}

/// Edit classification for undo coalescing.  A boundary (movement,
/// newline, whitespace after a word, any structural edit) breaks the
/// run and the next edit pushes a fresh snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditKind {
    /// No edit yet / a boundary was crossed.
    Boundary,
    /// Typing word characters (letters, digits, `_`).
    TypingWord,
    /// Typing non-word, non-newline characters.
    TypingOther,
    /// Deleting backwards with backspace.
    Backspacing,
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
            h_scroll: 0,
            fullscreen: false,
            clipboard: String::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: EditKind::Boundary,
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
        self.h_scroll = 0;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_edit = EditKind::Boundary;
    }

    /// Get content as a string
    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Get current line
    pub fn current_line(&self) -> &str {
        self.lines
            .get(self.cursor.0)
            .map(|s| s.as_str())
            .unwrap_or("")
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

    /// Save an undo delta for an edit about to touch lines
    /// `first..=last` (always pushes; marks a coalescing boundary).
    fn save_undo(&mut self, first: usize, last: usize) {
        self.last_edit = EditKind::Boundary;
        self.push_undo_snapshot(first, last);
    }

    /// Save an undo delta, coalescing consecutive edits of the same
    /// kind: typing "hello" is ONE undo entry, not five.  The span of
    /// the FIRST edit in a run wins — later same-kind edits extend the
    /// run only when they stay inside a one-line window, which typing
    /// and backspacing within a line do; anything structural passes
    /// `EditKind::Boundary` and snapshots unconditionally.
    fn save_undo_coalesced(&mut self, kind: EditKind, first: usize, last: usize) {
        if self.last_edit != kind || kind == EditKind::Boundary {
            self.push_undo_snapshot(first, last);
        }
        self.last_edit = kind;
    }

    /// Record the pre-edit content of lines `first..=last` as a delta.
    /// O(span), never O(buffer) — the 1 MB-buffer contract.
    fn push_undo_snapshot(&mut self, first: usize, last: usize) {
        let first = first.min(self.lines.len().saturating_sub(1));
        let last = last.min(self.lines.len().saturating_sub(1)).max(first);
        self.undo_stack.push(EditorSnapshot {
            first,
            old_lines: self.lines[first..=last].to_vec(),
            len_before: self.lines.len(),
            cursor: self.cursor,
        });
        self.redo_stack.clear();
        // Limit undo history
        if self.undo_stack.len() > 200 {
            self.undo_stack.remove(0);
        }
    }

    /// Any cursor movement breaks an undo-coalescing run.
    fn break_coalescing(&mut self) {
        self.last_edit = EditKind::Boundary;
    }

    /// Apply `snapshot` to the buffer, returning the inverse delta
    /// (what redo/undo in the other direction needs).
    ///
    /// The edit replaced the snapshot's span with
    /// `len_now - len_before + old_lines.len()` lines starting at the
    /// same `first` — that count is exact for any single edit whose
    /// touched lines form one contiguous span, which every EditorState
    /// operation guarantees.
    fn apply_snapshot(&mut self, snapshot: EditorSnapshot) -> EditorSnapshot {
        let span_now = (self.lines.len() + snapshot.old_lines.len())
            .saturating_sub(snapshot.len_before)
            .min(self.lines.len() - snapshot.first);
        let inverse = EditorSnapshot {
            first: snapshot.first,
            old_lines: self.lines[snapshot.first..snapshot.first + span_now].to_vec(),
            len_before: self.lines.len(),
            cursor: self.cursor,
        };
        self.lines.splice(
            snapshot.first..snapshot.first + span_now,
            snapshot.old_lines.into_iter(),
        );
        self.cursor = snapshot.cursor;
        self.cursor.0 = self.cursor.0.min(self.lines.len().saturating_sub(1));
        self.cursor.1 = self
            .cursor
            .1
            .min(Self::char_len(&self.lines[self.cursor.0]));
        self.selection = None;
        inverse
    }

    /// Undo last change
    pub fn undo(&mut self) -> bool {
        self.break_coalescing();
        if let Some(snapshot) = self.undo_stack.pop() {
            let inverse = self.apply_snapshot(snapshot);
            self.redo_stack.push(inverse);
            true
        } else {
            false
        }
    }

    /// Redo last undone change
    pub fn redo(&mut self) -> bool {
        self.break_coalescing();
        if let Some(snapshot) = self.redo_stack.pop() {
            let inverse = self.apply_snapshot(snapshot);
            self.undo_stack.push(inverse);
            true
        } else {
            false
        }
    }

    /// Insert a character at cursor (char-based cursor)
    pub fn insert_char(&mut self, c: char) {
        let kind = if c == '\n' {
            EditKind::Boundary
        } else if c.is_alphanumeric() || c == '_' {
            EditKind::TypingWord
        } else {
            EditKind::TypingOther
        };
        let had_selection = self
            .selection
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if had_selection {
            // delete_selection saves its own snapshot.
            self.delete_selection();
            self.last_edit = kind;
        } else {
            let row = self.cursor.0;
            self.save_undo_coalesced(kind, row, row);
        }

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

    /// Insert a newline inheriting the current line's indentation, plus
    /// one level (4 spaces) when the text before the cursor ends with an
    /// opening bracket.  When the character AT the cursor is the matching
    /// closing bracket, the closer is pushed onto its own line at the
    /// original indentation — the `{<cursor>}` → "open, body, close"
    /// shape every code editor produces.
    pub fn insert_newline_auto_indent(&mut self) {
        if !self.delete_selection() {
            let row = self.cursor.0;
            self.save_undo(row, row);
        } else {
            self.break_coalescing();
        }

        let (row, char_col) = self.cursor;
        while row >= self.lines.len() {
            self.lines.push(String::new());
        }
        let line = self.lines[row].clone();
        let indent: String = line
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        let chars: Vec<char> = line.chars().collect();
        let before = chars.get(char_col.wrapping_sub(1)).copied();
        let at = chars.get(char_col).copied();
        let opens = matches!(before, Some('{') | Some('(') | Some('['));
        let closes_next = matches!(
            (before, at),
            (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
        );

        let byte_pos = Self::char_to_byte(&line, char_col);
        let rest = line[byte_pos..].to_string();
        self.lines[row].truncate(byte_pos);

        let body_indent = if opens {
            format!("{indent}    ")
        } else {
            indent.clone()
        };
        if closes_next {
            // `{|}` → open line, indented empty body line, closer line.
            self.lines.insert(row + 1, format!("{indent}{rest}"));
            self.lines.insert(row + 1, body_indent.clone());
            self.cursor = (row + 1, Self::char_len(&body_indent));
        } else {
            self.lines.insert(row + 1, format!("{body_indent}{rest}"));
            self.cursor = (row + 1, Self::char_len(&body_indent));
        }
    }

    /// Insert `c` with bracket/quote intelligence:
    /// * opener (`(`/`[`/`{`) → insert the pair, cursor between — unless
    ///   the next char is word-like (typing before a word stays literal);
    /// * closer typed where that closer already sits → skip over it;
    /// * `"` → paired the same way (skip-over when on a `"`).
    ///
    /// A non-empty selection wraps in the pair instead of replacing —
    /// select + `(` gives `(selection)`.
    pub fn insert_char_smart(&mut self, c: char) {
        let close_of = |c: char| match c {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            '"' => Some('"'),
            _ => None,
        };
        let is_closer = matches!(c, ')' | ']' | '}');

        // Wrap a live selection in the typed pair.
        if let Some(closer) = close_of(c)
            && let Some(sel) = self.selection
            && !sel.is_empty()
        {
            let sel = sel.normalize();
            self.save_undo(sel.start.0, sel.end.0);
            let end_line = &self.lines[sel.end.0];
            let be = Self::char_to_byte(end_line, sel.end.1);
            self.lines[sel.end.0].insert(be, closer);
            let start_line = &self.lines[sel.start.0];
            let bs = Self::char_to_byte(start_line, sel.start.1);
            self.lines[sel.start.0].insert(bs, c);
            let mut new_sel = sel;
            new_sel.start.1 += 1;
            if sel.start.0 == sel.end.0 {
                new_sel.end.1 += 1;
            }
            self.selection = Some(new_sel);
            self.cursor = new_sel.end;
            self.break_coalescing();
            return;
        }

        let at = self.char_at_cursor();
        // Skip over an already-present closer (also closing quote).
        if (is_closer || c == '"') && at == Some(c) {
            self.move_right(false);
            self.break_coalescing();
            return;
        }
        // Auto-close, unless gluing a quote/opener onto a word.
        if let Some(closer) = close_of(c) {
            let next_is_wordy = at.map(|n| n.is_alphanumeric() || n == '_').unwrap_or(false);
            // `"` before a word or right after a word stays literal
            // (closing a string typed left-to-right).
            let prev = self.char_before_cursor();
            let quote_after_word = c == '"'
                && prev.map(|p| p.is_alphanumeric() || p == '_' || p == '"').unwrap_or(false);
            if !next_is_wordy && !quote_after_word {
                self.insert_char(c);
                self.insert_char(closer);
                self.move_left(false);
                // The pair is one undo entry; keep typing coalescing alive.
                self.last_edit = EditKind::TypingOther;
                return;
            }
        }
        self.insert_char(c);
    }

    fn char_at_cursor(&self) -> Option<char> {
        self.lines
            .get(self.cursor.0)
            .and_then(|l| l.chars().nth(self.cursor.1))
    }

    fn char_before_cursor(&self) -> Option<char> {
        if self.cursor.1 == 0 {
            return None;
        }
        self.lines
            .get(self.cursor.0)
            .and_then(|l| l.chars().nth(self.cursor.1 - 1))
    }

    /// Insert a string at cursor (char-based)
    pub fn insert_str(&mut self, s: &str) {
        if !self.delete_selection() {
            let row = self.cursor.0;
            self.save_undo(row, row);
        } else {
            self.break_coalescing();
        }

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

    /// Delete character before cursor (backspace).  Inside an empty
    /// bracket/quote pair, removes BOTH halves — the inverse of the
    /// smart insert.
    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }

        // A column-0 backspace merges with the PREVIOUS line — outside
        // a same-line coalescing run, so it snapshots unconditionally
        // with the two-line span.
        let (srow, scol) = self.cursor;
        if scol > 0 {
            self.save_undo_coalesced(EditKind::Backspacing, srow, srow);
        } else if srow > 0 {
            self.save_undo(srow - 1, srow);
        } else {
            return;
        }

        let empty_pair = matches!(
            (self.char_before_cursor(), self.char_at_cursor()),
            (Some('('), Some(')'))
                | (Some('['), Some(']'))
                | (Some('{'), Some('}'))
                | (Some('"'), Some('"'))
        );

        let (row, char_col) = self.cursor;

        if char_col > 0 {
            let end_col = if empty_pair { char_col + 1 } else { char_col };
            let byte_start = Self::char_to_byte(&self.lines[row], char_col - 1);
            let byte_end = Self::char_to_byte(&self.lines[row], end_col);
            self.lines[row].drain(byte_start..byte_end);
            self.cursor.1 -= 1;
        } else if row > 0 {
            let current = self.lines.remove(row);
            self.cursor.0 -= 1;
            self.cursor.1 = Self::char_len(&self.lines[self.cursor.0]);
            self.lines[self.cursor.0].push_str(&current);
        }
    }

    /// Delete the word (or whitespace run) before the cursor.
    /// (`delete_selection` records the undo snapshot.)
    pub fn delete_word_left(&mut self) {
        if self.delete_selection() {
            return;
        }
        let target = {
            let mut probe = self.clone();
            probe.move_word_left(false);
            probe.cursor
        };
        self.selection = Some(Selection {
            start: target,
            end: self.cursor,
        });
        self.delete_selection();
        self.break_coalescing();
    }

    /// Delete the word (or whitespace run) after the cursor.
    /// (`delete_selection` records the undo snapshot.)
    pub fn delete_word_right(&mut self) {
        if self.delete_selection() {
            return;
        }
        let target = {
            let mut probe = self.clone();
            probe.move_word_right(false);
            probe.cursor
        };
        self.selection = Some(Selection {
            start: self.cursor,
            end: target,
        });
        self.delete_selection();
        self.break_coalescing();
    }

    /// Delete character at cursor
    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }

        let (row, char_col) = self.cursor;
        let line_chars = self.lines.get(row).map(|l| Self::char_len(l)).unwrap_or(0);
        if char_col < line_chars {
            self.save_undo(row, row);
        } else {
            self.save_undo(row, row + 1);
        }

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

            let norm = sel.normalize();
            self.save_undo(norm.start.0, norm.end.0);
            let sel = norm;

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
            self.cursor.1 = self
                .cursor
                .1
                .min(Self::char_len(&self.lines[self.cursor.0]));
        }

        self.end_selection(with_selection);
    }

    /// Move cursor down (clamp to line length in chars)
    pub fn move_down(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);

        if self.cursor.0 < self.lines.len() - 1 {
            self.cursor.0 += 1;
            self.cursor.1 = self
                .cursor
                .1
                .min(Self::char_len(&self.lines[self.cursor.0]));
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
        if with_selection && let Some(ref mut sel) = self.selection {
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

    /// Keep the cursor inside the horizontal window of `text_width`
    /// chars — long lines PAN instead of hiding the cursor past the
    /// right edge.  A small margin keeps context visible while typing.
    pub fn ensure_cursor_visible_h(&mut self, text_width: usize) {
        if text_width == 0 {
            return;
        }
        let margin = 4.min(text_width / 4);
        if self.cursor.1 < self.h_scroll + margin {
            self.h_scroll = self.cursor.1.saturating_sub(margin);
        } else if self.cursor.1 >= self.h_scroll + text_width - margin.min(text_width - 1) {
            self.h_scroll = self.cursor.1 + margin - text_width + 1;
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
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i -= 1;
        }
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
            if row < self.lines.len() - 1 {
                self.cursor.0 += 1;
                self.cursor.1 = 0;
            }
            self.end_selection(select);
            return;
        }
        let mut i = char_col;
        while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor.1 = i;
        self.end_selection(select);
    }

    /// Insert a tab (4 spaces with alignment).
    pub fn insert_tab(&mut self) {
        let col = self.cursor.1;
        let spaces = 4 - (col % 4);
        for _ in 0..spaces {
            self.insert_char(' ');
        }
    }

    /// Line span covered by the selection (or just the cursor line).
    fn selected_line_range(&self) -> (usize, usize) {
        match self.selection.map(|s| s.normalize()) {
            Some(sel) if !sel.is_empty() => (sel.start.0, sel.end.0.min(self.lines.len() - 1)),
            _ => (self.cursor.0, self.cursor.0),
        }
    }

    /// Duplicate the current line (no selection) or the selected line
    /// block, placing the copy below and moving the cursor onto it.
    pub fn duplicate_lines(&mut self) {
        let (first, last) = self.selected_line_range();
        self.save_undo(first, last);
        let block: Vec<String> = self.lines[first..=last].to_vec();
        let count = block.len();
        for (i, line) in block.into_iter().enumerate() {
            self.lines.insert(last + 1 + i, line);
        }
        self.cursor.0 += count;
        if let Some(ref mut sel) = self.selection {
            sel.start.0 += count;
            sel.end.0 += count;
        }
        self.break_coalescing();
    }

    /// Delete the current line (or every line the selection touches).
    pub fn delete_lines(&mut self) {
        let (first, last) = self.selected_line_range();
        self.save_undo(first, last);
        for _ in first..=last {
            if self.lines.len() > 1 {
                self.lines.remove(first);
            } else {
                self.lines[0].clear();
            }
        }
        self.selection = None;
        self.cursor.0 = first.min(self.lines.len() - 1);
        self.cursor.1 = self
            .cursor
            .1
            .min(Self::char_len(&self.lines[self.cursor.0]));
        self.break_coalescing();
    }

    /// Move the current line (or selected block) up one line.
    pub fn move_lines_up(&mut self) {
        let (first, last) = self.selected_line_range();
        if first == 0 {
            return;
        }
        self.save_undo(first - 1, last);
        let above = self.lines.remove(first - 1);
        self.lines.insert(last, above);
        self.cursor.0 -= 1;
        if let Some(ref mut sel) = self.selection {
            sel.start.0 -= 1;
            sel.end.0 -= 1;
        }
        self.break_coalescing();
    }

    /// Move the current line (or selected block) down one line.
    pub fn move_lines_down(&mut self) {
        let (first, last) = self.selected_line_range();
        if last + 1 >= self.lines.len() {
            return;
        }
        self.save_undo(first, last + 1);
        let below = self.lines.remove(last + 1);
        self.lines.insert(first, below);
        self.cursor.0 += 1;
        if let Some(ref mut sel) = self.selection {
            sel.start.0 += 1;
            sel.end.0 += 1;
        }
        self.break_coalescing();
    }

    /// Toggle `// ` line comments on the selected lines (or the cursor
    /// line).  All-commented → uncomment; otherwise comment every
    /// non-blank line at the block's minimum indentation.
    pub fn toggle_comment(&mut self) {
        let (first, last) = self.selected_line_range();
        self.save_undo(first, last);
        let all_commented = self.lines[first..=last]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .all(|l| l.trim_start().starts_with("//"));
        if all_commented {
            for line in &mut self.lines[first..=last] {
                if let Some(pos) = line.find("//") {
                    let after = if line[pos + 2..].starts_with(' ') {
                        pos + 3
                    } else {
                        pos + 2
                    };
                    line.replace_range(pos..after, "");
                }
            }
        } else {
            let min_indent = self.lines[first..=last]
                .iter()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.chars().take_while(|c| *c == ' ').count())
                .min()
                .unwrap_or(0);
            for line in &mut self.lines[first..=last] {
                if !line.trim().is_empty() {
                    let byte = Self::char_to_byte(line, min_indent);
                    line.insert_str(byte, "// ");
                }
            }
        }
        self.cursor.1 = self
            .cursor
            .1
            .min(Self::char_len(&self.lines[self.cursor.0]));
        self.break_coalescing();
    }

    /// Indent the selected lines (or the cursor line) by 4 spaces.
    pub fn indent_lines(&mut self) {
        let (first, last) = self.selected_line_range();
        self.save_undo(first, last);
        for line in &mut self.lines[first..=last] {
            if !line.is_empty() {
                line.insert_str(0, "    ");
            }
        }
        self.cursor.1 += 4;
        if let Some(ref mut sel) = self.selection {
            sel.start.1 += 4;
            sel.end.1 += 4;
        }
        self.break_coalescing();
    }

    /// Dedent the selected lines (or the cursor line) by up to 4 spaces.
    pub fn dedent_lines(&mut self) {
        let (first, last) = self.selected_line_range();
        self.save_undo(first, last);
        let mut cursor_shift = 0usize;
        for (i, line) in self.lines[first..=last].iter_mut().enumerate() {
            let strip = line.chars().take_while(|c| *c == ' ').count().min(4);
            line.replace_range(..strip, "");
            if first + i == self.cursor.0 {
                cursor_shift = strip;
            }
        }
        self.cursor.1 = self.cursor.1.saturating_sub(cursor_shift);
        if let Some(ref mut sel) = self.selection {
            sel.start.1 = sel.start.1.saturating_sub(4);
            sel.end.1 = sel.end.1.saturating_sub(4);
        }
        self.break_coalescing();
    }

    /// Smart Home: jump to the first non-whitespace character, or to
    /// column 0 when already there.
    pub fn move_home_smart(&mut self, with_selection: bool) {
        self.begin_selection(with_selection);
        let first_nonws = self
            .current_line()
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        self.cursor.1 = if self.cursor.1 == first_nonws {
            0
        } else {
            first_nonws
        };
        self.end_selection(with_selection);
        self.break_coalescing();
    }

    /// Kill from the cursor to the end of the line (joins the next line
    /// when already at the end) — the Ctrl+K everyone's fingers know.
    pub fn kill_to_eol(&mut self) {
        let (row, char_col) = self.cursor;
        let line_chars = Self::char_len(&self.lines[row]);
        if char_col < line_chars {
            self.save_undo(row, row);
        } else {
            self.save_undo(row, row + 1);
        }
        if char_col < line_chars {
            let byte = Self::char_to_byte(&self.lines[row], char_col);
            self.lines[row].truncate(byte);
        } else if row + 1 < self.lines.len() {
            let next = self.lines.remove(row + 1);
            self.lines[row].push_str(&next);
        }
        self.break_coalescing();
    }

    /// Join the next line onto this one with a single space.
    pub fn join_lines(&mut self) {
        let row = self.cursor.0;
        if row + 1 >= self.lines.len() {
            return;
        }
        self.save_undo(row, row + 1);
        let next = self.lines.remove(row + 1);
        let trimmed_len = self.lines[row].trim_end().len();
        self.lines[row].truncate(trimmed_len);
        self.cursor.1 = Self::char_len(&self.lines[row]);
        if !self.lines[row].is_empty() && !next.trim_start().is_empty() {
            self.lines[row].push(' ');
            self.cursor.1 += 1;
        }
        self.lines[row].push_str(next.trim_start());
        self.break_coalescing();
    }

    /// Move the cursor a page up/down (page = `page` lines).
    pub fn page_move(&mut self, down: bool, page: usize, with_selection: bool) {
        self.begin_selection(with_selection);
        if down {
            self.cursor.0 = (self.cursor.0 + page).min(self.lines.len() - 1);
        } else {
            self.cursor.0 = self.cursor.0.saturating_sub(page);
        }
        self.cursor.1 = self
            .cursor
            .1
            .min(Self::char_len(&self.lines[self.cursor.0]));
        self.end_selection(with_selection);
        self.break_coalescing();
    }

    /// The bracket pair under (or just before) the cursor, as
    /// `(cursor_bracket, matching_bracket)` positions — `None` when the
    /// cursor is not on a bracket or the match is unbalanced.  Drives
    /// the render-time match highlight.
    pub fn matching_bracket(&self) -> Option<((usize, usize), (usize, usize))> {
        let probe = |pos: (usize, usize)| -> Option<char> {
            self.lines.get(pos.0).and_then(|l| l.chars().nth(pos.1))
        };
        // Prefer the char at the cursor; fall back to the one before it.
        let (pos, c) = if let Some(c) = probe(self.cursor).filter(|c| "()[]{}".contains(*c)) {
            (self.cursor, c)
        } else if self.cursor.1 > 0 {
            let before = (self.cursor.0, self.cursor.1 - 1);
            let c = probe(before).filter(|c| "()[]{}".contains(*c))?;
            (before, c)
        } else {
            return None;
        };

        let (open, close, forward) = match c {
            '(' => ('(', ')', true),
            '[' => ('[', ']', true),
            '{' => ('{', '}', true),
            ')' => ('(', ')', false),
            ']' => ('[', ']', false),
            '}' => ('{', '}', false),
            _ => return None,
        };

        let mut depth = 0i32;
        if forward {
            let mut row = pos.0;
            let mut col = pos.1;
            loop {
                let line: Vec<char> = self.lines.get(row)?.chars().collect();
                while col < line.len() {
                    let ch = line[col];
                    if ch == open {
                        depth += 1;
                    } else if ch == close {
                        depth -= 1;
                        if depth == 0 {
                            return Some((pos, (row, col)));
                        }
                    }
                    col += 1;
                }
                row += 1;
                col = 0;
                if row >= self.lines.len() {
                    return None;
                }
            }
        } else {
            let mut row = pos.0;
            let mut col = pos.1 as isize;
            loop {
                let line: Vec<char> = self.lines.get(row)?.chars().collect();
                while col >= 0 {
                    let ch = line[col as usize];
                    if ch == close {
                        depth += 1;
                    } else if ch == open {
                        depth -= 1;
                        if depth == 0 {
                            return Some((pos, (row, col as usize)));
                        }
                    }
                    col -= 1;
                }
                if row == 0 {
                    return None;
                }
                row -= 1;
                col = self.lines[row].chars().count() as isize - 1;
            }
        }
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
            DiagnosticSeverity::Error => Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::UNDERLINED),
            DiagnosticSeverity::Warning => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::UNDERLINED),
            DiagnosticSeverity::Info => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED),
            DiagnosticSeverity::Hint => Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::UNDERLINED),
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

}

impl<'a> Widget for EditorWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.state.fullscreen {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green)
        };

        // The title line doubles as the editor's status line: position,
        // buffer size, selection size — always current, no extra row.
        let sel_note = self
            .state
            .selected_text()
            .map(|t| format!("  sel {}", t.chars().count()))
            .unwrap_or_default();
        let title = format!(
            "{}{} — Ln {}, Col {} ({} lines){}",
            self.title,
            if self.state.fullscreen {
                " [FULLSCREEN — Ctrl+F/Esc]"
            } else {
                ""
            },
            self.state.cursor.0 + 1,
            self.state.cursor.1 + 1,
            self.state.lines.len(),
            sel_note,
        );

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
        let h_scroll = self.state.h_scroll;
        let bracket_pair = self.state.matching_bracket();

        // Render visible lines
        for (i, line_idx) in
            (self.state.scroll_offset..(self.state.scroll_offset + visible_lines)).enumerate()
        {
            let y = inner.y + i as u16;

            if line_idx >= self.state.lines.len() {
                // Render ~ for lines past end of file
                if self.show_line_numbers {
                    buf.set_string(inner.x, y, "~", Style::default().fg(Color::DarkGray));
                }
                continue;
            }

            // Render line number
            if self.show_line_numbers {
                let num_str = format!(
                    "{:>width$} ",
                    line_idx + 1,
                    width = line_num_width as usize - 1
                );
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

            // One highlight carrier for cells and editor alike.
            let spans = super::cell::highlight_verum_line(line);
            // char_idx walks the FULL line; screen column subtracts the
            // horizontal scroll (long lines pan under the window).
            let mut char_idx = 0usize;

            // Pre-normalized selection for the row (cheap per line).
            let sel = self.state.selection.map(|s| s.normalize());

            'line: for span in spans {
                let content = span.content.as_ref();
                for c in content.chars() {
                    if char_idx < h_scroll {
                        char_idx += 1;
                        continue;
                    }
                    let col = (char_idx - h_scroll) as u16;
                    if col >= text_width {
                        break 'line;
                    }

                    let is_selected = sel.as_ref().is_some_and(|sel| {
                        let pos = (line_idx, char_idx);
                        (sel.start.0 < pos.0 || (sel.start.0 == pos.0 && sel.start.1 <= pos.1))
                            && (pos.0 < sel.end.0 || (pos.0 == sel.end.0 && pos.1 < sel.end.1))
                    });

                    let mut style = span.style;
                    if is_selected {
                        style = style.bg(Color::Blue);
                    }
                    if let Some((a, b)) = bracket_pair
                        && ((line_idx, char_idx) == a || (line_idx, char_idx) == b)
                    {
                        style = style
                            .bg(Color::DarkGray)
                            .add_modifier(Modifier::BOLD);
                    }

                    // Diagnostics columns are char-based for display.
                    for diag in self.diagnostics {
                        if diag.line == line_idx
                            && char_idx >= diag.col_start
                            && char_idx < diag.col_end
                        {
                            style = style.patch(diag.severity.style());
                        }
                    }

                    if let Some(cell) = buf.cell_mut((text_x + col, y)) {
                        cell.set_char(c);
                        cell.set_style(style);
                    }
                    char_idx += 1;
                }
            }

            // A panned line hints its hidden tail/head with `…`.
            if h_scroll > 0 {
                if let Some(cell) = buf.cell_mut((text_x, y))
                    && char_idx > 0
                {
                    cell.set_char('…');
                }
            }

            // Render cursor
            if line_idx == self.state.cursor.0 && self.state.cursor.1 >= h_scroll {
                let cursor_col = (self.state.cursor.1 - h_scroll) as u16;
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
            let mut scrollbar_state =
                ScrollbarState::new(self.state.lines.len()).position(self.state.scroll_offset);

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
