//! Meta Evaluation Metrics
//!
//! This module provides metrics tracking for meta-programming execution,
//! enabling performance monitoring, debugging, and optimization.
//!
//! ## Tracked Metrics
//!
//! - Builtin call counts and durations
//! - Cache hit/miss rates for type lookups
//! - Memory usage estimates
//! - Recursion depth tracking
//! - Pattern matching statistics
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use verum_common::{List, Map, Text};

/// Metrics for meta function evaluation
#[derive(Debug, Default)]
pub struct MetaEvalMetrics {
    /// Total number of meta expressions evaluated
    expr_count: AtomicU64,
    /// Total number of builtin function calls
    builtin_count: AtomicU64,
    /// Total evaluation time
    total_time_ns: AtomicU64,
    /// Maximum recursion depth reached
    max_recursion_depth: AtomicU64,
    /// Current recursion depth
    current_depth: AtomicU64,
    /// Cache hits for type lookups
    cache_hits: AtomicU64,
    /// Cache misses for type lookups
    cache_misses: AtomicU64,
    /// Pattern match attempts
    pattern_match_attempts: AtomicU64,
    /// Pattern match successes
    pattern_match_successes: AtomicU64,
    /// Per-builtin call counts
    builtin_calls: parking_lot::RwLock<Map<Text, u64>>,
    /// Per-builtin total time (nanoseconds)
    builtin_times: parking_lot::RwLock<Map<Text, u64>>,
    /// Memory allocation estimates (bytes)
    memory_allocated: AtomicU64,
    /// Error count
    error_count: AtomicU64,
}

impl MetaEvalMetrics {
    /// Create a new metrics tracker
    pub fn new() -> Self {
        Self::default()
    }

    /// Record evaluation of a meta expression
    pub fn record_expr_eval(&self) {
        self.expr_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a builtin function call
    pub fn record_builtin_call(&self, name: &Text, duration: Duration) {
        self.builtin_count.fetch_add(1, Ordering::Relaxed);
        let nanos = duration.as_nanos() as u64;
        self.total_time_ns.fetch_add(nanos, Ordering::Relaxed);

        let mut calls = self.builtin_calls.write();
        *calls.entry(name.clone()).or_insert(0) += 1;

        let mut times = self.builtin_times.write();
        *times.entry(name.clone()).or_insert(0) += nanos;
    }

    /// Record entering a recursion level
    pub fn enter_recursion(&self) -> u64 {
        let depth = self.current_depth.fetch_add(1, Ordering::SeqCst) + 1;
        let mut max = self.max_recursion_depth.load(Ordering::Relaxed);
        while depth > max {
            match self.max_recursion_depth.compare_exchange_weak(
                max,
                depth,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(current) => max = current,
            }
        }
        depth
    }

    /// Record exiting a recursion level
    pub fn exit_recursion(&self) {
        self.current_depth.fetch_sub(1, Ordering::SeqCst);
    }

    /// Record a cache hit
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a pattern match attempt
    pub fn record_pattern_attempt(&self, success: bool) {
        self.pattern_match_attempts.fetch_add(1, Ordering::Relaxed);
        if success {
            self.pattern_match_successes.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record memory allocation
    pub fn record_allocation(&self, bytes: usize) {
        self.memory_allocated.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Record an error
    pub fn record_error(&self) {
        self.error_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get total expression count
    pub fn expr_count(&self) -> u64 {
        self.expr_count.load(Ordering::Relaxed)
    }

    /// Get total builtin call count
    pub fn builtin_count(&self) -> u64 {
        self.builtin_count.load(Ordering::Relaxed)
    }

    /// Get total evaluation time
    pub fn total_time(&self) -> Duration {
        Duration::from_nanos(self.total_time_ns.load(Ordering::Relaxed))
    }

    /// Get maximum recursion depth reached
    pub fn max_recursion_depth(&self) -> u64 {
        self.max_recursion_depth.load(Ordering::Relaxed)
    }

    /// Get cache hit rate (0.0 - 1.0)
    pub fn cache_hit_rate(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::Relaxed);
        let misses = self.cache_misses.load(Ordering::Relaxed);
        let total = hits + misses;
        if total == 0 {
            0.0
        } else {
            hits as f64 / total as f64
        }
    }

    /// Get pattern match success rate (0.0 - 1.0)
    pub fn pattern_match_rate(&self) -> f64 {
        let attempts = self.pattern_match_attempts.load(Ordering::Relaxed);
        let successes = self.pattern_match_successes.load(Ordering::Relaxed);
        if attempts == 0 {
            0.0
        } else {
            successes as f64 / attempts as f64
        }
    }

    /// Get estimated memory usage
    pub fn memory_allocated(&self) -> u64 {
        self.memory_allocated.load(Ordering::Relaxed)
    }

    /// Get error count
    pub fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    /// Get per-builtin statistics
    pub fn builtin_stats(&self) -> List<BuiltinStats> {
        let calls = self.builtin_calls.read();
        let times = self.builtin_times.read();

        calls
            .iter()
            .map(|(name, count)| {
                let total_time = times.get(name).copied().unwrap_or(0);
                let avg_time = if *count > 0 {
                    total_time / count
                } else {
                    0
                };
                BuiltinStats {
                    name: name.clone(),
                    call_count: *count,
                    total_time_ns: total_time,
                    avg_time_ns: avg_time,
                }
            })
            .collect()
    }

    /// Get summary report
    pub fn summary(&self) -> MetricsSummary {
        MetricsSummary {
            expr_count: self.expr_count(),
            builtin_count: self.builtin_count(),
            total_time: self.total_time(),
            max_recursion_depth: self.max_recursion_depth(),
            cache_hit_rate: self.cache_hit_rate(),
            pattern_match_rate: self.pattern_match_rate(),
            memory_allocated: self.memory_allocated(),
            error_count: self.error_count(),
        }
    }

    /// Reset all metrics
    pub fn reset(&self) {
        self.expr_count.store(0, Ordering::Relaxed);
        self.builtin_count.store(0, Ordering::Relaxed);
        self.total_time_ns.store(0, Ordering::Relaxed);
        self.max_recursion_depth.store(0, Ordering::Relaxed);
        self.current_depth.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.pattern_match_attempts.store(0, Ordering::Relaxed);
        self.pattern_match_successes.store(0, Ordering::Relaxed);
        self.memory_allocated.store(0, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        self.builtin_calls.write().clear();
        self.builtin_times.write().clear();
    }
}

/// Per-builtin statistics
#[derive(Debug, Clone)]
pub struct BuiltinStats {
    /// Builtin function name
    pub name: Text,
    /// Number of calls
    pub call_count: u64,
    /// Total time in nanoseconds
    pub total_time_ns: u64,
    /// Average time per call in nanoseconds
    pub avg_time_ns: u64,
}

/// Metrics summary
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    /// Total expressions evaluated
    pub expr_count: u64,
    /// Total builtin calls
    pub builtin_count: u64,
    /// Total evaluation time
    pub total_time: Duration,
    /// Maximum recursion depth
    pub max_recursion_depth: u64,
    /// Cache hit rate (0.0 - 1.0)
    pub cache_hit_rate: f64,
    /// Pattern match success rate (0.0 - 1.0)
    pub pattern_match_rate: f64,
    /// Memory allocated in bytes
    pub memory_allocated: u64,
    /// Error count
    pub error_count: u64,
}

impl std::fmt::Display for MetricsSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Meta Evaluation Metrics Summary:")?;
        writeln!(f, "  Expressions evaluated: {}", self.expr_count)?;
        writeln!(f, "  Builtin calls: {}", self.builtin_count)?;
        writeln!(f, "  Total time: {:?}", self.total_time)?;
        writeln!(f, "  Max recursion depth: {}", self.max_recursion_depth)?;
        writeln!(f, "  Cache hit rate: {:.2}%", self.cache_hit_rate * 100.0)?;
        writeln!(
            f,
            "  Pattern match rate: {:.2}%",
            self.pattern_match_rate * 100.0
        )?;
        writeln!(f, "  Memory allocated: {} bytes", self.memory_allocated)?;
        writeln!(f, "  Errors: {}", self.error_count)?;
        Ok(())
    }
}

/// RAII guard for tracking recursion depth
pub struct RecursionGuard<'a> {
    metrics: &'a MetaEvalMetrics,
}

impl<'a> RecursionGuard<'a> {
    /// Create a new recursion guard
    pub fn new(metrics: &'a MetaEvalMetrics) -> Self {
        metrics.enter_recursion();
        Self { metrics }
    }
}

impl Drop for RecursionGuard<'_> {
    fn drop(&mut self) {
        self.metrics.exit_recursion();
    }
}

/// RAII guard for timing a builtin call
pub struct BuiltinTimingGuard<'a> {
    metrics: &'a MetaEvalMetrics,
    name: Text,
    start: Instant,
}

impl<'a> BuiltinTimingGuard<'a> {
    /// Create a new timing guard
    pub fn new(metrics: &'a MetaEvalMetrics, name: Text) -> Self {
        Self {
            metrics,
            name,
            start: Instant::now(),
        }
    }
}

impl Drop for BuiltinTimingGuard<'_> {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        self.metrics.record_builtin_call(&self.name, duration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_new() {
        let metrics = MetaEvalMetrics::new();
        assert_eq!(metrics.expr_count(), 0);
        assert_eq!(metrics.builtin_count(), 0);
        assert_eq!(metrics.max_recursion_depth(), 0);
    }

    #[test]
    fn test_record_expr_eval() {
        let metrics = MetaEvalMetrics::new();
        metrics.record_expr_eval();
        metrics.record_expr_eval();
        metrics.record_expr_eval();
        assert_eq!(metrics.expr_count(), 3);
    }

    #[test]
    fn test_record_builtin_call() {
        let metrics = MetaEvalMetrics::new();
        let name = Text::from("test_builtin");
        metrics.record_builtin_call(&name, Duration::from_micros(100));
        metrics.record_builtin_call(&name, Duration::from_micros(200));
        assert_eq!(metrics.builtin_count(), 2);

        let stats = metrics.builtin_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].call_count, 2);
        assert_eq!(stats[0].total_time_ns, 300_000);
    }

    #[test]
    fn test_recursion_depth() {
        let metrics = MetaEvalMetrics::new();

        metrics.enter_recursion(); // depth 1
        metrics.enter_recursion(); // depth 2
        metrics.enter_recursion(); // depth 3
        assert_eq!(metrics.max_recursion_depth(), 3);

        metrics.exit_recursion(); // depth 2
        metrics.exit_recursion(); // depth 1
        metrics.enter_recursion(); // depth 2
        assert_eq!(metrics.max_recursion_depth(), 3); // still 3

        metrics.enter_recursion(); // depth 3
        metrics.enter_recursion(); // depth 4
        assert_eq!(metrics.max_recursion_depth(), 4);
    }

    #[test]
    fn test_cache_hit_rate() {
        let metrics = MetaEvalMetrics::new();
        assert_eq!(metrics.cache_hit_rate(), 0.0);

        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();
        assert!((metrics.cache_hit_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_pattern_match_rate() {
        let metrics = MetaEvalMetrics::new();
        assert_eq!(metrics.pattern_match_rate(), 0.0);

        metrics.record_pattern_attempt(true);
        metrics.record_pattern_attempt(true);
        metrics.record_pattern_attempt(false);
        metrics.record_pattern_attempt(true);
        assert!((metrics.pattern_match_rate() - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_recursion_guard() {
        let metrics = MetaEvalMetrics::new();

        {
            let _guard = RecursionGuard::new(&metrics);
            assert_eq!(metrics.max_recursion_depth(), 1);
            {
                let _guard2 = RecursionGuard::new(&metrics);
                assert_eq!(metrics.max_recursion_depth(), 2);
            }
        }
        // Guards dropped, but max depth should still be 2
        assert_eq!(metrics.max_recursion_depth(), 2);
    }

    #[test]
    fn test_reset() {
        let metrics = MetaEvalMetrics::new();
        metrics.record_expr_eval();
        metrics.record_cache_hit();
        metrics.record_error();
        metrics.enter_recursion();

        metrics.reset();

        assert_eq!(metrics.expr_count(), 0);
        assert_eq!(metrics.cache_hit_rate(), 0.0);
        assert_eq!(metrics.error_count(), 0);
        assert_eq!(metrics.max_recursion_depth(), 0);
    }

    #[test]
    fn test_summary_display() {
        let metrics = MetaEvalMetrics::new();
        metrics.record_expr_eval();
        metrics.record_builtin_call(&Text::from("test"), Duration::from_micros(50));
        metrics.record_cache_hit();
        metrics.record_pattern_attempt(true);

        let summary = metrics.summary();
        let display = format!("{}", summary);
        assert!(display.contains("Expressions evaluated: 1"));
        assert!(display.contains("Builtin calls: 1"));
    }
}
