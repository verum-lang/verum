// UI utilities for the Verum CLI
// Provides Cargo-style status messages, progress bars, spinners, colored output,
// and user-friendly formatting that reflects Verum's philosophy of semantic honesty
// and cost transparency.
//
// Visibility levels (from most to least visible):
//   - error()    — always shown (stderr)
//   - warn()     — shown unless --quiet
//   - status()   — shown unless --quiet (Cargo-style "   Compiling ...")
//   - success()  — shown unless --quiet
//   - step()     — shown unless --quiet
//   - note()     — shown unless --quiet (dimmed supplementary info)
//   - detail()   — shown unless --quiet (key-value pair)
//   - output()   — shown unless --quiet (raw output)
//   - info()     — verbose only
//   - section()  — verbose only
//   - header()   — verbose only
//   - build_header() — verbose only (branded box)
//   - debug()    — verbose + debug build only

use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use verum_common::{List, Text};

// ---------------------------------------------------------------------------
// Global UI state
// ---------------------------------------------------------------------------

static UI_STATE: Mutex<Option<UiState>> = Mutex::new(None);

struct UiState {
    verbose: bool,
    quiet: bool,
    #[allow(dead_code)]
    colors_enabled: bool,
}

pub fn init(verbose: bool, quiet: bool, color: &str) -> crate::error::Result<()> {
    let colors_enabled = match color {
        "always" => true,
        "never" => false,
        "auto" | _ => is_terminal::is_terminal(std::io::stdout()),
    };

    colored::control::set_override(colors_enabled);

    // Initialise the tracing subscriber so that compiler-emitted
    // `tracing::warn!` / `error!` lines are actually shown to the
    // user. Prior behaviour: warn! calls were no-ops unless the user
    // set VERUM_LOG=warn manually — real problems like the
    // "Conflicting export" phantom duplicates silently vanished.
    //
    // Level selection:
    //   * quiet   → error only
    //   * verbose → debug (everything except trace)
    //   * default → warn  (errors + warnings; no info chatter)
    //
    // VERUM_LOG env var overrides if set — same semantics as
    // RUST_LOG, forwarded through EnvFilter.
    let default_level = if quiet {
        "error"
    } else if verbose {
        "debug"
    } else {
        "warn"
    };
    let filter = std::env::var("VERUM_LOG").unwrap_or_else(|_| default_level.to_string());
    // Best-effort — if the subscriber is already installed (e.g. by
    // the LSP startup path, tests, or a host embedding us), silently
    // ignore.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();

    let mut state = UI_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *state = Some(UiState {
        verbose,
        quiet,
        colors_enabled,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// State queries
// ---------------------------------------------------------------------------

/// Check if quiet mode is active.
pub fn is_quiet() -> bool {
    let state = UI_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.as_ref().map_or(false, |ui| ui.quiet)
}

/// Check if verbose mode is active.
pub fn is_verbose() -> bool {
    let state = UI_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.as_ref().map_or(false, |ui| ui.verbose)
}

/// Get verbose level as u8 for interpreter/codegen integration.
/// Returns: 0 = quiet, 1 = normal, 2 = verbose (debug output enabled).
/// Note: Level 2 is only available in debug builds.
pub fn verbose_level() -> u8 {
    let state = UI_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(ref ui) = *state {
        if ui.quiet {
            0
        } else if ui.verbose {
            #[cfg(debug_assertions)]
            {
                2
            }
            #[cfg(not(debug_assertions))]
            {
                1
            }
        } else {
            1
        }
    } else {
        1 // Default to normal
    }
}

// ---------------------------------------------------------------------------
// Helpers (private)
// ---------------------------------------------------------------------------

fn shown_by_default() -> bool {
    let state = UI_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.as_ref().map_or(true, |ui| !ui.quiet)
}

fn shown_verbose() -> bool {
    let state = UI_STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    state.as_ref().map_or(false, |ui| ui.verbose && !ui.quiet)
}

// ---------------------------------------------------------------------------
// Core message functions
// ---------------------------------------------------------------------------

/// Cargo-style status line, shown by default.
///
/// ```text
///    Compiling main.vr (AOT, release)
///     Checking 42 contracts...
///     Linking target/debug/my_project
///    Finished release in 1.32s (CBGR: 87% checks eliminated)
/// ```
///
/// The verb is right-aligned to 12 characters, green bold. Message in default color.
pub fn status(verb: &str, message: &str) {
    if shown_by_default() {
        println!("{:>12} {}", verb.green().bold(), message);
    }
}

/// Success message — shown by default.
/// Displays as a Cargo-style status with the verb "Finished".
pub fn success(msg: &str) {
    if shown_by_default() {
        println!("{:>12} {}", "Finished".green().bold(), msg);
    }
}

/// Error message — always shown (stderr).
pub fn error(msg: &str) {
    eprintln!("{}{} {}", "error".red().bold(), ":".bold(), msg);
}

/// Warning message — shown unless quiet.
pub fn warn(msg: &str) {
    if shown_by_default() {
        println!("{}{} {}", "warning".yellow().bold(), ":".bold(), msg);
    }
}

/// Compilation step message — shown by default.
/// Displays as a right-aligned arrow for phase progression.
pub fn step(msg: &str) {
    if shown_by_default() {
        println!("{:>12} {}", "-->".cyan().bold(), msg);
    }
}

/// Info message — verbose only.
pub fn info(msg: &str) {
    if shown_verbose() {
        println!("{:>12} {}", "info".blue().bold(), msg);
    }
}

/// Debug message — verbose + debug builds only.
pub fn debug(msg: &str) {
    #[cfg(debug_assertions)]
    {
        if shown_verbose() {
            println!("{:>12} {}", "debug".cyan(), msg);
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = msg;
    }
}

/// Note — shown by default, dimmed. For supplementary information.
pub fn note(msg: &str) {
    if shown_by_default() {
        println!("{:>12} {}", "note".dimmed().bold(), msg.dimmed());
    }
}

/// Detail — key-value output shown by default.
///
/// ```text
///       Binary target/debug/main (4.2 MB)
///         CBGR 87% checks eliminated (234/269)
/// ```
pub fn detail(key: &str, value: &str) {
    if shown_by_default() {
        println!("{:>12} {}", key.bold(), value);
    }
}

/// Raw output — shown unless quiet. For tables, lists, free-form text.
pub fn output(msg: &str) {
    if shown_by_default() {
        println!("{}", msg);
    }
}

// ---------------------------------------------------------------------------
// Verum-branded header (verbose only)
// ---------------------------------------------------------------------------

/// Print a branded build header in a Unicode box (verbose only).
///
/// ```text
///   ╔══════════════════════════════════════╗
///   ║  verum build · AOT (LLVM 21)        ║
///   ╚══════════════════════════════════════╝
/// ```
pub fn build_header(command: &str, detail_text: &str) {
    if !shown_verbose() {
        return;
    }
    let inner = format!("  {} \u{00B7} {}  ", command, detail_text);
    let width = inner.len().max(38);
    let padded = format!("{:<width$}", inner, width = width);

    println!();
    println!(
        "  {}{}{}",
        "\u{2554}".cyan(),
        "\u{2550}".repeat(width).cyan(),
        "\u{2557}".cyan()
    );
    println!(
        "  {}{}{}",
        "\u{2551}".cyan(),
        padded.white().bold(),
        "\u{2551}".cyan()
    );
    println!(
        "  {}{}{}",
        "\u{255A}".cyan(),
        "\u{2550}".repeat(width).cyan(),
        "\u{255D}".cyan()
    );
}

/// Print a prominent header (verbose only).
pub fn header(msg: &str) {
    if shown_verbose() {
        println!();
        println!("{}", "=".repeat(60).cyan());
        println!("{}", msg.cyan().bold());
        println!("{}", "=".repeat(60).cyan());
    }
}

/// Print a section header (verbose only).
pub fn section(msg: &str) {
    if shown_verbose() {
        println!();
        println!("{}", msg.yellow().bold());
        println!("{}", "-".repeat(40).yellow());
    }
}

// ---------------------------------------------------------------------------
// Build summary
// ---------------------------------------------------------------------------

/// CBGR statistics for build summary.
pub struct CbgrStats {
    pub checks_eliminated: u64,
    pub checks_total: u64,
}

impl CbgrStats {
    pub fn elimination_pct(&self) -> f64 {
        if self.checks_total == 0 {
            0.0
        } else {
            (self.checks_eliminated as f64 / self.checks_total as f64) * 100.0
        }
    }
}

/// Print a compilation summary block.
///
/// ```text
///    Finished release [optimized] target(s) in 1.32s
///      Binary target/debug/my_project (4.2 MB)
///        CBGR 87% checks eliminated (234/269)
/// ```
pub fn print_build_summary(
    duration: Duration,
    binary_path: Option<&Path>,
    binary_size: Option<u64>,
    cbgr: Option<&CbgrStats>,
    warnings: usize,
    errors: usize,
) {
    if !shown_by_default() {
        return;
    }

    if errors > 0 {
        // Don't print success summary when there are errors.
        return;
    }

    let profile = "release [optimized]";
    status(
        "Finished",
        &format!("{} target(s) in {}", profile, format_duration(duration)),
    );

    if let Some(path) = binary_path {
        let size_str = binary_size
            .map(|s| format!(" ({})", format_size(s)))
            .unwrap_or_default();
        detail("Binary", &format!("{}{}", path.display(), size_str));
    }

    if let Some(stats) = cbgr {
        if stats.checks_total > 0 {
            detail(
                "CBGR",
                &format!(
                    "{:.0}% checks eliminated ({}/{})",
                    stats.elimination_pct(),
                    stats.checks_eliminated,
                    stats.checks_total,
                ),
            );
        }
    }

    if warnings > 0 {
        println!(
            "{}{} {} warning{} emitted",
            "warning".yellow().bold(),
            ":".bold(),
            warnings,
            if warnings == 1 { "" } else { "s" },
        );
    }
}

// ---------------------------------------------------------------------------
// Diagnostic summary
// ---------------------------------------------------------------------------

/// Print a diagnostic summary line.
///
/// On failure:
/// ```text
/// error: could not compile `my_project` due to 3 previous errors; 2 warnings emitted
/// ```
///
/// On success with warnings:
/// ```text
/// warning: 2 warnings emitted
/// ```
pub fn print_diagnostic_summary(project_name: &str, errors: usize, warnings: usize) {
    if !shown_by_default() {
        return;
    }

    if errors > 0 {
        let warn_part = if warnings > 0 {
            format!(
                "; {} warning{} emitted",
                warnings,
                if warnings == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        eprintln!(
            "{}{} could not compile `{}` due to {} previous error{}{}",
            "error".red().bold(),
            ":".bold(),
            project_name,
            errors,
            if errors == 1 { "" } else { "s" },
            warn_part,
        );
    } else if warnings > 0 {
        println!(
            "{}{} {} warning{} emitted",
            "warning".yellow().bold(),
            ":".bold(),
            warnings,
            if warnings == 1 { "" } else { "s" },
        );
    }
}

// ---------------------------------------------------------------------------
// Table printing (colored)
// ---------------------------------------------------------------------------

/// Print a table with colored headers, dim separator, and alternating row shading.
pub fn print_table(headers: &[&str], rows: &[List<Text>]) {
    if rows.is_empty() {
        return;
    }

    // Calculate column widths.
    let mut widths: List<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    // Header row — bold cyan.
    print!("| ");
    for (i, h) in headers.iter().enumerate() {
        print!("{:width$}", h.bold().cyan(), width = widths[i]);
        if i < headers.len() - 1 {
            print!(" | ");
        }
    }
    println!(" |");

    // Separator — dim gray.
    print!("{}", "|".dimmed());
    for (i, &width) in widths.iter().enumerate() {
        print!("{}", "-".repeat(width + 2).dimmed());
        if i < widths.len() - 1 {
            print!("{}", "+".dimmed());
        }
    }
    println!("{}", "|".dimmed());

    // Data rows — alternate dim/normal.
    for (row_idx, row) in rows.iter().enumerate() {
        print!("| ");
        let is_alt = row_idx % 2 == 1;
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                if is_alt {
                    print!("{:width$}", cell.dimmed(), width = widths[i]);
                } else {
                    print!("{:width$}", cell, width = widths[i]);
                }
                if i < row.len() - 1 {
                    print!(" | ");
                }
            }
        }
        println!(" |");
    }
}

// ---------------------------------------------------------------------------
// Tree printing (colored)
// ---------------------------------------------------------------------------

/// Item color hint for tree nodes.
pub enum TreeColor {
    Default,
    Green,
    Yellow,
    Red,
    Cyan,
    Dimmed,
}

/// Print a tree with colored branches.
///
/// Each item is `(depth, text)`. Root nodes (depth 0) are bold white,
/// branch glyphs are dim gray.
pub fn print_tree(items: &[(usize, Text)]) {
    print_tree_colored(
        &items
            .iter()
            .map(|(d, t)| (*d, t.clone(), TreeColor::Default))
            .collect::<Vec<_>>(),
    );
}

/// Print a tree with explicit per-item color hints.
pub fn print_tree_colored(items: &[(usize, Text, TreeColor)]) {
    let len = items.len();
    for (idx, (depth, item, color)) in items.iter().enumerate() {
        if *depth == 0 {
            // Root node — bold white.
            println!("{}", item.bold().white());
        } else {
            // Determine if this is the last sibling at this depth level by
            // checking whether any following item exists at the same depth
            // before we return to a shallower depth.
            let is_last = items[idx + 1..len]
                .iter()
                .take_while(|(d, _, _)| *d >= *depth)
                .all(|(d, _, _)| *d > *depth);

            let connector = if is_last {
                "\u{2514}\u{2500}\u{2500} " // └──
            } else {
                "\u{251C}\u{2500}\u{2500} " // ├──
            };

            let indent = "    ".repeat(depth - 1);
            let branch = format!("{}{}", indent, connector).dimmed();

            let colored_item = match color {
                TreeColor::Default => item.normal(),
                TreeColor::Green => item.green(),
                TreeColor::Yellow => item.yellow(),
                TreeColor::Red => item.red(),
                TreeColor::Cyan => item.cyan(),
                TreeColor::Dimmed => item.dimmed(),
            };
            println!("{}{}", branch, colored_item);
        }
    }
}

// ---------------------------------------------------------------------------
// Progress reporting
// ---------------------------------------------------------------------------

pub struct ProgressReporter {
    bar: ProgressBar,
}

impl ProgressReporter {
    pub fn new(total: u64, msg: &str) -> Self {
        let bar = ProgressBar::new(total);
        bar.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                )
                .unwrap()
                .progress_chars("#>-"),
        );
        bar.set_message(msg.to_string());
        Self { bar }
    }

    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta);
    }

    pub fn set_message(&self, msg: &str) {
        self.bar.set_message(msg.to_string());
    }

    pub fn finish(&self) {
        self.bar.finish_with_message("Done");
    }

    pub fn finish_with_message(&self, msg: &str) {
        self.bar.finish_with_message(msg.to_string());
    }
}

pub struct Spinner {
    bar: ProgressBar,
}

impl Spinner {
    pub fn new(msg: &str) -> Self {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap(),
        );
        bar.set_message(msg.to_string());
        bar.enable_steady_tick(Duration::from_millis(100));
        Self { bar }
    }

    pub fn set_message(&self, msg: &str) {
        self.bar.set_message(msg.to_string());
    }

    pub fn finish(&self) {
        self.bar.finish_and_clear();
    }

    pub fn finish_with_message(&self, msg: &str) {
        self.bar.finish_with_message(msg.to_string());
    }
}

// ---------------------------------------------------------------------------
// Interactive prompts
// ---------------------------------------------------------------------------

pub fn confirm(prompt: &str) -> bool {
    use std::io::{self, Write};

    print!("{} [y/N]: ", prompt.yellow());
    let _ = io::stdout().flush();

    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);

    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

pub fn select(prompt: &str, options: &[&str]) -> Option<usize> {
    use std::io::{self, Write};

    println!("{}", prompt.cyan().bold());
    for (i, option) in options.iter().enumerate() {
        println!("  {} {}", format!("[{}]", i + 1).green(), option);
    }

    print!("\nSelect (1-{}): ", options.len());
    let _ = io::stdout().flush();

    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);

    input.trim().parse::<usize>().ok().and_then(|n| {
        if n > 0 && n <= options.len() {
            Some(n - 1)
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Formatting utilities
// ---------------------------------------------------------------------------

pub fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{:.2}s", duration.as_secs_f64())
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes < KB {
        format!("{} B", bytes)
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}
