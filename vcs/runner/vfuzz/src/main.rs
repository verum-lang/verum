//! vfuzz - VCS fuzzer for property-based testing of the Verum compiler
//!
//! This is the main CLI entry point for the fuzzer.
//!
//! # Usage
//!
//! ```bash
//! # Run fuzzing with default settings
//! vfuzz run
//!
//! # Run with specific iteration count
//! vfuzz run --iterations 10000
//!
//! # Run differential testing only
//! vfuzz run --differential
//!
//! # Minimize a crash
//! vfuzz shrink path/to/crash.vr
//!
//! # Generate random programs
//! vfuzz generate --count 100 --output ./generated
//!
//! # Show corpus statistics
//! vfuzz corpus stats
//!
//! # Import seeds
//! vfuzz corpus import ./seeds
//! ```

// VCS fuzzer infrastructure - suppress clippy warnings for test tooling
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

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use verum_vfuzz::{
    FuzzConfig, FuzzEngine, FuzzStats,
    corpus::CorpusManager,
    generator::{Generator, GeneratorConfig, GeneratorKind},
    shrink::{ShrinkConfig, ShrinkResult, ShrinkStrategy, Shrinker},
};

/// VCS Fuzzer - Property-based testing for the Verum compiler
#[derive(Parser)]
#[command(name = "vfuzz")]
#[command(author = "Verum Team")]
#[command(version = "0.1.0")]
#[command(about = "Property-based fuzz testing for the Verum compiler", long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Output format (text, json)
    #[arg(long, global = true, default_value = "text")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "text" => Ok(OutputFormat::Text),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!("Unknown format: {}", s)),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Run the fuzzer
    Run {
        /// Number of iterations (0 = infinite)
        #[arg(short, long, default_value = "0")]
        iterations: usize,

        /// Number of parallel workers
        #[arg(short, long)]
        workers: Option<usize>,

        /// Timeout per test in milliseconds
        #[arg(long, default_value = "10000")]
        timeout: u64,

        /// Directory for crash artifacts
        #[arg(long, default_value = "vcs/fuzz/crashes")]
        crash_dir: PathBuf,

        /// Directory for corpus
        #[arg(long, default_value = "vcs/fuzz/corpus")]
        corpus_dir: PathBuf,

        /// Directory for seed inputs
        #[arg(long, default_value = "vcs/fuzz/seeds")]
        seed_dir: PathBuf,

        /// Enable differential testing (Tier 0 vs Tier 3)
        #[arg(long)]
        differential: bool,

        /// Minimize crashing inputs
        #[arg(long)]
        minimize: bool,

        /// Random seed for reproducibility
        #[arg(long)]
        seed: Option<u64>,

        /// Maximum program size in bytes
        #[arg(long, default_value = "100000")]
        max_size: usize,

        /// Maximum AST depth
        #[arg(long, default_value = "10")]
        max_depth: usize,
    },

    /// Shrink a failing test case
    Shrink {
        /// Path to the input file
        input: PathBuf,

        /// Output path for minimized input
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Maximum shrink iterations
        #[arg(long, default_value = "1000")]
        max_iterations: usize,

        /// Shrinking strategy
        #[arg(long, default_value = "combined")]
        strategy: String,
    },

    /// Generate random programs
    Generate {
        /// Number of programs to generate
        #[arg(short, long, default_value = "10")]
        count: usize,

        /// Output directory
        #[arg(short, long, default_value = "./generated")]
        output: PathBuf,

        /// Generator type (grammar, type-aware, edge-case, mixed)
        #[arg(long, default_value = "mixed")]
        generator: String,

        /// Maximum AST depth
        #[arg(long, default_value = "10")]
        max_depth: usize,

        /// Maximum statements per function
        #[arg(long, default_value = "50")]
        max_statements: usize,

        /// Random seed
        #[arg(long)]
        seed: Option<u64>,
    },

    /// Corpus management
    Corpus {
        #[command(subcommand)]
        action: CorpusAction,
    },

    /// Show fuzzer statistics
    Stats {
        /// Path to stats file
        #[arg(default_value = "vcs/fuzz/stats.json")]
        path: PathBuf,
    },

    /// Generate a fuzzing report
    Report {
        /// Input directory or stats file
        #[arg(default_value = "vcs/fuzz")]
        input: PathBuf,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Report format (text, json, html, markdown)
        #[arg(long, default_value = "text")]
        format: String,

        /// Include crash details
        #[arg(long, default_value = "true")]
        include_crashes: bool,

        /// Include coverage information
        #[arg(long, default_value = "true")]
        include_coverage: bool,
    },

    /// Minimize a crashing input
    Minimize {
        /// Path to the crashing input
        input: PathBuf,

        /// Output path for minimized input
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Maximum shrink iterations
        #[arg(long, default_value = "1000")]
        max_iterations: usize,
    },

    /// Reproduce a crash from a saved artifact
    Reproduce {
        /// Path to the crash artifact
        crash: PathBuf,

        /// Execution tier (0, 1, 2, 3)
        #[arg(long, default_value = "0")]
        tier: u8,

        /// Verbose output
        #[arg(long)]
        trace: bool,
    },
}

#[derive(Subcommand)]
enum CorpusAction {
    /// Show corpus statistics
    Stats {
        /// Corpus directory
        #[arg(default_value = "vcs/fuzz/corpus")]
        dir: PathBuf,
    },

    /// Import seeds from a directory
    Import {
        /// Source directory
        source: PathBuf,

        /// Corpus directory
        #[arg(long, default_value = "vcs/fuzz/corpus")]
        corpus_dir: PathBuf,
    },

    /// Cull corpus to reduce size
    Cull {
        /// Corpus directory
        #[arg(default_value = "vcs/fuzz/corpus")]
        dir: PathBuf,

        /// Maximum number of entries to keep
        #[arg(long, default_value = "1000")]
        max_size: usize,
    },

    /// List corpus entries
    List {
        /// Corpus directory
        #[arg(default_value = "vcs/fuzz/corpus")]
        dir: PathBuf,

        /// Maximum entries to show
        #[arg(long, default_value = "20")]
        limit: usize,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Initialize logging
    let filter = if cli.verbose { "debug" } else { "info" };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::new(filter))
        .init();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}: {}", "Error".red().bold(), e);
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Run {
            iterations,
            workers,
            timeout,
            crash_dir,
            corpus_dir,
            seed_dir,
            differential,
            minimize,
            seed,
            max_size,
            max_depth,
        } => run_fuzzer(
            iterations,
            workers,
            timeout,
            crash_dir,
            corpus_dir,
            seed_dir,
            differential,
            minimize,
            seed,
            max_size,
            max_depth,
            cli.verbose,
            cli.format,
        ),

        Commands::Shrink {
            input,
            output,
            max_iterations,
            strategy,
        } => shrink_input(input, output, max_iterations, strategy, cli.format),

        Commands::Generate {
            count,
            output,
            generator,
            max_depth,
            max_statements,
            seed,
        } => generate_programs(count, output, generator, max_depth, max_statements, seed),

        Commands::Corpus { action } => match action {
            CorpusAction::Stats { dir } => corpus_stats(dir, cli.format),
            CorpusAction::Import { source, corpus_dir } => corpus_import(source, corpus_dir),
            CorpusAction::Cull { dir, max_size } => corpus_cull(dir, max_size),
            CorpusAction::List { dir, limit } => corpus_list(dir, limit),
        },

        Commands::Stats { path } => show_stats(path, cli.format),

        Commands::Report { .. } => {
            eprintln!("Report command not yet implemented");
            Ok(())
        }

        Commands::Minimize { .. } => {
            eprintln!("Minimize command not yet implemented");
            Ok(())
        }

        Commands::Reproduce { .. } => {
            eprintln!("Reproduce command not yet implemented");
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_fuzzer(
    iterations: usize,
    workers: Option<usize>,
    timeout: u64,
    crash_dir: PathBuf,
    corpus_dir: PathBuf,
    seed_dir: PathBuf,
    differential: bool,
    minimize: bool,
    seed: Option<u64>,
    max_size: usize,
    max_depth: usize,
    verbose: bool,
    format: OutputFormat,
) -> Result<()> {
    println!("{}", "VCS Fuzzer v0.1.0".cyan().bold());
    println!();

    let config = FuzzConfig {
        iterations,
        timeout_ms: timeout,
        workers: workers.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        }),
        crash_dir: crash_dir.clone(),
        corpus_dir: corpus_dir.clone(),
        minimize,
        max_program_size: max_size,
        seed,
        differential,
        max_depth,
        verbose,
        ..Default::default()
    };

    println!("{}", "Configuration:".yellow());
    println!(
        "  Iterations:   {}",
        if iterations == 0 {
            "infinite".to_string()
        } else {
            iterations.to_string()
        }
    );
    println!("  Workers:      {}", config.workers);
    println!("  Timeout:      {}ms", timeout);
    println!(
        "  Differential: {}",
        if differential { "enabled" } else { "disabled" }
    );
    println!(
        "  Minimize:     {}",
        if minimize { "enabled" } else { "disabled" }
    );
    println!("  Crash dir:    {:?}", crash_dir);
    println!("  Corpus dir:   {:?}", corpus_dir);
    println!("  Seed dir:     {:?}", seed_dir);
    println!();

    let mut engine = FuzzEngine::new(config).context("Failed to create fuzz engine")?;

    // Load seeds
    let seed_count = engine
        .load_seeds(&seed_dir)
        .context("Failed to load seeds")?;
    println!("{} {} seed files", "Loaded".green(), seed_count);
    println!();

    // Set up progress bar
    let pb = if iterations > 0 {
        let pb = ProgressBar::new(iterations as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({per_sec}) | crashes: {msg}")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    println!("{}", "Starting fuzzer...".green().bold());
    println!();

    // Handle Ctrl+C
    let running = engine.running.clone();
    ctrlc::set_handler(move || {
        running.store(false, std::sync::atomic::Ordering::SeqCst);
    })?;

    let stats = engine.run(iterations)?;

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    println!();
    print_stats(&stats, format);

    // Save stats
    let stats_path = corpus_dir.join("stats.json");
    let stats_json = serde_json::to_string_pretty(&stats)?;
    std::fs::write(&stats_path, stats_json)?;
    println!("\nStats saved to {:?}", stats_path);

    Ok(())
}

fn shrink_input(
    input: PathBuf,
    output: Option<PathBuf>,
    max_iterations: usize,
    strategy: String,
    format: OutputFormat,
) -> Result<()> {
    println!("{}", "VCS Fuzzer - Shrinking".cyan().bold());
    println!();

    let content = std::fs::read_to_string(&input)
        .with_context(|| format!("Failed to read input file: {:?}", input))?;

    println!("Input size: {} bytes", content.len());

    let strategy = match strategy.to_lowercase().as_str() {
        "binary" => ShrinkStrategy::BinarySearch,
        "ddmin" | "delta" => ShrinkStrategy::DeltaDebugging,
        "lines" => ShrinkStrategy::LineByLine,
        "tokens" => ShrinkStrategy::TokenByToken,
        "hierarchical" => ShrinkStrategy::Hierarchical,
        _ => ShrinkStrategy::Combined,
    };

    let config = ShrinkConfig {
        max_iterations,
        strategy,
        ..Default::default()
    };

    let shrinker = Shrinker::new(config);

    // Simple test function (for demonstration - real one would test the compiler)
    let test_fn = |s: &str| !s.is_empty();

    println!("Shrinking...");
    let (result, stats) = shrinker.shrink_with_stats(&content, test_fn);

    match result {
        ShrinkResult::Success(minimized) => {
            println!();
            println!("{}", "Shrinking successful!".green().bold());
            println!("Original size: {} bytes", stats.original_size);
            println!("Minimized size: {} bytes", stats.final_size);
            println!("Reduction: {:.1}%", stats.reduction_pct());
            println!("Duration: {}ms", stats.duration_ms);

            let output_path = output.unwrap_or_else(|| {
                let stem = input.file_stem().unwrap().to_string_lossy();
                input.with_file_name(format!("{}_minimized.vr", stem))
            });

            std::fs::write(&output_path, &minimized)?;
            println!("\nMinimized output written to: {:?}", output_path);

            if format == OutputFormat::Json {
                let json = serde_json::json!({
                    "success": true,
                    "original_size": stats.original_size,
                    "final_size": stats.final_size,
                    "reduction_pct": stats.reduction_pct(),
                    "output_path": output_path.to_string_lossy(),
                });
                println!("\n{}", serde_json::to_string_pretty(&json)?);
            }
        }
        ShrinkResult::NoProgress => {
            println!("{}", "No reduction possible".yellow());
        }
        ShrinkResult::Error(e) => {
            println!("{}: {}", "Error".red(), e);
        }
    }

    Ok(())
}

fn generate_programs(
    count: usize,
    output: PathBuf,
    generator: String,
    max_depth: usize,
    max_statements: usize,
    seed: Option<u64>,
) -> Result<()> {
    println!("{}", "VCS Fuzzer - Generator".cyan().bold());
    println!();

    std::fs::create_dir_all(&output)?;

    let kind = match generator.to_lowercase().as_str() {
        "grammar" => GeneratorKind::Grammar,
        "type-aware" | "typed" => GeneratorKind::TypeAware,
        "edge-case" | "edge" => GeneratorKind::EdgeCase,
        _ => GeneratorKind::Mixed,
    };

    let config = GeneratorConfig {
        max_depth,
        max_statements,
        kind,
        ..Default::default()
    };

    let mut generator = Generator::new(config);

    let seed_value = seed.unwrap_or_else(|| rand::random());
    let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed_value);

    println!("Generator: {:?}", kind);
    println!("Max depth: {}", max_depth);
    println!("Max statements: {}", max_statements);
    println!("Seed: {}", seed_value);
    println!();

    let pb = ProgressBar::new(count as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
            .unwrap()
            .progress_chars("#>-"),
    );

    for i in 0..count {
        let program = generator.generate(&mut rng);
        let path = output.join(format!("generated_{:06}.vr", i));
        std::fs::write(&path, program)?;
        pb.inc(1);
    }

    pb.finish_and_clear();

    println!("{} Generated {} programs", "Done!".green(), count);
    println!("Output directory: {:?}", output);

    Ok(())
}

fn corpus_stats(dir: PathBuf, format: OutputFormat) -> Result<()> {
    let manager = CorpusManager::new(&dir)?;
    let stats = manager.stats();

    if format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("{}", "Corpus Statistics".cyan().bold());
        println!();
        println!("Total entries:    {}", stats.total_entries);
        println!("Total size:       {} bytes", stats.total_size);
        println!("Average size:     {:.1} bytes", stats.average_size);
        println!("Seed entries:     {}", stats.seed_entries);
        println!("Mutation entries: {}", stats.mutation_entries);
        println!("Unique coverage:  {}", stats.unique_coverage);
    }

    Ok(())
}

fn corpus_import(source: PathBuf, corpus_dir: PathBuf) -> Result<()> {
    println!("{}", "Importing seeds...".cyan());

    let mut manager = CorpusManager::new(&corpus_dir)?;
    let count = manager.load_seeds(&source)?;

    println!("{} {} files from {:?}", "Imported".green(), count, source);

    Ok(())
}

fn corpus_cull(dir: PathBuf, max_size: usize) -> Result<()> {
    println!("{}", "Culling corpus...".cyan());

    let mut manager = CorpusManager::new(&dir)?;
    let before = manager.len();

    manager.cull(max_size);

    let after = manager.len();
    println!(
        "{} Reduced from {} to {} entries",
        "Done!".green(),
        before,
        after
    );

    Ok(())
}

fn corpus_list(dir: PathBuf, limit: usize) -> Result<()> {
    let manager = CorpusManager::new(&dir)?;

    println!("{}", "Corpus Entries".cyan().bold());
    println!();

    let mut count = 0;
    for entry in std::fs::read_dir(&dir)? {
        if count >= limit {
            break;
        }

        let entry = entry?;
        let path = entry.path();

        if path.extension().map_or(false, |ext| ext == "vr") {
            let size = std::fs::metadata(&path)?.len();
            let name = path.file_name().unwrap().to_string_lossy();
            println!("  {} ({} bytes)", name, size);
            count += 1;
        }
    }

    if manager.len() > limit {
        println!("  ... and {} more", manager.len() - limit);
    }

    Ok(())
}

fn show_stats(path: PathBuf, format: OutputFormat) -> Result<()> {
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read stats file: {:?}", path))?;

    let stats: FuzzStats = serde_json::from_str(&content)?;

    if format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        print_stats(&stats, format);
    }

    Ok(())
}

fn print_stats(stats: &FuzzStats, format: OutputFormat) {
    if format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&stats).unwrap());
        return;
    }

    println!("{}", "Fuzzing Results".cyan().bold());
    println!("{}", "=".repeat(40));
    println!();
    println!("Iterations:       {}", stats.iterations);
    println!("Duration:         {:.2}s", stats.duration_secs);
    println!("Throughput:       {:.1} tests/sec", stats.throughput);
    println!();
    println!("{}", "Issues Found".yellow());
    println!("  Crashes:        {}", stats.crashes);
    println!("  Unique crashes: {}", stats.unique_crashes);
    println!("  Diff bugs:      {}", stats.differential_bugs);
    println!("  Timeouts:       {}", stats.timeouts);
    println!();
    println!("{}", "Coverage".yellow());
    println!("  Interesting:    {}", stats.interesting_inputs);
    println!("  Corpus size:    {}", stats.corpus_size);
    if let Some(pct) = stats.coverage_pct {
        println!("  Coverage:       {:.1}%", pct);
    }
}

// Need to use rand_chacha for seeded RNG
use rand::SeedableRng;
