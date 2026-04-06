//! Watch mode for automatic test re-runs.
//!
//! This module provides file watching capabilities to automatically
//! re-run tests when source files change.
//!
//! # Features
//!
//! - File watching with debouncing to prevent rapid re-runs
//! - Smart affected test detection
//! - Terminal UI with status updates
//! - Cross-platform support via the `notify` crate

use crate::{RunnerConfig, RunnerError, VTestRunner};
use colored::Colorize;
use notify::RecursiveMode;
use notify_debouncer_mini::{DebounceEventResult, new_debouncer};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use verum_common::{List, Set, Text};

/// Event types that trigger test re-runs.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A file was modified
    FileModified(PathBuf),
    /// A file was created
    FileCreated(PathBuf),
    /// A file was deleted
    FileDeleted(PathBuf),
    /// User requested manual re-run
    ManualTrigger,
    /// Stop watching
    Stop,
}

/// Configuration for watch mode.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    /// Paths to watch for changes
    pub watch_paths: List<PathBuf>,
    /// Debounce duration (wait for changes to settle)
    pub debounce_ms: u64,
    /// File extensions to watch
    pub extensions: Set<Text>,
    /// Whether to clear screen before each run
    pub clear_screen: bool,
    /// Whether to run all tests or only affected
    pub run_all: bool,
    /// Notification sound on failure
    pub notify_on_failure: bool,
    /// Show only failures (quiet mode)
    pub quiet: bool,
    /// Poll interval for file checking (for polling watcher)
    pub poll_interval_ms: u64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        let mut extensions = Set::new();
        extensions.insert("vr".to_string().into());
        extensions.insert("verum".to_string().into());

        Self {
            watch_paths: vec![PathBuf::from("specs"), PathBuf::from("src")].into(),
            debounce_ms: 200,
            extensions,
            clear_screen: true,
            run_all: true,
            notify_on_failure: false,
            quiet: false,
            poll_interval_ms: 500,
        }
    }
}

/// Watch mode runner.
pub struct FileWatcher {
    config: WatchConfig,
    runner_config: RunnerConfig,
    last_run: Option<Instant>,
    run_count: u32,
    last_exit_code: i32,
}

impl FileWatcher {
    /// Create a new file watcher.
    pub fn new(watch_config: WatchConfig, runner_config: RunnerConfig) -> Self {
        Self {
            config: watch_config,
            runner_config,
            last_run: None,
            run_count: 0,
            last_exit_code: 0,
        }
    }

    /// Run watch mode (blocking).
    pub async fn run(&mut self) -> Result<(), RunnerError> {
        self.print_header();

        // Initial run
        self.run_tests().await?;

        // Set up file watcher
        let (tx, rx) = channel();
        let debounce_duration = Duration::from_millis(self.config.debounce_ms);

        // Create debounced watcher
        let mut debouncer = new_debouncer(debounce_duration, move |res: DebounceEventResult| {
            if let Ok(events) = res {
                for event in events {
                    let _ = tx.send(event);
                }
            }
        })
        .map_err(|e| RunnerError::ConfigError(format!("Failed to create file watcher: {}", e).into()))?;

        // Watch all configured paths
        for path in &self.config.watch_paths {
            if path.exists() {
                debouncer
                    .watcher()
                    .watch(path, RecursiveMode::Recursive)
                    .map_err(|e| {
                        RunnerError::ConfigError(format!(
                            "Failed to watch {}: {}",
                            path.display(),
                            e
                        ).into())
                    })?;
            }
        }

        self.print_watching_status();

        // Main event loop
        loop {
            // Check for file events (non-blocking with timeout)
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(event) => {
                    if self.should_trigger(&event.path) {
                        self.print_change_detected(&event.path);
                        self.run_tests().await?;
                        self.print_watching_status();
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // No events, continue
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(RunnerError::ConfigError(
                        "File watcher disconnected".to_string().into(),
                    ));
                }
            }
        }
    }

    /// Print the watch mode header.
    fn print_header(&self) {
        println!();
        println!("{}", "=".repeat(60).dimmed());
        println!("  {} {}", "VTEST".bold(), "Watch Mode".dimmed());
        println!("{}", "=".repeat(60).dimmed());
        println!();
        println!("  Watching for changes in:");
        for path in &self.config.watch_paths {
            println!("    {} {}", "-".dimmed(), path.display());
        }
        println!();
        println!(
            "  Extensions: {}",
            self.config
                .extensions
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
                .cyan()
        );
        println!("  Debounce: {}ms", self.config.debounce_ms);
        println!();
        println!("  Press {} to stop.", "Ctrl+C".yellow());
        println!();
    }

    /// Print status when waiting for changes.
    fn print_watching_status(&self) {
        let status = if self.last_exit_code == 0 {
            format!("{} All tests passing", "OK".green().bold())
        } else {
            format!("{} {} failure(s)", "FAIL".red().bold(), self.last_exit_code)
        };

        println!();
        println!("{}", "-".repeat(60).dimmed());
        println!(
            "  {} | Run #{} | {}",
            status,
            self.run_count,
            "Watching for changes...".dimmed()
        );
        println!("{}", "-".repeat(60).dimmed());
    }

    /// Print message when a change is detected.
    fn print_change_detected(&self, path: &PathBuf) {
        println!();
        println!(
            "  {} {}",
            "Change detected:".yellow(),
            path.display().to_string().cyan()
        );
        println!();
    }

    /// Run tests once.
    async fn run_tests(&mut self) -> Result<(), RunnerError> {
        if self.config.clear_screen && self.run_count > 0 {
            // ANSI escape sequence to clear screen
            print!("\x1B[2J\x1B[1;1H");
        }

        self.run_count += 1;
        self.last_run = Some(Instant::now());

        let start = Instant::now();
        let runner = VTestRunner::new(self.runner_config.clone());
        let exit_code = runner.run_and_report().await?;
        let duration = start.elapsed();

        self.last_exit_code = exit_code;

        // Print timing info
        println!();
        println!(
            "  {} Completed in {:.2}s",
            "->".dimmed(),
            duration.as_secs_f64()
        );

        if exit_code != 0 && self.config.notify_on_failure {
            // Bell character for notification
            print!("\x07");
        }

        Ok(())
    }

    /// Check if a file should trigger a re-run.
    fn should_trigger(&self, path: &PathBuf) -> bool {
        if let Some(ext) = path.extension() {
            let ext_str: Text = ext.to_string_lossy().to_string().into();
            self.config.extensions.contains(&ext_str)
        } else {
            false
        }
    }
}

// Keep the old Watcher struct for backward compatibility
/// Watch mode runner (legacy).
#[deprecated(since = "0.2.0", note = "Use FileWatcher instead")]
pub struct Watcher {
    config: WatchConfig,
    runner_config: RunnerConfig,
    last_run: Option<Instant>,
    pending_events: List<WatchEvent>,
}

#[allow(deprecated)]
impl Watcher {
    /// Create a new watcher.
    pub fn new(watch_config: WatchConfig, runner_config: RunnerConfig) -> Self {
        Self {
            config: watch_config,
            runner_config,
            last_run: None,
            pending_events: List::new(),
        }
    }

    /// Run watch mode (blocking).
    pub async fn run(&mut self) -> Result<(), RunnerError> {
        // Delegate to the new FileWatcher
        let mut file_watcher = FileWatcher::new(self.config.clone(), self.runner_config.clone());
        file_watcher.run().await
    }

    /// Check if a file should trigger a re-run.
    pub fn should_trigger(&self, path: &PathBuf) -> bool {
        if let Some(ext) = path.extension() {
            let ext_str: Text = ext.to_string_lossy().to_string().into();
            self.config.extensions.contains(&ext_str)
        } else {
            false
        }
    }
}

/// State for tracking affected tests.
#[derive(Debug, Default)]
pub struct AffectedTests {
    /// Test files that were directly modified
    pub modified_tests: Set<PathBuf>,
    /// Tests that import modified modules
    pub dependent_tests: Set<PathBuf>,
}

impl AffectedTests {
    /// Check if any tests were affected.
    pub fn is_empty(&self) -> bool {
        self.modified_tests.is_empty() && self.dependent_tests.is_empty()
    }

    /// Get all affected test paths.
    pub fn all_paths(&self) -> List<PathBuf> {
        let mut result: List<PathBuf> = self.modified_tests.iter().cloned().collect();
        result.extend(self.dependent_tests.iter().cloned());
        result
    }

    /// Add a modified test file.
    pub fn add_modified(&mut self, path: PathBuf) {
        self.modified_tests.insert(path);
    }

    /// Add a dependent test file.
    pub fn add_dependent(&mut self, path: PathBuf) {
        self.dependent_tests.insert(path);
    }

    /// Clear all tracked tests.
    pub fn clear(&mut self) {
        self.modified_tests.clear();
        self.dependent_tests.clear();
    }
}

/// Simple debouncer for file events.
pub struct Debouncer {
    delay: Duration,
    last_event: Option<Instant>,
    pending: bool,
}

impl Debouncer {
    /// Create a new debouncer with the given delay.
    pub fn new(delay_ms: u64) -> Self {
        Self {
            delay: Duration::from_millis(delay_ms),
            last_event: None,
            pending: false,
        }
    }

    /// Record an event.
    pub fn event(&mut self) {
        self.last_event = Some(Instant::now());
        self.pending = true;
    }

    /// Check if debounce period has passed and we should trigger.
    pub fn should_trigger(&mut self) -> bool {
        if !self.pending {
            return false;
        }

        if let Some(last) = self.last_event {
            if last.elapsed() >= self.delay {
                self.pending = false;
                return true;
            }
        }

        false
    }

    /// Reset the debouncer.
    pub fn reset(&mut self) {
        self.last_event = None;
        self.pending = false;
    }

    /// Get remaining time until trigger (for UI).
    pub fn remaining(&self) -> Option<Duration> {
        if !self.pending {
            return None;
        }

        self.last_event.map(|last| {
            let elapsed = last.elapsed();
            if elapsed >= self.delay {
                Duration::ZERO
            } else {
                self.delay - elapsed
            }
        })
    }
}

/// Terminal UI for watch mode.
pub struct WatchUI {
    /// Current status message
    pub status: Text,
    /// Number of tests run
    pub run_count: u32,
    /// Last run duration
    pub last_duration: Option<Duration>,
    /// Last exit code
    pub last_exit_code: Option<i32>,
    /// Show colors
    pub use_colors: bool,
}

impl WatchUI {
    /// Create a new watch UI.
    pub fn new() -> Self {
        Self {
            status: "Initializing...".to_string().into(),
            run_count: 0,
            last_duration: None,
            last_exit_code: None,
            use_colors: true,
        }
    }

    /// Update status to show running tests.
    pub fn set_running(&mut self) {
        self.status = "Running tests...".to_string().into();
    }

    /// Update status to show waiting for changes.
    pub fn set_watching(&mut self) {
        self.status = "Watching for changes...".to_string().into();
    }

    /// Update with test results.
    pub fn set_results(&mut self, exit_code: i32, duration: Duration) {
        self.run_count += 1;
        self.last_duration = Some(duration);
        self.last_exit_code = Some(exit_code);

        if exit_code == 0 {
            self.status = "All tests passing".to_string().into();
        } else {
            self.status = format!("{} test(s) failed", exit_code).into();
        }
    }

    /// Render the UI to a string.
    pub fn render(&self) -> String {
        let mut output = String::new();

        // Status line
        let status_colored = if self.use_colors {
            match self.last_exit_code {
                Some(0) => format!("{}", self.status.as_str().green()),
                Some(_) => format!("{}", self.status.as_str().red()),
                None => format!("{}", self.status.as_str().yellow()),
            }
        } else {
            self.status.to_string()
        };

        output.push_str(&format!("[Run #{}] {}", self.run_count, status_colored));

        // Duration
        if let Some(duration) = self.last_duration {
            output.push_str(&format!(" ({:.2}s)", duration.as_secs_f64()));
        }

        output
    }
}

impl Default for WatchUI {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_config_default() {
        let config = WatchConfig::default();
        assert!(!config.watch_paths.is_empty());
        assert!(config.extensions.contains(&"vr".to_string().into()));
        assert_eq!(config.debounce_ms, 200);
    }

    #[test]
    fn test_debouncer() {
        let mut debouncer = Debouncer::new(10);

        // No event yet
        assert!(!debouncer.should_trigger());

        // Event recorded
        debouncer.event();

        // Wait for debounce
        std::thread::sleep(Duration::from_millis(15));

        // Should trigger now
        assert!(debouncer.should_trigger());

        // Should not trigger again
        assert!(!debouncer.should_trigger());
    }

    #[test]
    fn test_debouncer_remaining() {
        let mut debouncer = Debouncer::new(100);

        // No event yet
        assert!(debouncer.remaining().is_none());

        // Event recorded
        debouncer.event();

        // Should have remaining time
        let remaining = debouncer.remaining();
        assert!(remaining.is_some());
        assert!(remaining.unwrap() > Duration::ZERO);

        // Wait and check again
        std::thread::sleep(Duration::from_millis(150));
        assert!(debouncer.should_trigger());
        assert!(debouncer.remaining().is_none());
    }

    #[test]
    fn test_affected_tests() {
        let mut affected = AffectedTests::default();
        assert!(affected.is_empty());

        affected.add_modified(PathBuf::from("test1.vr"));
        assert!(!affected.is_empty());

        affected.add_dependent(PathBuf::from("test2.vr"));
        let paths = affected.all_paths();
        assert_eq!(paths.len(), 2);

        affected.clear();
        assert!(affected.is_empty());
    }

    #[test]
    #[allow(deprecated)]
    fn test_watcher_should_trigger() {
        let watcher = Watcher::new(WatchConfig::default(), RunnerConfig::default());

        assert!(watcher.should_trigger(&PathBuf::from("test.vr")));
        assert!(watcher.should_trigger(&PathBuf::from("src/lib.verum")));
        assert!(!watcher.should_trigger(&PathBuf::from("readme.md")));
        assert!(!watcher.should_trigger(&PathBuf::from("config.toml")));
    }

    #[test]
    fn test_watch_ui() {
        let mut ui = WatchUI::new();
        ui.use_colors = false;

        ui.set_running();
        assert!(ui.render().contains("Running"));

        ui.set_results(0, Duration::from_secs(1));
        let output = ui.render();
        assert!(output.contains("Run #1"));
        assert!(output.contains("passing"));
        assert!(output.contains("1.00s"));

        ui.set_results(2, Duration::from_millis(500));
        let output = ui.render();
        assert!(output.contains("Run #2"));
        assert!(output.contains("failed"));
    }
}
