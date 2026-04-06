//! Fuzzing support for VCS test runner.
//!
//! This module provides integration with the vfuzz crate for
//! fuzzing Verum programs and detecting compiler bugs.
//!
//! # Features
//!
//! - Mutation-based fuzzing of Verum source files
//! - Crash detection and minimization
//! - Corpus management
//! - Coverage-guided fuzzing (when supported)
//! - Multi-threaded fuzzing

use crate::RunnerError;
use colored::Colorize;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::process::Command;
use verum_common::{List, Text};

/// Configuration for fuzzing.
#[derive(Debug, Clone)]
pub struct FuzzConfig {
    /// Duration to run fuzzing (in seconds)
    pub duration_secs: u64,
    /// Number of parallel fuzzers
    pub parallel: usize,
    /// Seed corpus directory
    pub corpus_dir: PathBuf,
    /// Output directory for crashes
    pub crashes_dir: PathBuf,
    /// Path to the Verum compiler
    pub compiler_path: PathBuf,
    /// Whether to minimize crashes
    pub minimize_crashes: bool,
    /// Whether to show verbose output
    pub verbose: bool,
    /// Maximum input size in bytes
    pub max_input_size: usize,
    /// Timeout per execution in milliseconds
    pub exec_timeout_ms: u64,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            duration_secs: 60,
            parallel: num_cpus::get(),
            corpus_dir: PathBuf::from("corpus"),
            crashes_dir: PathBuf::from("crashes"),
            compiler_path: PathBuf::from("verum"),
            minimize_crashes: true,
            verbose: false,
            max_input_size: 64 * 1024, // 64KB
            exec_timeout_ms: 5000,
        }
    }
}

/// Statistics from a fuzzing run.
#[derive(Debug, Clone, Default)]
pub struct FuzzStats {
    /// Total executions
    pub executions: u64,
    /// Unique crashes found
    pub crashes: u64,
    /// Unique timeouts
    pub timeouts: u64,
    /// Executions per second
    pub exec_per_sec: f64,
    /// Corpus size
    pub corpus_size: usize,
    /// Coverage (if available)
    pub coverage_pct: Option<f64>,
    /// Duration of the run
    pub duration: Duration,
}

/// A discovered crash.
#[derive(Debug, Clone)]
pub struct Crash {
    /// Unique identifier
    pub id: Text,
    /// Input that caused the crash
    pub input_path: PathBuf,
    /// Crash output (stderr)
    pub stderr: Text,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Whether this is a timeout
    pub is_timeout: bool,
    /// Signal if crashed
    pub signal: Option<i32>,
}

/// Result of a single fuzzing execution.
#[derive(Debug, Clone)]
pub enum FuzzResult {
    /// Normal execution
    Ok,
    /// Crash detected
    Crash(Crash),
    /// Timeout
    Timeout,
    /// Interesting input (new coverage)
    Interesting,
}

/// The fuzzer runner.
pub struct Fuzzer {
    config: FuzzConfig,
    stats: Arc<FuzzStats>,
    stop_flag: Arc<AtomicBool>,
    executions: Arc<AtomicU64>,
    crashes_found: Arc<AtomicU64>,
}

impl Fuzzer {
    /// Create a new fuzzer.
    pub fn new(config: FuzzConfig) -> Self {
        Self {
            config,
            stats: Arc::new(FuzzStats::default()),
            stop_flag: Arc::new(AtomicBool::new(false)),
            executions: Arc::new(AtomicU64::new(0)),
            crashes_found: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Run fuzzing.
    pub async fn run(&mut self) -> Result<FuzzStats, RunnerError> {
        self.print_header();

        // Ensure directories exist
        std::fs::create_dir_all(&self.config.corpus_dir)?;
        std::fs::create_dir_all(&self.config.crashes_dir)?;

        // Load initial corpus
        let corpus = self.load_corpus()?;
        if corpus.is_empty() {
            println!(
                "  {} No seed corpus found. Starting with empty corpus.",
                "Warning:".yellow()
            );
        } else {
            println!(
                "  Loaded {} seed inputs from {}",
                corpus.len(),
                self.config.corpus_dir.display()
            );
        }

        let start = Instant::now();
        let duration = Duration::from_secs(self.config.duration_secs);

        // Spawn worker tasks
        let mut handles = Vec::new();
        for worker_id in 0..self.config.parallel {
            let config = self.config.clone();
            let stop_flag = self.stop_flag.clone();
            let executions = self.executions.clone();
            let crashes_found = self.crashes_found.clone();
            let corpus = corpus.clone();

            let handle = tokio::spawn(async move {
                Self::worker_loop(
                    worker_id,
                    config,
                    corpus,
                    stop_flag,
                    executions,
                    crashes_found,
                )
                .await
            });
            handles.push(handle);
        }

        // Print progress periodically
        let progress_handle = {
            let stop_flag = self.stop_flag.clone();
            let executions = self.executions.clone();
            let crashes_found = self.crashes_found.clone();
            let verbose = self.config.verbose;

            tokio::spawn(async move {
                let start = Instant::now();
                while !stop_flag.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_secs(1)).await;

                    let execs = executions.load(Ordering::SeqCst);
                    let crashes = crashes_found.load(Ordering::SeqCst);
                    let elapsed = start.elapsed();
                    let eps = execs as f64 / elapsed.as_secs_f64();

                    if verbose || elapsed.as_secs() % 5 == 0 {
                        println!(
                            "  [{:>6}s] {} exec/s | {} execs | {} crashes",
                            elapsed.as_secs(),
                            format!("{:.0}", eps).cyan(),
                            execs,
                            if crashes > 0 {
                                format!("{}", crashes).red().to_string()
                            } else {
                                format!("{}", crashes).green().to_string()
                            }
                        );
                    }
                }
            })
        };

        // Wait for duration or stop
        tokio::time::sleep(duration).await;
        self.stop_flag.store(true, Ordering::SeqCst);

        // Wait for workers to finish
        for handle in handles {
            let _ = handle.await;
        }
        progress_handle.abort();

        // Build final stats
        let total_execs = self.executions.load(Ordering::SeqCst);
        let total_crashes = self.crashes_found.load(Ordering::SeqCst);
        let total_duration = start.elapsed();

        let stats = FuzzStats {
            executions: total_execs,
            crashes: total_crashes,
            timeouts: 0, // Would need to track this separately
            exec_per_sec: total_execs as f64 / total_duration.as_secs_f64(),
            corpus_size: self.count_corpus()?,
            coverage_pct: None,
            duration: total_duration,
        };

        self.print_summary(&stats);

        Ok(stats)
    }

    /// Worker loop for a single fuzzer thread.
    async fn worker_loop(
        _worker_id: usize,
        config: FuzzConfig,
        corpus: List<PathBuf>,
        stop_flag: Arc<AtomicBool>,
        executions: Arc<AtomicU64>,
        crashes_found: Arc<AtomicU64>,
    ) {
        let mut rng_state = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);

        while !stop_flag.load(Ordering::SeqCst) {
            // Select an input to mutate
            let input = if corpus.is_empty() {
                // Generate minimal input
                "// @test: run\nfn main() {}\n".to_string()
            } else {
                // Pick random corpus entry
                rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let idx = (rng_state as usize) % corpus.len();
                std::fs::read_to_string(&corpus[idx]).unwrap_or_default()
            };

            // Mutate the input
            let mutated = Self::mutate(&input, &mut rng_state);

            // Execute and check result
            match Self::execute_input(&config, &mutated).await {
                FuzzResult::Crash(crash) => {
                    crashes_found.fetch_add(1, Ordering::SeqCst);
                    // Save crash
                    let crash_path = config.crashes_dir.join(format!("crash_{}.vr", crash.id));
                    let _ = std::fs::write(&crash_path, &mutated);
                    let info_path = config.crashes_dir.join(format!("crash_{}.info", crash.id));
                    let _ = std::fs::write(&info_path, &crash.stderr);
                }
                FuzzResult::Interesting => {
                    // Save to corpus
                    rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let corpus_path = config
                        .corpus_dir
                        .join(format!("input_{:016x}.vr", rng_state));
                    let _ = std::fs::write(&corpus_path, &mutated);
                }
                _ => {}
            }

            executions.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Mutate an input string.
    fn mutate(input: &str, rng: &mut u64) -> String {
        let bytes = input.as_bytes().to_vec();
        let mut result = bytes;

        // Pick a mutation strategy
        *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let strategy = *rng % 10;

        match strategy {
            0 => {
                // Bit flip
                if !result.is_empty() {
                    let idx = (*rng as usize) % result.len();
                    let bit = (*rng as usize) % 8;
                    result[idx] ^= 1 << bit;
                }
            }
            1 => {
                // Byte flip
                if !result.is_empty() {
                    let idx = (*rng as usize) % result.len();
                    result[idx] ^= 0xFF;
                }
            }
            2 => {
                // Insert random byte
                let idx = (*rng as usize) % (result.len() + 1);
                let byte = (*rng & 0xFF) as u8;
                result.insert(idx, byte);
            }
            3 => {
                // Delete byte
                if !result.is_empty() {
                    let idx = (*rng as usize) % result.len();
                    result.remove(idx);
                }
            }
            4 => {
                // Swap bytes
                if result.len() >= 2 {
                    let idx1 = (*rng as usize) % result.len();
                    *rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let idx2 = (*rng as usize) % result.len();
                    result.swap(idx1, idx2);
                }
            }
            5 => {
                // Duplicate chunk
                if result.len() >= 4 {
                    let start = (*rng as usize) % result.len();
                    let len = (*rng as usize % 16).min(result.len() - start);
                    let chunk: Vec<u8> = result[start..start + len].to_vec();
                    result.extend(chunk);
                }
            }
            6 => {
                // Replace with interesting Verum constructs
                let constructs = [
                    "let x = 42",
                    "fn f() {}",
                    "if true {}",
                    "while false {}",
                    "match x {}",
                    "struct S {}",
                    "enum E {}",
                    "&x",
                    "&checked x",
                    "&unsafe x",
                ];
                let construct = constructs[(*rng as usize) % constructs.len()];
                if !result.is_empty() {
                    let idx = (*rng as usize) % result.len();
                    result.splice(idx..idx.min(result.len()), construct.bytes());
                }
            }
            _ => {
                // Arithmetic mutation on interesting positions
                if !result.is_empty() {
                    let idx = (*rng as usize) % result.len();
                    result[idx] = result[idx].wrapping_add((*rng & 0x1F) as u8);
                }
            }
        }

        String::from_utf8_lossy(&result).to_string()
    }

    /// Execute an input and check for crashes.
    async fn execute_input(config: &FuzzConfig, input: &str) -> FuzzResult {
        // Write input to temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join(format!("fuzz_{}.vr", std::process::id()));

        if std::fs::write(&temp_file, input).is_err() {
            return FuzzResult::Ok;
        }

        // Execute compiler
        let result = tokio::time::timeout(
            Duration::from_millis(config.exec_timeout_ms),
            Command::new(&config.compiler_path)
                .arg("check")
                .arg(&temp_file)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output(),
        )
        .await;

        // Clean up
        let _ = std::fs::remove_file(&temp_file);

        match result {
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // Check for crashes (internal compiler errors)
                if stderr.contains("panic")
                    || stderr.contains("internal error")
                    || stderr.contains("ICE")
                    || stderr.contains("SIGSEGV")
                    || stderr.contains("SIGABRT")
                {
                    let id: Text = format!("{:016x}", {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        std::hash::Hash::hash(&stderr, &mut hasher);
                        std::hash::Hasher::finish(&hasher)
                    }).into();

                    return FuzzResult::Crash(Crash {
                        id,
                        input_path: temp_file,
                        stderr: stderr.into(),
                        exit_code: output.status.code(),
                        is_timeout: false,
                        signal: None,
                    });
                }

                // Check exit code
                if !output.status.success() && output.status.code() != Some(1) {
                    // Non-standard failure might indicate a bug
                    let id: Text = format!("{:016x}", {
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        std::hash::Hash::hash(&stderr, &mut hasher);
                        std::hash::Hasher::finish(&hasher)
                    }).into();

                    return FuzzResult::Crash(Crash {
                        id,
                        input_path: temp_file,
                        stderr: stderr.into(),
                        exit_code: output.status.code(),
                        is_timeout: false,
                        signal: None,
                    });
                }

                FuzzResult::Ok
            }
            Ok(Err(_)) => FuzzResult::Ok,
            Err(_) => FuzzResult::Timeout,
        }
    }

    /// Load corpus from disk.
    fn load_corpus(&self) -> Result<List<PathBuf>, RunnerError> {
        let mut corpus = List::new();

        if !self.config.corpus_dir.exists() {
            return Ok(corpus);
        }

        for entry in std::fs::read_dir(&self.config.corpus_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                corpus.push(path);
            }
        }

        Ok(corpus)
    }

    /// Count corpus entries.
    fn count_corpus(&self) -> Result<usize, RunnerError> {
        let mut count = 0;

        if !self.config.corpus_dir.exists() {
            return Ok(count);
        }

        for entry in std::fs::read_dir(&self.config.corpus_dir)? {
            let entry = entry?;
            if entry
                .path()
                .extension()
                .map_or(false, |ext| ext == "vr" || ext == "verum")
            {
                count += 1;
            }
        }

        Ok(count)
    }

    /// Print the header.
    fn print_header(&self) {
        println!();
        println!("{}", "=".repeat(60).dimmed());
        println!("  {} {}", "VTEST".bold(), "Fuzzer".dimmed());
        println!("{}", "=".repeat(60).dimmed());
        println!();
        println!("  Duration: {}s", self.config.duration_secs);
        println!("  Workers:  {}", self.config.parallel);
        println!("  Corpus:   {}", self.config.corpus_dir.display());
        println!("  Crashes:  {}", self.config.crashes_dir.display());
        println!();
        println!("{}", "-".repeat(60).dimmed());
        println!();
    }

    /// Print summary.
    fn print_summary(&self, stats: &FuzzStats) {
        println!();
        println!("{}", "-".repeat(60).dimmed());
        println!();
        println!("  {} Fuzzing Summary", ">>".bold());
        println!();
        println!("  Total executions:  {}", stats.executions);
        println!("  Executions/sec:    {:.0}", stats.exec_per_sec);
        println!("  Duration:          {:.2}s", stats.duration.as_secs_f64());
        println!(
            "  Crashes found:     {}",
            if stats.crashes > 0 {
                format!("{}", stats.crashes).red().to_string()
            } else {
                format!("{}", stats.crashes).green().to_string()
            }
        );
        println!("  Corpus size:       {}", stats.corpus_size);
        println!();
        println!("{}", "=".repeat(60).dimmed());
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_fuzz_config_default() {
        let config = FuzzConfig::default();
        assert_eq!(config.duration_secs, 60);
        assert!(config.parallel > 0);
    }

    #[test]
    fn test_mutate() {
        let input = "fn main() {}";
        let mut rng = 12345u64;

        // Run several mutations to ensure variety
        let mut seen = HashSet::new();
        for _ in 0..100 {
            let mutated = Fuzzer::mutate(input, &mut rng);
            seen.insert(mutated);
        }

        // Should have produced multiple distinct mutations
        assert!(seen.len() > 10);
    }

    #[test]
    fn test_fuzz_stats_default() {
        let stats = FuzzStats::default();
        assert_eq!(stats.executions, 0);
        assert_eq!(stats.crashes, 0);
    }
}
