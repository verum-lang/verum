//! VCS Proof Stability CLI
//!
//! Command-line interface for running proof stability tests.

// VCS proof stability infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

use clap::{Parser, Subcommand};
use colored::Colorize;
use proof_stability::{
    StabilityError,
    config::StabilityConfig,
    report::StabilityReportFormat,
    runner::ProofStabilityRunner,
};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "proof-stability")]
#[command(about = "VCS Proof Stability Testing Tool", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Configuration file path
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run stability tests
    Run {
        /// Test paths (overrides config)
        #[arg(short, long)]
        path: Vec<PathBuf>,

        /// Glob pattern for test files
        #[arg(short = 'g', long, default_value = "**/*.vr")]
        pattern: String,

        /// Number of stability runs per proof
        #[arg(short = 'n', long, default_value = "5")]
        runs: usize,

        /// Random seed for reproducibility
        #[arg(short, long)]
        seed: Option<u64>,

        /// Output format (console, json, html, markdown)
        #[arg(short, long, default_value = "console")]
        format: String,

        /// Output file (stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Baseline file for regression detection
        #[arg(short, long)]
        baseline: Option<PathBuf>,

        /// Fail if stability is below threshold (0-100)
        #[arg(long, default_value = "95")]
        threshold: f64,
    },

    /// Create a stability baseline
    Baseline {
        /// Test paths
        #[arg(short, long)]
        path: Vec<PathBuf>,

        /// Output baseline file
        #[arg(short, long, default_value = "stability_baseline.json")]
        output: PathBuf,

        /// Compiler version tag
        #[arg(long)]
        version: Option<String>,
    },

    /// Compare current results to baseline
    Compare {
        /// Current results file
        #[arg(short, long)]
        current: PathBuf,

        /// Baseline file
        #[arg(short, long)]
        baseline: PathBuf,

        /// Output format
        #[arg(short, long, default_value = "console")]
        format: String,
    },

    /// Show cache statistics
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },

    /// List flaky proofs from cache
    Flaky {
        /// Minimum flakiness threshold (0-100)
        #[arg(short, long, default_value = "80")]
        threshold: f64,
    },
}

#[derive(Subcommand)]
enum CacheAction {
    /// Show cache statistics
    Stats,
    /// Clear the cache
    Clear,
    /// Export cache to JSON
    Export {
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("{}: {}", "Error".red().bold(), e);
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, StabilityError> {
    // Load config
    let mut config = if let Some(ref path) = cli.config {
        StabilityConfig::from_file(path)?
    } else {
        StabilityConfig::load_default().unwrap_or_default()
    };

    config.reporting.verbose = cli.verbose;

    match cli.command {
        Commands::Run {
            path,
            pattern,
            runs,
            seed,
            format,
            output,
            baseline,
            threshold,
        } => {
            // Override config with CLI options
            if !path.is_empty() {
                config.execution.test_paths = path.into();
            }
            config.execution.test_pattern = pattern.into();
            config.execution.stability_runs = runs;
            if let Some(s) = seed {
                config.solver.random_seed = Some(s);
            }
            if let Some(ref b) = baseline {
                config.reporting.baseline_path = Some(b.clone());
            }

            // Run stability tests
            let mut runner = ProofStabilityRunner::new(config.clone());
            runner.initialize().await?;

            let report = runner.run_and_report().await?;

            // Output report
            let format = StabilityReportFormat::from_str(&format).ok_or_else(|| {
                StabilityError::ConfigError(format!("Unknown format: {}", format).into())
            })?;

            if let Some(ref path) = output {
                let mut file = std::fs::File::create(path)?;
                report.generate(&mut file, format)?;
                if cli.verbose {
                    println!("Report written to: {}", path.display());
                }
            } else {
                let mut stdout = std::io::stdout();
                report.generate(&mut stdout, format)?;
            }

            // Save cache
            runner.save_cache()?;

            // Determine exit code
            if report.metrics.overall_stability < threshold {
                Ok(ExitCode::FAILURE)
            } else if report.exit_code != 0 {
                Ok(ExitCode::FAILURE)
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }

        Commands::Baseline {
            path,
            output,
            version,
        } => {
            if !path.is_empty() {
                config.execution.test_paths = path.into();
            }

            let mut runner = ProofStabilityRunner::new(config);
            runner.initialize().await?;

            let (_metrics, _) = runner.run().await?;

            // Create baseline from metrics
            // This would require converting StabilityMetrics back to ProofMetrics
            // For now, print a placeholder message
            println!("{}", "Baseline creation not yet fully implemented".yellow());
            println!("Would save baseline to: {}", output.display());
            if let Some(v) = version {
                println!("Version: {}", v);
            }

            Ok(ExitCode::SUCCESS)
        }

        Commands::Compare {
            current,
            baseline,
            format: _,
        } => {
            println!("{}", "Comparison not yet fully implemented".yellow());
            println!("Current: {}", current.display());
            println!("Baseline: {}", baseline.display());

            Ok(ExitCode::SUCCESS)
        }

        Commands::Cache { action } => match action {
            CacheAction::Stats => {
                let cache = proof_stability::cache::ProofCache::new(config.cache);
                let stats = cache.get_statistics();

                println!("{}", "Proof Cache Statistics".bold());
                println!("─────────────────────────");
                println!("Total proofs: {}", stats.total_proofs);
                println!("Stable: {}", stats.stable_proofs.to_string().green());
                println!("Flaky: {}", stats.flaky_proofs.to_string().red());
                println!("Unknown: {}", stats.unknown_proofs);
                println!("Stability: {:.1}%", stats.stability_percentage);

                Ok(ExitCode::SUCCESS)
            }
            CacheAction::Clear => {
                let mut cache = proof_stability::cache::ProofCache::new(config.cache);
                cache.clear();
                cache.save()?;
                println!("{}", "Cache cleared".green());
                Ok(ExitCode::SUCCESS)
            }
            CacheAction::Export { output } => {
                println!("Would export cache to: {}", output.display());
                Ok(ExitCode::SUCCESS)
            }
        },

        Commands::Flaky { threshold: _ } => {
            let mut cache = proof_stability::cache::ProofCache::new(config.cache);
            cache.load()?;

            let flaky = cache.get_flaky_proofs();

            if flaky.is_empty() {
                println!("{}", "No flaky proofs found".green());
            } else {
                println!("{} flaky proofs found:", flaky.len());
                println!();

                for entry in flaky {
                    println!("{} {}", "!".red().bold(), entry.proof_id.source_path);
                    println!("  Stability: {:.1}%", entry.stability_percentage);
                    println!("  Status: {}", entry.stability_status);
                    println!("  Attempts: {}", entry.attempts.len());
                    println!();
                }
            }

            Ok(ExitCode::SUCCESS)
        }
    }
}
