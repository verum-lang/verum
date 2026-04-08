//! Fuzz testing harnesses for Verum
//!
//! This module provides various test harnesses for fuzzing the Verum
//! compiler and runtime.
//!
//! # Harness Types
//!
//! - **Differential Harness**: Compares results across execution tiers
//! - **Crash Harness**: Detects and reports crashes
//! - **Memory Harness**: Detects memory issues (CBGR integration)

pub mod crash_harness;
pub mod differential_harness;
pub mod memory_harness;

pub use differential_harness::{
    BatchResult, DiffError, DifferentialConfig, DifferentialHarness, Tier, TierResult, Value,
};

pub use crash_harness::{
    CompilerPhase, CrashConfig, CrashHarness, CrashInfo, CrashStats, CrashType,
};

pub use memory_harness::{
    MemoryAnalysisResult, MemoryConfig, MemoryHarness, MemoryIssue, MemoryReport, MemoryStats,
    OverflowDirection,
};

use std::time::{Duration, Instant};

/// Unified fuzzing harness that combines all detection capabilities
pub struct UnifiedHarness {
    differential: DifferentialHarness,
    crash: CrashHarness,
    memory: MemoryHarness,
}

impl UnifiedHarness {
    /// Create a unified harness with default configuration
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            differential: DifferentialHarness::new(),
            crash: CrashHarness::new(CrashConfig::default())?,
            memory: MemoryHarness::new(MemoryConfig::default()),
        })
    }

    /// Create a unified harness with custom configurations
    pub fn with_configs(
        diff_config: DifferentialConfig,
        crash_config: CrashConfig,
        memory_config: MemoryConfig,
    ) -> std::io::Result<Self> {
        Ok(Self {
            differential: DifferentialHarness::with_config(diff_config),
            crash: CrashHarness::new(crash_config)?,
            memory: MemoryHarness::new(memory_config),
        })
    }

    /// Run all tests on a source program
    pub fn test(&self, source: &str) -> UnifiedResult {
        let start = Instant::now();

        // Run differential testing
        let diff_result = self.differential.test(source);

        // Check for crashes
        let crash_result = self.crash.test(source);

        // Analyze memory
        let memory_result = self.memory.analyze(source);

        UnifiedResult {
            source: source.to_string(),
            differential_result: diff_result,
            crash_info: crash_result,
            memory_issues: memory_result.issues,
            memory_stats: memory_result.stats,
            duration: start.elapsed(),
        }
    }

    /// Run tests on multiple sources
    pub fn test_batch(&self, sources: &[&str]) -> Vec<UnifiedResult> {
        sources.iter().map(|s| self.test(s)).collect()
    }

    /// Get summary statistics
    pub fn get_summary(&self) -> UnifiedStats {
        let crash_stats = self.crash.get_stats();
        let memory_stats = self.memory.get_stats();

        UnifiedStats {
            total_crashes: crash_stats.total_crashes,
            unique_crashes: crash_stats.unique_crashes,
            memory_issues_detected: 0, // Would track across tests
            generation_mismatches: 0,
            reference_validations: memory_stats.reference_validations,
        }
    }
}

impl Default for UnifiedHarness {
    fn default() -> Self {
        Self::new().expect("Failed to create unified harness")
    }
}

/// Result from unified testing
#[derive(Debug)]
pub struct UnifiedResult {
    /// Original source code
    pub source: String,
    /// Result from differential testing
    pub differential_result: Result<Vec<TierResult>, DiffError>,
    /// Crash information if crash detected
    pub crash_info: Option<CrashInfo>,
    /// Memory issues detected
    pub memory_issues: Vec<MemoryReport>,
    /// Memory statistics
    pub memory_stats: MemoryStats,
    /// Total test duration
    pub duration: Duration,
}

impl UnifiedResult {
    /// Check if any issues were found
    pub fn has_issues(&self) -> bool {
        self.differential_result.is_err()
            || self.crash_info.is_some()
            || !self.memory_issues.is_empty()
    }

    /// Get severity of the most critical issue
    pub fn max_severity(&self) -> u8 {
        let mut max = 0u8;

        if self.differential_result.is_err() {
            max = max.max(8);
        }

        if self.crash_info.is_some() {
            max = max.max(10);
        }

        for issue in &self.memory_issues {
            max = max.max(issue.severity);
        }

        max
    }
}

/// Summary statistics for unified harness
#[derive(Debug, Default)]
pub struct UnifiedStats {
    pub total_crashes: usize,
    pub unique_crashes: usize,
    pub memory_issues_detected: usize,
    pub generation_mismatches: usize,
    pub reference_validations: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unified_harness_creation() {
        let harness = UnifiedHarness::new();
        assert!(harness.is_ok());
    }

    #[test]
    fn test_unified_result_severity() {
        let result = UnifiedResult {
            source: String::new(),
            differential_result: Ok(vec![]),
            crash_info: None,
            memory_issues: vec![],
            memory_stats: MemoryStats::default(),
            duration: Duration::from_secs(0),
        };

        assert_eq!(result.max_severity(), 0);
        assert!(!result.has_issues());
    }
}
