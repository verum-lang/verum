//! Verum Fuzz Testing Framework
//!
//! This crate provides comprehensive fuzzing infrastructure for the Verum
//! language compiler and runtime. It includes:
//!
//! - **Generators**: Create random valid/invalid programs
//! - **Harnesses**: Execute and analyze programs for issues
//! - **Seeds**: Minimal test corpus for fuzzing
//!
//! # Quick Start
//!
//! ```rust
//! use verum_fuzz::{generators::GrammarGenerator, harness::UnifiedHarness};
//! use rand::rng;
//!
//! // Create a generator and harness
//! let generator = GrammarGenerator::builder()
//!     .max_depth(5)
//!     .max_statements(20)
//!     .build();
//!
//! let harness = UnifiedHarness::new().unwrap();
//!
//! // Generate and test programs
//! let mut rng = rng();
//! for _ in 0..100 {
//!     let program = generator.generate_program(&mut rng);
//!     let result = harness.test(&program);
//!
//!     if result.has_issues() {
//!         println!("Found issue: {:?}", result);
//!     }
//! }
//! ```
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                        verum_fuzz                               │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  ┌─────────────────────────────────────────────────────────┐   │
//! │  │                     GENERATORS                           │   │
//! │  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐     │   │
//! │  │  │   Grammar    │ │  Type-Aware  │ │  Refinement  │     │   │
//! │  │  └──────────────┘ └──────────────┘ └──────────────┘     │   │
//! │  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐     │   │
//! │  │  │    Async     │ │ CBGR Stress  │ │   Mutation   │     │   │
//! │  │  └──────────────┘ └──────────────┘ └──────────────┘     │   │
//! │  └─────────────────────────────────────────────────────────┘   │
//! │                              │                                  │
//! │                              ▼                                  │
//! │  ┌─────────────────────────────────────────────────────────┐   │
//! │  │                      HARNESSES                           │   │
//! │  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────┐     │   │
//! │  │  │ Differential │ │    Crash     │ │   Memory     │     │   │
//! │  │  │   (Tier 0-3) │ │  Detection   │ │   (CBGR)     │     │   │
//! │  │  └──────────────┘ └──────────────┘ └──────────────┘     │   │
//! │  └─────────────────────────────────────────────────────────┘   │
//! │                              │                                  │
//! │                              ▼                                  │
//! │  ┌─────────────────────────────────────────────────────────┐   │
//! │  │                   SEED CORPUS                            │   │
//! │  │  minimal.vr  primitives.vr  control_flow.vr  ...        │   │
//! │  └─────────────────────────────────────────────────────────┘   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Generators
//!
//! | Generator | Purpose | Output |
//! |-----------|---------|--------|
//! | `GrammarGenerator` | Syntax-valid programs | Parseable Verum |
//! | `TypeAwareGenerator` | Type-correct programs | Type-checked Verum |
//! | `RefinementGenerator` | Refinement type tests | SMT-verified Verum |
//! | `AsyncGenerator` | Async/await patterns | Concurrent Verum |
//! | `CbgrStressGenerator` | Memory pressure tests | CBGR edge cases |
//! | `MutationFuzzer` | Mutate existing code | Variants of inputs |
//!
//! # Harnesses
//!
//! | Harness | Purpose | Detection |
//! |---------|---------|-----------|
//! | `DifferentialHarness` | Cross-tier comparison | Semantic bugs |
//! | `CrashHarness` | Crash detection | Panics, segfaults |
//! | `MemoryHarness` | Memory safety | CBGR violations |
//! | `UnifiedHarness` | All of the above | Comprehensive |
//!
//! # Performance Targets
//!
//! - Generator throughput: > 1000 programs/sec
//! - Harness latency: < 100ms per test
//! - Memory overhead: < 100MB per worker
//! - CBGR validation: < 15ns per reference

// VCS fuzz testing infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]
#![allow(missing_docs)]
#![allow(private_interfaces)]
#![allow(deprecated)]

pub mod generators;
pub mod harness;

// Re-export commonly used types
pub use generators::{
    AsyncConfig, AsyncGenerator, CbgrStressConfig, CbgrStressGenerator, CombinedGenerator,
    GrammarConfig, GrammarGenerator, MutationConfig, MutationFuzzer, MutationType,
    ProgramGenerator, RefinementConfig, RefinementGenerator, TypeAwareConfig, TypeAwareGenerator,
};

pub use harness::{
    CrashConfig, CrashHarness, CrashInfo, CrashType, DiffError, DifferentialConfig,
    DifferentialHarness, MemoryConfig, MemoryHarness, MemoryIssue, MemoryReport, Tier,
    UnifiedHarness, UnifiedResult, UnifiedStats, Value,
};

use rand::Rng;
use std::path::Path;
use std::time::{Duration, Instant};

/// Fuzzing campaign configuration
#[derive(Debug, Clone)]
pub struct FuzzConfig {
    /// Number of iterations to run
    pub iterations: usize,
    /// Timeout per test
    pub timeout: Duration,
    /// Number of parallel workers
    pub workers: usize,
    /// Directory to save crashes
    pub crash_dir: Option<std::path::PathBuf>,
    /// Directory to save interesting inputs
    pub corpus_dir: Option<std::path::PathBuf>,
    /// Whether to minimize crashing inputs
    pub minimize: bool,
    /// Maximum program size to generate
    pub max_program_size: usize,
    /// Random seed (None for random)
    pub seed: Option<u64>,
}

impl Default for FuzzConfig {
    fn default() -> Self {
        Self {
            iterations: 10_000,
            timeout: Duration::from_secs(10),
            workers: num_cpus(),
            crash_dir: None,
            corpus_dir: None,
            minimize: true,
            max_program_size: 10_000,
            seed: None,
        }
    }
}

/// Statistics from a fuzzing campaign
#[derive(Debug, Default)]
pub struct FuzzStats {
    /// Total iterations run
    pub iterations: usize,
    /// Total time spent
    pub duration: Duration,
    /// Number of crashes found
    pub crashes: usize,
    /// Number of unique crashes
    pub unique_crashes: usize,
    /// Number of differential bugs
    pub differential_bugs: usize,
    /// Number of memory issues
    pub memory_issues: usize,
    /// Number of timeouts
    pub timeouts: usize,
    /// Programs per second
    pub throughput: f64,
}

/// Run a fuzzing campaign
pub fn run_campaign<R: Rng>(config: FuzzConfig, rng: &mut R) -> std::io::Result<FuzzStats> {
    let start = Instant::now();
    let mut stats = FuzzStats::default();

    // Create generators and harness
    let generator = CombinedGenerator::new();
    let harness = UnifiedHarness::new()?;

    // Create output directories
    if let Some(ref crash_dir) = config.crash_dir {
        std::fs::create_dir_all(crash_dir)?;
    }
    if let Some(ref corpus_dir) = config.corpus_dir {
        std::fs::create_dir_all(corpus_dir)?;
    }

    // Run iterations
    for i in 0..config.iterations {
        let (program, _generator_name) = generator.generate(rng);

        // Skip if program is too large
        if program.len() > config.max_program_size {
            continue;
        }

        // Test the program
        let result = harness.test(&program);
        stats.iterations += 1;

        // Process results
        if result.has_issues() {
            if result.crash_info.is_some() {
                stats.crashes += 1;

                // Save crash
                if let Some(ref crash_dir) = config.crash_dir {
                    let path = crash_dir.join(format!("crash_{:06}.vr", i));
                    let _ = std::fs::write(path, &program);
                }
            }

            if result.differential_result.is_err() {
                stats.differential_bugs += 1;
            }

            stats.memory_issues += result.memory_issues.len();
        }

        // Progress reporting
        if i % 1000 == 0 && i > 0 {
            let elapsed = start.elapsed().as_secs_f64();
            let throughput = i as f64 / elapsed;
            eprintln!(
                "Progress: {}/{} ({:.1}/s), crashes: {}, diff bugs: {}, memory: {}",
                i,
                config.iterations,
                throughput,
                stats.crashes,
                stats.differential_bugs,
                stats.memory_issues
            );
        }
    }

    stats.duration = start.elapsed();
    stats.throughput = stats.iterations as f64 / stats.duration.as_secs_f64();

    Ok(stats)
}

/// Load seed corpus from directory
pub fn load_seeds(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut seeds = Vec::new();

    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "vr") {
                let content = std::fs::read_to_string(&path)?;
                seeds.push(content);
            }
        }
    }

    Ok(seeds)
}

/// Get number of CPUs
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_basic_fuzzing() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let generator = GrammarGenerator::builder()
            .max_depth(3)
            .max_statements(5)
            .build();

        for _ in 0..10 {
            let program = generator.generate_program(&mut rng);
            assert!(!program.is_empty());
            assert!(program.contains("fn"));
        }
    }

    #[test]
    fn test_fuzz_config_defaults() {
        let config = FuzzConfig::default();
        assert_eq!(config.iterations, 10_000);
        assert!(config.minimize);
    }
}
