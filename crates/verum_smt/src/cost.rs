//! Cost tracking and reporting for SMT verification (P0 for v1.0).
//!
//! This module provides comprehensive tracking of verification costs and
//! suggests optimizations when verification becomes expensive.

use std::fmt;
use std::time::{Duration, Instant};
use verum_common::{List, Text};

/// Tracks verification costs across multiple checks.
#[derive(Debug, Clone)]
pub struct CostTracker {
    /// All recorded verification costs
    costs: List<VerificationCost>,

    /// Total time spent in verification
    total_time: Duration,

    /// Threshold for suggesting runtime checks (default: 5s)
    slow_threshold: Duration,
}

impl CostTracker {
    /// Create a new cost tracker.
    pub fn new() -> Self {
        Self {
            costs: List::new(),
            total_time: Duration::ZERO,
            slow_threshold: Duration::from_secs(5),
        }
    }

    /// Create a cost tracker with custom slow threshold.
    pub fn with_threshold(threshold: Duration) -> Self {
        Self {
            costs: List::new(),
            total_time: Duration::ZERO,
            slow_threshold: threshold,
        }
    }

    /// Record a verification cost.
    pub fn record(&mut self, cost: VerificationCost) {
        self.total_time += cost.duration;
        self.costs.push(cost);
    }

    /// Get all recorded costs.
    pub fn costs(&self) -> &[VerificationCost] {
        &self.costs
    }

    /// Get total verification time.
    pub fn total_time(&self) -> Duration {
        self.total_time
    }

    /// Get average verification time.
    pub fn avg_time(&self) -> Duration {
        if self.costs.is_empty() {
            Duration::ZERO
        } else {
            self.total_time / self.costs.len() as u32
        }
    }

    /// Get the slowest verification.
    pub fn slowest(&self) -> Option<&VerificationCost> {
        self.costs.iter().max_by_key(|c| c.duration)
    }

    /// Get all slow verifications (above threshold).
    pub fn slow_verifications(&self) -> List<&VerificationCost> {
        self.costs
            .iter()
            .filter(|c| c.duration >= self.slow_threshold)
            .collect()
    }

    /// Check if we should suggest runtime checks.
    pub fn should_suggest_runtime(&self) -> bool {
        !self.slow_verifications().is_empty()
    }

    /// Generate a cost report.
    pub fn report(&self) -> CostReport {
        CostReport {
            total_verifications: self.costs.len(),
            total_time: self.total_time,
            avg_time: self.avg_time(),
            slowest: self.slowest().cloned(),
            slow_verifications: self.slow_verifications().into_iter().cloned().collect(),
            should_suggest_runtime: self.should_suggest_runtime(),
        }
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Cost of a single verification operation.
#[derive(Debug, Clone)]
pub struct VerificationCost {
    /// Function or location being verified
    pub location: Text,

    /// Time taken for verification
    pub duration: Duration,

    /// Whether verification succeeded
    pub succeeded: bool,

    /// Number of Z3 solver checks
    pub num_checks: u64,

    /// Complexity estimate (0-100)
    pub complexity: u32,

    /// Whether timeout occurred
    pub timed_out: bool,

    /// Category of verification (e.g., "loop_invariant", "termination_check", "frame_condition")
    pub category: Text,
}

impl VerificationCost {
    /// Create a new verification cost.
    pub fn new(location: Text, duration: Duration, succeeded: bool) -> Self {
        Self {
            location,
            duration,
            succeeded,
            num_checks: 1,
            complexity: 0,
            timed_out: false,
            category: Text::from("general"),
        }
    }

    /// Create a new verification cost with a specific category.
    pub fn with_category(
        location: Text,
        duration: Duration,
        succeeded: bool,
        category: &str,
    ) -> Self {
        Self {
            location,
            duration,
            succeeded,
            num_checks: 1,
            complexity: 0,
            timed_out: false,
            category: Text::from(category),
        }
    }

    /// Set the number of solver checks.
    pub fn with_checks(mut self, checks: u64) -> Self {
        self.num_checks = checks;
        self
    }

    /// Set the complexity estimate.
    pub fn with_complexity(mut self, complexity: u32) -> Self {
        self.complexity = complexity.min(100);
        self
    }

    /// Mark as timed out.
    pub fn with_timeout(mut self) -> Self {
        self.timed_out = true;
        self
    }

    /// Get time in milliseconds.
    pub fn time_ms(&self) -> u64 {
        self.duration.as_millis() as u64
    }

    /// Get time in seconds.
    pub fn time_secs(&self) -> f64 {
        self.duration.as_secs_f64()
    }

    /// Check if this is a slow verification (>1s).
    pub fn is_slow(&self) -> bool {
        self.duration >= Duration::from_secs(1)
    }

    /// Check if this is very slow (>5s).
    pub fn is_very_slow(&self) -> bool {
        self.duration >= Duration::from_secs(5)
    }

    /// Merge two verification costs
    ///
    /// Combines the duration and check counts, preserving the location
    /// and category of the first cost.
    pub fn merge(self, other: VerificationCost) -> Self {
        Self {
            location: self.location,
            duration: self.duration + other.duration,
            succeeded: self.succeeded && other.succeeded,
            num_checks: self.num_checks + other.num_checks,
            complexity: self.complexity.max(other.complexity),
            timed_out: self.timed_out || other.timed_out,
            category: self.category,
        }
    }
}

/// Summary report of verification costs.
#[derive(Debug, Clone)]
pub struct CostReport {
    /// Total number of verifications performed
    pub total_verifications: usize,

    /// Total time spent across all verifications
    pub total_time: Duration,

    /// Average time per verification
    pub avg_time: Duration,

    /// Slowest verification (if any)
    pub slowest: Option<VerificationCost>,

    /// All slow verifications (above threshold)
    pub slow_verifications: List<VerificationCost>,

    /// Whether to suggest switching to runtime checks
    pub should_suggest_runtime: bool,
}

impl CostReport {
    /// Format the report for display.
    pub fn format(&self) -> Text {
        let mut output = Text::new();

        output.push_str(&format!(
            "Verification Summary:\n  Total: {} checks in {:.2}s\n  Average: {:.2}s per check\n",
            self.total_verifications,
            self.total_time.as_secs_f64(),
            self.avg_time.as_secs_f64()
        ));

        if let Some(slowest) = &self.slowest {
            output.push_str(&format!(
                "  Slowest: {} ({:.2}s)\n",
                slowest.location,
                slowest.time_secs()
            ));
        }

        if !self.slow_verifications.is_empty() {
            output.push_str(&format!(
                "\nSlow verifications ({}):\n",
                self.slow_verifications.len()
            ));
            for cost in &self.slow_verifications {
                output.push_str(&format!(
                    "  • {} - {:.2}s{}\n",
                    cost.location,
                    cost.time_secs(),
                    if cost.timed_out { " (timeout)" } else { "" }
                ));
            }
        }

        if self.should_suggest_runtime {
            output.push_str("\nSuggestion:\n");
            output.push_str("  Consider using @verify(runtime) for expensive checks\n");
            output.push_str("  This will use runtime validation instead of compile-time SMT\n");
        }

        output
    }
}

impl fmt::Display for CostReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format())
    }
}

/// Helper to measure verification time.
#[derive(Debug)]
pub struct CostMeasurement {
    location: Text,
    category: Text,
    start: Instant,
    num_checks: u64,
    complexity: u32,
}

impl CostMeasurement {
    /// Start measuring verification cost.
    ///
    /// The location is also used as the default category.
    pub fn start(location: impl Into<Text>) -> Self {
        let loc = location.into();
        Self {
            location: loc.clone(),
            category: loc,
            start: Instant::now(),
            num_checks: 1,
            complexity: 0,
        }
    }

    /// Start measuring with explicit category.
    pub fn start_with_category(location: impl Into<Text>, category: impl Into<Text>) -> Self {
        Self {
            location: location.into(),
            category: category.into(),
            start: Instant::now(),
            num_checks: 1,
            complexity: 0,
        }
    }

    /// Set the number of solver checks.
    pub fn with_checks(mut self, checks: u64) -> Self {
        self.num_checks = checks;
        self
    }

    /// Set complexity estimate.
    pub fn with_complexity(mut self, complexity: u32) -> Self {
        self.complexity = complexity;
        self
    }

    /// Complete the measurement.
    pub fn finish(self, succeeded: bool) -> VerificationCost {
        let duration = self.start.elapsed();
        VerificationCost {
            location: self.location,
            duration,
            succeeded,
            num_checks: self.num_checks,
            complexity: self.complexity,
            timed_out: false,
            category: self.category,
        }
    }

    /// Complete with timeout.
    pub fn finish_timeout(self) -> VerificationCost {
        let duration = self.start.elapsed();
        VerificationCost {
            location: self.location,
            duration,
            succeeded: false,
            num_checks: self.num_checks,
            complexity: self.complexity,
            timed_out: true,
            category: self.category,
        }
    }
}

/// Format a success message with cost information.
pub fn format_success(location: &str, cost: &VerificationCost) -> Text {
    let mut msg = format!("✓ Verified {} in {:.2}s", location, cost.time_secs());

    if cost.is_very_slow() {
        msg.push_str("\n⚠ Consider @verify(runtime) for faster builds");
    } else if cost.is_slow() {
        msg.push_str(" (slow)");
    }

    Text::from(msg)
}

/// Format a failure message with cost information.
pub fn format_failure(location: &str, constraint: &str, cost: &VerificationCost) -> Text {
    let mut msg = format!(
        "✗ Cannot prove {} after {:.2}s\n  Constraint: {}",
        location,
        cost.time_secs(),
        constraint
    );

    if cost.timed_out {
        msg.push_str("\n  Note: Verification timed out - result may be inconclusive");
        msg.push_str("\n  Suggestion: Try @verify(runtime) or increase timeout");
    }

    Text::from(msg)
}
