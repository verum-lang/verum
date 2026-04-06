//! Test Utilities and Infrastructure
//!
//! Provides common testing utilities, assertions, and helpers for integration tests.

use std::sync::Arc;
use std::time::{Duration, Instant};
use verum_ast::Module;
use verum_diagnostics::Diagnostic;
use verum_error::{Result as VerumResult, VerumError};
use verum_lexer::Lexer;
use verum_parser::Parser;
use verum_std::core::{List, Text, Map, Maybe};
use verum_types::TypeChecker;

// ============================================================================
// Test Assertions
// ============================================================================

/// Assert that two values are equal with a custom error message
#[macro_export]
macro_rules! assert_integration_eq {
    ($left:expr, $right:expr, $msg:expr) => {
        if $left != $right {
            panic!(
                "Integration test assertion failed: {}\n  left: {:?}\n  right: {:?}",
                $msg, $left, $right
            );
        }
    };
}

/// Assert that a result is Ok
#[macro_export]
macro_rules! assert_integration_ok {
    ($result:expr, $msg:expr) => {
        match $result {
            Ok(_) => {},
            Err(e) => panic!("Integration test assertion failed: {}\n  error: {:?}", $msg, e),
        }
    };
}

/// Assert that a result is Err
#[macro_export]
macro_rules! assert_integration_err {
    ($result:expr, $msg:expr) => {
        match $result {
            Ok(v) => panic!("Expected error but got Ok: {}\n  value: {:?}", $msg, v),
            Err(_) => {},
        }
    };
}

// ============================================================================
// Pipeline Helpers
// ============================================================================

/// Complete compilation pipeline result
pub struct CompilationResult {
    pub source: String,
    pub tokens: usize,
    pub module: Module,
    pub type_checked: bool,
    pub diagnostics: List<Diagnostic>,
    pub compile_time: Duration,
}

/// Run complete compilation pipeline
pub fn compile_source(source: &str) -> VerumResult<CompilationResult> {
    let start = Instant::now();

    // Lex
    let lexer = Lexer::new(source);
    let token_count = lexer.count();

    // Parse
    let mut parser = Parser::new(source);
    let module = parser.parse_module()
        .map_err(|e| VerumError::new(format!("Parse error: {:?}", e), verum_error::ErrorKind::ParseError))?;

    // Type check
    let mut type_checker = TypeChecker::new();
    let type_checked = type_checker.check_module(&module).is_ok();

    let compile_time = start.elapsed();

    Ok(CompilationResult {
        source: source.to_string(),
        tokens: token_count,
        module,
        type_checked,
        diagnostics: List::new(),
        compile_time,
    })
}

/// Parse and type check an expression
pub fn type_check_expr(source: &str) -> VerumResult<verum_types::TypedExpr> {
    let mut parser = Parser::new(source);
    let expr = parser.parse_expr()
        .map_err(|e| VerumError::new(format!("Parse error: {:?}", e), verum_error::ErrorKind::ParseError))?;

    let mut type_checker = TypeChecker::new();
    type_checker.synth_expr(&expr)
        .map_err(|e| VerumError::new(format!("Type error: {:?}", e), verum_error::ErrorKind::TypeError))
}

// ============================================================================
// Performance Measurement
// ============================================================================

/// Measure execution time of a function
pub fn measure_time<F, T>(f: F) -> (T, Duration)
where
    F: FnOnce() -> T,
{
    let start = Instant::now();
    let result = f();
    let duration = start.elapsed();
    (result, duration)
}

/// Measure async execution time
pub async fn measure_time_async<F, Fut, T>(f: F) -> (T, Duration)
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let result = f().await;
    let duration = start.elapsed();
    (result, duration)
}

/// Performance statistics
#[derive(Debug, Clone)]
pub struct PerfStats {
    pub min: Duration,
    pub max: Duration,
    pub avg: Duration,
    pub median: Duration,
    pub p95: Duration,
    pub p99: Duration,
}

impl PerfStats {
    /// Calculate statistics from a list of durations
    pub fn from_durations(mut durations: Vec<Duration>) -> Self {
        durations.sort();
        let len = durations.len();

        let min = durations[0];
        let max = durations[len - 1];
        let sum: Duration = durations.iter().sum();
        let avg = sum / len as u32;
        let median = durations[len / 2];
        let p95 = durations[(len as f64 * 0.95) as usize];
        let p99 = durations[(len as f64 * 0.99) as usize];

        Self {
            min,
            max,
            avg,
            median,
            p95,
            p99,
        }
    }
}

// ============================================================================
// Memory Tracking
// ============================================================================

/// Memory statistics
#[derive(Debug, Clone)]
pub struct MemoryStats {
    pub initial: usize,
    pub final_size: usize,
    pub peak: usize,
    pub allocated: usize,
    pub freed: usize,
}

/// Track memory usage during test execution
pub struct MemoryTracker {
    initial: usize,
    peak: usize,
}

impl MemoryTracker {
    /// Create a new memory tracker
    pub fn new() -> Self {
        let initial = Self::current_usage();
        Self {
            initial,
            peak: initial,
        }
    }

    /// Update peak memory usage
    pub fn update(&mut self) {
        let current = Self::current_usage();
        if current > self.peak {
            self.peak = current;
        }
    }

    /// Get current memory usage (approximate)
    fn current_usage() -> usize {
        // This is a simplified implementation
        // In production, would use platform-specific APIs
        0
    }

    /// Get final statistics
    pub fn stats(&self) -> MemoryStats {
        let final_size = Self::current_usage();
        MemoryStats {
            initial: self.initial,
            final_size,
            peak: self.peak,
            allocated: self.peak - self.initial,
            freed: if self.peak > final_size {
                self.peak - final_size
            } else {
                0
            },
        }
    }
}

// ============================================================================
// Concurrency Testing
// ============================================================================

/// Run a test function concurrently N times
pub async fn run_concurrent<F, Fut>(n: usize, f: F) -> Vec<Duration>
where
    F: Fn(usize) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let f = Arc::new(f);
    let mut handles = Vec::new();

    for i in 0..n {
        let f = Arc::clone(&f);
        let handle = tokio::spawn(async move {
            let start = Instant::now();
            f(i).await;
            start.elapsed()
        });
        handles.push(handle);
    }

    let mut durations = Vec::new();
    for handle in handles {
        if let Ok(duration) = handle.await {
            durations.push(duration);
        }
    }

    durations
}

/// Stress test helper - run until failure or max iterations
pub async fn stress_test<F, Fut>(
    max_iterations: usize,
    f: F,
) -> Result<usize, String>
where
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    for i in 0..max_iterations {
        match f(i).await {
            Ok(()) => continue,
            Err(e) => return Err(format!("Failed at iteration {}: {}", i, e)),
        }
    }
    Ok(max_iterations)
}

// ============================================================================
// Test Data Generation
// ============================================================================

/// Generate a random Verum program
pub fn generate_random_program(size: usize) -> String {
    let mut program = String::new();

    for i in 0..size {
        program.push_str(&format!(
            "fn func{}(x: Int) -> Int {{ x + {} }}\n",
            i, i
        ));
    }

    program
}

/// Generate a deeply nested expression
pub fn generate_nested_expr(depth: usize) -> String {
    let mut expr = "1".to_string();

    for _ in 0..depth {
        expr = format!("({} + 1)", expr);
    }

    expr
}

// ============================================================================
// Fixture Helpers
// ============================================================================

/// Load a test fixture file
pub fn load_fixture(name: &str) -> String {
    let path = format!("tests/fixtures/{}.vr", name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("Failed to load fixture: {}", name))
}

/// Create a temporary test file
pub fn create_temp_file(content: &str) -> std::path::PathBuf {
    use std::io::Write;

    let mut temp_file = tempfile::NamedTempFile::new()
        .expect("Failed to create temp file");

    temp_file.write_all(content.as_bytes())
        .expect("Failed to write temp file");

    temp_file.path().to_path_buf()
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Assert compilation succeeds
pub fn assert_compiles(source: &str) {
    match compile_source(source) {
        Ok(_) => {},
        Err(e) => panic!("Expected compilation to succeed but got error: {:?}", e),
    }
}

/// Assert compilation fails
pub fn assert_compile_error(source: &str) {
    match compile_source(source) {
        Ok(_) => panic!("Expected compilation to fail but it succeeded"),
        Err(_) => {},
    }
}

/// Assert type checking succeeds
pub fn assert_type_checks(source: &str) {
    match type_check_expr(source) {
        Ok(_) => {},
        Err(e) => panic!("Expected type checking to succeed but got error: {:?}", e),
    }
}

/// Assert type checking fails
pub fn assert_type_error(source: &str) {
    match type_check_expr(source) {
        Ok(_) => panic!("Expected type checking to fail but it succeeded"),
        Err(_) => {},
    }
}

/// Assert duration is less than target
pub fn assert_duration_lt(duration: Duration, target: Duration, msg: &str) {
    if duration >= target {
        panic!(
            "Performance assertion failed: {}\n  actual: {:?}\n  target: {:?}",
            msg, duration, target
        );
    }
}

/// Assert memory usage is within bounds
pub fn assert_memory_bounded(stats: &MemoryStats, max_mb: usize) {
    let max_bytes = max_mb * 1024 * 1024;
    if stats.peak > max_bytes {
        panic!(
            "Memory usage exceeded bounds: {} MB > {} MB",
            stats.peak / (1024 * 1024),
            max_mb
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_simple_program() {
        let source = "fn add(x: Int, y: Int) -> Int { x + y }";
        let result = compile_source(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_measure_time() {
        let (result, duration) = measure_time(|| {
            std::thread::sleep(Duration::from_millis(10));
            42
        });
        assert_eq!(result, 42);
        assert!(duration >= Duration::from_millis(10));
    }

    #[test]
    fn test_perf_stats() {
        let durations = vec![
            Duration::from_millis(1),
            Duration::from_millis(2),
            Duration::from_millis(3),
            Duration::from_millis(4),
            Duration::from_millis(5),
        ];
        let stats = PerfStats::from_durations(durations);
        assert_eq!(stats.min, Duration::from_millis(1));
        assert_eq!(stats.max, Duration::from_millis(5));
    }
}
