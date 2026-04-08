//! Tier Executor - Execute tests on Tier 0 and Tier 3
//!
//! This module handles the execution of test programs on different tiers:
//! - Tier 0: Tree-walking interpreter
//! - Tier 1: Bytecode VM
//! - Tier 2: JIT compiler
//! - Tier 3: AOT compiler
//!
//! Each tier is executed as a separate process with timeout handling
//! and resource monitoring.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::Tier;

/// Configuration for the executor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutorConfig {
    /// Path to the interpreter binary (Tier 0)
    pub interpreter_path: PathBuf,
    /// Path to the bytecode VM binary (Tier 1)
    pub bytecode_path: Option<PathBuf>,
    /// Path to the JIT binary (Tier 2)
    pub jit_path: Option<PathBuf>,
    /// Path to the AOT binary (Tier 3)
    pub aot_path: PathBuf,
    /// Timeout in milliseconds
    pub timeout_ms: u64,
    /// Environment variables
    pub env_vars: HashMap<String, String>,
    /// Extra arguments for all tiers
    pub extra_args: Vec<String>,
}

impl Default for ExecutorConfig {
    fn default() -> Self {
        Self {
            interpreter_path: PathBuf::from("verum-interpret"),
            bytecode_path: Some(PathBuf::from("verum-bc")),
            jit_path: Some(PathBuf::from("verum-jit")),
            aot_path: PathBuf::from("verum-run"),
            timeout_ms: 30_000,
            env_vars: HashMap::new(),
            extra_args: Vec::new(),
        }
    }
}

/// Specification for a tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierSpec {
    /// Tier identifier
    pub tier: Tier,
    /// Binary path
    pub binary_path: PathBuf,
    /// Additional arguments specific to this tier
    pub extra_args: Vec<String>,
    /// Whether this tier is available
    pub available: bool,
}

/// Result of executing a test on a tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Which tier was executed
    pub tier: Tier,
    /// Whether execution succeeded (process exited normally)
    pub success: bool,
    /// Exit code (if available)
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Execution duration
    pub duration: Duration,
    /// Duration in milliseconds (for convenience)
    pub duration_ms: u64,
    /// Whether execution timed out
    pub timed_out: bool,
    /// Whether the process crashed
    pub crashed: bool,
    /// Signal that killed the process (if any)
    pub signal: Option<i32>,
    /// Peak memory usage in bytes (if available)
    pub peak_memory: Option<usize>,
}

impl ExecutionResult {
    /// Create a successful result
    pub fn success(
        tier: Tier,
        exit_code: i32,
        stdout: String,
        stderr: String,
        duration: Duration,
    ) -> Self {
        Self {
            tier,
            success: exit_code == 0,
            exit_code: Some(exit_code),
            stdout,
            stderr,
            duration,
            duration_ms: duration.as_millis() as u64,
            timed_out: false,
            crashed: false,
            signal: None,
            peak_memory: None,
        }
    }

    /// Create a timeout result
    pub fn timeout(tier: Tier, stdout: String, stderr: String, duration: Duration) -> Self {
        Self {
            tier,
            success: false,
            exit_code: None,
            stdout,
            stderr,
            duration,
            duration_ms: duration.as_millis() as u64,
            timed_out: true,
            crashed: false,
            signal: None,
            peak_memory: None,
        }
    }

    /// Create a crash result
    pub fn crash(
        tier: Tier,
        stdout: String,
        stderr: String,
        duration: Duration,
        signal: Option<i32>,
    ) -> Self {
        Self {
            tier,
            success: false,
            exit_code: None,
            stdout,
            stderr,
            duration,
            duration_ms: duration.as_millis() as u64,
            timed_out: false,
            crashed: true,
            signal,
            peak_memory: None,
        }
    }

    /// Check if this result represents a failure
    pub fn is_failure(&self) -> bool {
        !self.success || self.timed_out || self.crashed
    }
}

/// Executor for running tests on different tiers
pub struct TierExecutor {
    config: ExecutorConfig,
    tier_specs: HashMap<Tier, TierSpec>,
}

impl TierExecutor {
    /// Create a new executor with the given configuration
    pub fn new(config: ExecutorConfig) -> Self {
        let mut tier_specs = HashMap::new();

        // Tier 0: Interpreter
        tier_specs.insert(
            Tier::Interpreter,
            TierSpec {
                tier: Tier::Interpreter,
                binary_path: config.interpreter_path.clone(),
                extra_args: vec!["--interpret".to_string()],
                available: true, // Assume available, will check on first use
            },
        );

        // Tier 1: Bytecode VM
        if let Some(ref path) = config.bytecode_path {
            tier_specs.insert(
                Tier::Bytecode,
                TierSpec {
                    tier: Tier::Bytecode,
                    binary_path: path.clone(),
                    extra_args: vec!["--bytecode".to_string()],
                    available: true,
                },
            );
        }

        // Tier 2: JIT
        if let Some(ref path) = config.jit_path {
            tier_specs.insert(
                Tier::Jit,
                TierSpec {
                    tier: Tier::Jit,
                    binary_path: path.clone(),
                    extra_args: vec!["--jit".to_string()],
                    available: true,
                },
            );
        }

        // Tier 3: AOT
        tier_specs.insert(
            Tier::Aot,
            TierSpec {
                tier: Tier::Aot,
                binary_path: config.aot_path.clone(),
                extra_args: vec![], // AOT is the default mode
                available: true,
            },
        );

        Self { config, tier_specs }
    }

    /// Check if a tier is available
    pub fn is_tier_available(&self, tier: Tier) -> bool {
        self.tier_specs.get(&tier).map_or(false, |spec| {
            spec.available && spec.binary_path.exists()
        })
    }

    /// Get the specification for a tier
    pub fn get_tier_spec(&self, tier: Tier) -> Option<&TierSpec> {
        self.tier_specs.get(&tier)
    }

    /// Execute a test file on a specific tier
    pub fn execute(&self, tier: Tier, test_path: &Path) -> Result<ExecutionResult> {
        let spec = self.tier_specs.get(&tier)
            .ok_or_else(|| anyhow::anyhow!("Tier {} not configured", tier))?;

        self.execute_with_spec(spec, test_path)
    }

    /// Execute using a specific tier specification
    fn execute_with_spec(&self, spec: &TierSpec, test_path: &Path) -> Result<ExecutionResult> {
        let start = Instant::now();
        let timeout = Duration::from_millis(self.config.timeout_ms);

        // Build command
        let mut cmd = Command::new(&spec.binary_path);
        cmd.arg(test_path);
        cmd.args(&spec.extra_args);
        cmd.args(&self.config.extra_args);
        cmd.envs(&self.config.env_vars);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Spawn process
        let mut child = cmd.spawn()
            .with_context(|| format!("Failed to spawn {}", spec.binary_path.display()))?;

        // Set up output capture
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // Wait with timeout
        let result = self.wait_with_timeout(&mut child, timeout);
        let duration = start.elapsed();

        // Capture output
        let stdout = stdout_handle
            .map(|h| self.read_output(h))
            .unwrap_or_default();
        let stderr = stderr_handle
            .map(|h| self.read_output(h))
            .unwrap_or_default();

        match result {
            WaitResult::Exited(exit_code) => {
                Ok(ExecutionResult::success(spec.tier, exit_code, stdout, stderr, duration))
            }
            WaitResult::TimedOut => {
                // Kill the process
                let _ = child.kill();
                let _ = child.wait();
                Ok(ExecutionResult::timeout(spec.tier, stdout, stderr, duration))
            }
            WaitResult::Signaled(signal) => {
                Ok(ExecutionResult::crash(spec.tier, stdout, stderr, duration, Some(signal)))
            }
            WaitResult::Error(e) => {
                Err(anyhow::anyhow!("Execution error: {}", e))
            }
        }
    }

    /// Wait for process with timeout
    fn wait_with_timeout(
        &self,
        child: &mut std::process::Child,
        timeout: Duration,
    ) -> WaitResult {
        let start = Instant::now();

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if let Some(code) = status.code() {
                        return WaitResult::Exited(code);
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        if let Some(signal) = status.signal() {
                            return WaitResult::Signaled(signal);
                        }
                    }
                    return WaitResult::Exited(-1);
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        return WaitResult::TimedOut;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    return WaitResult::Error(e.to_string());
                }
            }
        }
    }

    /// Read output from a handle
    fn read_output<R: std::io::Read>(&self, reader: R) -> String {
        let reader = BufReader::new(reader);
        reader.lines()
            .filter_map(|l| l.ok())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Execute on multiple tiers in sequence
    pub fn execute_all(&self, tiers: &[Tier], test_path: &Path) -> Result<HashMap<Tier, ExecutionResult>> {
        let mut results = HashMap::new();

        for &tier in tiers {
            let result = self.execute(tier, test_path)?;
            results.insert(tier, result);
        }

        Ok(results)
    }

    /// Execute on multiple tiers in parallel
    pub fn execute_all_parallel(
        &self,
        tiers: &[Tier],
        test_path: &Path,
    ) -> Result<HashMap<Tier, ExecutionResult>> {
        use std::sync::Arc;
        use std::thread;

        let results = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let test_path = test_path.to_path_buf();

        thread::scope(|s| {
            for &tier in tiers {
                let results = Arc::clone(&results);
                let test_path = test_path.clone();
                let spec = self.tier_specs.get(&tier).cloned();
                let config = self.config.clone();

                s.spawn(move || {
                    if let Some(spec) = spec {
                        let executor = TierExecutor::new(config);
                        if let Ok(result) = executor.execute_with_spec(&spec, &test_path) {
                            let mut results = results.lock().unwrap();
                            results.insert(tier, result);
                        }
                    }
                });
            }
        });

        Ok(Arc::try_unwrap(results).unwrap().into_inner().unwrap())
    }
}

/// Result of waiting for a process
enum WaitResult {
    /// Process exited with code
    Exited(i32),
    /// Process was killed by signal
    Signaled(i32),
    /// Process timed out
    TimedOut,
    /// Error occurred
    Error(String),
}

/// Builder for execution options
pub struct ExecutionBuilder {
    tier: Tier,
    test_path: PathBuf,
    timeout_ms: Option<u64>,
    env_vars: HashMap<String, String>,
    extra_args: Vec<String>,
    stdin_input: Option<String>,
    working_dir: Option<PathBuf>,
}

impl ExecutionBuilder {
    /// Create a new builder
    pub fn new(tier: Tier, test_path: PathBuf) -> Self {
        Self {
            tier,
            test_path,
            timeout_ms: None,
            env_vars: HashMap::new(),
            extra_args: Vec::new(),
            stdin_input: None,
            working_dir: None,
        }
    }

    /// Set timeout
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Add environment variable
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Add extra argument
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Set stdin input
    pub fn stdin(mut self, input: impl Into<String>) -> Self {
        self.stdin_input = Some(input.into());
        self
    }

    /// Set working directory
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Execute the test
    pub fn execute(self, executor: &TierExecutor) -> Result<ExecutionResult> {
        // For now, just use the standard execute
        // In a full implementation, this would respect all builder options
        executor.execute(self.tier, &self.test_path)
    }
}

/// Helper to detect available tiers
pub fn detect_available_tiers() -> Vec<Tier> {
    let mut available = Vec::new();

    // Check for interpreter
    if which::which("verum-interpret").is_ok() {
        available.push(Tier::Interpreter);
    }

    // Check for bytecode VM
    if which::which("verum-bc").is_ok() {
        available.push(Tier::Bytecode);
    }

    // Check for JIT
    if which::which("verum-jit").is_ok() {
        available.push(Tier::Jit);
    }

    // Check for AOT
    if which::which("verum-run").is_ok() || which::which("verum").is_ok() {
        available.push(Tier::Aot);
    }

    available
}

/// Helper to get binary path for a tier
pub fn get_tier_binary(tier: Tier) -> Option<PathBuf> {
    match tier {
        Tier::Interpreter => which::which("verum-interpret").ok(),
        Tier::Bytecode => which::which("verum-bc").ok(),
        Tier::Jit => which::which("verum-jit").ok(),
        Tier::Aot => which::which("verum-run")
            .or_else(|_| which::which("verum"))
            .ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_result_success() {
        let result = ExecutionResult::success(
            Tier::Interpreter,
            0,
            "hello".to_string(),
            String::new(),
            Duration::from_millis(100),
        );

        assert!(result.success);
        assert!(!result.is_failure());
        assert_eq!(result.exit_code, Some(0));
        assert_eq!(result.stdout, "hello");
        assert!(!result.timed_out);
        assert!(!result.crashed);
    }

    #[test]
    fn test_execution_result_timeout() {
        let result = ExecutionResult::timeout(
            Tier::Aot,
            String::new(),
            String::new(),
            Duration::from_secs(30),
        );

        assert!(!result.success);
        assert!(result.is_failure());
        assert!(result.timed_out);
        assert!(!result.crashed);
    }

    #[test]
    fn test_execution_result_crash() {
        let result = ExecutionResult::crash(
            Tier::Jit,
            String::new(),
            "segfault".to_string(),
            Duration::from_millis(50),
            Some(11), // SIGSEGV
        );

        assert!(!result.success);
        assert!(result.is_failure());
        assert!(!result.timed_out);
        assert!(result.crashed);
        assert_eq!(result.signal, Some(11));
    }

    #[test]
    fn test_executor_config_default() {
        let config = ExecutorConfig::default();
        assert_eq!(config.timeout_ms, 30_000);
        assert!(config.bytecode_path.is_some());
        assert!(config.jit_path.is_some());
    }

    #[test]
    fn test_execution_builder() {
        let builder = ExecutionBuilder::new(Tier::Interpreter, PathBuf::from("test.vr"))
            .timeout(5000)
            .env("DEBUG", "1")
            .arg("--verbose");

        assert_eq!(builder.tier, Tier::Interpreter);
        assert_eq!(builder.timeout_ms, Some(5000));
        assert_eq!(builder.env_vars.get("DEBUG"), Some(&"1".to_string()));
        assert!(builder.extra_args.contains(&"--verbose".to_string()));
    }
}
