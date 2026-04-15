//! Test command - discovers and executes Verum tests
//!
//! Discovers test files (`.vr`) in the project's `tests/` directory,
//! compiles each through the AOT pipeline, and executes the resulting binary.
//! A test passes when the process exits with code 0.

use colored::Colorize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use verum_common::{List, Text};

use crate::config::Manifest;
use crate::error::{CliError, Result};
use crate::ui;
use verum_ast::{FileId, ItemKind};
use verum_compiler::options::{CompilerOptions, OutputFormat, VerifyMode};
use verum_compiler::pipeline::CompilationPipeline;
use verum_compiler::session::Session;
use verum_lexer::Lexer;
use verum_parser::VerumParser;

/// Resolved per-test runner settings derived from `[test]` in
/// `verum.toml` (plus any `-Z test.*` CLI overrides). Threaded from
/// `execute` into `run_single_test` so the test binary build and
/// execution honor the project's policy.
///
/// `language_features` carries the FULL merged feature set (safety,
/// meta, context, types, …) so every per-test compilation inherits
/// `-Z` overrides. Without this, each test was being built with
/// default features and silently ignored every override.
#[derive(Debug, Clone)]
struct TestRunConfig {
    /// Per-test execution timeout in seconds. 0 = no timeout.
    timeout_secs: u64,
    /// Treat compile warnings as errors during the per-test build.
    deny_warnings: bool,
    /// Emit coverage instrumentation.
    coverage: bool,
    /// Capture stdout/stderr unless this is set (matches existing flag).
    nocapture: bool,
    /// Full resolved language-feature set. Each per-test
    /// CompilerOptions gets a clone so type-check / safety gates /
    /// meta gates / context gates all fire during the per-test build.
    language_features: verum_compiler::language_features::LanguageFeatures,
}

/// Execute the `verum test` command
pub fn execute(
    filter: Option<Text>,
    _release: bool,
    nocapture: bool,
    _test_threads: Option<usize>,
    coverage: bool,
    _verify: Option<Text>,
) -> Result<()> {
    let start = Instant::now();

    // Load manifest, then apply CLI-supplied language-feature overrides
    // (high-level flags + -Z key=value pairs) before use.
    let manifest_dir = Manifest::find_manifest_dir()?;
    let manifest_path = Manifest::manifest_path(&manifest_dir);
    let mut manifest = Manifest::from_file(&manifest_path)?;
    crate::feature_overrides::apply_global(&mut manifest)?;

    // Resolve [test] settings. CLI --coverage flag OR [test].coverage
    // enables coverage; CLI --deny-warnings is not yet exposed, but
    // `[test] deny_warnings = true` in verum.toml takes effect.
    // Convert the fully-merged manifest into LanguageFeatures once;
    // each per-test build clones the result.
    let language_features =
        crate::feature_overrides::manifest_to_features(&manifest)?;
    let test_cfg = TestRunConfig {
        timeout_secs: manifest.test.timeout_secs,
        deny_warnings: manifest.test.deny_warnings,
        coverage: coverage || manifest.test.coverage,
        nocapture,
        language_features,
    };
    // Print effective [test] config so users can verify their policy
    // is taking effect. One-line summary after the Testing header.
    tracing::debug!(
        "test config: parallel={}, timeout_secs={}, deny_warnings={}, coverage={}",
        manifest.test.parallel,
        test_cfg.timeout_secs,
        test_cfg.deny_warnings,
        test_cfg.coverage,
    );

    // Print test header
    ui::output("");
    ui::status(
        "Testing",
        &format!(
            "{} v{}",
            manifest.cog.name.as_str(),
            manifest.cog.version
        ),
    );
    ui::output("");

    ui::step("Discovering tests");

    // Find test files
    let test_files = find_test_files(&manifest_dir)?;

    if test_files.is_empty() {
        ui::warn("No test files found in tests/");
        return Ok(());
    }

    // Discover tests
    let mut all_tests = List::new();
    for file in &test_files {
        let tests = discover_tests(file)?;
        all_tests.extend(tests);
    }

    // Filter tests
    let filtered_tests: List<_> = if let Some(ref f) = filter {
        all_tests
            .into_iter()
            .filter(|t| t.name.as_str().contains(f.as_str()))
            .collect()
    } else {
        all_tests
    };

    let total = filtered_tests.len();
    if total == 0 {
        ui::warn("No tests matched the filter");
        return Ok(());
    }

    // Prepare output directory for test binaries
    let test_target_dir = manifest_dir.join("target").join("test");
    std::fs::create_dir_all(&test_target_dir).ok();

    ui::output(&format!("running {} tests", total));

    // Run tests
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut ignored = 0usize;
    let mut failures: List<TestFailure> = List::new();

    for test in &filtered_tests {
        if test.ignored {
            ui::output(&format!(
                "test {} ... {}",
                test.name,
                "ignored".yellow()
            ));
            ignored += 1;
            continue;
        }

        let result = run_single_test(test, &test_target_dir, &test_cfg);
        match result {
            TestResult::Pass { duration, stdout, stderr } => {
                let duration_str = format_test_duration(duration);
                ui::output(&format!(
                    "test {} ... {} ({})",
                    test.name,
                    "ok".green(),
                    duration_str
                ));
                if nocapture && !stdout.is_empty() {
                    for line in stdout.lines() {
                        ui::output(&format!("  {}", line));
                    }
                }
                if nocapture && !stderr.is_empty() {
                    for line in stderr.lines() {
                        ui::output(&format!("  {}", line));
                    }
                }
                passed += 1;
            }
            TestResult::Fail { duration, stdout, stderr, exit_code, error } => {
                let duration_str = format_test_duration(duration);
                ui::output(&format!(
                    "test {} ... {} ({})",
                    test.name,
                    "FAILED".red().bold(),
                    duration_str
                ));
                failures.push(TestFailure {
                    name: test.name.clone(),
                    stdout,
                    stderr,
                    exit_code,
                    error,
                });
                failed += 1;
            }
            TestResult::CompileError { duration, error } => {
                let duration_str = format_test_duration(duration);
                ui::output(&format!(
                    "test {} ... {} ({})",
                    test.name,
                    "FAILED".red().bold(),
                    duration_str
                ));
                failures.push(TestFailure {
                    name: test.name.clone(),
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: None,
                    error: format!("compilation failed: {}", error),
                });
                failed += 1;
            }
        }
    }

    let total_duration = start.elapsed();

    // Print failures detail
    if !failures.is_empty() {
        ui::output("");
        ui::output(&format!("{}", "failures:".bold()));
        ui::output("");
        for failure in &failures {
            ui::output(&format!("  --- {} ---", failure.name));
            if !failure.error.is_empty() {
                ui::output(&format!("  {}", failure.error));
            }
            if !failure.stdout.is_empty() {
                ui::output("  stdout:");
                for line in failure.stdout.lines().take(20) {
                    ui::output(&format!("    {}", line));
                }
                let line_count = failure.stdout.lines().count();
                if line_count > 20 {
                    ui::output(&format!("    ... ({} more lines)", line_count - 20));
                }
            }
            if !failure.stderr.is_empty() {
                ui::output("  stderr:");
                for line in failure.stderr.lines().take(20) {
                    ui::output(&format!("    {}", line));
                }
                let line_count = failure.stderr.lines().count();
                if line_count > 20 {
                    ui::output(&format!("    ... ({} more lines)", line_count - 20));
                }
            }
            ui::output("");
        }
    }

    // Print summary
    ui::output("");
    let result_word = if failed > 0 {
        "FAILED".red().bold().to_string()
    } else {
        "ok".green().bold().to_string()
    };
    ui::output(&format!(
        "test result: {}. {} passed; {} failed; {} ignored; {} total; finished in {}",
        result_word,
        passed,
        failed,
        ignored,
        total,
        format_test_duration(total_duration),
    ));
    ui::output("");

    // Coverage summary
    if coverage {
        ui::output(&format!("{}", "coverage:".bold()));
        ui::output(&format!("  Functions instrumented: {}", total));
        ui::output(&format!(
            "  Coverage data written to {}/coverage/",
            test_target_dir.display()
        ));
        ui::output("  Use `llvm-cov report` to generate detailed reports");
        ui::output("");
    }

    if failed > 0 {
        Err(CliError::TestsFailed { passed, failed })
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test execution
// ---------------------------------------------------------------------------

enum TestResult {
    Pass {
        duration: Duration,
        stdout: String,
        stderr: String,
    },
    Fail {
        duration: Duration,
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
        error: String,
    },
    CompileError {
        duration: Duration,
        error: String,
    },
}

struct TestFailure {
    name: Text,
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    error: String,
}

/// Compile and execute a single test file.
fn run_single_test(test: &Test, target_dir: &Path, cfg: &TestRunConfig) -> TestResult {
    let compile_start = Instant::now();

    // Derive a unique binary name from the test file path
    let stem = test
        .file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("test");
    let binary_name = format!("test_{}", stem);
    let output_path = target_dir.join(&binary_name);

    // Compile via AOT pipeline. Honor [test].deny_warnings via
    // LintConfig so warning-laden test code fails the build.
    let mut lint_config = verum_compiler::lint::LintConfig::default();
    if cfg.deny_warnings {
        lint_config.deny_warnings = true;
    }
    let options = CompilerOptions {
        input: test.file.clone(),
        output: output_path.clone(),
        verify_mode: VerifyMode::Runtime,
        output_format: OutputFormat::Human,
        coverage: cfg.coverage,
        lint_config,
        // CRITICAL: inherit the full merged feature set. Without this
        // clone, per-test builds reset every `-Z` / `[safety]` / etc.
        // override to defaults, and tests silently pass code that
        // should have been gated.
        language_features: cfg.language_features.clone(),
        ..Default::default()
    };

    let mut session = Session::new(options);
    let mut pipeline = CompilationPipeline::new(&mut session);

    let executable = match pipeline.run_native_compilation() {
        Ok(exe) => exe,
        Err(e) => {
            return TestResult::CompileError {
                duration: compile_start.elapsed(),
                error: e.to_string(),
            };
        }
    };

    // Execute the compiled binary and capture output
    let child = Command::new(&executable)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            return TestResult::Fail {
                duration: compile_start.elapsed(),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: format!("failed to spawn test binary: {}", e),
            };
        }
    };

    // Honor [test].timeout_secs: poll the child every 25 ms up to the
    // timeout; if it's still alive, kill and report as failure. A
    // timeout of 0 means "no limit" — existing behavior preserved.
    let output = if cfg.timeout_secs == 0 {
        child.wait_with_output()
    } else {
        let deadline = Instant::now() + Duration::from_secs(cfg.timeout_secs);
        let poll = Duration::from_millis(25);
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => break child.wait_with_output(),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        // Kill the still-running child and report.
                        let _ = child.kill();
                        return TestResult::Fail {
                            duration: compile_start.elapsed(),
                            stdout: String::new(),
                            stderr: String::new(),
                            exit_code: None,
                            error: format!(
                                "test exceeded [test].timeout_secs = {} (killed)",
                                cfg.timeout_secs
                            ),
                        };
                    }
                    std::thread::sleep(poll);
                }
                Err(e) => {
                    return TestResult::Fail {
                        duration: compile_start.elapsed(),
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: None,
                        error: format!("failed to poll test binary: {}", e),
                    };
                }
            }
        }
    };
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            return TestResult::Fail {
                duration: compile_start.elapsed(),
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                error: format!("failed to wait for test binary: {}", e),
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // Total duration includes compilation + execution
    let total_duration = compile_start.elapsed();

    if output.status.success() {
        TestResult::Pass {
            duration: total_duration,
            stdout,
            stderr,
        }
    } else {
        let code = output.status.code();
        let error = if let Some(c) = code {
            format!("process exited with code {}", c)
        } else {
            "process terminated by signal".to_string()
        };
        TestResult::Fail {
            duration: total_duration,
            stdout,
            stderr,
            exit_code: code,
            error,
        }
    }
}

// ---------------------------------------------------------------------------
// Test discovery (preserved from original implementation)
// ---------------------------------------------------------------------------

struct Test {
    name: Text,
    file: PathBuf,
    ignored: bool,
}

fn discover_tests(file: &Path) -> Result<List<Test>> {
    // Read source
    let source = std::fs::read_to_string(file)?;

    // Try AST-based discovery first
    let file_id = FileId::new(0);
    let lexer = Lexer::new(&source, file_id);
    let parser = VerumParser::new();

    if let Ok(module) = parser.parse_module(lexer, file_id) {
        // AST-based test discovery
        let mut tests = List::new();
        let module_name = file.file_stem().unwrap().to_str().unwrap();

        // Check if any function has @test attribute
        let has_test_attrs = module.items.iter().any(|item| {
            if let ItemKind::Function(func) = &item.kind {
                func.attributes.iter().any(|attr| attr.name.as_str() == "test")
            } else {
                false
            }
        });

        if has_test_attrs {
            // File uses @test attributes: discover individual test functions
            for item in &module.items {
                if let ItemKind::Function(func) = &item.kind {
                    let is_test = func
                        .attributes
                        .iter()
                        .any(|attr| attr.name.as_str() == "test");

                    if is_test {
                        let is_ignored = func.attributes.iter().any(|attr| {
                            attr.name.as_str() == "ignore" || attr.name.as_str() == "ignored"
                        });

                        tests.push(Test {
                            name: format!("{}::{}", module_name, func.name).into(),
                            file: file.to_path_buf(),
                            ignored: is_ignored,
                        });
                    }
                }
            }
        } else {
            // No @test attributes: treat the whole file as a single test
            // (file must have main() and exit 0 to pass)
            let has_main = module.items.iter().any(|item| {
                if let ItemKind::Function(func) = &item.kind {
                    func.name.as_str() == "main"
                } else {
                    false
                }
            });

            if has_main {
                // Check for @ignore at file level (in comments)
                let is_ignored = source
                    .lines()
                    .take(10)
                    .any(|line| {
                        let t = line.trim();
                        t.contains("@ignore") || t.contains("@ignored")
                    });

                tests.push(Test {
                    name: module_name.into(),
                    file: file.to_path_buf(),
                    ignored: is_ignored,
                });
            }
        }

        return Ok(tests);
    }

    // Fallback: pattern-based discovery for files that fail to parse
    let mut tests = List::new();

    for (i, line) in source.lines().enumerate() {
        let line_trim = line.trim();
        // Support both @test and #[test] syntax
        if line_trim.starts_with("@test") || line_trim.starts_with("#[test]") {
            // Next line should be function
            if let Some(next_line) = source.lines().nth(i + 1)
                && let Some(name) = extract_function_name(next_line)
            {
                let is_ignored = line_trim.contains("ignore");
                tests.push(Test {
                    name: format!(
                        "{}::{}",
                        file.file_stem().unwrap().to_str().unwrap(),
                        name
                    )
                    .into(),
                    file: file.to_path_buf(),
                    ignored: is_ignored,
                });
            }
        }
    }

    // If no @test attributes found, treat as whole-file test if it looks like
    // it has a main function
    if tests.is_empty() {
        let has_main = source.lines().any(|line| {
            let t = line.trim();
            t.starts_with("fn main(") || t.starts_with("async fn main(")
        });
        if has_main {
            let module_name = file.file_stem().unwrap().to_str().unwrap();
            tests.push(Test {
                name: module_name.into(),
                file: file.to_path_buf(),
                ignored: false,
            });
        }
    }

    Ok(tests)
}

fn extract_function_name(line: &str) -> Option<Text> {
    // Simple extraction: "fn test_name()"
    let trimmed = line.trim();
    if trimmed.starts_with("fn ") {
        let parts: List<&str> = trimmed.split(&['(', ' '][..]).collect();
        if parts.len() >= 2 {
            return Some(Text::from(parts[1]));
        }
    }
    None
}

fn find_test_files(project_dir: &Path) -> Result<List<PathBuf>> {
    let tests_dir = project_dir.join("tests");

    if !tests_dir.exists() {
        return Ok(List::new());
    }

    let mut files = List::new();
    for entry in walkdir::WalkDir::new(tests_dir).follow_links(false) {
        let entry = entry?;
        let path = entry.path();

        match path.extension().and_then(|s| s.to_str()) {
            Some("vr") => {
                files.push(path.to_path_buf());
            }
            _ => {}
        }
    }

    // Sort for deterministic ordering
    files.sort();

    Ok(files)
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_test_duration(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        format!("{:.1}ms", ms)
    } else if ms < 1000.0 {
        format!("{:.0}ms", ms)
    } else {
        format!("{:.1}s", d.as_secs_f64())
    }
}
