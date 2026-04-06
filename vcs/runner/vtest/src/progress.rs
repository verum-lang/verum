//! Progress reporting for VCS test runner.
//!
//! Provides real-time progress indicators for test execution:
//!
//! - **Simple**: Just dots and test counts
//! - **Bar**: ASCII/ANSI progress bar with percentage
//! - **Verbose**: Full test names and outcomes
//! - **Quiet**: Minimal output (errors only)
//! - **Streaming**: Real-time output for CI/CD
//!
//! # Progress Bar Formats
//!
//! ```text
//! Simple:  ...........F....S..
//! Bar:     [====================      ] 75% (150/200) [2 failed]
//! Verbose: PASS test_ownership_move [12ms]
//!          FAIL test_borrow_checker: Expected E302
//! ```

use crate::executor::{TestOutcome, TestResult};
use colored::Colorize;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use verum_common::Text;

/// Progress output style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressStyle {
    /// Simple dots/characters for each test
    Simple,
    /// ANSI progress bar with percentage
    #[default]
    Bar,
    /// Full verbose output with test names
    Verbose,
    /// Minimal output (errors only)
    Quiet,
    /// Streaming output for CI (no cursor control)
    Streaming,
    /// No output (for programmatic use)
    Silent,
}

impl ProgressStyle {
    /// Parse from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "simple" | "dots" => Some(Self::Simple),
            "bar" | "progress" => Some(Self::Bar),
            "verbose" | "v" => Some(Self::Verbose),
            "quiet" | "q" => Some(Self::Quiet),
            "streaming" | "stream" | "ci" => Some(Self::Streaming),
            "silent" | "none" => Some(Self::Silent),
            _ => None,
        }
    }

    /// Check if this style supports ANSI escape codes.
    pub fn supports_ansi(&self) -> bool {
        matches!(self, Self::Bar | Self::Verbose)
    }
}

/// Configuration for progress reporting.
#[derive(Debug, Clone)]
pub struct ProgressConfig {
    /// Output style
    pub style: ProgressStyle,
    /// Progress bar width (for Bar style)
    pub bar_width: usize,
    /// Whether to use colors
    pub use_colors: bool,
    /// Whether terminal supports ANSI
    pub ansi_support: bool,
    /// Update interval in milliseconds
    pub update_interval_ms: u64,
    /// Show timing information
    pub show_timing: bool,
    /// Show memory usage (if available)
    pub show_memory: bool,
    /// Maximum test name length before truncation
    pub max_name_length: usize,
}

impl Default for ProgressConfig {
    fn default() -> Self {
        Self {
            style: ProgressStyle::Bar,
            bar_width: 40,
            use_colors: true,
            ansi_support: Self::detect_ansi_support(),
            update_interval_ms: 100,
            show_timing: true,
            show_memory: false,
            max_name_length: 50,
        }
    }
}

impl ProgressConfig {
    /// Detect if terminal supports ANSI escape codes.
    fn detect_ansi_support() -> bool {
        // Check if we're in a TTY
        if !atty::is(atty::Stream::Stdout) {
            return false;
        }

        // Check common environment variables
        if std::env::var("NO_COLOR").is_ok() {
            return false;
        }

        if std::env::var("CI").is_ok() {
            // Many CI systems support ANSI
            return std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
        }

        std::env::var("TERM").map(|t| t != "dumb").unwrap_or(false)
    }

    /// Create a config for CI environments.
    pub fn for_ci() -> Self {
        Self {
            style: ProgressStyle::Streaming,
            use_colors: std::env::var("NO_COLOR").is_err(),
            ansi_support: false,
            ..Default::default()
        }
    }

    /// Create a config for quiet mode.
    pub fn quiet() -> Self {
        Self {
            style: ProgressStyle::Quiet,
            ..Default::default()
        }
    }

    /// Create a config for verbose mode.
    pub fn verbose() -> Self {
        Self {
            style: ProgressStyle::Verbose,
            ..Default::default()
        }
    }
}

/// Progress state for tracking test execution.
#[derive(Debug)]
pub struct ProgressState {
    /// Total number of tests to run
    pub total: AtomicUsize,
    /// Number of tests completed
    pub completed: AtomicUsize,
    /// Number of passed tests
    pub passed: AtomicUsize,
    /// Number of failed tests
    pub failed: AtomicUsize,
    /// Number of skipped tests
    pub skipped: AtomicUsize,
    /// Whether progress is active
    pub active: AtomicBool,
    /// Start time
    pub start_time: Instant,
    /// Last update time
    last_update: std::sync::Mutex<Instant>,
    /// Current test name
    current_test: std::sync::Mutex<Option<Text>>,
}

impl ProgressState {
    /// Create new progress state.
    pub fn new(total: usize) -> Self {
        Self {
            total: AtomicUsize::new(total),
            completed: AtomicUsize::new(0),
            passed: AtomicUsize::new(0),
            failed: AtomicUsize::new(0),
            skipped: AtomicUsize::new(0),
            active: AtomicBool::new(false),
            start_time: Instant::now(),
            last_update: std::sync::Mutex::new(Instant::now()),
            current_test: std::sync::Mutex::new(None),
        }
    }

    /// Record a test completion.
    pub fn record_result(&self, outcome: &TestOutcome) {
        self.completed.fetch_add(1, Ordering::SeqCst);
        match outcome {
            TestOutcome::Pass { .. } => {
                self.passed.fetch_add(1, Ordering::SeqCst);
            }
            TestOutcome::Fail { .. } => {
                self.failed.fetch_add(1, Ordering::SeqCst);
            }
            TestOutcome::Skip { .. } => {
                self.skipped.fetch_add(1, Ordering::SeqCst);
            }
            TestOutcome::Error { .. } => {
                self.failed.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    /// Set current test name.
    pub fn set_current_test(&self, name: Option<Text>) {
        *self.current_test.lock().unwrap() = name;
    }

    /// Get current test name.
    pub fn current_test(&self) -> Option<Text> {
        self.current_test.lock().unwrap().clone()
    }

    /// Get completion percentage.
    pub fn percentage(&self) -> f64 {
        let total = self.total.load(Ordering::SeqCst);
        if total == 0 {
            return 100.0;
        }
        let completed = self.completed.load(Ordering::SeqCst);
        (completed as f64 / total as f64) * 100.0
    }

    /// Get elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Check if update is needed (based on interval).
    pub fn needs_update(&self, interval_ms: u64) -> bool {
        let mut last = self.last_update.lock().unwrap();
        let now = Instant::now();
        if now.duration_since(*last).as_millis() as u64 >= interval_ms {
            *last = now;
            true
        } else {
            false
        }
    }
}

/// Progress reporter for real-time test execution feedback.
pub struct ProgressReporter {
    config: ProgressConfig,
    state: Arc<ProgressState>,
    /// Buffer for output
    output_buffer: std::sync::Mutex<Vec<u8>>,
}

impl ProgressReporter {
    /// Create a new progress reporter.
    pub fn new(config: ProgressConfig, total: usize) -> Self {
        Self {
            config,
            state: Arc::new(ProgressState::new(total)),
            output_buffer: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Get shared state reference.
    pub fn state(&self) -> Arc<ProgressState> {
        Arc::clone(&self.state)
    }

    /// Start progress reporting.
    pub fn start(&self) {
        self.state.active.store(true, Ordering::SeqCst);

        match self.config.style {
            ProgressStyle::Bar => {
                self.print_bar_header();
            }
            ProgressStyle::Verbose => {
                self.print_verbose_header();
            }
            ProgressStyle::Streaming => {
                self.print_streaming_header();
            }
            _ => {}
        }
    }

    /// Stop progress reporting.
    pub fn stop(&self) {
        self.state.active.store(false, Ordering::SeqCst);

        match self.config.style {
            ProgressStyle::Bar => {
                // Clear the progress line
                if self.config.ansi_support {
                    print!("\r\x1b[K");
                }
                println!();
            }
            ProgressStyle::Simple => {
                println!();
            }
            _ => {}
        }
    }

    /// Report a test starting.
    pub fn test_starting(&self, name: &str) {
        self.state.set_current_test(Some(name.to_string().into()));

        match self.config.style {
            ProgressStyle::Verbose => {
                print!("  {} {}", "RUNNING".yellow(), self.truncate_name(name));
                let _ = io::stdout().flush();
            }
            ProgressStyle::Streaming => {
                println!("[START] {}", name);
            }
            _ => {}
        }
    }

    /// Report a test result.
    pub fn test_completed(&self, result: &TestResult) {
        for outcome in &result.outcomes {
            self.state.record_result(outcome);
        }

        match self.config.style {
            ProgressStyle::Simple => {
                self.print_simple_result(result);
            }
            ProgressStyle::Bar => {
                if self.state.needs_update(self.config.update_interval_ms) {
                    self.print_progress_bar();
                }
            }
            ProgressStyle::Verbose => {
                self.print_verbose_result(result);
            }
            ProgressStyle::Streaming => {
                self.print_streaming_result(result);
            }
            ProgressStyle::Quiet => {
                // Only print failures
                if !result.all_pass() {
                    self.print_verbose_result(result);
                }
            }
            ProgressStyle::Silent => {}
        }
    }

    /// Force update the display.
    pub fn update(&self) {
        if self.config.style == ProgressStyle::Bar {
            self.print_progress_bar();
        }
    }

    // Private helper methods

    fn truncate_name(&self, name: &str) -> String {
        if name.len() <= self.config.max_name_length {
            name.to_string()
        } else {
            format!(
                "...{}",
                &name[name.len() - self.config.max_name_length + 3..]
            )
        }
    }

    fn print_bar_header(&self) {
        println!();
        println!("{}", "=".repeat(60).dimmed());
        println!("  {} Test Suite", "VCS".bold());
        println!("{}", "=".repeat(60).dimmed());
        println!();
    }

    fn print_verbose_header(&self) {
        println!();
        println!("{}", "-".repeat(60).dimmed());
        println!(
            "  Running {} tests...",
            self.state.total.load(Ordering::SeqCst)
        );
        println!("{}", "-".repeat(60).dimmed());
        println!();
    }

    fn print_streaming_header(&self) {
        println!(
            "[VCS] Starting test run: {} tests",
            self.state.total.load(Ordering::SeqCst)
        );
    }

    fn print_simple_result(&self, result: &TestResult) {
        let char = if result.all_pass() {
            if self.config.use_colors {
                ".".green().to_string()
            } else {
                ".".to_string()
            }
        } else if result.skip_count() > 0 {
            if self.config.use_colors {
                "S".yellow().to_string()
            } else {
                "S".to_string()
            }
        } else {
            if self.config.use_colors {
                "F".red().to_string()
            } else {
                "F".to_string()
            }
        };

        print!("{}", char);
        let _ = io::stdout().flush();
    }

    fn print_progress_bar(&self) {
        let total = self.state.total.load(Ordering::SeqCst);
        let completed = self.state.completed.load(Ordering::SeqCst);
        let _passed = self.state.passed.load(Ordering::SeqCst);
        let failed = self.state.failed.load(Ordering::SeqCst);
        let percentage = self.state.percentage();

        // Calculate bar fill
        let fill_width = ((percentage / 100.0) * self.config.bar_width as f64) as usize;
        let empty_width = self.config.bar_width.saturating_sub(fill_width);

        // Build the bar
        let filled = "=".repeat(fill_width);
        let empty = " ".repeat(empty_width);

        // Build status suffix
        let status = if failed > 0 {
            format!(" [{} failed]", failed).red().to_string()
        } else {
            String::new()
        };

        // Format timing
        let elapsed = self.state.elapsed();
        let timing = if self.config.show_timing {
            format!(" [{:.1}s]", elapsed.as_secs_f64())
        } else {
            String::new()
        };

        // Current test indicator
        let current = self.state.current_test();
        let current_str = current
            .map(|n| format!(" {}", self.truncate_name(&n)))
            .unwrap_or_default();

        // Print with ANSI cursor control if supported
        if self.config.ansi_support {
            print!(
                "\r\x1b[K  [{}{}] {:>3.0}% ({}/{}){}{}{}",
                filled.green(),
                empty.dimmed(),
                percentage,
                completed,
                total,
                status,
                timing.dimmed(),
                current_str.dimmed()
            );
        } else {
            print!(
                "\r  [{}{}] {:>3.0}% ({}/{}){}{}",
                filled,
                empty,
                percentage,
                completed,
                total,
                if failed > 0 {
                    format!(" [{} failed]", failed)
                } else {
                    String::new()
                },
                timing
            );
        }
        let _ = io::stdout().flush();
    }

    fn print_verbose_result(&self, result: &TestResult) {
        let name = result.directives.display_name();

        // Clear the "RUNNING" line if we printed one
        if self.config.ansi_support {
            print!("\r\x1b[K");
        } else {
            print!("\r");
        }

        for outcome in &result.outcomes {
            let (status, style, extra) = match outcome {
                TestOutcome::Pass { duration, tier } => {
                    let time_str = if self.config.show_timing {
                        format!(" [{}ms, tier {}]", duration.as_millis(), *tier as u8)
                    } else {
                        String::new()
                    };
                    ("PASS", "green", time_str)
                }
                TestOutcome::Fail {
                    reason,
                    tier,
                    duration,
                    ..
                } => {
                    let time_str = if self.config.show_timing {
                        format!(" [{}ms, tier {}]", duration.as_millis(), *tier as u8)
                    } else {
                        String::new()
                    };
                    let reason_str = format!("{}: {}", time_str, reason);
                    ("FAIL", "red", reason_str)
                }
                TestOutcome::Skip { reason, tier } => {
                    let reason_str = format!(": {} (tier {})", reason, *tier as u8);
                    ("SKIP", "yellow", reason_str)
                }
                TestOutcome::Error { error, tier } => {
                    let error_str = format!(": {} (tier {})", error, *tier as u8);
                    ("ERROR", "red", error_str)
                }
            };

            let status_str = if self.config.use_colors {
                match style {
                    "green" => format!("{}", status.green()),
                    "red" => format!("{}", status.red()),
                    "yellow" => format!("{}", status.yellow()),
                    _ => status.to_string(),
                }
            } else {
                status.to_string()
            };

            println!("  {} {}{}", status_str, name, extra.dimmed());
        }
    }

    fn print_streaming_result(&self, result: &TestResult) {
        let name = result.directives.display_name();
        let duration = result.total_duration.as_millis();

        for outcome in &result.outcomes {
            let (status, detail) = match outcome {
                TestOutcome::Pass { tier, .. } => ("PASS", format!("tier {}", *tier as u8)),
                TestOutcome::Fail { reason, tier, .. } => {
                    ("FAIL", format!("tier {}: {}", *tier as u8, reason))
                }
                TestOutcome::Skip { reason, tier } => {
                    ("SKIP", format!("tier {}: {}", *tier as u8, reason))
                }
                TestOutcome::Error { error, tier } => {
                    ("ERROR", format!("tier {}: {}", *tier as u8, error))
                }
            };

            println!("[{}] {} [{}ms] {}", status, name, duration, detail);
        }
    }
}

/// Spinner for long-running operations.
pub struct Spinner {
    frames: Vec<&'static str>,
    current: AtomicUsize,
    message: std::sync::Mutex<Text>,
    active: AtomicBool,
}

impl Spinner {
    /// Create a new spinner with default frames.
    pub fn new(message: &str) -> Self {
        Self {
            frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            current: AtomicUsize::new(0),
            message: std::sync::Mutex::new(message.to_string().into()),
            active: AtomicBool::new(false),
        }
    }

    /// Create a spinner with ASCII frames (for non-Unicode terminals).
    pub fn new_ascii(message: &str) -> Self {
        Self {
            frames: vec!["|", "/", "-", "\\"],
            current: AtomicUsize::new(0),
            message: std::sync::Mutex::new(message.to_string().into()),
            active: AtomicBool::new(false),
        }
    }

    /// Update the spinner message.
    pub fn set_message(&self, message: &str) {
        *self.message.lock().unwrap() = message.to_string().into();
    }

    /// Get the next frame and advance.
    pub fn tick(&self) -> &'static str {
        let current = self.current.fetch_add(1, Ordering::SeqCst);
        self.frames[current % self.frames.len()]
    }

    /// Print the current state.
    pub fn print(&self) {
        let frame = self.tick();
        let message = self.message.lock().unwrap();
        print!("\r{} {}", frame.cyan(), message);
        let _ = io::stdout().flush();
    }

    /// Clear the spinner line.
    pub fn clear(&self) {
        print!("\r\x1b[K");
        let _ = io::stdout().flush();
    }

    /// Complete with a success message.
    pub fn success(&self, message: &str) {
        self.clear();
        println!("{} {}", "✓".green(), message);
    }

    /// Complete with a failure message.
    pub fn fail(&self, message: &str) {
        self.clear();
        println!("{} {}", "✗".red(), message);
    }
}

/// Summary bar for final results.
pub struct SummaryBar {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total: usize,
    pub duration: Duration,
}

impl SummaryBar {
    /// Create from test results.
    pub fn from_results(results: &[TestResult]) -> Self {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut duration = Duration::ZERO;

        for result in results {
            passed += result.pass_count();
            failed += result.fail_count();
            skipped += result.skip_count();
            duration += result.total_duration;
        }

        Self {
            passed,
            failed,
            skipped,
            total: results.len(),
            duration,
        }
    }

    /// Print the summary bar.
    pub fn print(&self, use_colors: bool) {
        println!();
        println!("{}", "═".repeat(60));

        // Status line
        let status = if self.failed > 0 {
            if use_colors {
                "FAILED".red().bold().to_string()
            } else {
                "FAILED".to_string()
            }
        } else {
            if use_colors {
                "PASSED".green().bold().to_string()
            } else {
                "PASSED".to_string()
            }
        };

        println!("  Test Result: {}", status);
        println!();

        // Stats line
        let passed_str = if use_colors {
            format!("{} passed", self.passed).green().to_string()
        } else {
            format!("{} passed", self.passed)
        };

        let failed_str = if use_colors {
            if self.failed > 0 {
                format!("{} failed", self.failed).red().to_string()
            } else {
                format!("{} failed", self.failed).to_string()
            }
        } else {
            format!("{} failed", self.failed)
        };

        let skipped_str = if use_colors {
            format!("{} skipped", self.skipped).yellow().to_string()
        } else {
            format!("{} skipped", self.skipped)
        };

        println!(
            "  Tests: {} | {} | {} | {} total",
            passed_str, failed_str, skipped_str, self.total
        );
        println!("  Time:  {:.2}s", self.duration.as_secs_f64());
        println!("{}", "═".repeat(60));
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_style_from_str() {
        assert_eq!(
            ProgressStyle::from_str("simple"),
            Some(ProgressStyle::Simple)
        );
        assert_eq!(ProgressStyle::from_str("bar"), Some(ProgressStyle::Bar));
        assert_eq!(
            ProgressStyle::from_str("verbose"),
            Some(ProgressStyle::Verbose)
        );
        assert_eq!(ProgressStyle::from_str("quiet"), Some(ProgressStyle::Quiet));
        assert_eq!(
            ProgressStyle::from_str("streaming"),
            Some(ProgressStyle::Streaming)
        );
        assert_eq!(ProgressStyle::from_str("invalid"), None);
    }

    #[test]
    fn test_progress_state_percentage() {
        let state = ProgressState::new(100);
        assert_eq!(state.percentage(), 0.0);

        state.completed.store(50, Ordering::SeqCst);
        assert_eq!(state.percentage(), 50.0);

        state.completed.store(100, Ordering::SeqCst);
        assert_eq!(state.percentage(), 100.0);
    }

    #[test]
    fn test_progress_state_empty() {
        let state = ProgressState::new(0);
        assert_eq!(state.percentage(), 100.0);
    }

    #[test]
    fn test_progress_config_default() {
        let config = ProgressConfig::default();
        assert_eq!(config.style, ProgressStyle::Bar);
        assert_eq!(config.bar_width, 40);
        assert!(config.show_timing);
    }

    #[test]
    fn test_progress_config_for_ci() {
        let config = ProgressConfig::for_ci();
        assert_eq!(config.style, ProgressStyle::Streaming);
        assert!(!config.ansi_support);
    }
}
