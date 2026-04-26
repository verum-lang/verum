//! Span-underlined human output for `verum lint --format human`.
//!
//! The shape of `rustc` / `clippy` / `ruff`: rule name in brackets,
//! filename with `--> `, the offending source line, a caret
//! underline at the column the issue points to, and a help
//! suggestion when the rule provides one.
//!
//! Example output:
//!
//! ```text
//! error[no-unwrap-in-prod]: use `?` or `expect("why")` instead of unwrap()
//!   --> src/main.vr:8:13
//!    |
//!  8 |     let y = x.unwrap();
//!    |             ^^^^^^^^^^
//!    |
//!    = help: consider matching on the variant explicitly
//! ```
//!
//! ANSI colour is added via the existing `colored` crate which
//! honours `NO_COLOR` automatically. CI logs that strip ANSI render
//! the same diagnostic in monochrome.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use colored::Colorize;

use super::lint::{LintIssue, LintLevel};

/// Source-file lookup: a single file is opened once per render
/// pass and its lines cached, so a corpus with N issues across M
/// files reads each file only once.
pub struct SourceMap {
    files: HashMap<std::path::PathBuf, Vec<String>>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    fn lines_for(&mut self, path: &Path) -> Option<&[String]> {
        if !self.files.contains_key(path) {
            let content = fs::read_to_string(path).ok()?;
            let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
            self.files.insert(path.to_path_buf(), lines);
        }
        self.files.get(path).map(|v| v.as_slice())
    }
}

impl Default for SourceMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Render one issue as a multi-line human diagnostic block. The
/// returned string includes a trailing blank line so successive
/// blocks are separated. Colour is applied through `colored`,
/// which the user can disable via `NO_COLOR=1` or `--color never`.
pub fn render_issue(issue: &LintIssue, sources: &mut SourceMap) -> String {
    let level_label = level_label(issue.level);
    let header = format!(
        "{level}[{rule}]: {msg}",
        level = level_label,
        rule = issue.rule.cyan(),
        msg = issue.message,
    );

    let location = format!(
        "  {arrow} {file}:{line}:{col}",
        arrow = "-->".bright_blue().bold(),
        file = issue.file.display(),
        line = issue.line,
        col = issue.column,
    );

    // Source-line block — when we can find the file. If not
    // (single-file mode, in-memory test fixtures) we fall back to
    // just the header + location.
    let source_block = render_source_block(issue, sources);

    let help = issue
        .suggestion
        .as_ref()
        .map(|s| format!("   {} {}: {}", "=".bright_blue().bold(), "help".green().bold(), s))
        .unwrap_or_default();

    let mut out = String::new();
    out.push_str(&header);
    out.push('\n');
    out.push_str(&location);
    out.push('\n');
    if !source_block.is_empty() {
        out.push_str(&source_block);
        out.push('\n');
    }
    if !help.is_empty() {
        out.push_str(&help);
        out.push('\n');
    }
    out.push('\n');
    out
}

/// Build the source-line + caret block. Returns empty string if
/// the file isn't readable (no panic, no error — just no source
/// context).
fn render_source_block(issue: &LintIssue, sources: &mut SourceMap) -> String {
    let lines = match sources.lines_for(&issue.file) {
        Some(l) => l,
        None => return String::new(),
    };
    let idx = issue.line.saturating_sub(1);
    if idx >= lines.len() {
        return String::new();
    }
    let source_line = &lines[idx];

    // Right-align the line number column to the width of the
    // largest digit so multi-line spans align.
    let line_num_str = format!("{}", issue.line);
    let gutter_width = line_num_str.len();
    let pad = " ".repeat(gutter_width);
    let pipe = "|".bright_blue().bold();

    // Caret underline. The column is 1-indexed; the caret length
    // is a heuristic — most of our diagnostics don't carry an
    // explicit end-column, so we underline to the end of the
    // identifier-like token starting at the column.
    let col_zero = issue.column.saturating_sub(1);
    let underline_len = caret_underline_len(source_line, col_zero);
    let mut caret = String::new();
    for _ in 0..col_zero {
        caret.push(' ');
    }
    for _ in 0..underline_len.max(1) {
        caret.push('^');
    }

    let caret_coloured = match issue.level {
        LintLevel::Error => caret.red().bold().to_string(),
        LintLevel::Warning => caret.yellow().bold().to_string(),
        LintLevel::Info | LintLevel::Hint => caret.bright_blue().bold().to_string(),
        LintLevel::Off => caret,
    };

    format!(
        "   {pad} {pipe}\n \
          {n} {pipe} {src}\n   \
          {pad} {pipe} {caret}",
        pad = pad,
        pipe = pipe,
        n = line_num_str.bright_blue().bold(),
        src = source_line,
        caret = caret_coloured,
    )
}

/// Heuristic caret length: extend through whatever identifier-like
/// or operator-like token starts at `col`. Punctuation boundaries
/// stop the underline so we don't paint the whole line.
fn caret_underline_len(line: &str, col_zero: usize) -> usize {
    let bytes = line.as_bytes();
    if col_zero >= bytes.len() {
        return 1;
    }
    let mut len = 0;
    let mut i = col_zero;
    let starting = bytes[i];
    let starts_alnum = starting.is_ascii_alphanumeric() || starting == b'_';
    while i < bytes.len() {
        let b = bytes[i];
        let is_alnum = b.is_ascii_alphanumeric() || b == b'_';
        if starts_alnum {
            if !is_alnum {
                break;
            }
        } else if b == b' ' || b == b'\t' || b == b'\n' {
            break;
        }
        len += 1;
        i += 1;
        // Cap the underline at 80 chars so a runaway long line
        // doesn't fill the screen.
        if len >= 80 {
            break;
        }
    }
    len
}

fn level_label(level: LintLevel) -> String {
    match level {
        LintLevel::Error => "error".red().bold().to_string(),
        LintLevel::Warning => "warning".yellow().bold().to_string(),
        LintLevel::Info => "info".bright_blue().bold().to_string(),
        LintLevel::Hint => "hint".bright_blue().to_string(),
        LintLevel::Off => "off".dimmed().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use verum_common::Text;

    fn strip_ansi(s: &str) -> String {
        // Tiny ANSI stripper: removes ESC[... m sequences.
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1B && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() && bytes[i] != b'm' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        out
    }

    fn issue() -> LintIssue {
        LintIssue {
            rule: "todo-in-code",
            level: LintLevel::Warning,
            file: PathBuf::from("nonexistent.vr"),
            line: 8,
            column: 13,
            message: "TODO comment in code".to_string(),
            suggestion: Some(Text::from("TODO(#0000)")),
            fixable: true,
        }
    }

    #[test]
    fn header_contains_level_rule_message() {
        let mut sm = SourceMap::new();
        let out = strip_ansi(&render_issue(&issue(), &mut sm));
        assert!(out.contains("warning"), "got: {out}");
        assert!(out.contains("[todo-in-code]"), "got: {out}");
        assert!(out.contains("TODO comment in code"), "got: {out}");
    }

    #[test]
    fn location_block_uses_arrow() {
        let mut sm = SourceMap::new();
        let out = strip_ansi(&render_issue(&issue(), &mut sm));
        assert!(out.contains("-->"), "got: {out}");
        assert!(out.contains("nonexistent.vr:8:13"), "got: {out}");
    }

    #[test]
    fn no_source_block_when_file_missing() {
        // The fixture file doesn't exist — render must not panic
        // and must omit the source-line section.
        let mut sm = SourceMap::new();
        let out = strip_ansi(&render_issue(&issue(), &mut sm));
        // The pipe character `|` should not appear without a source.
        assert!(
            !out.contains(" 8 |"),
            "expected no source block for missing file: {out}"
        );
    }

    #[test]
    fn source_block_renders_with_caret_when_file_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("main.vr");
        std::fs::write(
            &path,
            "fn main() {\n\
             let x = 1;\n\
             let y = 2;\n\
             let z = 3;\n\
             let q = 4;\n\
             let r = 5;\n\
             let s = 6;\n    \
             let a = something.unwrap();\n\
             }\n",
        )
        .expect("fixture");
        let mut iss = issue();
        iss.file = path.clone();
        iss.message = "use `?` instead of unwrap()".to_string();

        let mut sm = SourceMap::new();
        let out = strip_ansi(&render_issue(&iss, &mut sm));
        assert!(out.contains(" 8 |"), "expected line gutter: {out}");
        assert!(out.contains("^"), "expected caret underline: {out}");
    }

    #[test]
    fn help_line_appears_only_when_suggestion_present() {
        let mut iss = issue();
        let mut sm = SourceMap::new();
        let with = strip_ansi(&render_issue(&iss, &mut sm));
        assert!(with.contains("help"), "got: {with}");

        iss.suggestion = None;
        let mut sm2 = SourceMap::new();
        let without = strip_ansi(&render_issue(&iss, &mut sm2));
        assert!(!without.contains("help"), "got: {without}");
    }

    #[test]
    fn caret_underline_extends_through_identifier() {
        // `let y = x.unwrap();` with col=13 (1-indexed) points at
        // the `x`. The underline should walk through `x` then stop
        // at the `.` boundary. So length = 1.
        let len = caret_underline_len("let y = x.unwrap();", 8);
        assert_eq!(len, 1, "got: {len}");

        // col=10 points at `unwrap` — length = 6 (the identifier).
        let len2 = caret_underline_len("let y = x.unwrap();", 10);
        assert_eq!(len2, 6, "got: {len2}");
    }
}
