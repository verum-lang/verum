//! VCS Differential Testing CLI
//!
//! Comprehensive command-line interface for differential testing across
//! execution tiers and implementations.
//!
//! # Usage
//!
//! ```bash
//! # Run differential test on a single file
//! vcs-diff run test.vr --tiers 0,3
//!
//! # Run batch differential tests
//! vcs-diff batch specs/ --parallel --report json
//!
//! # Compare implementations
//! vcs-diff cross-impl test.vr --reference interpreter --alternatives aot,jit
//!
//! # Generate regression tests from divergences
//! vcs-diff generate --from-corpus corpus/ --output generated_tests/
//!
//! # Generate report
//! vcs-diff report results.json --format html --output report.html
//! ```

// VCS differential testing infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

mod cross_impl;
mod differential;
mod divergence;
mod normalizer;
mod semantic_equiv;
mod test_generator;
mod vtest_integration;

use cross_impl::{CrossImplConfig, CrossImplRunner, Implementation, standard_implementations};
use differential::{DiffResult, DifferentialRunner};
use divergence::{DivergenceReporter, ReportFormat, Tier};
use normalizer::{NormalizationConfig, Normalizer};
use semantic_equiv::{EquivalenceConfig, SemanticEquivalenceChecker};
use test_generator::{
    EdgeCaseGenerator, FuzzerCorpusGenerator, GeneratorConfig, StressTestGenerator, TestGenerator,
};
use vtest_integration::{DifferentialExecutor, DifferentialTestConfig};

/// VCS Differential Testing Tool
#[derive(Parser, Debug)]
#[command(name = "vcs-diff")]
#[command(author = "Verum Team")]
#[command(version = "0.1.0")]
#[command(about = "Differential testing across Verum execution tiers")]
#[command(long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Enable debug output
    #[arg(short, long, global = true)]
    debug: bool,

    /// Configuration file path
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run differential test on a single file
    Run {
        /// Path to the test file (.vr)
        path: PathBuf,

        /// Tiers to compare (comma-separated: 0,1,2,3)
        #[arg(short, long, default_value = "0,3")]
        tiers: String,

        /// Reference tier for comparison
        #[arg(short, long, default_value = "0")]
        reference: u8,

        /// Timeout in milliseconds
        #[arg(long, default_value = "30000")]
        timeout: u64,

        /// Enable semantic comparison (looser matching)
        #[arg(long)]
        semantic: bool,

        /// Float epsilon for comparison
        #[arg(long, default_value = "1e-10")]
        float_epsilon: f64,

        /// Output format
        #[arg(short, long, value_enum, default_value = "text")]
        output: OutputFormat,
    },

    /// Run batch differential tests on a directory
    Batch {
        /// Directory containing test files
        path: PathBuf,

        /// Tiers to compare (comma-separated)
        #[arg(short, long, default_value = "0,3")]
        tiers: String,

        /// Run tests in parallel
        #[arg(short, long)]
        parallel: bool,

        /// Number of worker threads
        #[arg(short, long, default_value = "0")]
        workers: usize,

        /// Stop on first failure
        #[arg(long)]
        fail_fast: bool,

        /// Filter by pattern (glob)
        #[arg(long)]
        filter: Option<String>,

        /// Exclude pattern (glob)
        #[arg(long)]
        exclude: Option<String>,

        /// Output report format
        #[arg(long, value_enum, default_value = "text")]
        report: OutputFormat,

        /// Output directory for reports
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Compare across implementations
    CrossImpl {
        /// Path to test file or directory
        path: PathBuf,

        /// Reference implementation (interpreter, aot, jit, bytecode)
        #[arg(long, default_value = "interpreter")]
        reference: String,

        /// Alternative implementations to compare (comma-separated)
        #[arg(long, default_value = "aot")]
        alternatives: String,

        /// Path to interpreter binary
        #[arg(long)]
        interpreter_bin: Option<PathBuf>,

        /// Path to AOT binary
        #[arg(long)]
        aot_bin: Option<PathBuf>,

        /// Path to JIT binary
        #[arg(long)]
        jit_bin: Option<PathBuf>,

        /// Output format
        #[arg(short, long, value_enum, default_value = "text")]
        output: OutputFormat,
    },

    /// Generate tests from various sources
    Generate {
        /// Generate from fuzzer corpus
        #[arg(long)]
        from_corpus: Option<PathBuf>,

        /// Generate edge case tests
        #[arg(long)]
        edge_cases: bool,

        /// Generate stress tests
        #[arg(long)]
        stress: bool,

        /// Output directory for generated tests
        #[arg(short, long, default_value = "generated_tests")]
        output: PathBuf,

        /// Maximum iterations for stress tests
        #[arg(long, default_value = "10000")]
        max_iterations: usize,
    },

    /// Generate report from results
    Report {
        /// Input results file (JSON)
        input: PathBuf,

        /// Output format
        #[arg(short, long, value_enum, default_value = "html")]
        format: OutputFormat,

        /// Output file path
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Include detailed divergence info
        #[arg(long)]
        detailed: bool,
    },

    /// Show divergence between two outputs
    Diff {
        /// First file or output
        file1: PathBuf,

        /// Second file or output
        file2: PathBuf,

        /// Enable semantic comparison
        #[arg(long)]
        semantic: bool,

        /// Normalize output before comparison
        #[arg(long)]
        normalize: bool,

        /// Show context lines
        #[arg(short = 'C', long, default_value = "3")]
        context: usize,
    },

    /// Normalize output for comparison
    Normalize {
        /// Input file
        input: PathBuf,

        /// Output file (stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Normalization mode
        #[arg(short, long, value_enum, default_value = "semantic")]
        mode: NormalizationMode,
    },

    /// Check version compatibility
    VersionCompat {
        /// Path to test file
        path: PathBuf,

        /// Versions directory
        #[arg(long)]
        versions_dir: PathBuf,

        /// Specific versions to test (comma-separated, or 'all')
        #[arg(long, default_value = "all")]
        versions: String,

        /// Output format
        #[arg(short, long, value_enum, default_value = "text")]
        output: OutputFormat,
    },
}

#[derive(Clone, Debug, ValueEnum)]
enum OutputFormat {
    Text,
    Json,
    Html,
    Markdown,
}

#[derive(Clone, Debug, ValueEnum)]
enum NormalizationMode {
    /// Minimal normalization
    Exact,
    /// Standard semantic normalization
    Semantic,
    /// Aggressive normalization
    Aggressive,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("Error: {:#}", e);
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn run(cli: Cli) -> Result<()> {
    let start = Instant::now();

    match cli.command {
        Commands::Run {
            path,
            tiers,
            reference,
            timeout,
            semantic,
            float_epsilon,
            output,
        } => {
            run_single_test(
                path,
                &tiers,
                reference,
                timeout,
                semantic,
                float_epsilon,
                output,
                cli.verbose,
            )?;
        }

        Commands::Batch {
            path,
            tiers,
            parallel,
            workers,
            fail_fast,
            filter,
            exclude,
            report,
            output,
        } => {
            run_batch_tests(
                path,
                &tiers,
                parallel,
                workers,
                fail_fast,
                filter,
                exclude,
                report,
                output,
                cli.verbose,
            )?;
        }

        Commands::CrossImpl {
            path,
            reference,
            alternatives,
            interpreter_bin,
            aot_bin,
            jit_bin,
            output,
        } => {
            run_cross_impl(
                path,
                reference,
                alternatives,
                interpreter_bin,
                aot_bin,
                jit_bin,
                output,
                cli.verbose,
            )?;
        }

        Commands::Generate {
            from_corpus,
            edge_cases,
            stress,
            output,
            max_iterations,
        } => {
            run_generate(
                from_corpus,
                edge_cases,
                stress,
                output,
                max_iterations,
                cli.verbose,
            )?;
        }

        Commands::Report {
            input,
            format,
            output,
            detailed,
        } => {
            run_report(input, format, output, detailed)?;
        }

        Commands::Diff {
            file1,
            file2,
            semantic,
            normalize,
            context,
        } => {
            run_diff(file1, file2, semantic, normalize, context)?;
        }

        Commands::Normalize {
            input,
            output,
            mode,
        } => {
            run_normalize(input, output, mode)?;
        }

        Commands::VersionCompat {
            path,
            versions_dir,
            versions,
            output,
        } => {
            run_version_compat(path, versions_dir, versions, output, cli.verbose)?;
        }
    }

    if cli.verbose {
        let duration = start.elapsed();
        eprintln!("Completed in {:?}", duration);
    }

    Ok(())
}

fn parse_tiers(tiers_str: &str) -> Vec<u8> {
    tiers_str
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .filter(|&t| t <= 3)
        .collect()
}

fn run_single_test(
    path: PathBuf,
    tiers_str: &str,
    reference: u8,
    timeout: u64,
    semantic: bool,
    float_epsilon: f64,
    output: OutputFormat,
    verbose: bool,
) -> Result<()> {
    let tiers = parse_tiers(tiers_str);
    if tiers.is_empty() {
        anyhow::bail!("No valid tiers specified");
    }

    if verbose {
        eprintln!("Running differential test: {}", path.display());
        eprintln!("Tiers: {:?}, Reference: {}", tiers, reference);
    }

    let config = DifferentialTestConfig {
        interpreter_path: PathBuf::from("verum-interpret"),
        bytecode_path: Some(PathBuf::from("verum-bc")),
        jit_path: Some(PathBuf::from("verum-jit")),
        aot_path: PathBuf::from("verum-run"),
        timeout_ms: timeout,
        semantic_comparison: semantic,
        float_epsilon,
        tiers: tiers.clone(),
        reference_tier: reference,
        ..Default::default()
    };

    let executor = DifferentialExecutor::new(config);
    let result = executor.run(&path)?;

    match output {
        OutputFormat::Text => {
            println!("Test: {}", path.display());
            println!("Tiers: {:?}", tiers);
            println!("Success: {}", result.success);

            if !result.success {
                println!("\nDivergences:");
                for div in &result.divergences {
                    println!("  - {} vs {}: {}", div.tier1, div.tier2, div.summary);
                }
            }
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);
        }
        OutputFormat::Html | OutputFormat::Markdown => {
            // TODO: Implement HTML/Markdown output
            eprintln!("HTML/Markdown output not yet implemented for single test");
        }
    }

    if !result.success {
        std::process::exit(1);
    }

    Ok(())
}

fn run_batch_tests(
    path: PathBuf,
    tiers_str: &str,
    parallel: bool,
    workers: usize,
    fail_fast: bool,
    filter: Option<String>,
    exclude: Option<String>,
    report: OutputFormat,
    output: Option<PathBuf>,
    verbose: bool,
) -> Result<()> {
    let tiers = parse_tiers(tiers_str);
    if tiers.is_empty() {
        anyhow::bail!("No valid tiers specified");
    }

    if verbose {
        eprintln!("Running batch differential tests: {}", path.display());
        eprintln!("Tiers: {:?}, Parallel: {}", tiers, parallel);
    }

    let config = DifferentialTestConfig {
        interpreter_path: PathBuf::from("verum-interpret"),
        bytecode_path: Some(PathBuf::from("verum-bc")),
        jit_path: Some(PathBuf::from("verum-jit")),
        aot_path: PathBuf::from("verum-run"),
        tiers: tiers.clone(),
        ..Default::default()
    };

    let executor = DifferentialExecutor::new(config);
    let results = executor.run_directory(&path, parallel, workers.max(1))?;

    let total = results.len();
    let passed = results.iter().filter(|r| r.success).count();
    let failed = total - passed;

    match report {
        OutputFormat::Text => {
            println!("Batch Results:");
            println!("  Total:  {}", total);
            println!(
                "  Passed: {} ({:.1}%)",
                passed,
                100.0 * passed as f64 / total as f64
            );
            println!(
                "  Failed: {} ({:.1}%)",
                failed,
                100.0 * failed as f64 / total as f64
            );

            if failed > 0 {
                println!("\nFailed Tests:");
                for result in results.iter().filter(|r| !r.success) {
                    println!("  - {}", result.path.display());
                }
            }
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&results)?;
            if let Some(output_path) = output {
                std::fs::write(&output_path, &json)?;
                println!("Report written to: {}", output_path.display());
            } else {
                println!("{}", json);
            }
        }
        OutputFormat::Html => {
            // TODO: Generate HTML report
            eprintln!("HTML report generation not yet implemented");
        }
        OutputFormat::Markdown => {
            // TODO: Generate Markdown report
            eprintln!("Markdown report generation not yet implemented");
        }
    }

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}

fn run_cross_impl(
    path: PathBuf,
    reference: String,
    alternatives: String,
    interpreter_bin: Option<PathBuf>,
    aot_bin: Option<PathBuf>,
    jit_bin: Option<PathBuf>,
    output: OutputFormat,
    verbose: bool,
) -> Result<()> {
    if verbose {
        eprintln!("Running cross-implementation test: {}", path.display());
        eprintln!("Reference: {}, Alternatives: {}", reference, alternatives);
    }

    let mut config = CrossImplConfig::default();

    // Build implementations list
    let interp_path = interpreter_bin.unwrap_or_else(|| PathBuf::from("verum-interpret"));
    let aot_path = aot_bin.unwrap_or_else(|| PathBuf::from("verum-run"));
    let jit_path = jit_bin.unwrap_or_else(|| PathBuf::from("verum-jit"));

    if reference == "interpreter" || alternatives.contains("interpreter") {
        let mut impl_ = Implementation::new("interpreter", &interp_path);
        if reference == "interpreter" {
            impl_ = impl_.as_reference();
        }
        config = config.with_implementation(impl_);
    }

    if reference == "aot" || alternatives.contains("aot") {
        let mut impl_ = Implementation::new("aot", &aot_path);
        if reference == "aot" {
            impl_ = impl_.as_reference();
        }
        config = config.with_implementation(impl_);
    }

    if reference == "jit" || alternatives.contains("jit") {
        let mut impl_ = Implementation::new("jit", &jit_path);
        if reference == "jit" {
            impl_ = impl_.as_reference();
        }
        config = config.with_implementation(impl_);
    }

    let runner = CrossImplRunner::new(config);
    let result = runner.run(&path)?;

    match output {
        OutputFormat::Text => {
            println!("Cross-Implementation Test: {}", path.display());
            println!("Reference: {}", result.reference);
            println!("Consensus: {}", result.consensus);

            if !result.consensus {
                println!("\nDivergent implementations:");
                for impl_name in &result.divergent {
                    println!("  - {}", impl_name);
                }
            }
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);
        }
        _ => {
            eprintln!("Format not yet implemented");
        }
    }

    if !result.consensus {
        std::process::exit(1);
    }

    Ok(())
}

fn run_generate(
    from_corpus: Option<PathBuf>,
    edge_cases: bool,
    stress: bool,
    output: PathBuf,
    max_iterations: usize,
    verbose: bool,
) -> Result<()> {
    std::fs::create_dir_all(&output)?;

    let mut total_generated = 0;

    if let Some(corpus_dir) = from_corpus {
        if verbose {
            eprintln!(
                "Generating tests from fuzzer corpus: {}",
                corpus_dir.display()
            );
        }

        let generator = FuzzerCorpusGenerator::new(corpus_dir, output.clone());
        let result = generator.generate()?;
        total_generated += result.success_count();

        if verbose {
            eprintln!("Generated {} tests from corpus", result.success_count());
        }
    }

    if edge_cases {
        if verbose {
            eprintln!("Generating edge case tests");
        }

        let generator = EdgeCaseGenerator::new(output.clone());
        let result = generator.write_all()?;
        total_generated += result.success_count();

        if verbose {
            eprintln!("Generated {} edge case tests", result.success_count());
        }
    }

    if stress {
        if verbose {
            eprintln!(
                "Generating stress tests (max_iterations: {})",
                max_iterations
            );
        }

        let generator =
            StressTestGenerator::new(output.clone()).with_max_iterations(max_iterations);
        let result = generator.write_all()?;
        total_generated += result.success_count();

        if verbose {
            eprintln!("Generated {} stress tests", result.success_count());
        }
    }

    println!("Total tests generated: {}", total_generated);
    println!("Output directory: {}", output.display());

    Ok(())
}

fn run_report(
    input: PathBuf,
    format: OutputFormat,
    output: Option<PathBuf>,
    detailed: bool,
) -> Result<()> {
    let content = std::fs::read_to_string(&input).context("Failed to read input file")?;

    let results: Vec<serde_json::Value> =
        serde_json::from_str(&content).context("Failed to parse JSON input")?;

    match format {
        OutputFormat::Html => {
            let html = generate_html_report(&results, detailed);
            if let Some(path) = output {
                std::fs::write(&path, html)?;
                println!("Report written to: {}", path.display());
            } else {
                println!("{}", html);
            }
        }
        OutputFormat::Markdown => {
            let md = generate_markdown_report(&results, detailed);
            if let Some(path) = output {
                std::fs::write(&path, md)?;
                println!("Report written to: {}", path.display());
            } else {
                println!("{}", md);
            }
        }
        OutputFormat::Text | OutputFormat::Json => {
            // Just pretty-print
            let pretty = serde_json::to_string_pretty(&results)?;
            if let Some(path) = output {
                std::fs::write(&path, pretty)?;
                println!("Report written to: {}", path.display());
            } else {
                println!("{}", pretty);
            }
        }
    }

    Ok(())
}

fn run_diff(
    file1: PathBuf,
    file2: PathBuf,
    semantic: bool,
    normalize: bool,
    context: usize,
) -> Result<()> {
    let content1 = std::fs::read_to_string(&file1)?;
    let content2 = std::fs::read_to_string(&file2)?;

    let (text1, text2) = if normalize {
        let normalizer = Normalizer::new(NormalizationConfig::semantic());
        (
            normalizer.normalize(&content1),
            normalizer.normalize(&content2),
        )
    } else {
        (content1, content2)
    };

    if semantic {
        let checker = SemanticEquivalenceChecker::new(EquivalenceConfig::default());
        let result = checker.check(&text1, &text2);

        if result.is_equivalent() {
            println!("Files are semantically equivalent");
        } else {
            println!("Files differ:");
            if let semantic_equiv::EquivalenceResult::Different(diffs) = result {
                for diff in diffs {
                    println!(
                        "  - At {}: {} vs {}",
                        diff.location, diff.expected, diff.actual
                    );
                }
            }
        }
    } else {
        // Simple line-by-line diff
        let lines1: Vec<_> = text1.lines().collect();
        let lines2: Vec<_> = text2.lines().collect();

        let mut has_diff = false;
        for (i, (l1, l2)) in lines1.iter().zip(lines2.iter()).enumerate() {
            if l1 != l2 {
                has_diff = true;
                println!("Line {}: ", i + 1);
                println!("  - {}", l1);
                println!("  + {}", l2);
            }
        }

        if lines1.len() != lines2.len() {
            has_diff = true;
            println!(
                "Different number of lines: {} vs {}",
                lines1.len(),
                lines2.len()
            );
        }

        if !has_diff {
            println!("Files are identical");
        }
    }

    Ok(())
}

fn run_normalize(input: PathBuf, output: Option<PathBuf>, mode: NormalizationMode) -> Result<()> {
    let content = std::fs::read_to_string(&input)?;

    let config = match mode {
        NormalizationMode::Exact => NormalizationConfig::exact(),
        NormalizationMode::Semantic => NormalizationConfig::semantic(),
        NormalizationMode::Aggressive => NormalizationConfig::aggressive(),
    };

    let normalizer = Normalizer::new(config);
    let normalized = normalizer.normalize(&content);

    if let Some(path) = output {
        std::fs::write(&path, &normalized)?;
        eprintln!("Normalized output written to: {}", path.display());
    } else {
        print!("{}", normalized);
    }

    Ok(())
}

fn run_version_compat(
    path: PathBuf,
    versions_dir: PathBuf,
    versions: String,
    output: OutputFormat,
    verbose: bool,
) -> Result<()> {
    if verbose {
        eprintln!("Running version compatibility test: {}", path.display());
        eprintln!("Versions directory: {}", versions_dir.display());
    }

    use cross_impl::{VersionCompatConfig, VersionCompatRunner};

    let versions_list = if versions == "all" {
        vec![] // Will auto-discover
    } else {
        versions.split(',').map(|s| s.trim().to_string()).collect()
    };

    let config = VersionCompatConfig {
        version_dir: versions_dir,
        versions: versions_list,
        ..Default::default()
    };

    let runner = VersionCompatRunner::new(config);
    let result = runner.run(&path)?;

    match output {
        OutputFormat::Text => {
            println!("Version Compatibility Test: {}", path.display());
            println!("Versions tested: {:?}", result.versions_tested);
            println!("\nCompatibility Matrix:");

            for ((v1, v2), compat) in &result.compatibility_matrix {
                let status = if compat.compatible { "OK" } else { "FAIL" };
                println!("  {} vs {}: {}", v1, v2, status);
            }
        }
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&result)?;
            println!("{}", json);
        }
        _ => {
            eprintln!("Format not yet implemented");
        }
    }

    Ok(())
}

fn generate_html_report(results: &[serde_json::Value], detailed: bool) -> String {
    let mut html = String::new();

    html.push_str(r#"<!DOCTYPE html>
<html>
<head>
    <title>VCS Differential Test Report</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 40px; }
        h1 { color: #333; }
        .summary { background: #f5f5f5; padding: 20px; border-radius: 8px; margin-bottom: 20px; }
        .passed { color: #28a745; }
        .failed { color: #dc3545; }
        table { border-collapse: collapse; width: 100%; margin-top: 20px; }
        th, td { border: 1px solid #ddd; padding: 12px; text-align: left; }
        th { background: #f5f5f5; }
        tr:hover { background: #f9f9f9; }
    </style>
</head>
<body>
    <h1>VCS Differential Test Report</h1>
"#);

    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| r.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();

    html.push_str(&format!(
        r#"
    <div class="summary">
        <h2>Summary</h2>
        <p>Total: {}</p>
        <p class="passed">Passed: {} ({:.1}%)</p>
        <p class="failed">Failed: {} ({:.1}%)</p>
    </div>
"#,
        total,
        passed,
        100.0 * passed as f64 / total as f64,
        total - passed,
        100.0 * (total - passed) as f64 / total as f64
    ));

    html.push_str(
        r#"
    <h2>Results</h2>
    <table>
        <tr>
            <th>Test</th>
            <th>Status</th>
            <th>Duration</th>
        </tr>
"#,
    );

    for result in results {
        let path = result
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let status = if success { "PASS" } else { "FAIL" };
        let class = if success { "passed" } else { "failed" };

        html.push_str(&format!(
            r#"
        <tr>
            <td>{}</td>
            <td class="{}">{}</td>
            <td>-</td>
        </tr>
"#,
            path, class, status
        ));
    }

    html.push_str(
        r#"
    </table>
</body>
</html>
"#,
    );

    html
}

fn generate_markdown_report(results: &[serde_json::Value], detailed: bool) -> String {
    let mut md = String::new();

    md.push_str("# VCS Differential Test Report\n\n");

    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| r.get("success").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();

    md.push_str("## Summary\n\n");
    md.push_str(&format!("- **Total:** {}\n", total));
    md.push_str(&format!(
        "- **Passed:** {} ({:.1}%)\n",
        passed,
        100.0 * passed as f64 / total as f64
    ));
    md.push_str(&format!(
        "- **Failed:** {} ({:.1}%)\n\n",
        total - passed,
        100.0 * (total - passed) as f64 / total as f64
    ));

    md.push_str("## Results\n\n");
    md.push_str("| Test | Status |\n");
    md.push_str("|------|--------|\n");

    for result in results {
        let path = result
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let success = result
            .get("success")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let status = if success { "PASS" } else { "FAIL" };

        md.push_str(&format!("| {} | {} |\n", path, status));
    }

    md
}
