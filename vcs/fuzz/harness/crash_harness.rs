//! Crash detection harness for Verum
//!
//! This module detects crashes, panics, and undefined behavior in the
//! Verum compiler and runtime. It monitors for:
//!
//! - Segmentation faults
//! - Stack overflows
//! - Assertion failures
//! - Panics in Rust code
//! - Hangs/infinite loops
//! - Memory corruption
//!
//! # Architecture
//!
//! The harness runs each test in a subprocess to isolate crashes,
//! captures crash information, and provides tools for reproducing
//! and minimizing crash cases.

use std::collections::HashMap;
use std::fmt;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Types of crashes that can be detected
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CrashType {
    /// Segmentation fault (SIGSEGV)
    Segfault,
    /// Abort signal (SIGABRT)
    Abort,
    /// Bus error (SIGBUS)
    BusError,
    /// Floating point exception (SIGFPE)
    FloatingPointException,
    /// Illegal instruction (SIGILL)
    IllegalInstruction,
    /// Stack overflow
    StackOverflow,
    /// Rust panic
    Panic { message: String },
    /// Assertion failure
    AssertionFailure { condition: String },
    /// Timeout/hang
    Timeout { duration: Duration },
    /// Out of memory
    OutOfMemory,
    /// Address sanitizer error
    AddressSanitizerError { kind: String },
    /// Memory sanitizer error
    MemorySanitizerError { kind: String },
    /// Thread sanitizer error
    ThreadSanitizerError { kind: String },
    /// Undefined behavior sanitizer error
    UBSanError { kind: String },
    /// Unknown crash
    Unknown { exit_code: i32, signal: Option<i32> },
}

impl fmt::Display for CrashType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrashType::Segfault => write!(f, "Segmentation fault"),
            CrashType::Abort => write!(f, "Aborted"),
            CrashType::BusError => write!(f, "Bus error"),
            CrashType::FloatingPointException => write!(f, "Floating point exception"),
            CrashType::IllegalInstruction => write!(f, "Illegal instruction"),
            CrashType::StackOverflow => write!(f, "Stack overflow"),
            CrashType::Panic { message } => write!(f, "Panic: {}", message),
            CrashType::AssertionFailure { condition } => {
                write!(f, "Assertion failed: {}", condition)
            }
            CrashType::Timeout { duration } => write!(f, "Timeout after {:?}", duration),
            CrashType::OutOfMemory => write!(f, "Out of memory"),
            CrashType::AddressSanitizerError { kind } => write!(f, "ASan: {}", kind),
            CrashType::MemorySanitizerError { kind } => write!(f, "MSan: {}", kind),
            CrashType::ThreadSanitizerError { kind } => write!(f, "TSan: {}", kind),
            CrashType::UBSanError { kind } => write!(f, "UBSan: {}", kind),
            CrashType::Unknown { exit_code, signal } => {
                write!(
                    f,
                    "Unknown crash (exit: {}, signal: {:?})",
                    exit_code, signal
                )
            }
        }
    }
}

/// Information about a crash
#[derive(Debug, Clone)]
pub struct CrashInfo {
    /// Type of crash
    pub crash_type: CrashType,
    /// Source code that triggered the crash
    pub source: String,
    /// Stack trace if available
    pub stack_trace: Option<String>,
    /// Stderr output
    pub stderr: String,
    /// Stdout output
    pub stdout: String,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Signal that killed the process
    pub signal: Option<i32>,
    /// Time when crash occurred
    pub timestamp: Instant,
    /// Compiler phase where crash occurred
    pub phase: Option<CompilerPhase>,
    /// Minimized source (if minimization was performed)
    pub minimized_source: Option<String>,
}

/// Compiler phases for identifying where crashes occur
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompilerPhase {
    Lexing,
    Parsing,
    TypeChecking,
    BorrowChecking,
    CodeGeneration,
    Optimization,
    Linking,
    Runtime,
}

impl fmt::Display for CompilerPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompilerPhase::Lexing => write!(f, "Lexing"),
            CompilerPhase::Parsing => write!(f, "Parsing"),
            CompilerPhase::TypeChecking => write!(f, "Type Checking"),
            CompilerPhase::BorrowChecking => write!(f, "Borrow Checking"),
            CompilerPhase::CodeGeneration => write!(f, "Code Generation"),
            CompilerPhase::Optimization => write!(f, "Optimization"),
            CompilerPhase::Linking => write!(f, "Linking"),
            CompilerPhase::Runtime => write!(f, "Runtime"),
        }
    }
}

/// Configuration for the crash harness
#[derive(Debug, Clone)]
pub struct CrashConfig {
    /// Path to the Verum compiler binary
    pub compiler_path: PathBuf,
    /// Timeout for compilation
    pub compile_timeout: Duration,
    /// Timeout for execution
    pub run_timeout: Duration,
    /// Maximum memory for the process (bytes)
    pub max_memory: usize,
    /// Whether to enable address sanitizer
    pub enable_asan: bool,
    /// Whether to enable memory sanitizer
    pub enable_msan: bool,
    /// Whether to enable thread sanitizer
    pub enable_tsan: bool,
    /// Whether to enable undefined behavior sanitizer
    pub enable_ubsan: bool,
    /// Working directory for temporary files
    pub work_dir: PathBuf,
    /// Whether to auto-minimize crashing inputs
    pub auto_minimize: bool,
    /// Number of parallel workers
    pub num_workers: usize,
}

impl Default for CrashConfig {
    fn default() -> Self {
        Self {
            compiler_path: PathBuf::from("verum"),
            compile_timeout: Duration::from_secs(30),
            run_timeout: Duration::from_secs(10),
            max_memory: 1024 * 1024 * 1024, // 1GB
            enable_asan: true,
            enable_msan: false,
            enable_tsan: false,
            enable_ubsan: true,
            work_dir: std::env::temp_dir().join("verum_fuzz"),
            auto_minimize: true,
            num_workers: num_cpus::get(),
        }
    }
}

/// Crash detection harness
pub struct CrashHarness {
    config: CrashConfig,
    crashes: Arc<Mutex<Vec<CrashInfo>>>,
    crash_dedup: Arc<Mutex<HashMap<String, usize>>>,
}

impl CrashHarness {
    /// Create a new crash harness
    pub fn new(config: CrashConfig) -> std::io::Result<Self> {
        // Create work directory
        std::fs::create_dir_all(&config.work_dir)?;

        Ok(Self {
            config,
            crashes: Arc::new(Mutex::new(Vec::new())),
            crash_dedup: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Test a single source for crashes
    pub fn test(&self, source: &str) -> Option<CrashInfo> {
        let result = self.run_in_subprocess(source);

        if let Some(mut crash) = result {
            // Deduplicate
            let crash_key = self.compute_crash_key(&crash);
            {
                let mut dedup = self.crash_dedup.lock().unwrap();
                *dedup.entry(crash_key.clone()).or_insert(0) += 1;
            }

            // Minimize if configured
            if self.config.auto_minimize {
                crash.minimized_source = self.minimize(source);
            }

            // Record crash
            {
                let mut crashes = self.crashes.lock().unwrap();
                crashes.push(crash.clone());
            }

            Some(crash)
        } else {
            None
        }
    }

    /// Run source in a subprocess and detect crashes
    fn run_in_subprocess(&self, source: &str) -> Option<CrashInfo> {
        // Write source to temp file
        let source_path = self.config.work_dir.join("test.vr");
        if std::fs::write(&source_path, source).is_err() {
            return None;
        }

        // Build command
        let mut cmd = Command::new(&self.config.compiler_path);
        cmd.arg("run")
            .arg(&source_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Add sanitizer flags
        let mut env_vars = Vec::new();
        if self.config.enable_asan {
            env_vars.push(("VERUM_SANITIZER", "address"));
        }
        if self.config.enable_ubsan {
            env_vars.push(("VERUM_SANITIZER", "undefined"));
        }

        for (key, value) in &env_vars {
            cmd.env(key, value);
        }

        // Set resource limits
        // (platform-specific, simplified here)

        // Spawn process
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(_) => return None,
        };

        // Wait with timeout
        let result = self.wait_with_timeout(child);

        // Analyze result
        self.analyze_result(result, source)
    }

    /// Wait for child process with timeout
    fn wait_with_timeout(&self, mut child: Child) -> ProcessResult {
        let start = Instant::now();
        let timeout = self.config.compile_timeout + self.config.run_timeout;

        // Read stdout/stderr in background
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let (stdout_tx, stdout_rx) = channel();
        let (stderr_tx, stderr_rx) = channel();

        if let Some(stdout) = stdout {
            let tx = stdout_tx;
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                let output: String = reader.lines().filter_map(|l| l.ok()).collect();
                let _ = tx.send(output);
            });
        }

        if let Some(stderr) = stderr {
            let tx = stderr_tx;
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                let output: String = reader.lines().filter_map(|l| l.ok()).collect();
                let _ = tx.send(output);
            });
        }

        // Poll for completion
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = stdout_rx
                        .recv_timeout(Duration::from_secs(1))
                        .unwrap_or_default();
                    let stderr = stderr_rx
                        .recv_timeout(Duration::from_secs(1))
                        .unwrap_or_default();

                    return ProcessResult {
                        exit_code: status.code(),
                        signal: None, // platform-specific
                        stdout,
                        stderr,
                        timed_out: false,
                        duration: start.elapsed(),
                    };
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        let _ = child.kill();
                        return ProcessResult {
                            exit_code: None,
                            signal: Some(9), // SIGKILL
                            stdout: stdout_rx
                                .recv_timeout(Duration::from_millis(100))
                                .unwrap_or_default(),
                            stderr: stderr_rx
                                .recv_timeout(Duration::from_millis(100))
                                .unwrap_or_default(),
                            timed_out: true,
                            duration: start.elapsed(),
                        };
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    return ProcessResult {
                        exit_code: None,
                        signal: None,
                        stdout: String::new(),
                        stderr: String::new(),
                        timed_out: false,
                        duration: start.elapsed(),
                    };
                }
            }
        }
    }

    /// Analyze process result to detect crash type
    fn analyze_result(&self, result: ProcessResult, source: &str) -> Option<CrashInfo> {
        // Check for timeout
        if result.timed_out {
            return Some(CrashInfo {
                crash_type: CrashType::Timeout {
                    duration: result.duration,
                },
                source: source.to_string(),
                stack_trace: None,
                stderr: result.stderr,
                stdout: result.stdout,
                exit_code: result.exit_code,
                signal: result.signal,
                timestamp: Instant::now(),
                phase: None,
                minimized_source: None,
            });
        }

        // Check exit code - 0 means success
        if result.exit_code == Some(0) {
            return None;
        }

        // Analyze crash type from signal/exit code and output
        let crash_type = self.determine_crash_type(&result);

        // Extract stack trace
        let stack_trace = self.extract_stack_trace(&result.stderr);

        // Determine compiler phase
        let phase = self.determine_phase(&result.stderr);

        Some(CrashInfo {
            crash_type,
            source: source.to_string(),
            stack_trace,
            stderr: result.stderr,
            stdout: result.stdout,
            exit_code: result.exit_code,
            signal: result.signal,
            timestamp: Instant::now(),
            phase,
            minimized_source: None,
        })
    }

    /// Determine crash type from process result
    fn determine_crash_type(&self, result: &ProcessResult) -> CrashType {
        // Check for sanitizer errors in stderr
        if result.stderr.contains("AddressSanitizer") {
            let kind = self.extract_asan_kind(&result.stderr);
            return CrashType::AddressSanitizerError { kind };
        }

        if result.stderr.contains("MemorySanitizer") {
            let kind = self.extract_msan_kind(&result.stderr);
            return CrashType::MemorySanitizerError { kind };
        }

        if result.stderr.contains("ThreadSanitizer") {
            let kind = self.extract_tsan_kind(&result.stderr);
            return CrashType::ThreadSanitizerError { kind };
        }

        if result.stderr.contains("UndefinedBehaviorSanitizer")
            || result.stderr.contains("runtime error:")
        {
            let kind = self.extract_ubsan_kind(&result.stderr);
            return CrashType::UBSanError { kind };
        }

        // Check for Rust panic
        if result.stderr.contains("panicked at") {
            let message = self.extract_panic_message(&result.stderr);
            return CrashType::Panic { message };
        }

        // Check for assertion failure
        if result.stderr.contains("assertion failed") || result.stderr.contains("assert!") {
            let condition = self.extract_assertion(&result.stderr);
            return CrashType::AssertionFailure { condition };
        }

        // Check for stack overflow
        if result.stderr.contains("stack overflow")
            || result.stderr.contains("SIGSEGV") && result.stderr.contains("stack")
        {
            return CrashType::StackOverflow;
        }

        // Check for OOM
        if result.stderr.contains("out of memory")
            || result.stderr.contains("memory allocation failed")
        {
            return CrashType::OutOfMemory;
        }

        // Check signal
        match result.signal {
            Some(11) => CrashType::Segfault,
            Some(6) => CrashType::Abort,
            Some(7) => CrashType::BusError,
            Some(8) => CrashType::FloatingPointException,
            Some(4) => CrashType::IllegalInstruction,
            _ => CrashType::Unknown {
                exit_code: result.exit_code.unwrap_or(-1),
                signal: result.signal,
            },
        }
    }

    fn extract_asan_kind(&self, stderr: &str) -> String {
        // Extract ASan error type
        if stderr.contains("heap-buffer-overflow") {
            "heap-buffer-overflow".to_string()
        } else if stderr.contains("stack-buffer-overflow") {
            "stack-buffer-overflow".to_string()
        } else if stderr.contains("use-after-free") {
            "use-after-free".to_string()
        } else if stderr.contains("double-free") {
            "double-free".to_string()
        } else if stderr.contains("heap-use-after-free") {
            "heap-use-after-free".to_string()
        } else if stderr.contains("stack-use-after-return") {
            "stack-use-after-return".to_string()
        } else if stderr.contains("memory leak") {
            "memory-leak".to_string()
        } else {
            "unknown".to_string()
        }
    }

    fn extract_msan_kind(&self, stderr: &str) -> String {
        if stderr.contains("uninitialized") {
            "use-of-uninitialized-value".to_string()
        } else {
            "unknown".to_string()
        }
    }

    fn extract_tsan_kind(&self, stderr: &str) -> String {
        if stderr.contains("data race") {
            "data-race".to_string()
        } else if stderr.contains("lock-order-inversion") {
            "lock-order-inversion".to_string()
        } else if stderr.contains("deadlock") {
            "deadlock".to_string()
        } else {
            "unknown".to_string()
        }
    }

    fn extract_ubsan_kind(&self, stderr: &str) -> String {
        if stderr.contains("signed integer overflow") {
            "signed-integer-overflow".to_string()
        } else if stderr.contains("unsigned integer overflow") {
            "unsigned-integer-overflow".to_string()
        } else if stderr.contains("division by zero") {
            "division-by-zero".to_string()
        } else if stderr.contains("null pointer") {
            "null-pointer-dereference".to_string()
        } else if stderr.contains("misaligned") {
            "misaligned-access".to_string()
        } else if stderr.contains("out of bounds") {
            "array-bounds".to_string()
        } else if stderr.contains("shift") {
            "shift-overflow".to_string()
        } else {
            "unknown".to_string()
        }
    }

    fn extract_panic_message(&self, stderr: &str) -> String {
        // Extract panic message from Rust-style panic output
        stderr
            .lines()
            .find(|line| line.contains("panicked at"))
            .map(|line| line.to_string())
            .unwrap_or_else(|| "unknown panic".to_string())
    }

    fn extract_assertion(&self, stderr: &str) -> String {
        stderr
            .lines()
            .find(|line| line.contains("assertion") || line.contains("assert"))
            .map(|line| line.to_string())
            .unwrap_or_else(|| "unknown assertion".to_string())
    }

    fn extract_stack_trace(&self, stderr: &str) -> Option<String> {
        // Look for stack trace patterns
        let stack_start = stderr
            .find("stack trace:")
            .or_else(|| stderr.find("backtrace:"));

        if let Some(start) = stack_start {
            let trace = &stderr[start..];
            // Find end of stack trace (empty line or known terminator)
            let end = trace
                .find("\n\n")
                .or_else(|| trace.find("note:"))
                .unwrap_or(trace.len());
            Some(trace[..end].to_string())
        } else {
            None
        }
    }

    fn determine_phase(&self, stderr: &str) -> Option<CompilerPhase> {
        if stderr.contains("lexing") || stderr.contains("tokenizing") {
            Some(CompilerPhase::Lexing)
        } else if stderr.contains("parsing") || stderr.contains("syntax error") {
            Some(CompilerPhase::Parsing)
        } else if stderr.contains("type checking") || stderr.contains("type error") {
            Some(CompilerPhase::TypeChecking)
        } else if stderr.contains("borrow") || stderr.contains("lifetime") {
            Some(CompilerPhase::BorrowChecking)
        } else if stderr.contains("code generation")
            || stderr.contains("codegen")
            || stderr.contains("LLVM")
        {
            Some(CompilerPhase::CodeGeneration)
        } else if stderr.contains("optimization") || stderr.contains("opt pass") {
            Some(CompilerPhase::Optimization)
        } else if stderr.contains("linking") || stderr.contains("linker") {
            Some(CompilerPhase::Linking)
        } else if stderr.contains("runtime") || stderr.contains("execution") {
            Some(CompilerPhase::Runtime)
        } else {
            None
        }
    }

    /// Compute a key for crash deduplication
    fn compute_crash_key(&self, crash: &CrashInfo) -> String {
        // Use crash type + first few stack frames
        let mut key = format!("{:?}", crash.crash_type);

        if let Some(ref trace) = crash.stack_trace {
            // Take first 5 frames
            for (i, line) in trace.lines().take(5).enumerate() {
                key.push_str(&format!(":{}:{}", i, line.trim()));
            }
        }

        key
    }

    /// Minimize a crashing input
    pub fn minimize(&self, source: &str) -> Option<String> {
        // Verify it actually crashes
        if self.run_in_subprocess(source).is_none() {
            return None;
        }

        let mut current = source.to_string();

        // Delta debugging - remove lines
        loop {
            let lines: Vec<&str> = current.lines().collect();
            if lines.len() <= 1 {
                break;
            }

            let mut made_progress = false;

            for i in 0..lines.len() {
                let candidate: String = lines
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, line)| *line)
                    .collect::<Vec<_>>()
                    .join("\n");

                if !candidate.is_empty() && self.run_in_subprocess(&candidate).is_some() {
                    current = candidate;
                    made_progress = true;
                    break;
                }
            }

            if !made_progress {
                break;
            }
        }

        // Delta debugging - remove tokens/characters
        // (simplified - could be more aggressive)

        Some(current)
    }

    /// Get all recorded crashes
    pub fn get_crashes(&self) -> Vec<CrashInfo> {
        self.crashes.lock().unwrap().clone()
    }

    /// Get crash statistics
    pub fn get_stats(&self) -> CrashStats {
        let crashes = self.crashes.lock().unwrap();
        let dedup = self.crash_dedup.lock().unwrap();

        let mut by_type: HashMap<String, usize> = HashMap::new();
        let mut by_phase: HashMap<Option<CompilerPhase>, usize> = HashMap::new();

        for crash in crashes.iter() {
            *by_type
                .entry(format!("{:?}", crash.crash_type))
                .or_insert(0) += 1;
            *by_phase.entry(crash.phase).or_insert(0) += 1;
        }

        CrashStats {
            total_crashes: crashes.len(),
            unique_crashes: dedup.len(),
            by_type,
            by_phase,
        }
    }

    /// Save crashes to disk
    pub fn save_crashes(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;

        let crashes = self.crashes.lock().unwrap();
        for (i, crash) in crashes.iter().enumerate() {
            let crash_dir = dir.join(format!("crash_{:04}", i));
            std::fs::create_dir_all(&crash_dir)?;

            // Save source
            std::fs::write(crash_dir.join("source.vr"), &crash.source)?;

            // Save minimized if available
            if let Some(ref minimized) = crash.minimized_source {
                std::fs::write(crash_dir.join("minimized.vr"), minimized)?;
            }

            // Save crash info
            let info = format!(
                "Crash Type: {}\nPhase: {:?}\nExit Code: {:?}\nSignal: {:?}\n\nStderr:\n{}\n\nStack Trace:\n{}\n",
                crash.crash_type,
                crash.phase,
                crash.exit_code,
                crash.signal,
                crash.stderr,
                crash.stack_trace.as_deref().unwrap_or("N/A")
            );
            std::fs::write(crash_dir.join("info.txt"), info)?;
        }

        Ok(())
    }
}

/// Result of running a subprocess
struct ProcessResult {
    exit_code: Option<i32>,
    signal: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    duration: Duration,
}

/// Crash statistics
#[derive(Debug)]
pub struct CrashStats {
    pub total_crashes: usize,
    pub unique_crashes: usize,
    pub by_type: HashMap<String, usize>,
    pub by_phase: HashMap<Option<CompilerPhase>, usize>,
}

// External crate for CPU count
mod num_cpus {
    pub fn get() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_type_display() {
        assert_eq!(format!("{}", CrashType::Segfault), "Segmentation fault");
        assert_eq!(
            format!(
                "{}",
                CrashType::Panic {
                    message: "test".to_string()
                }
            ),
            "Panic: test"
        );
    }

    #[test]
    fn test_crash_key_generation() {
        let config = CrashConfig::default();
        let harness = CrashHarness::new(config).unwrap();

        let crash = CrashInfo {
            crash_type: CrashType::Segfault,
            source: "test".to_string(),
            stack_trace: Some("frame1\nframe2\nframe3".to_string()),
            stderr: String::new(),
            stdout: String::new(),
            exit_code: Some(139),
            signal: Some(11),
            timestamp: Instant::now(),
            phase: Some(CompilerPhase::Runtime),
            minimized_source: None,
        };

        let key = harness.compute_crash_key(&crash);
        assert!(key.contains("Segfault"));
    }
}
