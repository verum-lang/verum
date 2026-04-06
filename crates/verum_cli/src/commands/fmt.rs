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

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_fmt_section = trimmed == "[fmt]" || trimmed == "[format]";
            continue;
        }
        if !in_fmt_section { continue; }

        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "max_width" => { if let Ok(v) = value.parse() { config.max_width = v; } }
                "indent_size" => { if let Ok(v) = value.parse() { config.indent_size = v; } }
                "use_spaces" => { config.use_spaces = value == "true"; }
                "trailing_comma" => { config.trailing_comma = value == "true"; }
                "sort_imports" => { config.sort_imports = value == "true"; }
                _ => {}
            }
        }
    }
    config
}

/// Main entry point for `verum fmt`.
pub fn execute(check: bool, verbose: bool) -> Result<()> {
    if check {
        ui::step("Checking formatting");
    } else {
        ui::step("Formatting source files");
    }

    let config = load_config();
    let mut total_files = 0;
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

    for search_dir in &search_dirs {
        for entry in WalkDir::new(search_dir)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && is_verum_file(path) {
                total_files += 1;

                match format_file(path, &config, check, verbose) {
                    Ok(FormatResult::Unchanged) => {}
                    Ok(FormatResult::Changed { diff }) => {
                        formatted_files += 1;
                        changed_files.push(path.to_path_buf());
                        if verbose {
                            if let Some(d) = &diff {
                                println!("{}", d);
                            }
                            ui::info(&format!("Formatted: {}", path.display()));
                        }
                    }
                    Err(e) => {
                        error_files.push((path.to_path_buf(), e.to_string()));
                        if verbose {
                            ui::error(&format!("Failed: {}: {}", path.display(), e));
                        }
                    }
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

    // Try AST-based formatting first
    let formatted = match try_ast_format(&content, config) {
        Some(f) => f,
        None => {
            // Fallback: whitespace normalization + import sorting
            if verbose {
                ui::debug(&format!("Parse failed, using fallback: {}", path.display()));
            }
            normalize_and_sort(&content, config)
        }
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
