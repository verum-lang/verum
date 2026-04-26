// Format command - auto-format Verum source code using AST-based pretty printing.
//
// Architecture:
// 1. Parse source → AST via verum_parser
// 2. Pretty-print AST via verum_ast::PrettyPrinter (primary)
// 3. Fallback: whitespace normalization (when parse fails)
//
// Supports: --check (dry run), --diff (show changes), --verbose, --config

use crate::error::{CliError, Result};
use crate::ui;
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use verum_ast::{FileId, PrettyConfig, PrettyPrinter};
use verum_common::{List, Text};
use verum_lexer::Lexer;
use verum_parser::VerumParser;
use walkdir::WalkDir;

/// Behaviour when the parser cannot turn a source file into an AST.
///
/// - **Fallback** (default): silently apply whitespace normalisation
///   and import sorting; print a warning naming the file. Backwards-
///   compatible with pre-policy behaviour.
/// - **Skip**: leave the file untouched; print a warning. Exit 0.
/// - **Error**: leave the file untouched; print an error with the
///   parse diagnostic. Exit non-zero (the rest of the corpus is
///   still processed first, so one bad file doesn't hide others).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnParseError {
    Fallback,
    Skip,
    Error,
}

impl OnParseError {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fallback" => Some(Self::Fallback),
            "skip" => Some(Self::Skip),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// Verum formatter configuration.
#[derive(Debug, Clone)]
pub struct FormatterConfig {
    /// Maximum line width (default: 100).
    pub max_width: usize,
    /// Indentation size in spaces (default: 4).
    pub indent_size: usize,
    /// Use spaces instead of tabs.
    pub use_spaces: bool,
    /// Insert trailing comma in multi-line expressions.
    pub trailing_comma: bool,
    /// Sort mount imports alphabetically.
    pub sort_imports: bool,
    /// Normalize blank lines (max 1 consecutive).
    pub normalize_blanks: bool,
    /// Ensure trailing newline.
    pub trailing_newline: bool,
    /// Behaviour when a file fails to parse — see [OnParseError].
    pub on_parse_error: OnParseError,
}

impl Default for FormatterConfig {
    fn default() -> Self {
        Self {
            max_width: 100,
            indent_size: 4,
            use_spaces: true,
            trailing_comma: true,
            sort_imports: true,
            normalize_blanks: true,
            trailing_newline: true,
            on_parse_error: OnParseError::Fallback,
        }
    }
}

impl From<&FormatterConfig> for PrettyConfig {
    fn from(config: &FormatterConfig) -> Self {
        PrettyConfig {
            max_width: config.max_width,
            indent_size: config.indent_size,
            use_spaces: config.use_spaces,
            trailing_comma: config.trailing_comma,
            multiline_threshold: 3,
        }
    }
}

/// Try to load config from `.verum.toml` or `verum.toml` in current directory.
fn load_config() -> FormatterConfig {
    for name in &[".verum.toml", "verum.toml"] {
        let path = PathBuf::from(name);
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                return parse_config(&content);
            }
        }
    }
    FormatterConfig::default()
}

/// Parse TOML-style config (simple key=value, no dependency on toml crate).
fn parse_config(content: &str) -> FormatterConfig {
    let mut config = FormatterConfig::default();
    let mut in_fmt_section = false;
    let mut in_policy_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_fmt_section = trimmed == "[fmt]" || trimmed == "[format]";
            in_policy_section =
                trimmed == "[fmt.policy]" || trimmed == "[format.policy]";
            continue;
        }
        if !in_fmt_section && !in_policy_section { continue; }

        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"');
            match key {
                "max_width" => { if let Ok(v) = value.parse() { config.max_width = v; } }
                "indent_size" => { if let Ok(v) = value.parse() { config.indent_size = v; } }
                "use_spaces" => { config.use_spaces = value == "true"; }
                "trailing_comma" => { config.trailing_comma = value == "true"; }
                "sort_imports" => { config.sort_imports = value == "true"; }
                "on_parse_error" if in_policy_section => {
                    if let Some(p) = OnParseError::parse(value) {
                        config.on_parse_error = p;
                    }
                }
                _ => {}
            }
        }
    }
    config
}

/// Main entry point for `verum fmt`. Uses the parse-error policy
/// from `[fmt.policy].on_parse_error` in the manifest, defaulting
/// to `Fallback`.
pub fn execute(check: bool, verbose: bool) -> Result<()> {
    execute_with_policy(check, verbose, None)
}

/// Like `execute` but with an explicit parse-error policy override
/// (the `--on-parse-error` CLI flag). When `policy` is `Some`, it
/// wins over the manifest setting for this run only.
pub fn execute_with_policy(
    check: bool,
    verbose: bool,
    policy: Option<OnParseError>,
) -> Result<()> {
    if check {
        ui::step("Checking formatting");
    } else {
        ui::step("Formatting source files");
    }

    let mut config = load_config();
    if let Some(p) = policy {
        config.on_parse_error = p;
    }
    let total_files;
    let mut formatted_files = 0;
    let mut changed_files = List::new();
    let mut error_files = List::new();

    // Find all .vr files in src/ and core/
    let search_dirs: Vec<PathBuf> = ["src", "core"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    if search_dirs.is_empty() {
        return Err(CliError::Custom("No src/ or core/ directory found".into()));
    }

    // Discover every Verum file across the search roots first, then
    // process them in parallel via rayon. Each file is independent:
    // the formatter is pure on (path, content, config), and the only
    // shared state is the result aggregator. Output is sorted into
    // a deterministic order before reporting so 1-thread and 8-thread
    // runs produce byte-identical reports.
    use rayon::prelude::*;
    let files: Vec<PathBuf> = search_dirs
        .iter()
        .flat_map(|d| {
            WalkDir::new(d)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .map(|e| e.into_path())
                .filter(|p| p.is_file() && is_verum_file(p))
                .collect::<Vec<_>>()
        })
        .collect();
    total_files = files.len();

    enum FmtOutcome {
        Unchanged,
        Changed { diff: Option<String> },
        Failed(String),
    }
    let mut results: Vec<(PathBuf, FmtOutcome)> = files
        .par_iter()
        .map(|path| {
            let outcome = match format_file(path, &config, check, verbose) {
                Ok(FormatResult::Unchanged) => FmtOutcome::Unchanged,
                Ok(FormatResult::Changed { diff }) => FmtOutcome::Changed { diff },
                Err(e) => FmtOutcome::Failed(e.to_string()),
            };
            (path.clone(), outcome)
        })
        .collect();
    // Deterministic order regardless of thread interleaving.
    results.sort_by(|a, b| a.0.cmp(&b.0));

    for (path, outcome) in results {
        match outcome {
            FmtOutcome::Unchanged => {}
            FmtOutcome::Changed { diff } => {
                formatted_files += 1;
                changed_files.push(path.clone());
                if verbose {
                    if let Some(d) = &diff {
                        println!("{}", d);
                    }
                    ui::info(&format!("Formatted: {}", path.display()));
                }
            }
            FmtOutcome::Failed(e) => {
                error_files.push((path.clone(), e.clone()));
                if verbose {
                    ui::error(&format!("Failed: {}: {}", path.display(), e));
                }
            }
        }
    }

    println!();

    if !error_files.is_empty() {
        ui::error(&format!("{} files had parse errors:", error_files.len()));
        for (file, err) in &error_files {
            println!("  {} {}", "-".red(), file.display());
            if verbose {
                println!("    {}", err);
            }
        }
        println!();
    }

    if check {
        if formatted_files == 0 && error_files.is_empty() {
            ui::success(&format!("All {} files are formatted correctly", total_files));
            Ok(())
        } else if formatted_files > 0 {
            ui::error(&format!("{} files need formatting:", formatted_files));
            for file in &changed_files {
                println!("  {} {}", "-".yellow(), file.display());
            }
            println!();
            println!("Run {} to format these files", "verum fmt".cyan().bold());
            Err(CliError::Custom(format!("{} files need formatting", formatted_files)))
        } else {
            Err(CliError::Custom(format!("{} files had parse errors", error_files.len())))
        }
    } else {
        if formatted_files == 0 {
            ui::success(&format!("All {} files already formatted", total_files));
        } else {
            ui::success(&format!("Formatted {} of {} files", formatted_files, total_files));
        }
        Ok(())
    }
}

/// Result of formatting a single file.
enum FormatResult {
    Unchanged,
    Changed { diff: Option<String> },
}

/// Check if file is a Verum source file (.vr).
fn is_verum_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext == "vr")
        .unwrap_or(false)
}

/// Format a single file. Returns whether the file was changed.
fn format_file(path: &Path, config: &FormatterConfig, check: bool, verbose: bool) -> Result<FormatResult> {
    let content = fs::read_to_string(path)?;

    // Try AST-based formatting first. On failure, the
    // `on_parse_error` policy decides what to do:
    //   - Fallback (default): silently fall through to whitespace
    //     normalisation + import sorting. Print a stderr warning so
    //     the user knows the AST formatter wasn't used.
    //   - Skip: leave the file untouched, exit 0 from this call.
    //   - Error: leave the file untouched, return Err. The caller
    //     accumulates errors and propagates them via exit code.
    let formatted = match try_ast_format(&content, config) {
        Some(f) => f,
        None => match config.on_parse_error {
            OnParseError::Fallback => {
                ui::warn(&format!(
                    "parse failed on {}; emitting whitespace-normalised form. \
                     Set `[fmt.policy].on_parse_error = \"error\"` to fail the run instead.",
                    path.display()
                ));
                normalize_and_sort(&content, config)
            }
            OnParseError::Skip => {
                if verbose {
                    ui::warn(&format!("parse failed on {}; skipping", path.display()));
                }
                return Ok(FormatResult::Unchanged);
            }
            OnParseError::Error => {
                return Err(CliError::Custom(format!(
                    "parse failed on {}: refusing to format under [fmt.policy].on_parse_error = \"error\"",
                    path.display()
                )));
            }
        },
    };

    if content == formatted.as_str() {
        return Ok(FormatResult::Unchanged);
    }

    let diff = if verbose {
        Some(compute_diff(path, &content, formatted.as_str()))
    } else {
        None
    };

    if !check {
        fs::write(path, formatted.as_str())?;
    }

    Ok(FormatResult::Changed { diff })
}

/// Try AST-based formatting via PrettyPrinter.
fn try_ast_format(content: &str, config: &FormatterConfig) -> Option<Text> {
    let file_id = FileId::new(1);
    let lexer = Lexer::new(content, file_id);
    let parser = VerumParser::new();

    let module = parser.parse_module(lexer, file_id).ok()?;

    let pretty_config = PrettyConfig::from(config);
    let mut printer = PrettyPrinter::new(pretty_config);
    let formatted: Text = printer.format_module(&module).into();

    // Sanity check: formatted output should be non-empty and re-parseable
    if formatted.is_empty() {
        return None;
    }

    // Apply post-processing: import sorting, blank line normalization
    let mut result = formatted.to_string();

    if config.sort_imports {
        result = sort_imports(&result);
    }

    if config.normalize_blanks {
        result = normalize_blank_lines(&result);
    }

    if config.trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    Some(Text::from(result.as_str()))
}

/// Fallback: normalize whitespace and sort imports.
fn normalize_and_sort(content: &str, config: &FormatterConfig) -> Text {
    let mut result = String::new();
    let mut prev_empty = false;

    for line in content.lines() {
        let trimmed = line.trim_end();

        if trimmed.is_empty() {
            if !prev_empty && config.normalize_blanks {
                result.push('\n');
                prev_empty = true;
            } else if !config.normalize_blanks {
                result.push('\n');
            }
        } else {
            // Normalize indentation
            let leading = line.len() - line.trim_start().len();
            let indent_level = leading / config.indent_size;
            let indent_str = if config.use_spaces {
                " ".repeat(indent_level * config.indent_size)
            } else {
                "\t".repeat(indent_level)
            };

            result.push_str(&indent_str);
            result.push_str(trimmed);
            result.push('\n');
            prev_empty = false;
        }
    }

    // Normalize trailing newlines
    while result.ends_with("\n\n") {
        result.pop();
    }
    if config.trailing_newline && !result.ends_with('\n') {
        result.push('\n');
    }

    if config.sort_imports {
        result = sort_imports(&result);
    }

    Text::from(result.as_str())
}

/// Sort `mount` import statements alphabetically within contiguous groups.
fn sort_imports(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut result = Vec::new();
    let mut import_group: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("mount ") {
            import_group.push(line);
        } else {
            if !import_group.is_empty() {
                import_group.sort_unstable();
                result.extend(import_group.drain(..));
            }
            result.push(line);
        }
        i += 1;
    }

    // Flush remaining imports
    if !import_group.is_empty() {
        import_group.sort_unstable();
        result.extend(import_group.drain(..));
    }

    let mut out = result.join("\n");
    if content.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Remove consecutive blank lines (keep max 1).
fn normalize_blank_lines(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut prev_empty = false;

    for line in content.lines() {
        if line.trim().is_empty() {
            if !prev_empty {
                result.push('\n');
                prev_empty = true;
            }
        } else {
            result.push_str(line);
            result.push('\n');
            prev_empty = false;
        }
    }

    // Ensure single trailing newline
    while result.ends_with("\n\n") {
        result.pop();
    }
    if !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

/// Compute a simple unified diff for display.
fn compute_diff(path: &Path, original: &str, formatted: &str) -> String {
    let mut diff = String::new();
    diff.push_str(&format!("--- {}\n+++ {}\n", path.display(), path.display()));

    let orig_lines: Vec<&str> = original.lines().collect();
    let fmt_lines: Vec<&str> = formatted.lines().collect();

    let max_len = orig_lines.len().max(fmt_lines.len());
    let mut in_hunk = false;

    for i in 0..max_len {
        let orig = orig_lines.get(i).copied().unwrap_or("");
        let fmt = fmt_lines.get(i).copied().unwrap_or("");

        if orig != fmt {
            if !in_hunk {
                diff.push_str(&format!("@@ line {} @@\n", i + 1));
                in_hunk = true;
            }
            if i < orig_lines.len() {
                diff.push_str(&format!("-{}\n", orig));
            }
            if i < fmt_lines.len() {
                diff.push_str(&format!("+{}\n", fmt));
            }
        } else {
            in_hunk = false;
        }
    }

    diff
}

/// Format specific path (for workspace support).
pub fn format_path(path: &Path, check: bool, verbose: bool) -> Result<()> {
    if !path.exists() {
        return Err(CliError::Custom(format!("Path not found: {}", path.display())));
    }

    let config = load_config();

    if path.is_file() {
        if is_verum_file(path) {
            match format_file(path, &config, check, verbose)? {
                FormatResult::Unchanged => ui::info(&format!("{} already formatted", path.display())),
                FormatResult::Changed { .. } => {
                    if check {
                        println!("{} needs formatting", path.display());
                    } else {
                        ui::success(&format!("Formatted {}", path.display()));
                    }
                }
            }
        } else {
            return Err(CliError::Custom(format!("Not a Verum file: {}", path.display())));
        }
    } else if path.is_dir() {
        let mut formatted = 0;
        let mut errors = 0;
        for entry in WalkDir::new(path).follow_links(false).into_iter().filter_map(|e| e.ok()) {
            let file_path = entry.path();
            if file_path.is_file() && is_verum_file(file_path) {
                match format_file(file_path, &config, check, verbose) {
                    Ok(FormatResult::Changed { .. }) => formatted += 1,
                    Ok(FormatResult::Unchanged) => {}
                    Err(_) => errors += 1,
                }
            }
        }
        if check {
            if formatted > 0 { ui::error(&format!("{} files need formatting", formatted)); }
            else { ui::success("All files formatted correctly"); }
        } else {
            ui::success(&format!("Formatted {} files ({} errors)", formatted, errors));
        }
    }
    Ok(())
}

/// Format a string directly (for testing and API use).
pub fn format_string(source: &str) -> Result<Text> {
    let config = load_config();
    match try_ast_format(source, &config) {
        Some(formatted) => Ok(formatted),
        None => Ok(normalize_and_sort(source, &config)),
    }
}

/// Read source from stdin, format it, write to stdout. The
/// canonical editor format-on-save protocol — every modern formatter
/// (rustfmt --emit stdout, gofmt, prettier --stdin-filepath, ruff
/// format -) implements this so an LSP / editor extension can pipe
/// the buffer through without a temp-file dance.
///
/// `filename_hint` is used for diagnostics and future
/// project-config resolution; the file at that path is NOT read.
pub fn execute_stdin(filename_hint: Option<String>) -> Result<()> {
    use std::io::{Read, Write};

    let mut source = String::new();
    std::io::stdin()
        .read_to_string(&mut source)
        .map_err(|e| CliError::Custom(format!("read stdin: {e}")))?;

    let config = load_config();
    let formatted: Text = match try_ast_format(&source, &config) {
        Some(t) => t,
        None => {
            // Parse failed — surface a stderr warning so the editor
            // can show it without polluting the formatted-output
            // pipe to stdout. We still emit the whitespace-fallback
            // form to stdout so the editor's "save" doesn't leave
            // the buffer empty.
            let hint = filename_hint
                .as_deref()
                .map(|p| format!(" ({})", p))
                .unwrap_or_default();
            eprintln!(
                "warning: parse failed{hint}; emitting whitespace-normalised form"
            );
            normalize_and_sort(&source, &config)
        }
    };

    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    handle
        .write_all(formatted.as_str().as_bytes())
        .map_err(|e| CliError::Custom(format!("write stdout: {e}")))?;
    handle
        .flush()
        .map_err(|e| CliError::Custom(format!("flush stdout: {e}")))?;
    Ok(())
}
