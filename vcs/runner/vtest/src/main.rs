//! VCS Test Runner CLI
//!
//! The `vtest` command-line tool for running Verum Compliance Suite tests.
//!
//! # Usage
//!
//! ```bash
//! # Run all tests
//! vtest run
//!
//! # Run tests at a specific level
//! vtest run --level L0
//! vtest run --level L1,L2
//!
//! # Run tests on a specific tier
//! vtest run --tier 0
//! vtest run --tier 3
//! vtest run --tier all
//!
//! # Filter by tags
//! vtest run --tags cbgr,memory-safety
//! vtest run --exclude-tags slow,gpu
//!
//! # Parallel execution
//! vtest run --parallel 8
//! vtest run --parallel auto
//!
//! # Generate reports
//! vtest run --format json --output results.json
//! vtest run --format html --output report.html
//! vtest run --format junit --output junit.xml
//! vtest run --format markdown --output report.md
//! vtest run --format tap --output results.tap
//!
//! # List tests
//! vtest list
//! vtest list --level L0 --tags cbgr
//!
//! # Update expectations
//! vtest run --update-expectations
//!
//! # Quiet mode (only show failures)
//! vtest run --quiet
//!
//! # Show only summary
//! vtest run --summary-only
//! ```

#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

use clap::{Parser, Subcommand, ValueEnum};
use colored::Colorize;
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::{debug, info, warn};
use verum_common::{FileId, Set, Text};
use vtest::{
    RunnerError, VTestRunner, VTestToml,
    directive::{Level, Tier},
    list_tests,
    report::ReportFormat,
};

/// Output verbosity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Verbosity {
    /// Show everything including debug output
    Debug,
    /// Normal output with all test results
    Normal,
    /// Show only failures and summary
    Quiet,
    /// Show only the final summary
    Summary,
}

/// Options for running tests (collected from CLI args).
#[derive(Debug)]
#[allow(dead_code)] // Some fields reserved for future implementation
struct RunOptions {
    paths: Vec<PathBuf>,
    level: Vec<Text>,
    tier: Vec<Text>,
    tags: Vec<Text>,
    exclude_tags: Vec<Text>,
    parallel: Text,
    format: Text,
    output: Option<PathBuf>,
    timeout: u64,
    fail_fast: bool,
    update_expectations: bool,
    summary_only: bool,
    filter: Option<Text>,
    retries: u32,
    save_baseline: Option<PathBuf>,
    compare_baseline: Option<PathBuf>,
    coverage: bool,
    show_diff: bool,
    isolation: Text,
    compile_time_only: bool,
    vbc_output: Option<PathBuf>,
    vbc_preserve_paths: bool,
}

/// Initialize logging based on verbosity level.
fn init_logging(verbosity: Verbosity, json_log: bool) {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};

    let filter = match verbosity {
        Verbosity::Debug => "debug",
        Verbosity::Normal => "info",
        Verbosity::Quiet => "warn",
        Verbosity::Summary => "error",
    };

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter));

    if json_log {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_target(false).without_time())
            .init();
    }
}

/// VCS Test Runner - The Verum Compliance Suite test execution tool
#[derive(Parser, Debug)]
#[command(name = "vtest")]
#[command(author = "Verum Team")]
#[command(version)]
#[command(about = "Verum Compliance Suite test runner", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Verbose output (shorthand for --verbosity=debug)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Quiet mode (shorthand for --verbosity=quiet)
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,

    /// Output verbosity level
    #[arg(long, global = true, value_enum, default_value = "normal")]
    verbosity: Verbosity,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    /// Enable JSON logging for machine parsing
    #[arg(long, global = true)]
    json_log: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run tests
    Run {
        /// Test paths or patterns
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Test level filter (L0, L1, L2, L3, L4)
        #[arg(short, long, value_delimiter = ',')]
        level: Vec<Text>,

        /// Execution tier filter (0, 1, 2, 3, all, compiled)
        #[arg(short, long, value_delimiter = ',')]
        tier: Vec<Text>,

        /// Include only tests with these tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<Text>,

        /// Exclude tests with these tags
        #[arg(long, value_delimiter = ',')]
        exclude_tags: Vec<Text>,

        /// Number of parallel test processes
        #[arg(short, long, default_value = "auto")]
        parallel: Text,

        /// Output format (console, json, html, junit, tap, markdown)
        #[arg(short, long, default_value = "console")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Default timeout in milliseconds
        #[arg(long, default_value = "30000")]
        timeout: u64,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,

        /// Update expected output files with actual results
        #[arg(long)]
        update_expectations: bool,

        /// Show only summary (no individual test results)
        #[arg(long)]
        summary_only: bool,

        /// Filter by test name pattern (supports glob patterns)
        #[arg(long)]
        filter: Option<Text>,

        /// Retry failed tests N times
        #[arg(long, default_value = "0")]
        retries: u32,

        /// Save test results to a baseline file for comparison
        #[arg(long)]
        save_baseline: Option<PathBuf>,

        /// Compare results against a baseline file
        #[arg(long)]
        compare_baseline: Option<PathBuf>,

        /// Generate coverage information
        #[arg(long)]
        coverage: bool,

        /// Show diffs for failures
        #[arg(long, default_value = "true")]
        show_diff: bool,

        /// Isolation level (none, process, directory, container)
        #[arg(long, default_value = "process")]
        isolation: Text,

        /// Compile-time only mode: Run/RunPanic tests verify only typecheck
        /// This is useful for verifying compile-time correctness during VBC migration
        #[arg(long)]
        compile_time_only: bool,

        /// Generate VBC output for tests
        /// When specified, VBC bytecode will be saved to this directory
        #[arg(long)]
        vbc_output: Option<PathBuf>,

        /// Save VBC files with test-relative paths (instead of flat)
        #[arg(long)]
        vbc_preserve_paths: bool,
    },

    /// List discovered tests
    List {
        /// Test paths or patterns
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Test level filter
        #[arg(short, long, value_delimiter = ',')]
        level: Vec<Text>,

        /// Include only tests with these tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<Text>,

        /// Output format (console, json)
        #[arg(short, long, default_value = "console")]
        format: Text,
    },

    /// Generate test report from previous run
    Report {
        /// Input file (JSON from previous run)
        #[arg(value_name = "INPUT")]
        input: PathBuf,

        /// Output format (console, html, junit)
        #[arg(short, long, default_value = "html")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Watch for file changes and re-run tests
    Watch {
        /// Paths to watch
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Test level filter
        #[arg(short, long, value_delimiter = ',')]
        level: Vec<Text>,

        /// Include only tests with these tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<Text>,
    },

    /// Run fuzz tests
    Fuzz {
        /// Duration in seconds
        #[arg(short, long, default_value = "60")]
        duration: u64,

        /// Number of parallel fuzzers
        #[arg(short, long, default_value = "auto")]
        parallel: Text,

        /// Seed corpus directory
        #[arg(long)]
        corpus: Option<PathBuf>,

        /// Output directory for crashes
        #[arg(long)]
        crashes: Option<PathBuf>,
    },

    /// Run benchmarks
    Bench {
        /// Benchmark patterns
        #[arg(value_name = "PATTERN")]
        patterns: Vec<Text>,

        /// Baseline version to compare against
        #[arg(long)]
        baseline: Option<Text>,

        /// Compare with other languages (rust, go, c)
        #[arg(long, value_delimiter = ',')]
        compare: Vec<Text>,

        /// Output format (console, json)
        #[arg(short, long, default_value = "console")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Test stdlib compilation pipeline
    ///
    /// Parses and type-checks all stdlib modules (stdlib/**/*.vr) to verify
    /// they can be compiled. Reports detailed statistics by module and phase.
    /// Use this to iteratively fix stdlib syntax and implement missing features.
    Stdlib {
        /// Stdlib directory path
        #[arg(value_name = "PATH", default_value = "stdlib")]
        path: PathBuf,

        /// Number of parallel compilation processes
        #[arg(short, long, default_value = "auto")]
        parallel: Text,

        /// Output format (console, json, markdown)
        #[arg(short, long, default_value = "console")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,

        /// Show only summary (no individual file results)
        #[arg(long)]
        summary_only: bool,

        /// Show first N failures in detail (default: 20)
        #[arg(long, default_value = "20")]
        show_failures: usize,

        /// Include parse phase only (skip typecheck)
        #[arg(long)]
        parse_only: bool,

        /// Filter files by module name pattern
        #[arg(long)]
        filter: Option<Text>,
    },

    /// Run parser tests from vcs/specs/parser
    ///
    /// Tests both success (parse-pass) and failure (parse-fail) cases.
    /// By default uses verum_fast_parser. Use --advanced to test verum_parser.
    Parser {
        /// Test paths or patterns (defaults to specs/parser)
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Include only tests with these tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<Text>,

        /// Exclude tests with these tags
        #[arg(long, value_delimiter = ',')]
        exclude_tags: Vec<Text>,

        /// Number of parallel test processes
        #[arg(short, long, default_value = "auto")]
        parallel: Text,

        /// Output format (console, json, markdown)
        #[arg(short, long, default_value = "console")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Default timeout in milliseconds
        #[arg(long, default_value = "10000")]
        timeout: u64,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,

        /// Show only summary (no individual test results)
        #[arg(long)]
        summary_only: bool,

        /// Show first N failures in detail (default: 20)
        #[arg(long, default_value = "20")]
        show_failures: usize,

        /// Filter by test name pattern (supports glob patterns)
        #[arg(long)]
        filter: Option<Text>,

        /// Use advanced parser (verum_parser) instead of default (verum_fast_parser)
        #[arg(long)]
        advanced: bool,

        /// Run only success tests (parse-pass)
        #[arg(long)]
        success_only: bool,

        /// Run only failure tests (parse-fail)
        #[arg(long)]
        fail_only: bool,
    },

    /// Verify common pipeline compliance (compile-time tests only)
    ///
    /// Runs all compile-time tests (parse-pass, parse-fail, typecheck-pass,
    /// typecheck-fail, verify-pass, verify-fail) through the common pipeline
    /// and reports detailed statistics by level and test type.
    Verify {
        /// Test paths or patterns
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Test level filter (L0, L1, L2, L3, L4)
        #[arg(short, long, value_delimiter = ',')]
        level: Vec<Text>,

        /// Include only tests with these tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<Text>,

        /// Exclude tests with these tags
        #[arg(long, value_delimiter = ',')]
        exclude_tags: Vec<Text>,

        /// Number of parallel test processes
        #[arg(short, long, default_value = "auto")]
        parallel: Text,

        /// Output format (console, json, markdown)
        #[arg(short, long, default_value = "console")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Default timeout in milliseconds
        #[arg(long, default_value = "30000")]
        timeout: u64,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,

        /// Show only summary (no individual test results)
        #[arg(long)]
        summary_only: bool,

        /// Show first N failures in detail (default: 20)
        #[arg(long, default_value = "20")]
        show_failures: usize,

        /// Strict mode: require 100% pass rate (exit 1 if any failures)
        #[arg(long)]
        strict: bool,
    },

    /// Run meta-system tests from vcs/specs/meta-system
    ///
    /// Tests the compile-time meta-system (meta fn, @const evaluation, builtins).
    /// This runs through the compiler pipeline up to TypedAST generation,
    /// without requiring the runtime.
    Meta {
        /// Test paths or patterns (defaults to specs/meta-system)
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Filter by meta subsystem (builtins, expressions, hygiene, quote, sandbox, type-level)
        #[arg(long, value_delimiter = ',')]
        subsystem: Vec<Text>,

        /// Filter by level (L0, L1, L2, L3)
        #[arg(short, long, value_delimiter = ',')]
        level: Vec<Text>,

        /// Include only tests with these tags
        #[arg(long, value_delimiter = ',')]
        tags: Vec<Text>,

        /// Exclude tests with these tags
        #[arg(long, value_delimiter = ',')]
        exclude_tags: Vec<Text>,

        /// Number of parallel test processes
        #[arg(short, long, default_value = "auto")]
        parallel: Text,

        /// Output format (console, json, markdown)
        #[arg(short, long, default_value = "console")]
        format: Text,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Default timeout in milliseconds
        #[arg(long, default_value = "30000")]
        timeout: u64,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,

        /// Show only summary (no individual test results)
        #[arg(long)]
        summary_only: bool,

        /// Show first N failures in detail (default: 20)
        #[arg(long, default_value = "20")]
        show_failures: usize,

        /// Filter by test name pattern (supports glob patterns)
        #[arg(long)]
        filter: Option<Text>,

        /// Run only success tests (meta-pass, meta-eval)
        #[arg(long)]
        success_only: bool,

        /// Run only failure tests (meta-fail)
        #[arg(long)]
        fail_only: bool,

        /// Verbose output (show detailed evaluation)
        #[arg(short, long)]
        verbose: bool,
    },
}

fn parse_levels(levels: &[Text]) -> Set<Level> {
    let mut result = Set::new();
    for level in levels {
        for part in level.split(",") {
            if let Ok(l) = Level::from_str(part.trim().as_str()) {
                result.insert(l);
            }
        }
    }
    result
}

fn parse_tiers(tiers: &[Text]) -> Set<Tier> {
    let mut result = Set::new();
    for tier in tiers {
        let tier = tier.trim().to_lowercase();
        if tier == "all" {
            return Tier::all().into_iter().collect();
        }
        if tier == "compiled" {
            return Tier::compiled().into_iter().collect();
        }
        for part in tier.split(",") {
            if let Ok(t) = Tier::from_str(part.trim().as_str()) {
                result.insert(t);
            }
        }
    }
    result
}

fn parse_parallel(s: &str) -> usize {
    if s == "auto" {
        // Cap parallelism to avoid OOM: each test can use 1-2GB during compilation.
        let cpus = num_cpus::get();
        std::cmp::min(cpus / 2, 4).max(1)
    } else {
        s.parse().unwrap_or_else(|_| {
            let cpus = num_cpus::get();
            std::cmp::min(cpus / 2, 4).max(1)
        })
    }
}

fn tags_to_set(tags: &[Text]) -> Set<Text> {
    tags.iter()
        .flat_map(|t| t.split(",").into_iter().map(|s| s.trim()))
        .filter(|t| !t.is_empty())
        .collect()
}

fn main() -> ExitCode {
    // Eager native-target initialisation — eliminates a nondeterministic
    // SIGSEGV in LLVM pass-constructor cxa guards (bug #SIGSEGV-arm64, fixed
    // in `verum_cli::main`). When vtest drives AOT tests through the
    // `verum_compiler` library, the same LLVM entry points are reached, and
    // the same race window opens against rayon workers spawned by the stdlib
    // parse. Running `Target::initialize_native` on the main thread before
    // any worker exists closes the window; the call is idempotent via an
    // internal `Once`, so later uses (e.g. inside `VbcToLlvmLowering::new`)
    // become no-ops.
    //
    // Without this guard, `vtest run --parallel 4 specs/L0-critical/vbc`
    // exits 139 (SIGSEGV) silently before the summary is written — the
    // symptom that first surfaced the bug on `verum build`.
    let _ = verum_llvm::targets::Target::initialize_native(
        &verum_llvm::targets::InitializationConfig::default(),
    );

    // Stack size for type checker threads. Recursive type inference on deeply
    // nested expressions can exceed the default 8MB stack. Measured peak is ~8MB;
    // 64MB provides headroom for complex stdlib modules (272 modules with nested
    // generics, protocols, and method chains). Reduced from 128MB to limit total
    // memory footprint when running tests in parallel.
    // Stack sizes: compilation runs on dedicated 512MB threads (spawned by executor).
    // Some stdlib types create deep recursive type resolution chains.
    const MAIN_STACK: usize = 512 * 1024 * 1024; // 512MB for main + warmup
    const WORKER_STACK: usize = 512 * 1024 * 1024; // 512MB — same as main (type checker needs deep stack)

    std::thread::Builder::new()
        .name("vtest-main".into())
        .stack_size(MAIN_STACK)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .thread_stack_size(WORKER_STACK)
                .enable_all()
                .build()
                .expect("Failed to build tokio runtime");

            runtime.block_on(async_main())
        })
        .expect("Failed to spawn main thread")
        .join()
        .expect("Main thread panicked")
}

async fn async_main() -> ExitCode {
    let cli = Cli::parse();

    // Determine effective verbosity
    let verbosity = if cli.verbose {
        Verbosity::Debug
    } else if cli.quiet {
        Verbosity::Quiet
    } else {
        cli.verbosity
    };

    // Initialize logging based on verbosity
    init_logging(verbosity, cli.json_log);

    // Load configuration
    let config = match &cli.config {
        Some(path) => VTestToml::from_file(path).unwrap_or_default(),
        None => VTestToml::load_default().unwrap_or_default(),
    };

    // Handle no-color flag
    if cli.no_color {
        colored::control::set_override(false);
    }

    debug!("Starting vtest with config: {:?}", cli.config);

    let result = match cli.command {
        Commands::Run {
            paths,
            level,
            tier,
            tags,
            exclude_tags,
            parallel,
            format,
            output,
            timeout,
            fail_fast,
            update_expectations,
            summary_only,
            filter,
            retries,
            save_baseline,
            compare_baseline,
            coverage,
            show_diff,
            isolation,
            compile_time_only,
            vbc_output,
            vbc_preserve_paths,
        } => {
            run_tests(
                RunOptions {
                    paths,
                    level,
                    tier,
                    tags,
                    exclude_tags,
                    parallel,
                    format,
                    output,
                    timeout,
                    fail_fast,
                    update_expectations,
                    summary_only,
                    filter,
                    retries,
                    save_baseline,
                    compare_baseline,
                    coverage,
                    show_diff,
                    isolation,
                    compile_time_only,
                    vbc_output,
                    vbc_preserve_paths,
                },
                verbosity,
                !cli.no_color,
                config,
            )
            .await
        }

        Commands::List {
            paths,
            level,
            tags,
            format,
        } => list_tests_cmd(paths, level, tags, format, cli.verbose, config).await,

        Commands::Report {
            input,
            format,
            output,
        } => generate_report(input, format, output, cli.verbose).await,

        Commands::Watch { paths, level, tags } => {
            watch_tests(paths, level, tags, cli.verbose, config).await
        }

        Commands::Fuzz {
            duration,
            parallel,
            corpus,
            crashes,
        } => run_fuzz(duration, parallel, corpus, crashes, cli.verbose).await,

        Commands::Bench {
            patterns,
            baseline,
            compare,
            format,
            output,
        } => run_benchmarks(patterns, baseline, compare, format, output, cli.verbose).await,

        Commands::Stdlib {
            path,
            parallel,
            format,
            output,
            fail_fast,
            summary_only,
            show_failures,
            parse_only,
            filter,
        } => {
            verify_stdlib(
                path,
                parallel,
                format,
                output,
                fail_fast,
                summary_only,
                show_failures,
                parse_only,
                filter,
                verbosity,
                !cli.no_color,
            )
            .await
        }

        Commands::Verify {
            paths,
            level,
            tags,
            exclude_tags,
            parallel,
            format,
            output,
            timeout,
            fail_fast,
            summary_only,
            show_failures,
            strict,
        } => {
            verify_common_pipeline(
                paths,
                level,
                tags,
                exclude_tags,
                parallel,
                format,
                output,
                timeout,
                fail_fast,
                summary_only,
                show_failures,
                strict,
                verbosity,
                !cli.no_color,
                config,
            )
            .await
        }

        Commands::Parser {
            paths,
            tags,
            exclude_tags,
            parallel,
            format,
            output,
            timeout,
            fail_fast,
            summary_only,
            show_failures,
            filter,
            advanced,
            success_only,
            fail_only,
        } => {
            run_parser_tests(
                paths,
                tags,
                exclude_tags,
                parallel,
                format,
                output,
                timeout,
                fail_fast,
                summary_only,
                show_failures,
                filter,
                advanced,
                success_only,
                fail_only,
                verbosity,
                !cli.no_color,
            )
            .await
        }

        Commands::Meta {
            paths,
            subsystem,
            level,
            tags,
            exclude_tags,
            parallel,
            format,
            output,
            timeout,
            fail_fast,
            summary_only,
            show_failures,
            filter,
            success_only,
            fail_only,
            verbose,
        } => {
            run_meta_tests(
                paths,
                subsystem,
                level,
                tags,
                exclude_tags,
                parallel,
                format,
                output,
                timeout,
                fail_fast,
                summary_only,
                show_failures,
                filter,
                success_only,
                fail_only,
                verbose,
                verbosity,
                !cli.no_color,
            )
            .await
        }
    };

    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("{}: {}", "Error".red().bold(), e);
            ExitCode::from(1)
        }
    }
}

async fn run_tests(
    opts: RunOptions,
    verbosity: Verbosity,
    use_colors: bool,
    toml_config: VTestToml,
) -> Result<i32, RunnerError> {
    let mut config = toml_config.to_runner_config();

    // Override with CLI arguments
    if !opts.paths.is_empty() {
        config.test_paths = opts.paths.into();
    }

    config.levels = parse_levels(&opts.level);
    config.tiers = parse_tiers(&opts.tier);
    config.include_tags = tags_to_set(&opts.tags);
    config.exclude_tags = tags_to_set(&opts.exclude_tags);
    config.parallel = parse_parallel(&opts.parallel);
    config.output_format = ReportFormat::from_str(&opts.format).unwrap_or(ReportFormat::Console);
    config.output_path = opts.output;
    config.default_timeout_ms = opts.timeout;
    config.fail_fast = opts.fail_fast;
    config.verbose = matches!(verbosity, Verbosity::Debug);
    config.use_colors = use_colors;
    config.compiler_version = get_compiler_version();

    // Set new options
    config.update_expectations = opts.update_expectations;
    config.summary_only = opts.summary_only || matches!(verbosity, Verbosity::Summary);
    config.filter_pattern = opts.filter;
    config.retries = opts.retries;
    config.save_baseline = opts.save_baseline;
    config.compare_baseline = opts.compare_baseline;
    config.coverage = opts.coverage;
    config.show_diff = opts.show_diff;
    config.quiet = matches!(verbosity, Verbosity::Quiet | Verbosity::Summary);
    config.executor_config.compile_time_only = opts.compile_time_only;
    config.executor_config.vbc_output_dir = opts.vbc_output;
    config.executor_config.vbc_preserve_paths = opts.vbc_preserve_paths;

    let runner = VTestRunner::new(config);

    // Print header (unless quiet/summary)
    if matches!(verbosity, Verbosity::Debug | Verbosity::Normal) {
        println!("{}", "Verum Compliance Suite".bold());
        println!("{}", "-".repeat(50).dimmed());
        println!();
    }

    info!("Discovering tests...");

    let result = runner.run_and_report().await;

    // Handle update expectations mode
    if opts.update_expectations {
        if let Ok(0) = &result {
            info!("All expectations are up to date");
        } else {
            warn!("Some expectations were updated - please review the changes");
        }
    }

    result
}

async fn list_tests_cmd(
    paths: Vec<PathBuf>,
    level: Vec<Text>,
    tags: Vec<Text>,
    format: Text,
    _verbose: bool,
    toml_config: VTestToml,
) -> Result<i32, RunnerError> {
    let config = toml_config.to_runner_config();

    let test_paths: Vec<PathBuf> = if paths.is_empty() {
        config.test_paths.to_vec()
    } else {
        paths
    };

    let levels = parse_levels(&level);
    let tag_set = tags_to_set(&tags);

    let tests = list_tests(
        &test_paths,
        &config.test_pattern,
        &config.exclude_patterns,
        &levels,
        &tag_set,
    )?;

    if format.to_lowercase() == "json" {
        let items: Vec<_> = tests
            .iter()
            .map(|t| {
                serde_json::json!({
                    "path": t.source_path,
                    "type": t.test_type.to_string(),
                    "level": t.level.to_string(),
                    "tags": t.tags.iter().collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items).unwrap());
    } else {
        println!("{} tests found", tests.len());
        println!("{}", "━".repeat(50).dimmed());

        for test in &tests {
            let level_color = match test.level {
                Level::L0 => test.level.to_string().red(),
                Level::L1 => test.level.to_string().yellow(),
                Level::L2 => test.level.to_string().green(),
                Level::L3 => test.level.to_string().blue(),
                Level::L4 => test.level.to_string().cyan(),
            };

            println!(
                "{} {} {} {}",
                level_color,
                format!("[{}]", test.test_type).dimmed(),
                test.display_name(),
                if test.tags.is_empty() {
                    String::new()
                } else {
                    format!(
                        "({})",
                        test.tags.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                    .dimmed()
                    .to_string()
                }
            );
        }
    }

    Ok(0)
}

async fn generate_report(
    input: PathBuf,
    format: Text,
    output: Option<PathBuf>,
    _verbose: bool,
) -> Result<i32, RunnerError> {
    let content = std::fs::read_to_string(&input)?;
    let report: vtest::report::Report = serde_json::from_str(&content)
        .map_err(|e| RunnerError::ConfigError(format!("Invalid JSON: {}", e).into()))?;

    let format = ReportFormat::from_str(&format).unwrap_or(ReportFormat::Html);

    // Create a reporter and add the results
    let reporter = vtest::report::Reporter::new(report.compiler_version);

    // We need to reconstruct results from the report data
    // For now, just output the raw report in the requested format
    if let Some(path) = output {
        reporter.generate_to_file(&path, format)?;
        println!("Report written to {}", path.display());
    } else {
        let mut stdout = std::io::stdout();
        reporter.generate(&mut stdout, format)?;
    }

    Ok(0)
}

async fn watch_tests(
    paths: Vec<PathBuf>,
    level: Vec<Text>,
    tags: Vec<Text>,
    verbose: bool,
    toml_config: VTestToml,
) -> Result<i32, RunnerError> {
    use vtest::watch::{FileWatcher, WatchConfig};

    let mut config = toml_config.to_runner_config();

    // Override with CLI arguments
    if !paths.is_empty() {
        config.test_paths = paths.clone().into();
    }

    config.levels = parse_levels(&level);
    config.include_tags = tags_to_set(&tags);
    config.verbose = verbose;
    config.use_colors = true;
    config.compiler_version = get_compiler_version();

    // Setup watch config
    let watch_config = WatchConfig {
        watch_paths: if paths.is_empty() {
            config.test_paths.clone()
        } else {
            paths.into()
        },
        debounce_ms: 200,
        ..WatchConfig::default()
    };

    let mut watcher = FileWatcher::new(watch_config, config);

    // Run the watch loop (this blocks until Ctrl+C or error)
    watcher.run().await?;
    Ok(0)
}

async fn run_fuzz(
    duration: u64,
    parallel: Text,
    corpus: Option<PathBuf>,
    crashes: Option<PathBuf>,
    verbose: bool,
) -> Result<i32, RunnerError> {
    use vtest::fuzz::{FuzzConfig, Fuzzer};

    let parallel_count = parse_parallel(&parallel);

    let config = FuzzConfig {
        duration_secs: duration,
        parallel: parallel_count,
        corpus_dir: corpus.unwrap_or_else(|| PathBuf::from("corpus")),
        crashes_dir: crashes.unwrap_or_else(|| PathBuf::from("crashes")),
        verbose,
        ..FuzzConfig::default()
    };

    let mut fuzzer = Fuzzer::new(config);
    let stats = fuzzer.run().await?;

    // Return exit code based on crashes found
    if stats.crashes > 0 { Ok(1) } else { Ok(0) }
}

async fn run_benchmarks(
    patterns: Vec<Text>,
    baseline: Option<Text>,
    compare: Vec<Text>,
    format: Text,
    output: Option<PathBuf>,
    verbose: bool,
) -> Result<i32, RunnerError> {
    use vtest::benchmark::{BenchmarkBaseline, BenchmarkConfig, BenchmarkRunner};

    let config = BenchmarkConfig {
        warmup_iterations: 100,
        measurement_iterations: 1000,
        min_samples: 10,
        max_time_ms: 30_000,
        regression_threshold: 10.0,
        collect_memory: true,
    };

    println!();
    println!("{}", "=".repeat(60).dimmed());
    println!("  {} {}", "VTEST".bold(), "Benchmarks".dimmed());
    println!("{}", "=".repeat(60).dimmed());
    println!();

    // Discover benchmark tests
    let toml_config = VTestToml::load_default().unwrap_or_default();
    let mut runner_config = toml_config.to_runner_config();
    runner_config.verbose = verbose;

    // Filter to only benchmark tests
    runner_config.include_tags.insert("benchmark".to_string().into());

    // Also filter by patterns if provided
    if !patterns.is_empty() {
        runner_config.filter_pattern = Some(patterns.join(",").into());
    }

    let runner = VTestRunner::new(runner_config);
    let tests = runner.discover()?;

    if tests.is_empty() {
        println!("  {} No benchmark tests found.", "Warning:".yellow());
        println!("  Make sure tests are tagged with @tags: benchmark");
        return Ok(0);
    }

    println!("  Found {} benchmark tests", tests.len());
    println!();

    // Run benchmarks
    let mut bench_runner = BenchmarkRunner::new(config.clone());
    let compiler_version = get_compiler_version();

    for test in &tests {
        let name = std::path::Path::new(&test.source_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        if verbose {
            println!("  Running: {}", name.cyan());
        }

        // Execute the test as a benchmark
        // This is a simplified approach - in a real implementation we'd
        // have the executor return timing information
        bench_runner.bench(&name, || {
            // Simulate a benchmark run
            std::hint::black_box(1 + 2);
        });
    }

    // Build report
    let report = bench_runner.build_report(compiler_version.clone());

    // Load baseline if specified
    if let Some(ref baseline_path) = baseline {
        let baseline_file = std::path::Path::new(baseline_path);
        if baseline_file.exists() {
            match BenchmarkBaseline::load(baseline_file) {
                Ok(baseline_data) => {
                    let detections =
                        baseline_data.compare_with_current(&report, config.regression_threshold);

                    let regressions: Vec<_> =
                        detections.iter().filter(|d| d.is_regression).collect();
                    if !regressions.is_empty() {
                        println!();
                        println!("  {} Regressions detected:", "WARNING:".yellow().bold());
                        for r in &regressions {
                            println!("    {} {}", "-".dimmed(), r.report());
                        }
                    }
                }
                Err(e) => {
                    println!("  {} Failed to load baseline: {}", "Warning:".yellow(), e);
                }
            }
        } else {
            println!(
                "  {} Baseline file not found: {}",
                "Warning:".yellow(),
                baseline_path
            );
        }
    }

    // Handle comparisons
    for compare_path in &compare {
        let compare_file = std::path::Path::new(compare_path);
        if compare_file.exists() {
            match BenchmarkBaseline::load(compare_file) {
                Ok(compare_data) => {
                    println!();
                    println!("  Comparison with {}:", compare_path.cyan());
                    let detections =
                        compare_data.compare_with_current(&report, config.regression_threshold);

                    for d in &detections {
                        let status = if d.is_regression {
                            "SLOWER".red().to_string()
                        } else if d.change_percent < -config.regression_threshold {
                            "FASTER".green().to_string()
                        } else {
                            "SAME".dimmed().to_string()
                        };
                        println!("    {} {}: {:.1}%", status, d.name, d.change_percent);
                    }
                }
                Err(e) => {
                    println!("  {} Failed to load comparison: {}", "Warning:".yellow(), e);
                }
            }
        }
    }

    // Output report
    let output_format = format.to_lowercase();
    match output_format.as_str() {
        "json" => {
            let json = serde_json::to_string_pretty(&report).map_err(|e| {
                RunnerError::ConfigError(format!("JSON serialization failed: {}", e).into())
            })?;
            if let Some(ref path) = output {
                std::fs::write(path, &json)?;
                println!("  Report written to: {}", path.display());
            } else {
                println!("{}", json);
            }
        }
        "console" | _ => {
            println!();
            println!("{}", "-".repeat(60).dimmed());
            println!("  {} Benchmark Results", ">>".bold());
            println!("{}", "-".repeat(60).dimmed());
            println!();

            for (name, stats) in &report.results {
                println!(
                    "  {} {} (n={}, CV={:.1}%)",
                    name,
                    stats.format_duration().cyan(),
                    stats.samples,
                    stats.cv() * 100.0
                );
            }

            println!();
            println!("{}", "=".repeat(60).dimmed());
        }
    }

    // Check for regressions
    if report.has_regressions() {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Verify stdlib compilation using dependency-ordered registration.
///
/// This function parses all stdlib files first, then uses StdlibTypeRegistry
/// to register types in dependency order before type-checking.
///
/// Pipeline:
/// 1. Parse all .vr files in stdlib
/// 2. Map file paths to module names (e.g., "core/maybe", "collections/list")
/// 3. Register types in dependency order via StdlibTypeRegistry
/// 4. Type-check all modules
#[allow(clippy::too_many_arguments)]
async fn verify_stdlib(
    stdlib_path: PathBuf,
    parallel: Text,
    format: Text,
    output: Option<PathBuf>,
    fail_fast: bool,
    summary_only: bool,
    show_failures: usize,
    parse_only: bool,
    filter: Option<Text>,
    verbosity: Verbosity,
    use_colors: bool,
) -> Result<i32, RunnerError> {
    use std::collections::HashMap;
    use std::time::Instant;
    use walkdir::WalkDir;
    use verum_ast::FileId;
    use verum_common::{Map, Maybe as CoreMaybe, Text as CoreText};
    use verum_lexer::Lexer;
    use verum_parser::VerumParser;
    use verum_types::TypeChecker;
    use verum_types::core_pipeline::{StdlibTypeRegistry, ModuleOrder, GlobalPassResult};

    let start = Instant::now();
    let verbose = matches!(verbosity, Verbosity::Debug);
    let quiet = matches!(verbosity, Verbosity::Quiet | Verbosity::Summary);

    // Print header
    if !quiet {
        println!();
        println!("{}", "═".repeat(65));
        println!("  {} - Stdlib Compilation Verification", "VTEST".bold());
        println!("  Pipeline: Source → Lexer → Parser → AST{}",
            if parse_only { "" } else { " → Register → TypeCheck" });
        println!("  Mode: {}", if parse_only { "Parse only" } else { "Dependency-ordered compilation" });
        println!("{}", "═".repeat(65));
        println!();
    }

    // Discover all .vr files in stdlib
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&stdlib_path)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "vr") {
            // Apply filter if specified
            if let Some(ref pattern) = filter {
                let path_str = path.to_string_lossy();
                if !path_str.contains(pattern.as_str()) {
                    continue;
                }
            }
            files.push(path.to_path_buf());
        }
    }

    files.sort();

    if files.is_empty() {
        println!("  {} No .vr files found in {}", "Warning:".yellow(), stdlib_path.display());
        return Ok(0);
    }

    if !quiet {
        println!("  Found {} files in {}", files.len().to_string().cyan(), stdlib_path.display());
        println!();
    }

    // ═══════════════════════════════════════════════════════════════════════
    // PHASE 1: Parse all files
    // ═══════════════════════════════════════════════════════════════════════
    let mut parsed_modules: Map<CoreText, verum_ast::Module> = Map::new();
    let mut file_to_module: HashMap<PathBuf, String> = HashMap::new();
    let mut by_module: HashMap<String, ModuleStats> = HashMap::new();
    let mut failures: Vec<StdlibFailure> = Vec::new();
    let mut parse_passed = 0;
    let mut parse_failed = 0;
    let should_stop = std::sync::atomic::AtomicBool::new(false);

    // Progress bar for parsing phase
    let show_progress = !verbose && !quiet;
    let progress_bar = if show_progress {
        let pb = indicatif::ProgressBar::new(files.len() as u64);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) Parsing... {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
        Some(pb)
    } else {
        None
    };

    if !quiet && !show_progress {
        println!("  Phase 1: Parsing {} files...", files.len());
    }

    for (idx, file_path) in files.iter().enumerate() {
        if should_stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        // Compute module name from path
        // e.g., stdlib/core/maybe.vr -> "core/maybe"
        let rel_path = file_path.strip_prefix(&stdlib_path).unwrap_or(file_path);
        let module_name = rel_path
            .with_extension("")
            .to_string_lossy()
            .replace("\\", "/"); // Handle Windows paths

        let display_module = if module_name.is_empty() { "mod" } else { &module_name };
        file_to_module.insert(file_path.clone(), display_module.to_string());

        let module_stats = by_module.entry(display_module.to_string()).or_default();
        module_stats.total += 1;

        // Read file
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                parse_failed += 1;
                module_stats.parse_failed += 1;
                failures.push(StdlibFailure {
                    path: file_path.to_string_lossy().to_string(),
                    module: display_module.to_string(),
                    phase: "read".to_string(),
                    error: format!("Failed to read file: {}", e),
                });
                if fail_fast {
                    should_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                continue;
            }
        };

        // Parse
        let file_id = FileId::new((idx + 1) as u32);
        let lexer = Lexer::new(&source, file_id);
        let parser = VerumParser::new();

        match parser.parse_module(lexer, file_id) {
            Ok(ast) => {
                parse_passed += 1;
                module_stats.parse_passed += 1;
                parsed_modules.insert(CoreText::from(display_module), ast);

                if verbose && !summary_only {
                    println!("  {} {} (parse)", "PASS".green(), display_module);
                }
            }
            Err(errors) => {
                parse_failed += 1;
                module_stats.parse_failed += 1;
                let error_msg = format!("{:?}", errors);
                failures.push(StdlibFailure {
                    path: file_path.to_string_lossy().to_string(),
                    module: display_module.to_string(),
                    phase: "parse".to_string(),
                    error: error_msg.clone(),
                });

                if verbose && !summary_only {
                    let short_err: String = error_msg.chars().take(80).collect();
                    println!("  {} {} - {}", "FAIL".red(), display_module, short_err);
                }

                if fail_fast {
                    should_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        }

        if let Some(ref pb) = progress_bar {
            pb.set_message(format!("{} pass, {} fail", parse_passed, parse_failed));
            pb.inc(1);
        }
    }

    if let Some(pb) = progress_bar {
        pb.finish_and_clear();
    }

    if !quiet {
        println!("  Phase 1 complete: {}/{} files parsed successfully",
            parse_passed.to_string().green(), files.len());
    }

    // If parse_only, skip registration and typechecking
    let mut typecheck_passed = 0;
    let mut typecheck_failed = 0;
    let mut _register_passed = 0;
    let mut _register_failed = 0;

    if !parse_only && parse_passed > 0 {
        // ═══════════════════════════════════════════════════════════════════════
        // PHASE 2: Register types using global passes
        // ═══════════════════════════════════════════════════════════════════════
        // Using global passes handles circular dependencies by:
        // 1. Registering ALL type names first (forward declarations)
        // 2. Registering ALL protocols
        // 3. Resolving ALL type definitions
        // 4. Registering ALL impl blocks
        if !quiet {
            println!();
            println!("  Phase 2: Registering types with global passes...");
        }

        // Create TypeChecker with minimal context for stdlib compilation
        let mut checker = TypeChecker::with_minimal_context();
        checker.register_builtins();

        // Create registry with verbose mode if enabled
        let mut registry = StdlibTypeRegistry::new().with_verbose(verbose);

        // Use global passes for registration (handles circular dependencies)
        let global_result = registry.register_all_global_passes(&mut checker, &parsed_modules);

        // Collect all module names for later use
        let all_modules: Vec<String> = parsed_modules.keys().map(|k| k.to_string()).collect();

        // Record registration failures
        for error in registry.errors() {
            failures.push(StdlibFailure {
                path: error.module.to_string(),
                module: error.module.to_string(),
                phase: format!("register:{}", error.phase),
                error: error.message.to_string(),
            });
        }

        _register_passed = global_result.total_modules.saturating_sub(global_result.total_errors());
        _register_failed = global_result.total_errors();

        if !quiet {
            println!("  Phase 2 complete:");
            println!("    Pass 1: {} type names registered", global_result.types_registered);
            println!("    Pass 2: {} protocols ({} errors)",
                global_result.protocols_registered, global_result.protocol_errors);
            println!("    Pass 3: {} type definition errors", global_result.type_definition_errors);
            println!("    Pass 4: {} impls ({} errors)",
                global_result.impls_registered, global_result.impl_errors);
        }

        // ═══════════════════════════════════════════════════════════════════════
        // PHASE 3: Type-check all modules
        // ═══════════════════════════════════════════════════════════════════════
        if !quiet {
            println!();
            println!("  Phase 3: Type-checking {} modules...", all_modules.len());
        }

        for module_name in &all_modules {
            let key = CoreText::from(module_name.as_str());
            if let CoreMaybe::Some(ast) = parsed_modules.get(&key) {
                match registry.typecheck_module(&mut checker, ast, module_name) {
                    Ok(items_checked) => {
                        typecheck_passed += 1;
                        if let Some(stats) = by_module.get_mut(module_name) {
                            stats.typecheck_passed += 1;
                        }
                        if verbose {
                            println!("  {} {} ({} items checked)", "PASS".green(), module_name, items_checked);
                        }
                    }
                    Err(e) => {
                        typecheck_failed += 1;
                        if let Some(stats) = by_module.get_mut(module_name) {
                            stats.typecheck_failed += 1;
                        }
                        failures.push(StdlibFailure {
                            path: module_name.to_string(),
                            module: module_name.to_string(),
                            phase: "typecheck".to_string(),
                            error: e.message.to_string(),
                        });
                        if verbose {
                            let short_err: String = e.message.chars().take(80).collect();
                            println!("  {} {} - {}", "FAIL".red(), module_name, short_err);
                        }
                        if fail_fast {
                            break;
                        }
                    }
                }
            }
        }

        if !quiet {
            println!("  Phase 3 complete: {}/{} modules type-checked successfully",
                typecheck_passed.to_string().green(), all_modules.len());
        }
    }

    let duration = start.elapsed();
    let total = files.len();

    // Output based on format
    let output_format = format.to_lowercase();
    match output_format.as_str() {
        "json" => {
            output_stdlib_json_report(
                total, parse_passed, parse_failed, typecheck_passed, typecheck_failed,
                &by_module, &failures, duration, output, parse_only,
            )?;
        }
        "markdown" | "md" => {
            output_stdlib_markdown_report(
                total, parse_passed, parse_failed, typecheck_passed, typecheck_failed,
                &by_module, &failures, duration, output, parse_only,
            )?;
        }
        _ => {
            output_stdlib_console_report(
                total, parse_passed, parse_failed, typecheck_passed, typecheck_failed,
                &by_module, &failures, duration, show_failures, use_colors, quiet, parse_only,
            );
        }
    }

    // Determine exit code
    if parse_failed > 0 || (!parse_only && typecheck_failed > 0) {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Module-level statistics for stdlib verification.
#[derive(Debug, Default)]
struct ModuleStats {
    total: usize,
    parse_passed: usize,
    parse_failed: usize,
    typecheck_passed: usize,
    typecheck_failed: usize,
}

/// Detail about a stdlib compilation failure.
#[derive(Debug)]
struct StdlibFailure {
    path: String,
    module: String,
    phase: String,
    error: String,
}

fn output_stdlib_console_report(
    total: usize,
    parse_passed: usize,
    parse_failed: usize,
    typecheck_passed: usize,
    typecheck_failed: usize,
    by_module: &std::collections::HashMap<String, ModuleStats>,
    failures: &[StdlibFailure],
    duration: std::time::Duration,
    show_failures: usize,
    use_colors: bool,
    quiet: bool,
    parse_only: bool,
) {
    if quiet && failures.is_empty() {
        println!("All {} files passed", total);
        return;
    }

    println!();
    println!("{}", "═".repeat(65));
    println!("  STDLIB COMPILATION SUMMARY");
    println!("{}", "─".repeat(65));

    // Overall stats
    let parse_rate = if total > 0 { 100.0 * parse_passed as f64 / total as f64 } else { 0.0 };

    println!("  Total Files:   {}", total.to_string().bold());
    println!(
        "  Parse Passed:  {} ({:.1}%)",
        parse_passed.to_string().green().bold(),
        parse_rate
    );
    if parse_failed > 0 {
        println!(
            "  Parse Failed:  {} ({:.1}%)",
            parse_failed.to_string().red().bold(),
            100.0 * parse_failed as f64 / total as f64
        );
    }

    if !parse_only {
        let tc_total = parse_passed; // only typecheck files that parsed
        let tc_rate = if tc_total > 0 { 100.0 * typecheck_passed as f64 / tc_total as f64 } else { 0.0 };
        println!(
            "  Typecheck Passed: {} ({:.1}%)",
            typecheck_passed.to_string().green().bold(),
            tc_rate
        );
        if typecheck_failed > 0 {
            println!(
                "  Typecheck Failed: {} ({:.1}%)",
                typecheck_failed.to_string().red().bold(),
                100.0 * typecheck_failed as f64 / tc_total as f64
            );
        }
    }

    // By module
    println!();
    println!("  By Module:");
    let mut modules: Vec<_> = by_module.iter().collect();
    modules.sort_by(|a, b| a.0.cmp(b.0));

    for (module, stats) in modules {
        let rate = if stats.total > 0 {
            100.0 * stats.parse_passed as f64 / stats.total as f64
        } else {
            0.0
        };
        let rate_str = if rate >= 100.0 {
            format!("{:.0}%", rate).green().to_string()
        } else if rate >= 80.0 {
            format!("{:.0}%", rate).yellow().to_string()
        } else {
            format!("{:.0}%", rate).red().to_string()
        };
        let module_display = if module.is_empty() { "root" } else { module.as_str() };
        println!(
            "    {}: {}/{} ({})",
            module_display.bold(),
            stats.parse_passed,
            stats.total,
            rate_str
        );
    }

    println!();
    println!("  Duration: {:.2}s", duration.as_secs_f64());
    println!("{}", "═".repeat(65));

    // Print failures
    if !failures.is_empty() && show_failures > 0 {
        let show_count = std::cmp::min(show_failures, failures.len());
        println!();
        println!(
            "  {} (showing first {}):",
            "FAILURES".red().bold(),
            show_count
        );
        println!();

        for (i, failure) in failures.iter().take(show_count).enumerate() {
            println!("  {}. {}", i + 1, failure.path.cyan());
            println!("     Module: {}, Phase: {}", failure.module, failure.phase.dimmed());
            // Truncate long error messages
            let error_preview: String = failure.error.chars().take(200).collect();
            println!("     Error: {}", error_preview.red());
            println!();
        }

        if failures.len() > show_count {
            println!(
                "  ... and {} more failures (use --show-failures to see more)",
                failures.len() - show_count
            );
        }
    }

    // Final result
    println!();
    if parse_failed == 0 && (parse_only || typecheck_failed == 0) {
        println!("  {} All stdlib files compiled successfully!", "RESULT:".green().bold());
    } else {
        println!(
            "  {} {} parse failures{}",
            "RESULT:".red().bold(),
            parse_failed,
            if !parse_only && typecheck_failed > 0 {
                format!(", {} typecheck failures", typecheck_failed)
            } else {
                String::new()
            }
        );
    }
    println!();
}

#[allow(clippy::too_many_arguments)]
fn output_stdlib_json_report(
    total: usize,
    parse_passed: usize,
    parse_failed: usize,
    typecheck_passed: usize,
    typecheck_failed: usize,
    by_module: &std::collections::HashMap<String, ModuleStats>,
    failures: &[StdlibFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
    parse_only: bool,
) -> Result<(), RunnerError> {
    let report = serde_json::json!({
        "summary": {
            "total": total,
            "parse_passed": parse_passed,
            "parse_failed": parse_failed,
            "typecheck_passed": typecheck_passed,
            "typecheck_failed": typecheck_failed,
            "parse_only": parse_only,
            "parse_rate": if total > 0 { 100.0 * parse_passed as f64 / total as f64 } else { 0.0 },
            "duration_secs": duration.as_secs_f64(),
        },
        "by_module": by_module.iter().map(|(name, stats)| {
            (name.clone(), serde_json::json!({
                "total": stats.total,
                "parse_passed": stats.parse_passed,
                "parse_failed": stats.parse_failed,
                "typecheck_passed": stats.typecheck_passed,
                "typecheck_failed": stats.typecheck_failed,
            }))
        }).collect::<serde_json::Map<_, _>>(),
        "failures": failures.iter().map(|f| {
            serde_json::json!({
                "path": f.path,
                "module": f.module,
                "phase": f.phase,
                "error": f.error,
            })
        }).collect::<Vec<_>>(),
    });

    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| RunnerError::ConfigError(format!("JSON error: {}", e).into()))?;

    if let Some(path) = output {
        std::fs::write(&path, &json)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", json);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn output_stdlib_markdown_report(
    total: usize,
    parse_passed: usize,
    parse_failed: usize,
    typecheck_passed: usize,
    typecheck_failed: usize,
    by_module: &std::collections::HashMap<String, ModuleStats>,
    failures: &[StdlibFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
    parse_only: bool,
) -> Result<(), RunnerError> {
    let mut md = String::new();

    md.push_str("# Stdlib Compilation Verification Report\n\n");
    md.push_str(&format!("Generated: {}\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Total Files | {} |\n", total));
    md.push_str(&format!("| Parse Passed | {} |\n", parse_passed));
    md.push_str(&format!("| Parse Failed | {} |\n", parse_failed));
    if !parse_only {
        md.push_str(&format!("| Typecheck Passed | {} |\n", typecheck_passed));
        md.push_str(&format!("| Typecheck Failed | {} |\n", typecheck_failed));
    }
    let parse_rate = if total > 0 { 100.0 * parse_passed as f64 / total as f64 } else { 0.0 };
    md.push_str(&format!("| Parse Rate | {:.1}% |\n", parse_rate));
    md.push_str(&format!("| Duration | {:.2}s |\n", duration.as_secs_f64()));
    md.push_str("\n");

    // By Module
    md.push_str("## Results by Module\n\n");
    md.push_str("| Module | Passed | Total | Rate |\n");
    md.push_str("|--------|--------|-------|------|\n");
    let mut modules: Vec<_> = by_module.iter().collect();
    modules.sort_by(|a, b| a.0.cmp(b.0));
    for (module, stats) in modules {
        let rate = if stats.total > 0 { 100.0 * stats.parse_passed as f64 / stats.total as f64 } else { 0.0 };
        let module_display = if module.is_empty() { "root" } else { module.as_str() };
        md.push_str(&format!("| {} | {} | {} | {:.1}% |\n", module_display, stats.parse_passed, stats.total, rate));
    }
    md.push_str("\n");

    // Failures
    if !failures.is_empty() {
        md.push_str("## Failures\n\n");
        for (i, failure) in failures.iter().enumerate() {
            md.push_str(&format!("### {}. `{}`\n\n", i + 1, failure.path));
            md.push_str(&format!("- **Module**: {}\n", failure.module));
            md.push_str(&format!("- **Phase**: {}\n", failure.phase));
            md.push_str(&format!("- **Error**: {}\n", failure.error.chars().take(500).collect::<String>()));
            md.push_str("\n");
        }
    }

    if let Some(path) = output {
        std::fs::write(&path, &md)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", md);
    }

    Ok(())
}

/// Verify common pipeline compliance.
///
/// This function runs all compile-time tests through the common pipeline
/// (Source → Parser → AST → Types → TypedAST) and reports detailed statistics.
#[allow(clippy::too_many_arguments)]
async fn verify_common_pipeline(
    paths: Vec<PathBuf>,
    level: Vec<Text>,
    tags: Vec<Text>,
    exclude_tags: Vec<Text>,
    parallel: Text,
    format: Text,
    output: Option<PathBuf>,
    timeout: u64,
    fail_fast: bool,
    summary_only: bool,
    show_failures: usize,
    strict: bool,
    verbosity: Verbosity,
    use_colors: bool,
    toml_config: VTestToml,
) -> Result<i32, RunnerError> {
    use std::collections::HashMap;
    use std::time::Instant;
    use vtest::directive::TestType;
    use vtest::executor::verify::{VerificationResult, VerificationStats, verify_test};

    let start = Instant::now();
    let verbose = matches!(verbosity, Verbosity::Debug);
    let quiet = matches!(verbosity, Verbosity::Quiet | Verbosity::Summary);

    // Print header
    if !quiet {
        println!();
        println!("{}", "═".repeat(65));
        println!("  {} - Common Pipeline Verification", "VTEST".bold());
        println!("  Pipeline: Source → Parser → AST → Types → TypedAST");
        println!("{}", "═".repeat(65));
        println!();
    }

    // Setup configuration
    let mut config = toml_config.to_runner_config();
    if !paths.is_empty() {
        config.test_paths = paths.into();
    }
    config.levels = parse_levels(&level);
    config.include_tags = tags_to_set(&tags);
    config.exclude_tags = tags_to_set(&exclude_tags);
    config.parallel = parse_parallel(&parallel);
    config.default_timeout_ms = timeout;
    config.fail_fast = fail_fast;
    config.verbose = verbose;
    config.use_colors = use_colors;

    // Discover tests
    let runner = VTestRunner::new(config.clone());
    let all_tests = runner.discover()?;

    // Filter to compile-time tests only
    let compile_time_tests: Vec<_> = all_tests
        .iter()
        .filter(|t| is_compile_time_test(t.test_type))
        .cloned()
        .collect();

    if compile_time_tests.is_empty() {
        println!("  {} No compile-time tests found.", "Warning:".yellow());
        return Ok(0);
    }

    if !quiet {
        println!(
            "  Found {} compile-time tests (out of {} total)",
            compile_time_tests.len().to_string().cyan(),
            all_tests.len()
        );
        println!();
    }

    // Statistics
    let mut stats = VerificationStats::default();
    let mut by_level: HashMap<Level, LevelVerifyStats> = HashMap::new();
    let mut by_type: HashMap<TestType, TypeVerifyStats> = HashMap::new();
    let mut failures: Vec<FailureDetail> = Vec::new();

    // Progress indicator
    let show_progress = config.show_progress && !verbose && !quiet;
    let progress_bar = if show_progress {
        let pb = indicatif::ProgressBar::new(compile_time_tests.len() as u64);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
        Some(pb)
    } else {
        None
    };

    // Run tests
    let should_stop = std::sync::atomic::AtomicBool::new(false);

    for (i, directives) in compile_time_tests.iter().enumerate() {
        if should_stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        // Skip if marked
        if directives.skip.is_some() {
            stats.skipped += 1;
            if let Some(ref pb) = progress_bar {
                pb.inc(1);
            }
            continue;
        }

        // Run the verification
        let result = verify_test(directives);

        // Update stats
        stats.total += 1;
        let level_stats = by_level.entry(directives.level).or_default();
        let type_stats = by_type.entry(directives.test_type).or_default();
        level_stats.total += 1;
        type_stats.total += 1;

        match &result {
            VerificationResult::Pass => {
                stats.passed += 1;
                level_stats.passed += 1;
                type_stats.passed += 1;
            }
            VerificationResult::Fail { reason, phase } => {
                stats.failed += 1;
                level_stats.failed += 1;
                type_stats.failed += 1;

                failures.push(FailureDetail {
                    path: directives.source_path.clone(),
                    test_type: directives.test_type,
                    level: directives.level,
                    phase: phase.clone(),
                    reason: reason.clone(),
                });

                // Print failure if verbose
                if verbose && !summary_only {
                    let rel_path = std::path::Path::new(directives.source_path.as_str())
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| directives.source_path.to_string());
                    println!(
                        "  {} {} - {} ({})",
                        "FAIL".red().bold(),
                        rel_path,
                        reason.dimmed(),
                        phase.dimmed()
                    );
                }

                // Fail fast
                if fail_fast {
                    should_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            VerificationResult::Error { message } => {
                stats.errors += 1;
                level_stats.failed += 1;
                type_stats.failed += 1;

                failures.push(FailureDetail {
                    path: directives.source_path.clone(),
                    test_type: directives.test_type,
                    level: directives.level,
                    phase: "error".to_string(),
                    reason: message.clone(),
                });

                if verbose && !summary_only {
                    println!(
                        "  {} {} - {}",
                        "ERROR".red().bold(),
                        directives.display_name(),
                        message.dimmed()
                    );
                }
            }
            VerificationResult::Skip { reason } => {
                stats.skipped += 1;
            }
        }

        // Update progress
        if let Some(ref pb) = progress_bar {
            pb.set_message(format!(
                "{} pass, {} fail",
                stats.passed,
                stats.failed + stats.errors
            ));
            pb.inc(1);
        }

        // Progress output for non-verbose
        if !verbose && !quiet && !show_progress && (i + 1) % 100 == 0 {
            println!("  ... processed {}/{} tests", i + 1, compile_time_tests.len());
        }
    }

    if let Some(pb) = progress_bar {
        pb.finish_and_clear();
    }

    let duration = start.elapsed();

    // Output based on format
    let output_format = format.to_lowercase();
    match output_format.as_str() {
        "json" => {
            output_json_report(&stats, &by_level, &by_type, &failures, duration, output)?;
        }
        "markdown" | "md" => {
            output_markdown_report(&stats, &by_level, &by_type, &failures, duration, output)?;
        }
        _ => {
            output_console_report(
                &stats,
                &by_level,
                &by_type,
                &failures,
                duration,
                show_failures,
                use_colors,
                quiet,
            );
        }
    }

    // Determine exit code
    let has_failures = stats.failed > 0 || stats.errors > 0;
    if strict && has_failures {
        Ok(1)
    } else if stats.passed == 0 && stats.total > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Check if a test type is a compile-time test.
fn is_compile_time_test(test_type: vtest::directive::TestType) -> bool {
    use vtest::directive::TestType;
    matches!(
        test_type,
        TestType::ParsePass
            | TestType::ParseFail
            | TestType::ParseRecover
            | TestType::TypecheckPass
            | TestType::TypecheckFail
            | TestType::VerifyPass
            | TestType::VerifyFail
            | TestType::CommonPipeline
            | TestType::CommonPipelineFail
            | TestType::CompileOnly
    )
}

/// Statistics for level-based verification.
#[derive(Debug, Default)]
struct LevelVerifyStats {
    total: usize,
    passed: usize,
    failed: usize,
}

/// Statistics for type-based verification.
#[derive(Debug, Default)]
struct TypeVerifyStats {
    total: usize,
    passed: usize,
    failed: usize,
}

/// Detail about a single failure.
#[derive(Debug)]
struct FailureDetail {
    path: Text,
    test_type: vtest::directive::TestType,
    level: vtest::directive::Level,
    phase: String,
    reason: String,
}

fn output_console_report(
    stats: &vtest::executor::verify::VerificationStats,
    by_level: &std::collections::HashMap<vtest::directive::Level, LevelVerifyStats>,
    by_type: &std::collections::HashMap<vtest::directive::TestType, TypeVerifyStats>,
    failures: &[FailureDetail],
    duration: std::time::Duration,
    show_failures: usize,
    use_colors: bool,
    quiet: bool,
) {
    use vtest::directive::{Level, TestType};

    if quiet && failures.is_empty() {
        println!("All {} tests passed", stats.passed);
        return;
    }

    println!();
    println!("{}", "═".repeat(65));
    println!("  VERIFICATION SUMMARY");
    println!("{}", "─".repeat(65));

    // Overall stats
    let total_run = stats.total;
    let pass_rate = if total_run > 0 {
        100.0 * stats.passed as f64 / total_run as f64
    } else {
        0.0
    };

    println!(
        "  Total:     {} tests",
        total_run.to_string().bold()
    );
    println!(
        "  Passed:    {} ({:.1}%)",
        stats.passed.to_string().green().bold(),
        pass_rate
    );
    if stats.failed > 0 {
        println!(
            "  Failed:    {} ({:.1}%)",
            stats.failed.to_string().red().bold(),
            100.0 * stats.failed as f64 / total_run as f64
        );
    }
    if stats.errors > 0 {
        println!(
            "  Errors:    {} ({:.1}%)",
            stats.errors.to_string().red().bold(),
            100.0 * stats.errors as f64 / total_run as f64
        );
    }
    if stats.skipped > 0 {
        println!(
            "  Skipped:   {} ({:.1}%)",
            stats.skipped.to_string().yellow(),
            100.0 * stats.skipped as f64 / (total_run + stats.skipped) as f64
        );
    }

    // By level
    println!();
    println!("  By Level:");
    let level_order = [Level::L0, Level::L1, Level::L2, Level::L3, Level::L4];
    for level in &level_order {
        if let Some(lstats) = by_level.get(level) {
            let rate = if lstats.total > 0 {
                100.0 * lstats.passed as f64 / lstats.total as f64
            } else {
                0.0
            };
            let rate_str = if rate >= 100.0 {
                format!("{:.1}%", rate).green().to_string()
            } else if rate >= 95.0 {
                format!("{:.1}%", rate).yellow().to_string()
            } else {
                format!("{:.1}%", rate).red().to_string()
            };
            println!(
                "    {}: {}/{} ({})",
                level.to_string().bold(),
                lstats.passed,
                lstats.total,
                rate_str
            );
        }
    }

    // By test type
    println!();
    println!("  By Test Type:");
    let type_order = [
        TestType::ParsePass,
        TestType::ParseFail,
        TestType::ParseRecover,
        TestType::TypecheckPass,
        TestType::TypecheckFail,
        TestType::VerifyPass,
        TestType::VerifyFail,
        TestType::CommonPipeline,
        TestType::CommonPipelineFail,
        TestType::CompileOnly,
    ];
    for ttype in &type_order {
        if let Some(tstats) = by_type.get(ttype) {
            let rate = if tstats.total > 0 {
                100.0 * tstats.passed as f64 / tstats.total as f64
            } else {
                0.0
            };
            let rate_str = if rate >= 100.0 {
                format!("{:.1}%", rate).green().to_string()
            } else if rate >= 95.0 {
                format!("{:.1}%", rate).yellow().to_string()
            } else {
                format!("{:.1}%", rate).red().to_string()
            };
            println!(
                "    {}: {}/{} ({})",
                ttype.to_string().bold(),
                tstats.passed,
                tstats.total,
                rate_str
            );
        }
    }

    println!();
    println!("  Duration: {:.2}s", duration.as_secs_f64());
    println!("{}", "═".repeat(65));

    // Print failures
    if !failures.is_empty() && show_failures > 0 {
        let show_count = std::cmp::min(show_failures, failures.len());
        println!();
        println!(
            "  {} (showing first {}):",
            "FAILURES".red().bold(),
            show_count
        );
        println!();

        for (i, failure) in failures.iter().take(show_count).enumerate() {
            println!("  {}. {}", i + 1, failure.path.as_str().cyan());
            println!(
                "     Type: {}, Level: {}, Phase: {}",
                failure.test_type,
                failure.level,
                failure.phase.dimmed()
            );
            println!("     Reason: {}", failure.reason.red());
            println!();
        }

        if failures.len() > show_count {
            println!(
                "  ... and {} more failures (use --show-failures to see more)",
                failures.len() - show_count
            );
        }
    }

    // Final result
    println!();
    if stats.failed == 0 && stats.errors == 0 {
        println!("  {} All compile-time tests passed!", "RESULT:".green().bold());
    } else {
        println!(
            "  {} {} failures, {} errors",
            "RESULT:".red().bold(),
            stats.failed,
            stats.errors
        );
    }
    println!();
}

fn output_json_report(
    stats: &vtest::executor::verify::VerificationStats,
    by_level: &std::collections::HashMap<vtest::directive::Level, LevelVerifyStats>,
    by_type: &std::collections::HashMap<vtest::directive::TestType, TypeVerifyStats>,
    failures: &[FailureDetail],
    duration: std::time::Duration,
    output: Option<PathBuf>,
) -> Result<(), RunnerError> {
    let report = serde_json::json!({
        "summary": {
            "total": stats.total,
            "passed": stats.passed,
            "failed": stats.failed,
            "errors": stats.errors,
            "skipped": stats.skipped,
            "pass_rate": if stats.total > 0 { 100.0 * stats.passed as f64 / stats.total as f64 } else { 0.0 },
            "duration_secs": duration.as_secs_f64(),
        },
        "by_level": by_level.iter().map(|(l, s)| {
            (l.to_string(), serde_json::json!({
                "total": s.total,
                "passed": s.passed,
                "failed": s.failed,
                "pass_rate": if s.total > 0 { 100.0 * s.passed as f64 / s.total as f64 } else { 0.0 },
            }))
        }).collect::<serde_json::Map<_, _>>(),
        "by_type": by_type.iter().map(|(t, s)| {
            (t.to_string(), serde_json::json!({
                "total": s.total,
                "passed": s.passed,
                "failed": s.failed,
                "pass_rate": if s.total > 0 { 100.0 * s.passed as f64 / s.total as f64 } else { 0.0 },
            }))
        }).collect::<serde_json::Map<_, _>>(),
        "failures": failures.iter().map(|f| {
            serde_json::json!({
                "path": f.path.as_str(),
                "test_type": f.test_type.to_string(),
                "level": f.level.to_string(),
                "phase": f.phase,
                "reason": f.reason,
            })
        }).collect::<Vec<_>>(),
    });

    let json = serde_json::to_string_pretty(&report)
        .map_err(|e| RunnerError::ConfigError(format!("JSON error: {}", e).into()))?;

    if let Some(path) = output {
        std::fs::write(&path, &json)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", json);
    }

    Ok(())
}

fn output_markdown_report(
    stats: &vtest::executor::verify::VerificationStats,
    by_level: &std::collections::HashMap<vtest::directive::Level, LevelVerifyStats>,
    by_type: &std::collections::HashMap<vtest::directive::TestType, TypeVerifyStats>,
    failures: &[FailureDetail],
    duration: std::time::Duration,
    output: Option<PathBuf>,
) -> Result<(), RunnerError> {
    use vtest::directive::{Level, TestType};

    let mut md = String::new();

    md.push_str("# Common Pipeline Verification Report\n\n");
    md.push_str(&format!("Generated: {}\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str("| Metric | Value |\n");
    md.push_str("|--------|-------|\n");
    md.push_str(&format!("| Total Tests | {} |\n", stats.total));
    md.push_str(&format!("| Passed | {} |\n", stats.passed));
    md.push_str(&format!("| Failed | {} |\n", stats.failed));
    md.push_str(&format!("| Errors | {} |\n", stats.errors));
    md.push_str(&format!("| Skipped | {} |\n", stats.skipped));
    let pass_rate = if stats.total > 0 { 100.0 * stats.passed as f64 / stats.total as f64 } else { 0.0 };
    md.push_str(&format!("| Pass Rate | {:.1}% |\n", pass_rate));
    md.push_str(&format!("| Duration | {:.2}s |\n", duration.as_secs_f64()));
    md.push_str("\n");

    // By Level
    md.push_str("## Results by Level\n\n");
    md.push_str("| Level | Passed | Total | Rate |\n");
    md.push_str("|-------|--------|-------|------|\n");
    for level in [Level::L0, Level::L1, Level::L2, Level::L3, Level::L4] {
        if let Some(lstats) = by_level.get(&level) {
            let rate = if lstats.total > 0 { 100.0 * lstats.passed as f64 / lstats.total as f64 } else { 0.0 };
            md.push_str(&format!("| {} | {} | {} | {:.1}% |\n", level, lstats.passed, lstats.total, rate));
        }
    }
    md.push_str("\n");

    // By Type
    md.push_str("## Results by Test Type\n\n");
    md.push_str("| Type | Passed | Total | Rate |\n");
    md.push_str("|------|--------|-------|------|\n");
    for ttype in [
        TestType::ParsePass, TestType::ParseFail, TestType::ParseRecover,
        TestType::TypecheckPass, TestType::TypecheckFail,
        TestType::VerifyPass, TestType::VerifyFail,
        TestType::CommonPipeline, TestType::CommonPipelineFail,
        TestType::CompileOnly,
    ] {
        if let Some(tstats) = by_type.get(&ttype) {
            let rate = if tstats.total > 0 { 100.0 * tstats.passed as f64 / tstats.total as f64 } else { 0.0 };
            md.push_str(&format!("| {} | {} | {} | {:.1}% |\n", ttype, tstats.passed, tstats.total, rate));
        }
    }
    md.push_str("\n");

    // Failures
    if !failures.is_empty() {
        md.push_str("## Failures\n\n");
        for (i, failure) in failures.iter().enumerate() {
            md.push_str(&format!("### {}. `{}`\n\n", i + 1, failure.path.as_str()));
            md.push_str(&format!("- **Type**: {}\n", failure.test_type));
            md.push_str(&format!("- **Level**: {}\n", failure.level));
            md.push_str(&format!("- **Phase**: {}\n", failure.phase));
            md.push_str(&format!("- **Reason**: {}\n", failure.reason));
            md.push_str("\n");
        }
    }

    if let Some(path) = output {
        std::fs::write(&path, &md)?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", md);
    }

    Ok(())
}

/// Run parser tests from vcs/specs/parser.
///
/// This function runs parser tests in two categories:
/// - success/ - parse-pass tests that should parse without errors
/// - fail/ - parse-fail tests that should produce expected errors
#[allow(clippy::too_many_arguments)]
async fn run_parser_tests(
    paths: Vec<PathBuf>,
    tags: Vec<Text>,
    exclude_tags: Vec<Text>,
    parallel: Text,
    format: Text,
    output: Option<PathBuf>,
    timeout: u64,
    fail_fast: bool,
    summary_only: bool,
    show_failures: usize,
    filter: Option<Text>,
    advanced: bool,
    success_only: bool,
    fail_only: bool,
    verbosity: Verbosity,
    use_colors: bool,
) -> Result<i32, RunnerError> {
    use std::collections::HashMap;
    use std::time::Instant;
    use walkdir::WalkDir;
    use verum_ast::FileId;
    use verum_lexer::Lexer;

    let start = Instant::now();
    let verbose = matches!(verbosity, Verbosity::Debug);
    let quiet = matches!(verbosity, Verbosity::Quiet | Verbosity::Summary);

    // Determine parser name for display
    let parser_name = if advanced { "verum_parser (advanced)" } else { "verum_fast_parser" };

    // Print header
    if !quiet {
        println!();
        println!("{}", "═".repeat(65));
        println!("  {} - Parser Test Suite", "VTEST".bold());
        println!("  Parser: {}", parser_name.cyan());
        println!("{}", "═".repeat(65));
        println!();
    }

    // Determine test paths
    let test_paths: Vec<PathBuf> = if paths.is_empty() {
        // Default to vcs/specs/parser
        let default_path = PathBuf::from("vcs/specs/parser");
        if default_path.exists() {
            vec![default_path]
        } else {
            // Try relative to current dir
            let cwd_path = std::env::current_dir()
                .map(|p| p.join("specs/parser"))
                .unwrap_or_else(|_| PathBuf::from("specs/parser"));
            if cwd_path.exists() {
                vec![cwd_path]
            } else {
                return Err(RunnerError::ConfigError(
                    "Parser test directory not found. Expected vcs/specs/parser or specs/parser".into()
                ));
            }
        }
    } else {
        paths
    };

    // Collect all .vr files
    let mut test_files: Vec<(PathBuf, bool)> = Vec::new(); // (path, is_success_test)

    for base_path in &test_paths {
        for entry in WalkDir::new(base_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                // Apply filter if specified
                if let Some(ref pattern) = filter {
                    let path_str = path.to_string_lossy();
                    if !path_str.contains(pattern.as_str()) {
                        continue;
                    }
                }

                // Determine if it's a success or fail test based on directory
                let path_str = path.to_string_lossy();
                let is_success = path_str.contains("/success/") || path_str.contains("\\success\\");
                let is_fail = path_str.contains("/fail/") || path_str.contains("\\fail\\");

                // Filter by success_only or fail_only
                if success_only && !is_success {
                    continue;
                }
                if fail_only && !is_fail {
                    continue;
                }

                // Skip if neither success nor fail (shouldn't happen with proper structure)
                if !is_success && !is_fail {
                    continue;
                }

                test_files.push((path.to_path_buf(), is_success));
            }
        }
    }

    test_files.sort_by(|a, b| a.0.cmp(&b.0));

    if test_files.is_empty() {
        println!("  {} No parser tests found in {:?}", "Warning:".yellow(), test_paths);
        return Ok(0);
    }

    let success_count = test_files.iter().filter(|(_, is_success)| *is_success).count();
    let fail_count = test_files.len() - success_count;

    if !quiet {
        println!("  Found {} tests ({} success, {} fail)",
            test_files.len().to_string().cyan(),
            success_count.to_string().green(),
            fail_count.to_string().yellow());
        println!();
    }

    // Statistics
    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut by_category: HashMap<String, CategoryStats> = HashMap::new();
    let mut failures: Vec<ParserTestFailure> = Vec::new();

    // Progress bar
    let show_progress = !verbose && !quiet;
    let progress_bar = if show_progress {
        let pb = indicatif::ProgressBar::new(test_files.len() as u64);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
        Some(pb)
    } else {
        None
    };

    let should_stop = std::sync::atomic::AtomicBool::new(false);

    for (idx, (file_path, is_success)) in test_files.iter().enumerate() {
        if should_stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        total += 1;

        // Categorize by subdirectory
        let category = file_path
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".to_string());

        let cat_stats = by_category.entry(category.clone()).or_default();
        cat_stats.total += 1;

        // Read the file
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                failed += 1;
                cat_stats.failed += 1;
                failures.push(ParserTestFailure {
                    path: file_path.to_string_lossy().to_string(),
                    category: category.clone(),
                    is_success_test: *is_success,
                    error: format!("Failed to read file: {}", e),
                    expected_error: None,
                });
                continue;
            }
        };

        // Parse the directives from the file
        let expected_error = parse_expected_error(&source);

        // Create lexer and parser
        let file_id = FileId::new((idx + 1) as u32);

        // Run parser and evaluate result
        // We need to handle both parser types separately due to different error types
        let test_passed = if advanced {
            // Use verum_parser (advanced) - still needs lexer
            let lexer = Lexer::new(&source, file_id);
            let parser = verum_parser::VerumParser::new();
            let parse_result = parser.parse_module(lexer, file_id);
            evaluate_parse_result_advanced(
                parse_result, *is_success, &expected_error, file_path, &category, &mut failures
            )
        } else {
            // Use verum_fast_parser (default) - use parse_module_str for better error analysis
            let parser = verum_fast_parser::FastParser::new();
            let parse_result = parser.parse_module_str(&source, file_id);
            evaluate_parse_result_fast(
                parse_result, *is_success, &expected_error, file_path, &category, &mut failures
            )
        };

        if test_passed {
            passed += 1;
            cat_stats.passed += 1;
            if verbose && !summary_only {
                let rel_path = file_path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.to_string_lossy().to_string());
                let test_type = if *is_success { "success" } else { "fail" };
                println!("  {} {} ({})", "PASS".green(), rel_path, test_type.dimmed());
            }
        } else {
            failed += 1;
            cat_stats.failed += 1;
            if verbose && !summary_only {
                let rel_path = file_path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.to_string_lossy().to_string());
                println!("  {} {}", "FAIL".red(), rel_path);
            }
            if fail_fast {
                should_stop.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }

        if let Some(ref pb) = progress_bar {
            pb.set_message(format!("{} pass, {} fail", passed, failed));
            pb.inc(1);
        }
    }

    if let Some(pb) = progress_bar {
        pb.finish_and_clear();
    }

    let duration = start.elapsed();

    // Output based on format
    let output_format = format.to_lowercase();
    match output_format.as_str() {
        "json" => {
            output_parser_json_report(
                total, passed, failed,
                &by_category, &failures, duration, output, parser_name,
            )?;
        }
        "markdown" | "md" => {
            output_parser_markdown_report(
                total, passed, failed,
                &by_category, &failures, duration, output, parser_name,
            )?;
        }
        _ => {
            output_parser_console_report(
                total, passed, failed,
                &by_category, &failures, duration, show_failures, use_colors, quiet, parser_name,
            );
        }
    }

    // Return exit code
    if failed > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Parse expected error code from test file directives.
fn parse_expected_error(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("// @expected-error:") {
            return Some(trimmed.trim_start_matches("// @expected-error:").trim().to_string());
        }
    }
    None
}

/// Evaluate parse result from verum_fast_parser.
fn evaluate_parse_result_fast(
    parse_result: Result<verum_ast::Module, verum_common::List<verum_fast_parser::ParseError>>,
    is_success: bool,
    expected_error: &Option<String>,
    file_path: &std::path::Path,
    category: &str,
    failures: &mut Vec<ParserTestFailure>,
) -> bool {
    if is_success {
        // Success test: should parse without errors
        match parse_result {
            Ok(_) => true,
            Err(errors) => {
                failures.push(ParserTestFailure {
                    path: file_path.to_string_lossy().to_string(),
                    category: category.to_string(),
                    is_success_test: true,
                    error: format!("Expected success but got {} errors: {:?}", errors.len(), errors),
                    expected_error: None,
                });
                false
            }
        }
    } else {
        // Fail test: should produce errors
        match parse_result {
            Ok(_) => {
                failures.push(ParserTestFailure {
                    path: file_path.to_string_lossy().to_string(),
                    category: category.to_string(),
                    is_success_test: false,
                    error: "Expected parse error but parsing succeeded".to_string(),
                    expected_error: expected_error.clone(),
                });
                false
            }
            Err(errors) => {
                // Check if expected error matches (if specified)
                if let Some(expected) = expected_error {
                    let error_str = format!("{:?}", errors);
                    if error_str.contains(expected) || errors.iter().any(|e| {
                        let e_str = format!("{:?}", e);
                        e_str.contains(expected)
                    }) {
                        true
                    } else {
                        failures.push(ParserTestFailure {
                            path: file_path.to_string_lossy().to_string(),
                            category: category.to_string(),
                            is_success_test: false,
                            error: format!("Error code mismatch. Got: {:?}", errors),
                            expected_error: Some(expected.clone()),
                        });
                        false
                    }
                } else {
                    // No specific error expected, any error is fine
                    true
                }
            }
        }
    }
}

/// Evaluate parse result from verum_parser (advanced).
fn evaluate_parse_result_advanced(
    parse_result: Result<verum_ast::Module, verum_common::List<verum_parser::ParseError>>,
    is_success: bool,
    expected_error: &Option<String>,
    file_path: &std::path::Path,
    category: &str,
    failures: &mut Vec<ParserTestFailure>,
) -> bool {
    if is_success {
        // Success test: should parse without errors
        match parse_result {
            Ok(_) => true,
            Err(errors) => {
                failures.push(ParserTestFailure {
                    path: file_path.to_string_lossy().to_string(),
                    category: category.to_string(),
                    is_success_test: true,
                    error: format!("Expected success but got {} errors: {:?}", errors.len(), errors),
                    expected_error: None,
                });
                false
            }
        }
    } else {
        // Fail test: should produce errors
        match parse_result {
            Ok(_) => {
                failures.push(ParserTestFailure {
                    path: file_path.to_string_lossy().to_string(),
                    category: category.to_string(),
                    is_success_test: false,
                    error: "Expected parse error but parsing succeeded".to_string(),
                    expected_error: expected_error.clone(),
                });
                false
            }
            Err(errors) => {
                // Check if expected error matches (if specified)
                if let Some(expected) = expected_error {
                    let error_str = format!("{:?}", errors);
                    if error_str.contains(expected) || errors.iter().any(|e| {
                        let e_str = format!("{:?}", e);
                        e_str.contains(expected)
                    }) {
                        true
                    } else {
                        failures.push(ParserTestFailure {
                            path: file_path.to_string_lossy().to_string(),
                            category: category.to_string(),
                            is_success_test: false,
                            error: format!("Error code mismatch. Got: {:?}", errors),
                            expected_error: Some(expected.clone()),
                        });
                        false
                    }
                } else {
                    // No specific error expected, any error is fine
                    true
                }
            }
        }
    }
}

// ============================================================================
// Unified Golden Test Output System
// ============================================================================

/// Unified category statistics for golden tests.
#[derive(Debug, Default, Clone)]
struct GoldenCategoryStats {
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
}

/// Unified test failure information for golden tests.
#[derive(Debug, Clone)]
struct GoldenTestFailure {
    path: String,
    category: String,
    test_type: String,
    error: String,
    expected_error: Option<String>,
    expected_value: Option<String>,
}

/// Configuration for golden test report output.
#[derive(Debug)]
struct GoldenReportConfig<'a> {
    /// Title for the test suite (e.g., "Parser Tests", "Meta-System Tests")
    suite_name: &'a str,
    /// Subtitle/additional info (e.g., parser name, phase description)
    subtitle: Option<&'a str>,
    /// Label for categories column (e.g., "Category", "Subsystem")
    category_label: &'a str,
    /// Whether to show skipped count
    show_skipped: bool,
}

/// Unified console report for golden tests (meta-style formatting).
fn output_golden_console_report(
    config: &GoldenReportConfig,
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    by_category: &std::collections::HashMap<String, GoldenCategoryStats>,
    failures: &[GoldenTestFailure],
    duration: std::time::Duration,
    show_failures: usize,
    _use_colors: bool,
    quiet: bool,
) {
    if quiet {
        // Just print summary line
        if failed > 0 {
            if config.show_skipped {
                println!("FAIL {} passed, {} failed, {} skipped ({:.2}s)",
                    passed, failed, skipped, duration.as_secs_f64());
            } else {
                println!("FAIL {} passed, {} failed ({:.2}s)",
                    passed, failed, duration.as_secs_f64());
            }
        } else {
            if config.show_skipped && skipped > 0 {
                println!("PASS {} passed, {} skipped ({:.2}s)",
                    passed, skipped, duration.as_secs_f64());
            } else {
                println!("PASS {} passed ({:.2}s)", passed, duration.as_secs_f64());
            }
        }
        return;
    }

    println!();
    println!("{}", "─".repeat(70));
    if let Some(subtitle) = config.subtitle {
        println!("  {} · {}", config.suite_name.bold(), subtitle.dimmed());
    } else {
        println!("  {}", config.suite_name.bold());
    }
    println!("{}", "─".repeat(70));
    println!();

    // Summary
    let pass_rate = if total > 0 { passed as f64 / total as f64 * 100.0 } else { 0.0 };

    println!("  Total:     {}", total.to_string().bold());
    println!("  Passed:    {}", passed.to_string().green());
    println!("  Failed:    {}", if failed > 0 { failed.to_string().red().to_string() } else { "0".to_string() });
    if config.show_skipped {
        println!("  Skipped:   {}", if skipped > 0 { skipped.to_string().yellow().to_string() } else { "0".to_string() });
    }
    println!("  Pass Rate: {:.1}%", pass_rate);
    println!("  Duration:  {:.2}s", duration.as_secs_f64());
    println!();

    // By category table
    println!("  {}:", format!("BY {}", config.category_label.to_uppercase()).bold());
    if config.show_skipped {
        println!("  {:<24} {:>8} {:>8} {:>8} {:>8}", config.category_label, "Total", "Pass", "Fail", "Skip");
        println!("  {}", "─".repeat(64));
    } else {
        println!("  {:<24} {:>8} {:>8} {:>8}", config.category_label, "Total", "Pass", "Fail");
        println!("  {}", "─".repeat(56));
    }

    let mut categories: Vec<_> = by_category.iter().collect();
    categories.sort_by(|a, b| a.0.cmp(b.0));

    for (name, stats) in categories {
        let status = if stats.failed > 0 {
            "FAIL".red()
        } else {
            "PASS".green()
        };
        if config.show_skipped {
            println!("  {:<24} {:>8} {:>8} {:>8} {:>8}  {}",
                name, stats.total, stats.passed, stats.failed, stats.skipped, status);
        } else {
            println!("  {:<24} {:>8} {:>8} {:>8}  {}",
                name, stats.total, stats.passed, stats.failed, status);
        }
    }
    println!();

    // Failures
    if !failures.is_empty() && show_failures > 0 {
        println!("  {}", "FAILURES:".bold().red());
        println!();

        for (idx, failure) in failures.iter().take(show_failures).enumerate() {
            println!("  {}. {}", idx + 1, failure.path.red());
            println!("     Category:  {}", failure.category.dimmed());
            println!("     Type:      {}", failure.test_type);
            // Truncate error for display
            let error_preview: String = failure.error.chars().take(300).collect();
            let error_lines: Vec<&str> = error_preview.lines().take(3).collect();
            println!("     Error:     {}", error_lines.join("\n              "));
            if let Some(ref expected) = failure.expected_error {
                println!("     Expected:  {}", expected.yellow());
            }
            if let Some(ref expected) = failure.expected_value {
                println!("     Expected Value: {}", expected.cyan());
            }
            println!();
        }

        if failures.len() > show_failures {
            println!("  ... and {} more failures (use --show-failures to see more)", failures.len() - show_failures);
            println!();
        }
    }

    // Final status
    println!("{}", "═".repeat(70));
    if failed > 0 {
        println!("  {} {} completed with {} failure{}",
            "FAIL".red().bold(),
            config.suite_name,
            failed,
            if failed == 1 { "" } else { "s" });
    } else {
        println!("  {} All {} {} passed!", "PASS".green().bold(), passed, config.suite_name.to_lowercase());
    }
    println!("{}", "═".repeat(70));
}

/// Unified JSON report for golden tests.
fn output_golden_json_report(
    config: &GoldenReportConfig,
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    by_category: &std::collections::HashMap<String, GoldenCategoryStats>,
    failures: &[GoldenTestFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
) -> Result<(), RunnerError> {
    let report = serde_json::json!({
        "suite": config.suite_name,
        "subtitle": config.subtitle,
        "summary": {
            "total": total,
            "passed": passed,
            "failed": failed,
            "skipped": skipped,
            "pass_rate": if total > 0 { 100.0 * passed as f64 / total as f64 } else { 0.0 },
            "duration_secs": duration.as_secs_f64(),
        },
        "by_category": by_category.iter().map(|(name, stats)| {
            (name.clone(), serde_json::json!({
                "total": stats.total,
                "passed": stats.passed,
                "failed": stats.failed,
                "skipped": stats.skipped,
            }))
        }).collect::<serde_json::Map<_, _>>(),
        "failures": failures.iter().map(|f| {
            serde_json::json!({
                "path": f.path,
                "category": f.category,
                "test_type": f.test_type,
                "error": f.error,
                "expected_error": f.expected_error,
                "expected_value": f.expected_value,
            })
        }).collect::<Vec<_>>(),
    });

    let json_str = serde_json::to_string_pretty(&report)
        .map_err(|e| RunnerError::IoError(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

    if let Some(path) = output {
        std::fs::write(&path, &json_str)
            .map_err(|e| RunnerError::IoError(e))?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", json_str);
    }

    Ok(())
}

/// Unified Markdown report for golden tests.
fn output_golden_markdown_report(
    config: &GoldenReportConfig,
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    by_category: &std::collections::HashMap<String, GoldenCategoryStats>,
    failures: &[GoldenTestFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
) -> Result<(), RunnerError> {
    let mut md = String::new();

    // Header
    md.push_str(&format!("# {}\n\n", config.suite_name));
    if let Some(subtitle) = config.subtitle {
        md.push_str(&format!("_{}_\n\n", subtitle));
    }

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str(&format!("| Metric | Value |\n"));
    md.push_str(&format!("|--------|-------|\n"));
    md.push_str(&format!("| Total | {} |\n", total));
    md.push_str(&format!("| Passed | {} |\n", passed));
    md.push_str(&format!("| Failed | {} |\n", failed));
    if config.show_skipped {
        md.push_str(&format!("| Skipped | {} |\n", skipped));
    }
    let pass_rate = if total > 0 { 100.0 * passed as f64 / total as f64 } else { 0.0 };
    md.push_str(&format!("| Pass Rate | {:.1}% |\n", pass_rate));
    md.push_str(&format!("| Duration | {:.2}s |\n", duration.as_secs_f64()));
    md.push_str("\n");

    // By category
    md.push_str(&format!("## By {}\n\n", config.category_label));
    if config.show_skipped {
        md.push_str(&format!("| {} | Total | Pass | Fail | Skip | Status |\n", config.category_label));
        md.push_str("|---|-------|------|------|------|--------|\n");
    } else {
        md.push_str(&format!("| {} | Total | Pass | Fail | Status |\n", config.category_label));
        md.push_str("|---|-------|------|------|--------|\n");
    }

    let mut categories: Vec<_> = by_category.iter().collect();
    categories.sort_by(|a, b| a.0.cmp(b.0));

    for (name, stats) in categories {
        let status = if stats.failed > 0 { "❌ FAIL" } else { "✅ PASS" };
        if config.show_skipped {
            md.push_str(&format!("| {} | {} | {} | {} | {} | {} |\n",
                name, stats.total, stats.passed, stats.failed, stats.skipped, status));
        } else {
            md.push_str(&format!("| {} | {} | {} | {} | {} |\n",
                name, stats.total, stats.passed, stats.failed, status));
        }
    }
    md.push_str("\n");

    // Failures
    if !failures.is_empty() {
        md.push_str("## Failures\n\n");
        for (idx, failure) in failures.iter().enumerate() {
            md.push_str(&format!("### {}. `{}`\n\n", idx + 1, failure.path));
            md.push_str(&format!("- **Category:** {}\n", failure.category));
            md.push_str(&format!("- **Type:** {}\n", failure.test_type));
            if let Some(ref expected) = failure.expected_error {
                md.push_str(&format!("- **Expected:** {}\n", expected));
            }
            if let Some(ref expected) = failure.expected_value {
                md.push_str(&format!("- **Expected Value:** {}\n", expected));
            }
            md.push_str(&format!("\n```\n{}\n```\n\n", failure.error));
        }
    }

    if let Some(path) = output {
        std::fs::write(&path, &md)
            .map_err(|e| RunnerError::IoError(e))?;
        println!("Report written to: {}", path.display());
    } else {
        println!("{}", md);
    }

    Ok(())
}

// ============================================================================
// Parser Tests (Legacy structures for backwards compatibility)
// ============================================================================

/// Category statistics for parser tests.
#[derive(Debug, Default)]
struct CategoryStats {
    total: usize,
    passed: usize,
    failed: usize,
}

/// Detail about a parser test failure.
#[derive(Debug)]
struct ParserTestFailure {
    path: String,
    category: String,
    is_success_test: bool,
    error: String,
    expected_error: Option<String>,
}

// Conversion helpers for parser tests
impl From<&CategoryStats> for GoldenCategoryStats {
    fn from(stats: &CategoryStats) -> Self {
        GoldenCategoryStats {
            total: stats.total,
            passed: stats.passed,
            failed: stats.failed,
            skipped: 0,
        }
    }
}

impl From<&ParserTestFailure> for GoldenTestFailure {
    fn from(f: &ParserTestFailure) -> Self {
        GoldenTestFailure {
            path: f.path.clone(),
            category: f.category.clone(),
            test_type: if f.is_success_test { "success".to_string() } else { "fail".to_string() },
            error: f.error.clone(),
            expected_error: f.expected_error.clone(),
            expected_value: None,
        }
    }
}

fn output_parser_console_report(
    total: usize,
    passed: usize,
    failed: usize,
    by_category: &std::collections::HashMap<String, CategoryStats>,
    failures: &[ParserTestFailure],
    duration: std::time::Duration,
    show_failures: usize,
    use_colors: bool,
    quiet: bool,
    parser_name: &str,
) {
    // Convert to unified format
    let golden_categories: std::collections::HashMap<String, GoldenCategoryStats> =
        by_category.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    let golden_failures: Vec<GoldenTestFailure> =
        failures.iter().map(|f| f.into()).collect();

    let config = GoldenReportConfig {
        suite_name: "Parser Tests",
        subtitle: Some(parser_name),
        category_label: "Category",
        show_skipped: false,
    };

    output_golden_console_report(
        &config,
        total,
        passed,
        failed,
        0, // no skipped for parser
        &golden_categories,
        &golden_failures,
        duration,
        show_failures,
        use_colors,
        quiet,
    );
}

#[allow(clippy::too_many_arguments)]
fn output_parser_json_report(
    total: usize,
    passed: usize,
    failed: usize,
    by_category: &std::collections::HashMap<String, CategoryStats>,
    failures: &[ParserTestFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
    parser_name: &str,
) -> Result<(), RunnerError> {
    // Convert to unified format
    let golden_categories: std::collections::HashMap<String, GoldenCategoryStats> =
        by_category.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    let golden_failures: Vec<GoldenTestFailure> =
        failures.iter().map(|f| f.into()).collect();

    let config = GoldenReportConfig {
        suite_name: "Parser Tests",
        subtitle: Some(parser_name),
        category_label: "Category",
        show_skipped: false,
    };

    output_golden_json_report(
        &config,
        total,
        passed,
        failed,
        0,
        &golden_categories,
        &golden_failures,
        duration,
        output,
    )
}

#[allow(clippy::too_many_arguments)]
fn output_parser_markdown_report(
    total: usize,
    passed: usize,
    failed: usize,
    by_category: &std::collections::HashMap<String, CategoryStats>,
    failures: &[ParserTestFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
    parser_name: &str,
) -> Result<(), RunnerError> {
    // Convert to unified format
    let golden_categories: std::collections::HashMap<String, GoldenCategoryStats> =
        by_category.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    let golden_failures: Vec<GoldenTestFailure> =
        failures.iter().map(|f| f.into()).collect();

    let config = GoldenReportConfig {
        suite_name: "Parser Tests",
        subtitle: Some(parser_name),
        category_label: "Category",
        show_skipped: false,
    };

    output_golden_markdown_report(
        &config,
        total,
        passed,
        failed,
        0,
        &golden_categories,
        &golden_failures,
        duration,
        output,
    )
}


/// Get the compiler version by running verum --version.
fn get_compiler_version() -> Text {
    std::process::Command::new("verum")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
        .into()
}

// =============================================================================
// META-SYSTEM TEST RUNNER
// =============================================================================

/// Statistics for a meta test category (subsystem).
#[derive(Default, Debug)]
struct MetaCategoryStats {
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
}

/// Represents a failure in a meta test.
#[derive(Debug)]
struct MetaTestFailure {
    path: Text,
    category: Text,
    test_type: Text,
    error: Text,
    expected_error: Option<Text>,
    expected_value: Option<Text>,
}

// Conversion helpers for meta tests
impl From<&MetaCategoryStats> for GoldenCategoryStats {
    fn from(stats: &MetaCategoryStats) -> Self {
        GoldenCategoryStats {
            total: stats.total,
            passed: stats.passed,
            failed: stats.failed,
            skipped: stats.skipped,
        }
    }
}

impl From<&MetaTestFailure> for GoldenTestFailure {
    fn from(f: &MetaTestFailure) -> Self {
        GoldenTestFailure {
            path: f.path.to_string(),
            category: f.category.to_string(),
            test_type: f.test_type.to_string(),
            error: f.error.to_string(),
            expected_error: f.expected_error.as_ref().map(|s| s.to_string()),
            expected_value: f.expected_value.as_ref().map(|s| s.to_string()),
        }
    }
}

/// Represents a meta test file with its directives.
#[derive(Debug)]
struct MetaTestFile {
    path: PathBuf,
    test_type: MetaTestType,
    expected_error: Option<Text>,
    expected_value: Option<Text>,
    expected_type: Option<Text>,
    contexts: Set<Text>,
    level: Level,
    tags: Set<Text>,
    skip: Option<Text>,
}

/// Meta test types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaTestType {
    Pass,  // meta-pass: should compile and evaluate without errors
    Fail,  // meta-fail: should fail with expected error
    Eval,  // meta-eval: should evaluate to expected value
}

impl MetaTestType {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "meta-pass" | "metapass" => Some(Self::Pass),
            "meta-fail" | "metafail" => Some(Self::Fail),
            "meta-eval" | "metaeval" => Some(Self::Eval),
            _ => None,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "meta-pass",
            Self::Fail => "meta-fail",
            Self::Eval => "meta-eval",
        }
    }
}

/// Run meta-system tests.
///
/// This function runs compile-time meta-system tests through the compiler pipeline
/// up to TypedAST generation (meta evaluation phase), without requiring runtime.
#[allow(clippy::too_many_arguments)]
async fn run_meta_tests(
    paths: Vec<PathBuf>,
    subsystem: Vec<Text>,
    level_filter: Vec<Text>,
    tags: Vec<Text>,
    exclude_tags: Vec<Text>,
    parallel: Text,
    format: Text,
    output: Option<PathBuf>,
    _timeout: u64,
    fail_fast: bool,
    summary_only: bool,
    show_failures: usize,
    filter: Option<Text>,
    success_only: bool,
    fail_only: bool,
    verbose_flag: bool,
    verbosity: Verbosity,
    use_colors: bool,
) -> Result<i32, RunnerError> {
    use std::collections::HashMap;
    use std::time::Instant;
    use walkdir::WalkDir;
    use verum_ast::FileId;

    let start = Instant::now();
    let verbose = verbose_flag || matches!(verbosity, Verbosity::Debug);
    let quiet = matches!(verbosity, Verbosity::Quiet | Verbosity::Summary);

    // Parse filters
    let level_set = parse_levels(&level_filter);
    let tags_set = tags_to_set(&tags);
    let exclude_tags_set = tags_to_set(&exclude_tags);
    let subsystem_set: Set<Text> = subsystem.iter().cloned().collect();

    // Print header
    if !quiet {
        println!();
        println!("{}", "═".repeat(65));
        println!("  {} - Meta-System Test Suite", "VTEST".bold());
        println!("  Phase: {}", "Compile-time (up to TypedAST)".cyan());
        println!("{}", "═".repeat(65));
        println!();
    }

    // Determine test paths
    let test_paths: Vec<PathBuf> = if paths.is_empty() {
        // Default to vcs/specs/meta-system
        let default_path = PathBuf::from("vcs/specs/meta-system");
        if default_path.exists() {
            vec![default_path]
        } else {
            // Try relative to current dir
            let cwd_path = std::env::current_dir()
                .map(|p| p.join("specs/meta-system"))
                .unwrap_or_else(|_| PathBuf::from("specs/meta-system"));
            if cwd_path.exists() {
                vec![cwd_path]
            } else {
                return Err(RunnerError::ConfigError(
                    "Meta-system test directory not found. Expected vcs/specs/meta-system or specs/meta-system".into()
                ));
            }
        }
    } else {
        paths
    };

    // Collect all .vr files with their metadata
    let mut test_files: Vec<MetaTestFile> = Vec::new();

    for base_path in &test_paths {
        for entry in WalkDir::new(base_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "vr") {
                // Apply filter if specified
                if let Some(ref pattern) = filter {
                    let path_str = path.to_string_lossy();
                    if !path_str.contains(pattern.as_str()) {
                        continue;
                    }
                }

                // Read and parse directives
                let source = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => continue, // Skip unreadable files
                };

                let meta_test = match parse_meta_test_file(path, &source) {
                    Some(t) => t,
                    None => continue, // Skip non-meta tests
                };

                // Apply subsystem filter
                if !subsystem_set.is_empty() {
                    let path_str = path.to_string_lossy();
                    let matches_subsystem = subsystem_set.iter().any(|sub| {
                        path_str.contains(&format!("/{}/", sub.as_str()))
                            || path_str.contains(&format!("\\{}\\", sub.as_str()))
                    });
                    if !matches_subsystem {
                        continue;
                    }
                }

                // Apply level filter
                if !level_set.is_empty() && !level_set.contains(&meta_test.level) {
                    continue;
                }

                // Apply tags filter
                if !tags_set.is_empty() && meta_test.tags.is_disjoint(&tags_set) {
                    continue;
                }

                // Apply exclude tags filter
                if !exclude_tags_set.is_empty() && !meta_test.tags.is_disjoint(&exclude_tags_set) {
                    continue;
                }

                // Apply success_only / fail_only filters
                if success_only && meta_test.test_type == MetaTestType::Fail {
                    continue;
                }
                if fail_only && meta_test.test_type != MetaTestType::Fail {
                    continue;
                }

                test_files.push(meta_test);
            }
        }
    }

    test_files.sort_by(|a, b| a.path.cmp(&b.path));

    if test_files.is_empty() {
        println!("  {} No meta-system tests found in {:?}", "Warning:".yellow(), test_paths);
        return Ok(0);
    }

    let pass_count = test_files.iter().filter(|t| t.test_type == MetaTestType::Pass).count();
    let fail_count = test_files.iter().filter(|t| t.test_type == MetaTestType::Fail).count();
    let eval_count = test_files.iter().filter(|t| t.test_type == MetaTestType::Eval).count();

    if !quiet {
        println!("  Found {} tests ({} meta-pass, {} meta-fail, {} meta-eval)",
            test_files.len().to_string().cyan(),
            pass_count.to_string().green(),
            fail_count.to_string().yellow(),
            eval_count.to_string().blue());
        println!();
    }

    // Statistics
    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut by_category: HashMap<String, MetaCategoryStats> = HashMap::new();
    let mut failures: Vec<MetaTestFailure> = Vec::new();

    // Progress bar
    let show_progress = !verbose && !quiet;
    let progress_bar = if show_progress {
        let pb = indicatif::ProgressBar::new(test_files.len() as u64);
        pb.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({percent}%) {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
        Some(pb)
    } else {
        None
    };

    let should_stop = std::sync::atomic::AtomicBool::new(false);

    for (idx, test_file) in test_files.iter().enumerate() {
        if should_stop.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        total += 1;

        // Categorize by subdirectory (subsystem)
        let category = extract_subsystem(&test_file.path)
            .unwrap_or_else(|| "root".to_string());

        let cat_stats = by_category.entry(category.clone()).or_default();
        cat_stats.total += 1;

        // Check if test should be skipped
        if let Some(ref skip_reason) = test_file.skip {
            skipped += 1;
            cat_stats.skipped += 1;
            if verbose && !summary_only {
                let rel_path = test_file.path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| test_file.path.to_string_lossy().to_string());
                println!("  {} {} ({})", "SKIP".yellow(), rel_path, skip_reason.dimmed());
            }
            if let Some(ref pb) = progress_bar {
                pb.set_message(format!("{} pass, {} fail", passed, failed));
                pb.inc(1);
            }
            continue;
        }

        // Read the file
        let source = match std::fs::read_to_string(&test_file.path) {
            Ok(s) => s,
            Err(e) => {
                failed += 1;
                cat_stats.failed += 1;
                failures.push(MetaTestFailure {
                    path: test_file.path.to_string_lossy().to_string().into(),
                    category: category.clone().into(),
                    test_type: test_file.test_type.as_str().into(),
                    error: format!("Failed to read file: {}", e).into(),
                    expected_error: test_file.expected_error.clone(),
                    expected_value: test_file.expected_value.clone(),
                });
                continue;
            }
        };

        // Run the meta test through the compiler pipeline
        let file_id = FileId::new((idx + 1) as u32);
        let test_result = run_single_meta_test(
            &test_file.path,
            &source,
            file_id,
            test_file,
        );

        match test_result {
            MetaTestResult::Passed => {
                passed += 1;
                cat_stats.passed += 1;
                if verbose && !summary_only {
                    let rel_path = test_file.path.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| test_file.path.to_string_lossy().to_string());
                    println!("  {} {} ({})", "PASS".green(), rel_path, test_file.test_type.as_str().dimmed());
                }
            }
            MetaTestResult::Failed(error) => {
                failed += 1;
                cat_stats.failed += 1;
                failures.push(MetaTestFailure {
                    path: test_file.path.to_string_lossy().to_string().into(),
                    category: category.clone().into(),
                    test_type: test_file.test_type.as_str().into(),
                    error,
                    expected_error: test_file.expected_error.clone(),
                    expected_value: test_file.expected_value.clone(),
                });
                if verbose && !summary_only {
                    let rel_path = test_file.path.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| test_file.path.to_string_lossy().to_string());
                    println!("  {} {}", "FAIL".red(), rel_path);
                }
                if fail_fast {
                    should_stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            MetaTestResult::Skipped(reason) => {
                skipped += 1;
                cat_stats.skipped += 1;
                if verbose && !summary_only {
                    let rel_path = test_file.path.file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| test_file.path.to_string_lossy().to_string());
                    println!("  {} {} ({})", "SKIP".yellow(), rel_path, reason.dimmed());
                }
            }
        }

        if let Some(ref pb) = progress_bar {
            pb.set_message(format!("{} pass, {} fail", passed, failed));
            pb.inc(1);
        }
    }

    if let Some(pb) = progress_bar {
        pb.finish_and_clear();
    }

    let duration = start.elapsed();

    // Output based on format
    let output_format = format.to_lowercase();
    match output_format.as_str() {
        "json" => {
            output_meta_json_report(
                total, passed, failed, skipped,
                &by_category, &failures, duration, output,
            )?;
        }
        "markdown" | "md" => {
            output_meta_markdown_report(
                total, passed, failed, skipped,
                &by_category, &failures, duration, output,
            )?;
        }
        _ => {
            output_meta_console_report(
                total, passed, failed, skipped,
                &by_category, &failures, duration, show_failures, use_colors, quiet,
            );
        }
    }

    // Return exit code
    if failed > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

/// Parse a meta test file and extract its directives.
fn parse_meta_test_file(path: &std::path::Path, source: &str) -> Option<MetaTestFile> {
    let mut test_type: Option<MetaTestType> = None;
    let mut expected_error: Option<Text> = None;
    let mut expected_value: Option<Text> = None;
    let mut expected_type: Option<Text> = None;
    let mut contexts: Set<Text> = Set::new();
    let mut level = Level::L1; // Default
    let mut tags: Set<Text> = Set::new();
    let mut skip: Option<Text> = None;

    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("//") {
            break; // Stop at first non-comment line
        }

        let content = trimmed.trim_start_matches("//").trim();

        if content.starts_with("@test:") {
            let value = content.trim_start_matches("@test:").trim();
            test_type = MetaTestType::from_str(value);
        } else if content.starts_with("@level:") {
            let value = content.trim_start_matches("@level:").trim();
            if let Ok(l) = Level::from_str(value) {
                level = l;
            }
        } else if content.starts_with("@expected-error:") {
            let value = content.trim_start_matches("@expected-error:").trim();
            expected_error = Some(value.into());
        } else if content.starts_with("@expected-value:") {
            let value = content.trim_start_matches("@expected-value:").trim();
            expected_value = Some(value.into());
        } else if content.starts_with("@expected-type:") {
            let value = content.trim_start_matches("@expected-type:").trim();
            expected_type = Some(value.into());
        } else if content.starts_with("@contexts:") {
            let value = content.trim_start_matches("@contexts:").trim();
            for ctx in value.split(',') {
                contexts.insert(ctx.trim().into());
            }
        } else if content.starts_with("@tags:") {
            let value = content.trim_start_matches("@tags:").trim();
            for tag in value.split(',') {
                tags.insert(tag.trim().into());
            }
        } else if content.starts_with("@skip:") {
            let value = content.trim_start_matches("@skip:").trim();
            skip = Some(value.into());
        }
    }

    // Only return if this is a meta test
    let test_type = test_type?;

    Some(MetaTestFile {
        path: path.to_path_buf(),
        test_type,
        expected_error,
        expected_value,
        expected_type,
        contexts,
        level,
        tags,
        skip,
    })
}

/// Extract subsystem name from path (e.g., "builtins", "expressions", "hygiene").
fn extract_subsystem(path: &std::path::Path) -> Option<String> {
    let path_str = path.to_string_lossy();

    // Look for known subsystems
    let subsystems = ["builtins", "expressions", "hygiene", "quote", "sandbox", "type_level", "const_eval", "integration"];

    for subsystem in &subsystems {
        if path_str.contains(&format!("/{}/", subsystem)) || path_str.contains(&format!("\\{}\\", subsystem)) {
            return Some(subsystem.to_string());
        }
    }

    // Fall back to parent directory name
    path.parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
}

/// Result of running a single meta test.
enum MetaTestResult {
    Passed,
    Failed(Text),
    Skipped(Text),
}

/// Run a single meta test through the compiler pipeline.
fn run_single_meta_test(
    _path: &std::path::Path,
    source: &str,
    file_id: FileId,
    test_file: &MetaTestFile,
) -> MetaTestResult {
    // Phase 1: Parse the file
    let parser = verum_fast_parser::FastParser::new();
    let parse_result = parser.parse_module_str(source, file_id);

    let module = match parse_result {
        Ok(m) => m,
        Err(errors) => {
            // Parse failed
            let error_msg = errors.iter()
                .map(|e| format!("{:?}", e))
                .collect::<Vec<_>>()
                .join("; ");

            return match test_file.test_type {
                MetaTestType::Fail => {
                    // Check if we expected a parse error
                    if let Some(ref expected) = test_file.expected_error {
                        // Case-insensitive matching for error messages
                        if error_msg.to_lowercase().contains(&expected.to_lowercase().as_str()) {
                            MetaTestResult::Passed
                        } else {
                            MetaTestResult::Failed(
                                format!("Expected error '{}' but got: {}", expected, error_msg).into()
                            )
                        }
                    } else {
                        // Any error is acceptable for meta-fail without specific expectation
                        MetaTestResult::Passed
                    }
                }
                _ => MetaTestResult::Failed(format!("Parse error: {}", error_msg).into()),
            };
        }
    };

    // Phase 2: Run meta evaluation
    // For now, we use verum_compiler's meta evaluation infrastructure
    let meta_result = run_meta_evaluation(&module, test_file);

    match test_file.test_type {
        MetaTestType::Pass => {
            // Should succeed without errors
            match meta_result {
                Ok(_) => MetaTestResult::Passed,
                Err(e) => MetaTestResult::Failed(e),
            }
        }
        MetaTestType::Fail => {
            // Should fail with expected error
            match meta_result {
                Ok(_) => MetaTestResult::Failed(
                    "Expected meta evaluation to fail, but it succeeded".into()
                ),
                Err(error) => {
                    if let Some(ref expected) = test_file.expected_error {
                        let error_lower = error.to_lowercase();
                        let expected_lower = expected.to_lowercase();
                        // Case-insensitive matching for error messages
                        // Also support mapping from legacy E-prefix codes to M-prefix codes:
                        //   E100 (undefined) → M001, M004
                        //   E101 (reflection/context) → M003, M201
                        //   E102 (arity) → M102
                        //   E103 (forbidden) → M301
                        //   E400 (type mismatch) → M003
                        let matches = error_lower.contains(&expected_lower.as_str())
                            || match expected_lower.as_str() {
                                "e100" => error_lower.contains("m001") || error_lower.contains("m004") || error_lower.contains("not found") || error_lower.contains("undefined"),
                                "e101" => error_lower.contains("m003") || error_lower.contains("m201") || error_lower.contains("requires") || error_lower.contains("type mismatch"),
                                "e102" => error_lower.contains("m102") || error_lower.contains("arguments"),
                                "e103" => error_lower.contains("m301") || error_lower.contains("forbidden"),
                                "e400" => error_lower.contains("m003") || error_lower.contains("type mismatch"),
                                _ => false,
                            };
                        if matches {
                            MetaTestResult::Passed
                        } else {
                            MetaTestResult::Failed(
                                format!("Expected error '{}' but got: {}", expected, error).into()
                            )
                        }
                    } else {
                        // Any error is acceptable
                        MetaTestResult::Passed
                    }
                }
            }
        }
        MetaTestType::Eval => {
            // Should evaluate to expected value
            match meta_result {
                Ok(value) => {
                    if let Some(ref expected) = test_file.expected_value {
                        if value.trim() == expected.trim() {
                            MetaTestResult::Passed
                        } else {
                            MetaTestResult::Failed(
                                format!("Expected value '{}' but got: '{}'", expected, value).into()
                            )
                        }
                    } else {
                        // No expected value specified - just check it doesn't error
                        MetaTestResult::Passed
                    }
                }
                Err(e) => MetaTestResult::Failed(e),
            }
        }
    }
}

/// Run meta evaluation on a parsed module using the compiler's actual pipeline.
/// Returns Ok(value_string) on success, Err(error_string) on failure.
///
/// This function uses verum_compiler's MetaRegistry and MetaContext::execute_user_meta_fn
/// to ensure test behavior matches the actual compiler implementation.
fn run_meta_evaluation(module: &verum_ast::Module, test_file: &MetaTestFile) -> Result<Text, Text> {
    use verum_ast::{Item, ItemKind};
    use verum_compiler::meta::{ConstValue, MetaContext, MetaRegistry};
    use verum_common::{Maybe, Text};

    // Create meta registry and register all meta functions from the module
    let mut registry = MetaRegistry::new();
    let module_path = Text::from("test");

    // Register all meta functions and extern functions
    for item in module.items.iter() {
        if let ItemKind::Function(func_decl) = &item.kind {
            if func_decl.is_meta {
                if let Err(e) = registry.register_meta_function(&module_path, func_decl) {
                    return Err(format!("{}", e).into());
                }
            } else if func_decl.extern_abi.is_some() {
                // Register extern (FFI) functions for sandbox detection
                registry.register_extern_function(
                    &module_path,
                    &Text::from(func_decl.name.name.as_str()),
                );
            }
        }
    }

    // Wrap registry in Arc for sharing between context and lookup
    let registry = std::sync::Arc::new(registry);

    // Create meta context with enabled contexts from test file directives
    // If no contexts specified, use default (only tier-0 builtins available)
    // This ensures fail tests for missing contexts actually fail
    let context_names: Vec<Text> = test_file.contexts.iter().cloned().collect();
    let mut ctx = if context_names.is_empty() {
        // No contexts specified - use empty context (tier-0 builtins only)
        MetaContext::new()
    } else {
        // Specific contexts requested - enable only those
        MetaContext::with_using_clause(&context_names)
    };

    // Set up the registry so meta functions can call other meta functions
    // This is critical for recursive meta function support
    ctx.set_registry(registry.clone());
    ctx.set_current_module(module_path.clone());

    // Set up BuildAssets project root if BuildAssets context is enabled
    // Use the test file's parent directory as the project root
    if context_names.iter().any(|c| c.as_str() == "BuildAssets") {
        if let Some(parent) = test_file.path.parent() {
            let project_root = parent.to_string_lossy().to_string();
            ctx.build_assets = verum_compiler::meta::BuildAssetsInfo::new()
                .with_project_root(project_root);
        }
    }

    // Register type definitions from the module (for reflection tests)
    for item in module.items.iter() {
        if let ItemKind::Type(type_decl) = &item.kind {
            use verum_ast::decl::{TypeDeclBody, ProtocolItemKind};
            let type_name = Text::from(type_decl.name.as_str());

            match &type_decl.body {
                TypeDeclBody::Record(fields) => {
                    // Register as struct with fields
                    let field_list: verum_common::List<(Text, verum_ast::ty::Type)> = fields
                        .iter()
                        .map(|f| (Text::from(f.name.as_str()), f.ty.clone()))
                        .collect();
                    ctx.register_struct(type_name, field_list);
                }
                TypeDeclBody::Variant(variants) => {
                    // Register as enum with variants
                    let variant_list: verum_common::List<(Text, verum_ast::ty::Type)> = variants
                        .iter()
                        .map(|v| {
                            let name = Text::from(v.name.as_str());
                            // For simple variants without data, use unit type
                            let ty = verum_ast::ty::Type::unit(v.span);
                            (name, ty)
                        })
                        .collect();
                    ctx.register_enum(type_name, variant_list);
                }
                TypeDeclBody::Protocol(proto_body) => {
                    // Register as protocol with method names
                    let methods: verum_common::List<Text> = proto_body.items
                        .iter()
                        .filter_map(|proto_item| {
                            if let ProtocolItemKind::Function { decl, .. } = &proto_item.kind {
                                Some(Text::from(decl.name.as_str()))
                            } else {
                                None
                            }
                        })
                        .collect();
                    ctx.register_protocol(type_name, methods);
                }
                TypeDeclBody::Alias(_) | TypeDeclBody::Newtype(_) | TypeDeclBody::Tuple(_) => {
                    // For now, skip aliases and newtypes - they need different handling
                }
                _ => {
                    // Handle any other variants (SigmaTuple, Unit, etc.)
                }
            }
        }
    }

    // Helper function to format a MetaValue for display
    fn format_value(value: &ConstValue) -> Text {
        use verum_ast::MetaValue;
        match value {
            MetaValue::Unit => "()".into(),
            MetaValue::Bool(b) => if *b { "true" } else { "false" }.into(),
            MetaValue::Int(i) => i.to_string().into(),
            MetaValue::UInt(u) => u.to_string().into(),
            MetaValue::Float(f) => f.to_string().into(),
            MetaValue::Char(c) => format!("'{}'", c).into(),
            MetaValue::Text(s) => format!("\"{}\"", s).into(),
            MetaValue::Bytes(bytes) => format!("Bytes[{}]", bytes.len()).into(),
            MetaValue::Array(items) => {
                let formatted: Vec<String> = items.iter().map(|v| format_value(v).to_string()).collect();
                format!("[{}]", formatted.join(", ")).into()
            }
            MetaValue::Tuple(items) => {
                let formatted: Vec<String> = items.iter().map(|v| format_value(v).to_string()).collect();
                format!("({})", formatted.join(", ")).into()
            }
            MetaValue::Maybe(maybe) => {
                match maybe {
                    verum_common::Maybe::Some(v) => format!("Some({})", format_value(v)).into(),
                    verum_common::Maybe::None => "None".into(),
                }
            }
            MetaValue::Expr(expr) => format!("Expr({:?})", expr.kind).into(),
            MetaValue::Type(ty) => format!("Type({:?})", ty).into(),
            MetaValue::Pattern(pat) => format!("Pattern({:?})", pat.kind).into(),
            MetaValue::Item(item) => format!("Item({:?})", item.kind).into(),
            MetaValue::Items(items) => {
                let formatted: Vec<String> = items.iter().map(|v| format_value(v).to_string()).collect();
                format!("Items[{}]", formatted.join(", ")).into()
            }
            MetaValue::Map(map) => {
                let formatted: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, format_value(v)))
                    .collect();
                format!("{{{}}}", formatted.join(", ")).into()
            }
            MetaValue::Set(set) => {
                let formatted: Vec<String> = set.iter().map(|s| s.to_string()).collect();
                format!("{{{}}}", formatted.join(", ")).into()
            }
        }
    }

    // Find the first zero-argument meta function to execute
    // Priority: functions named test_*, then any zero-arg meta function
    // Only consider functions with no required parameters since we call with empty args
    let mut test_function: Option<Text> = None;
    let mut any_meta_function: Option<Text> = None;

    for item in module.items.iter() {
        if let ItemKind::Function(func_decl) = &item.kind {
            if func_decl.is_meta {
                // Only consider functions with no required parameters
                use verum_ast::decl::FunctionParamKind;
                let has_required_params = func_decl.params.iter().any(|p| {
                    match &p.kind {
                        FunctionParamKind::Regular { default_value, .. } => default_value.is_none(),
                        // Self parameters don't count as required args for standalone calls
                        _ => false,
                    }
                });
                if has_required_params {
                    continue; // Skip functions that require arguments
                }

                let name = func_decl.name.as_str();
                if name.starts_with("test_") && test_function.is_none() {
                    test_function = Some(Text::from(name));
                } else if any_meta_function.is_none() {
                    any_meta_function = Some(Text::from(name));
                }
            }
        }
    }

    // Choose which function to execute
    let func_name = test_function.or(any_meta_function);

    if let Some(name) = func_name {
        // Get the meta function from registry
        if let Maybe::Some(meta_fn) = registry.get_user_meta_fn(&module_path, &name) {
            // Check context requirements only when the test specifies explicit contexts.
            // If the test has no @contexts directive, we assume all contexts are available.
            // This prevents meta-pass tests from failing just because they use `using [MetaTypes]`.
            if !context_names.is_empty() {
                if let Some(func_decl) = module.items.iter().find_map(|item| {
                    if let ItemKind::Function(fd) = &item.kind {
                        if fd.is_meta && fd.name.as_str() == name.as_str() { Some(fd) } else { None }
                    } else { None }
                }) {
                    if !func_decl.contexts.is_empty() {
                        for ctx_req in func_decl.contexts.iter() {
                            let ctx_str = ctx_req.path.segments.last()
                                .and_then(|s| if let verum_ast::ty::PathSegment::Name(id) = s { Some(id.name.as_str()) } else { None })
                                .unwrap_or("");
                            if !context_names.iter().any(|c| c.as_str() == ctx_str) {
                                return Err(format!("M201: Meta function requires context '{}' which is not provided", ctx_str).into());
                            }
                        }
                    }
                }
            }

            // Execute using the compiler's actual execute_user_meta_fn
            // This ensures identical behavior to the real compiler
            match ctx.execute_user_meta_fn(&meta_fn, vec![]) {
                Ok(value) => {
                    // Type-check the return value against declared return type
                    // Search by function name - try exact match and prefix match
                    let name_str = name.as_str();
                    // Type-check return value against declared type
                    if let Some(func_decl) = module.items.iter().find_map(|item| {
                        if let ItemKind::Function(fd) = &item.kind {
                            let fn_name = fd.name.name.as_str();
                            if fd.is_meta && (fn_name == name_str || name_str.ends_with(fn_name)) {
                                Some(fd)
                            } else { None }
                        } else { None }
                    }) {
                        if let verum_common::Maybe::Some(ref ret_ty) = func_decl.return_type {
                            use verum_ast::MetaValue;
                            let type_mismatch = match (&value, &ret_ty.kind) {
                                (MetaValue::Int(_), verum_ast::ty::TypeKind::Bool) => true,
                                (MetaValue::Bool(_), verum_ast::ty::TypeKind::Int) => true,
                                (MetaValue::Text(_), verum_ast::ty::TypeKind::Int) => true,
                                (MetaValue::Int(_), verum_ast::ty::TypeKind::Char) => true,
                                (MetaValue::Float(_), verum_ast::ty::TypeKind::Int) => true,
                                (MetaValue::Float(_), verum_ast::ty::TypeKind::Bool) => true,
                                (MetaValue::Bool(_), verum_ast::ty::TypeKind::Float) => true,
                                (MetaValue::Text(_), verum_ast::ty::TypeKind::Bool) => true,
                                (MetaValue::Text(_), verum_ast::ty::TypeKind::Float) => true,
                                // Also check Path types (user-defined type names)
                                (MetaValue::Int(_), verum_ast::ty::TypeKind::Path(path)) => {
                                    path.segments.last().and_then(|s| if let verum_ast::ty::PathSegment::Name(id) = s { Some(id.name.as_str()) } else { None })
                                        .map(|n| n == "Bool" || n == "Text").unwrap_or(false)
                                }
                                (MetaValue::Text(_), verum_ast::ty::TypeKind::Path(path)) => {
                                    path.segments.last().and_then(|s| if let verum_ast::ty::PathSegment::Name(id) = s { Some(id.name.as_str()) } else { None })
                                        .map(|n| n == "Int" || n == "Bool" || n == "Float").unwrap_or(false)
                                }
                                _ => false,
                            };
                            if type_mismatch {
                                return Err(format!("M003: Type mismatch: return value does not match declared return type").into());
                            }
                        }
                    }
                    return Ok(format_value(&value));
                }
                Err(e) => return Err(format!("{}", e).into()),
            }
        } else {
            return Err(format!("Meta function '{}' not found in registry", name).into());
        }
    }

    // Fallback: evaluate const declarations directly
    for item in module.items.iter() {
        if let ItemKind::Const(const_decl) = &item.kind {
            let meta_expr = ctx.ast_expr_to_meta_expr(&const_decl.value)
                .map_err(|e| Text::from(format!("{}", e)))?;
            match ctx.eval_meta_expr(&meta_expr) {
                Ok(value) => return Ok(format_value(&value)),
                Err(e) => return Err(format!("{}", e).into()),
            }
        }
    }

    // No evaluatable expression found - return unit
    Ok("()".into())
}

/// Output meta test results as JSON.
fn output_meta_json_report(
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    by_category: &std::collections::HashMap<String, MetaCategoryStats>,
    failures: &[MetaTestFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
) -> Result<(), RunnerError> {
    // Convert to unified format
    let golden_categories: std::collections::HashMap<String, GoldenCategoryStats> =
        by_category.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    let golden_failures: Vec<GoldenTestFailure> =
        failures.iter().map(|f| f.into()).collect();

    let config = GoldenReportConfig {
        suite_name: "Meta-System Tests",
        subtitle: Some("Compile-time evaluation"),
        category_label: "Subsystem",
        show_skipped: true,
    };

    output_golden_json_report(
        &config,
        total,
        passed,
        failed,
        skipped,
        &golden_categories,
        &golden_failures,
        duration,
        output,
    )
}

/// Output meta test results as Markdown.
fn output_meta_markdown_report(
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    by_category: &std::collections::HashMap<String, MetaCategoryStats>,
    failures: &[MetaTestFailure],
    duration: std::time::Duration,
    output: Option<PathBuf>,
) -> Result<(), RunnerError> {
    // Convert to unified format
    let golden_categories: std::collections::HashMap<String, GoldenCategoryStats> =
        by_category.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    let golden_failures: Vec<GoldenTestFailure> =
        failures.iter().map(|f| f.into()).collect();

    let config = GoldenReportConfig {
        suite_name: "Meta-System Tests",
        subtitle: Some("Compile-time evaluation"),
        category_label: "Subsystem",
        show_skipped: true,
    };

    output_golden_markdown_report(
        &config,
        total,
        passed,
        failed,
        skipped,
        &golden_categories,
        &golden_failures,
        duration,
        output,
    )
}

/// Output meta test results to console.
fn output_meta_console_report(
    total: usize,
    passed: usize,
    failed: usize,
    skipped: usize,
    by_category: &std::collections::HashMap<String, MetaCategoryStats>,
    failures: &[MetaTestFailure],
    duration: std::time::Duration,
    show_failures: usize,
    use_colors: bool,
    quiet: bool,
) {
    // Convert to unified format
    let golden_categories: std::collections::HashMap<String, GoldenCategoryStats> =
        by_category.iter().map(|(k, v)| (k.clone(), v.into())).collect();
    let golden_failures: Vec<GoldenTestFailure> =
        failures.iter().map(|f| f.into()).collect();

    let config = GoldenReportConfig {
        suite_name: "Meta-System Tests",
        subtitle: Some("Compile-time evaluation"),
        category_label: "Subsystem",
        show_skipped: true,
    };

    output_golden_console_report(
        &config,
        total,
        passed,
        failed,
        skipped,
        &golden_categories,
        &golden_failures,
        duration,
        show_failures,
        use_colors,
        quiet,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_levels() {
        let levels = parse_levels(&["L0".to_string().into(), "L1".to_string().into()]);
        assert!(levels.contains(&Level::L0));
        assert!(levels.contains(&Level::L1));

        let levels = parse_levels(&["L0,L2".to_string().into()]);
        assert!(levels.contains(&Level::L0));
        assert!(levels.contains(&Level::L2));
    }

    #[test]
    fn test_parse_tiers() {
        let tiers = parse_tiers(&["0".to_string().into(), "3".to_string().into()]);
        assert!(tiers.contains(&Tier::Tier0));
        assert!(tiers.contains(&Tier::Tier3));

        let tiers = parse_tiers(&["all".to_string().into()]);
        assert_eq!(tiers.len(), 4);

        let tiers = parse_tiers(&["compiled".to_string().into()]);
        assert_eq!(tiers.len(), 3);
        assert!(!tiers.contains(&Tier::Tier0));
    }

    #[test]
    fn test_parse_parallel() {
        assert!(parse_parallel("auto") > 0);
        assert_eq!(parse_parallel("4"), 4);
        assert_eq!(parse_parallel("8"), 8);
    }

    #[test]
    fn test_tags_to_set() {
        let tags = tags_to_set(&["cbgr,memory".to_string().into(), "ownership".to_string().into()]);
        assert!(tags.contains(&Text::from("cbgr")));
        assert!(tags.contains(&Text::from("memory")));
        assert!(tags.contains(&Text::from("ownership")));
    }
}
