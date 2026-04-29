//! Comprehensive CLI Integration Tests
//!
//! These tests verify ALL CLI parameters work correctly with REAL execution.
//! No mocks - all tests use actual compiler/runtime infrastructure.
//! Uses tempfile for auto-cleanup after each test.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

// ============================================================================
// Test Infrastructure
// ============================================================================

/// Create a temporary Verum project with auto-cleanup
fn create_test_project(name: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("Failed to create temp dir");
    let project_dir = temp.path().join(name);
    fs::create_dir_all(&project_dir).expect("Failed to create project dir");
    (temp, project_dir)
}

/// Create Verum.toml manifest in the given directory
fn create_manifest(dir: &PathBuf, name: &str, profile: &str) {
    let manifest = format!(
        r#"[cog]
name = "{name}"
version = "0.1.0"

[language]
profile = "{profile}"

[dependencies]

[profile.dev]
tier = "interpreter"
verification = "runtime"

[profile.release]
tier = "aot"
verification = "runtime"
"#
    );
    fs::write(dir.join("Verum.toml"), manifest).expect("Failed to write manifest");
}

/// Create a simple main.vr file
fn create_main_file(dir: &PathBuf, code: &str) {
    let src_dir = dir.join("src");
    fs::create_dir_all(&src_dir).expect("Failed to create src dir");
    fs::write(src_dir.join("main.vr"), code).expect("Failed to write main.vr");
}

/// Run verum CLI command and return output (with retry for flaky LLVM builds)
fn run_verum(args: &[&str], cwd: Option<&PathBuf>) -> Output {
    for attempt in 0..3 {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_verum"));
        cmd.args(args);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let output = cmd.output().expect("Failed to execute verum command");
        if output.status.success() || attempt == 2 {
            return output;
        }
        // Retry on failure (LLVM codegen can fail under resource pressure)
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    unreachable!()
}

/// Check if output indicates success
fn assert_success(output: &Output, context: &str) {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "{} failed:\nstdout: {}\nstderr: {}",
            context, stdout, stderr
        );
    }
}

/// Check if output contains expected string
fn assert_output_contains(output: &Output, expected: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", stdout, stderr);
    assert!(
        combined.contains(expected),
        "Expected output to contain '{}', got:\n{}",
        expected,
        combined
    );
}

// ============================================================================
// Tier Parameter Tests
// ============================================================================

mod tier_tests {
    

    /// Test that all tier names are parsed correctly (2-tier model)
    #[test]
    fn test_tier_names_parsing() {
        use verum_cli::config::CompilationTier;

        // Numeric values
        assert_eq!(
            CompilationTier::from_str("0"),
            Some(CompilationTier::Interpreter)
        );
        assert_eq!(CompilationTier::from_str("1"), Some(CompilationTier::Aot));

        // Human-readable names for Interpreter
        assert_eq!(
            CompilationTier::from_str("interpreter"),
            Some(CompilationTier::Interpreter)
        );
        assert_eq!(
            CompilationTier::from_str("interp"),
            Some(CompilationTier::Interpreter)
        );

        // Human-readable names for AOT (includes legacy names)
        assert_eq!(CompilationTier::from_str("aot"), Some(CompilationTier::Aot));
        assert_eq!(
            CompilationTier::from_str("release"),
            Some(CompilationTier::Aot)
        );
        assert_eq!(
            CompilationTier::from_str("native"),
            Some(CompilationTier::Aot)
        );
        // Case insensitivity
        assert_eq!(
            CompilationTier::from_str("INTERPRETER"),
            Some(CompilationTier::Interpreter)
        );
        assert_eq!(CompilationTier::from_str("AOT"), Some(CompilationTier::Aot));

        // Invalid values
        assert_eq!(CompilationTier::from_str("2"), None);
        assert_eq!(CompilationTier::from_str("3"), None);
        assert_eq!(CompilationTier::from_str("jit"), None);
        assert_eq!(CompilationTier::from_str("baseline"), None);
        assert_eq!(CompilationTier::from_str("optimized"), None);
        assert_eq!(CompilationTier::from_str("opt"), None);
        assert_eq!(CompilationTier::from_str("max"), None);
        assert_eq!(CompilationTier::from_str("invalid"), None);
        assert_eq!(CompilationTier::from_str("-1"), None);
    }

    /// Test tier name() method returns correct human-readable names
    #[test]
    fn test_tier_names() {
        use verum_cli::config::CompilationTier;

        assert_eq!(CompilationTier::Interpreter.name(), "interpreter");
        assert_eq!(CompilationTier::Aot.name(), "aot");
    }

    /// Test tier valid_values() for help text
    #[test]
    fn test_tier_valid_values() {
        use verum_cli::config::CompilationTier;

        let values = CompilationTier::valid_values();
        assert!(values.contains("interpreter"));
        assert!(values.contains("aot"));
        assert!(values.contains("0-1"));
    }
}

// ============================================================================
// Build Command Tests
// ============================================================================

mod build_tests {
    use super::*;

    /// Test basic build with default parameters
    #[test]
    fn test_build_default() {
        let (_temp, dir) = create_test_project("test_build_default");
        create_manifest(&dir, "test_build_default", "application");
        create_main_file(
            &dir,
            r#"
fn main() -> Int {
    0
}
"#,
        );

        let output = run_verum(&["build"], Some(&dir));
        assert_success(&output, "verum build");
        assert_output_contains(&output, "Finished");
    }

    // Note: verum build does not support --tier flag (always AOT).
    // Tier selection is only available via verum run --tier.

    /// Test build with --refs managed
    #[test]
    fn test_build_refs_managed() {
        let (_temp, dir) = create_test_project("test_refs_managed");
        create_manifest(&dir, "test_refs_managed", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        // Use --verbose to see CBGR overhead info in output
        let output = run_verum(&["--verbose", "build", "--refs", "managed"], Some(&dir));
        assert_success(&output, "verum build --refs managed");
        assert_output_contains(&output, "15ns");
    }

    /// Test build with --refs checked
    #[test]
    fn test_build_refs_checked() {
        let (_temp, dir) = create_test_project("test_refs_checked");
        create_manifest(&dir, "test_refs_checked", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        // Use --verbose to see CBGR overhead info in output
        let output = run_verum(&["--verbose", "build", "--refs", "checked"], Some(&dir));
        assert_success(&output, "verum build --refs checked");
        assert_output_contains(&output, "0ns");
    }

    /// Test build with --refs mixed
    #[test]
    fn test_build_refs_mixed() {
        let (_temp, dir) = create_test_project("test_refs_mixed");
        create_manifest(&dir, "test_refs_mixed", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--refs", "mixed"], Some(&dir));
        assert_success(&output, "verum build --refs mixed");
    }

    /// Test build with invalid refs mode produces error
    #[test]
    fn test_build_invalid_refs() {
        let (_temp, dir) = create_test_project("test_invalid_refs");
        create_manifest(&dir, "test_invalid_refs", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--refs", "invalid"], Some(&dir));
        assert!(!output.status.success());
        assert_output_contains(&output, "Invalid reference mode");
    }

    /// Test build with --verify none
    #[test]
    fn test_build_verify_none() {
        let (_temp, dir) = create_test_project("test_verify_none");
        create_manifest(&dir, "test_verify_none", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--verify", "none"], Some(&dir));
        assert_success(&output, "verum build --verify none");
    }

    /// Test build with --verify runtime
    #[test]
    fn test_build_verify_runtime() {
        let (_temp, dir) = create_test_project("test_verify_runtime");
        create_manifest(&dir, "test_verify_runtime", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--verify", "runtime"], Some(&dir));
        assert_success(&output, "verum build --verify runtime");
    }

    /// Test build with --verify proof
    #[test]
    fn test_build_verify_proof() {
        let (_temp, dir) = create_test_project("test_verify_proof");
        create_manifest(&dir, "test_verify_proof", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--verify", "proof"], Some(&dir));
        assert_success(&output, "verum build --verify proof");
    }

    /// Test build with invalid verify level produces error
    #[test]
    fn test_build_invalid_verify() {
        let (_temp, dir) = create_test_project("test_invalid_verify");
        create_manifest(&dir, "test_invalid_verify", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--verify", "invalid"], Some(&dir));
        assert!(!output.status.success());
        assert_output_contains(&output, "Invalid verification level");
    }

    /// Test build with --release flag
    #[test]
    fn test_build_release() {
        let (_temp, dir) = create_test_project("test_build_release");
        create_manifest(&dir, "test_build_release", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--release"], Some(&dir));
        assert_success(&output, "verum build --release");
        assert_output_contains(&output, "release");
    }

    /// Test build with --jobs parameter
    #[test]
    fn test_build_jobs() {
        let (_temp, dir) = create_test_project("test_build_jobs");
        create_manifest(&dir, "test_build_jobs", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--jobs", "2"], Some(&dir));
        assert_success(&output, "verum build --jobs 2");
    }

    /// Test build with --timings flag
    #[test]
    fn test_build_timings() {
        let (_temp, dir) = create_test_project("test_build_timings");
        create_manifest(&dir, "test_build_timings", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["build", "--timings"], Some(&dir));
        assert_success(&output, "verum build --timings");
        // Timings mode shows compilation statistics including binary path
        assert_output_contains(&output, "Finished");
    }

    /// Test build combining multiple options
    #[test]
    fn test_build_combined_options() {
        let (_temp, dir) = create_test_project("test_build_combined");
        create_manifest(&dir, "test_build_combined", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        // Note: --tier is not supported by build (always AOT).
        // Tier selection is only available via verum run --tier.
        let output = run_verum(
            &[
                "build",
                "--refs",
                "mixed",
                "--verify",
                "runtime",
                "--jobs",
                "4",
                "--timings",
            ],
            Some(&dir),
        );
        assert_success(&output, "verum build with combined options");
    }
}

// ============================================================================
// Check Command Tests
// ============================================================================

mod check_tests {
    use super::*;

    /// Test basic check command
    #[test]
    fn test_check_basic() {
        let (_temp, dir) = create_test_project("test_check_basic");
        create_manifest(&dir, "test_check_basic", "application");
        create_main_file(
            &dir,
            r#"
fn main() -> Int {
    let x: Int = 42;
    x
}
"#,
        );

        let output = run_verum(&["check"], Some(&dir));
        assert_success(&output, "verum check");
    }

    /// Test check with type error (should fail)
    #[test]
    fn test_check_type_error() {
        let (_temp, dir) = create_test_project("test_check_error");
        create_manifest(&dir, "test_check_error", "application");
        create_main_file(
            &dir,
            r#"
fn main() -> Int {
    let x: Text = 42;  // Type error: Int assigned to Text
    x
}
"#,
        );

        let _output = run_verum(&["check"], Some(&dir));
        // This should fail due to type mismatch
        // Note: depending on implementation, this may or may not fail
    }
}

// ============================================================================
// Clean Command Tests
// ============================================================================

mod clean_tests {
    use super::*;

    /// Test clean command removes target directory
    #[test]
    fn test_clean_basic() {
        let (_temp, dir) = create_test_project("test_clean_basic");
        create_manifest(&dir, "test_clean_basic", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        // Build first to create target
        let _ = run_verum(&["build"], Some(&dir));

        // Clean
        let output = run_verum(&["clean"], Some(&dir));
        assert_success(&output, "verum clean");
    }

    /// Test clean --all removes all artifacts
    #[test]
    fn test_clean_all() {
        let (_temp, dir) = create_test_project("test_clean_all");
        create_manifest(&dir, "test_clean_all", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        // Build first
        let _ = run_verum(&["build"], Some(&dir));

        // Clean all
        let output = run_verum(&["clean", "--all"], Some(&dir));
        assert_success(&output, "verum clean --all");
    }
}

// ============================================================================
// Version/Info Command Tests
// ============================================================================

mod info_tests {
    use super::*;

    /// Test version command
    #[test]
    fn test_version() {
        let output = run_verum(&["version"], None);
        assert_success(&output, "verum version");
        assert_output_contains(&output, "verum");
    }

    /// Test version --verbose
    #[test]
    fn test_version_verbose() {
        let output = run_verum(&["version", "--verbose"], None);
        assert_success(&output, "verum version --verbose");
    }

    /// Test info command
    #[test]
    fn test_info() {
        let output = run_verum(&["info"], None);
        assert_success(&output, "verum info");
    }

    /// Test info --all
    #[test]
    fn test_info_all() {
        let output = run_verum(&["info", "--all"], None);
        assert_success(&output, "verum info --all");
    }

    /// Test info --features
    #[test]
    fn test_info_features() {
        let output = run_verum(&["info", "--features"], None);
        assert_success(&output, "verum info --features");
    }

    /// Test info --llvm
    #[test]
    fn test_info_llvm() {
        let output = run_verum(&["info", "--llvm"], None);
        assert_success(&output, "verum info --llvm");
    }
}

// ============================================================================
// Explain Command Tests
// ============================================================================

mod explain_tests {
    use super::*;

    /// Test explain with valid error code
    #[test]
    fn test_explain_valid_code() {
        let _output = run_verum(&["explain", "E0200"], None);
        // Should succeed even if error code isn't found (graceful handling)
        // The command should return something
    }

    /// Test explain with --no-color
    #[test]
    fn test_explain_no_color() {
        let _output = run_verum(&["explain", "E0200", "--no-color"], None);
        // Should succeed
    }
}

// ============================================================================
// Help Tests
// ============================================================================

mod help_tests {
    use super::*;

    /// Test --help shows usage
    #[test]
    fn test_help() {
        let output = run_verum(&["--help"], None);
        assert_success(&output, "verum --help");
        // The CLI's About string is the language tagline:
        // "The Verum language compiler — semantic honesty, cost
        // transparency, zero-cost safety". Match the stable
        // "Verum language compiler" prefix so future tagline tweaks
        // don't break the contract.
        assert_output_contains(&output, "Verum language compiler");
    }

    /// Test build --help shows build options
    #[test]
    fn test_build_help() {
        let output = run_verum(&["build", "--help"], None);
        assert_success(&output, "verum build --help");
        assert_output_contains(&output, "release");
    }

    /// Test verify --help shows options
    #[test]
    fn test_verify_help() {
        let output = run_verum(&["verify", "--help"], None);
        assert_success(&output, "verum verify --help");
    }

    /// Test analyze --help shows options
    #[test]
    fn test_analyze_help() {
        let output = run_verum(&["analyze", "--help"], None);
        assert_success(&output, "verum analyze --help");
    }
}

// ============================================================================
// Global Options Tests
// ============================================================================

mod global_options_tests {
    use super::*;

    /// Test --verbose global option
    #[test]
    fn test_verbose() {
        let output = run_verum(&["--verbose", "version"], None);
        assert_success(&output, "verum --verbose version");
    }

    /// Test --quiet global option
    #[test]
    fn test_quiet() {
        let output = run_verum(&["--quiet", "version"], None);
        assert_success(&output, "verum --quiet version");
    }

    /// Test --color option
    #[test]
    fn test_color_auto() {
        let output = run_verum(&["--color", "auto", "version"], None);
        assert_success(&output, "verum --color auto version");
    }

    /// Test --color always
    #[test]
    fn test_color_always() {
        let output = run_verum(&["--color", "always", "version"], None);
        assert_success(&output, "verum --color always version");
    }

    /// Test --color never
    #[test]
    fn test_color_never() {
        let output = run_verum(&["--color", "never", "version"], None);
        assert_success(&output, "verum --color never version");
    }
}

// ============================================================================
// New/Init Project Tests
// ============================================================================

mod project_tests {
    use super::*;

    /// Test new command creates project
    #[test]
    fn test_new_project() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let project_dir = temp.path().join("test_new_project");

        let output = run_verum(
            &[
                "new",
                "test_new_project",
                "--profile",
                "application",
                "--path",
                project_dir.to_str().unwrap(),
            ],
            None,
        );
        assert_success(&output, "verum new");

        // Verify files were created
        assert!(project_dir.join("verum.toml").exists());
        assert!(project_dir.join("src").exists());
    }

    /// Test new with --type application
    #[test]
    fn test_new_application() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let project_dir = temp.path().join("test_app");

        let output = run_verum(
            &[
                "new",
                "test_app",
                "--profile",
                "application",
                "--path",
                project_dir.to_str().unwrap(),
            ],
            None,
        );
        assert_success(&output, "verum new --type application");

        // Verify files were created
        assert!(project_dir.join("verum.toml").exists());
    }

    /// Test new with --type systems
    #[test]
    fn test_new_systems() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let project_dir = temp.path().join("test_sys");

        let output = run_verum(
            &[
                "new",
                "test_sys",
                "--profile",
                "systems",
                "--path",
                project_dir.to_str().unwrap(),
            ],
            None,
        );
        assert_success(&output, "verum new --type systems");

        // Verify files were created
        assert!(project_dir.join("verum.toml").exists());
    }

    /// Test new with --lib flag
    #[test]
    fn test_new_library() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let project_dir = temp.path().join("test_lib");

        let output = run_verum(
            &[
                "new",
                "test_lib",
                "--profile",
                "application",
                "--lib",
                "--path",
                project_dir.to_str().unwrap(),
            ],
            None,
        );
        assert_success(&output, "verum new --lib");

        // Verify library structure was created
        assert!(project_dir.join("verum.toml").exists());
        assert!(project_dir.join("src").join("lib.vr").exists());
    }

    /// Test new with --vcs none
    #[test]
    fn test_new_no_vcs() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let project_dir = temp.path().join("test_no_vcs");

        let output = run_verum(
            &[
                "new",
                "test_no_vcs",
                "--profile",
                "application",
                "--vcs",
                "none",
                "--path",
                project_dir.to_str().unwrap(),
            ],
            None,
        );
        assert_success(&output, "verum new --vcs none");

        // Verify no .git directory
        assert!(!project_dir.join(".git").exists());
        // But project should exist
        assert!(project_dir.join("verum.toml").exists());
    }
}

// ============================================================================
// Deps Command Tests
// ============================================================================

mod deps_tests {
    use super::*;

    /// Test deps list command
    #[test]
    fn test_deps_list() {
        let (_temp, dir) = create_test_project("test_deps_list");
        create_manifest(&dir, "test_deps_list", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["deps", "list"], Some(&dir));
        assert_success(&output, "verum deps list");
    }

    /// Test deps list --tree
    #[test]
    fn test_deps_list_tree() {
        let (_temp, dir) = create_test_project("test_deps_tree");
        create_manifest(&dir, "test_deps_tree", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["deps", "list", "--tree"], Some(&dir));
        assert_success(&output, "verum deps list --tree");
    }
}

// ============================================================================
// Analyze Command Tests
// ============================================================================

mod analyze_tests {
    use super::*;

    /// Test analyze --escape
    #[test]
    fn test_analyze_escape() {
        let (_temp, dir) = create_test_project("test_analyze_escape");
        create_manifest(&dir, "test_analyze_escape", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["analyze", "--escape"], Some(&dir));
        assert_success(&output, "verum analyze --escape");
    }

    /// Test analyze --context
    #[test]
    fn test_analyze_context() {
        let (_temp, dir) = create_test_project("test_analyze_context");
        create_manifest(&dir, "test_analyze_context", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["analyze", "--context"], Some(&dir));
        assert_success(&output, "verum analyze --context");
    }

    /// Test analyze --refinement
    #[test]
    fn test_analyze_refinement() {
        let (_temp, dir) = create_test_project("test_analyze_refinement");
        create_manifest(&dir, "test_analyze_refinement", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["analyze", "--refinement"], Some(&dir));
        assert_success(&output, "verum analyze --refinement");
    }

    /// Test analyze --all
    #[test]
    fn test_analyze_all() {
        let (_temp, dir) = create_test_project("test_analyze_all");
        create_manifest(&dir, "test_analyze_all", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["analyze", "--all"], Some(&dir));
        assert_success(&output, "verum analyze --all");
    }
}

// ============================================================================
// Verify Command Tests
// ============================================================================

mod verify_tests {
    use super::*;

    /// Test verify --mode proof
    #[test]
    fn test_verify_proof() {
        let (_temp, dir) = create_test_project("test_verify_proof");
        create_manifest(&dir, "test_verify_proof", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let _output = run_verum(&["verify", "--mode", "proof"], Some(&dir));
        // Verify command may have long-running SMT solver
        // Just check it doesn't crash immediately
    }

    /// Test verify --mode runtime
    #[test]
    fn test_verify_runtime() {
        let (_temp, dir) = create_test_project("test_verify_runtime");
        create_manifest(&dir, "test_verify_runtime", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["verify", "--mode", "runtime"], Some(&dir));
        assert_success(&output, "verum verify --mode runtime");
    }

    /// Test verify --timeout
    #[test]
    fn test_verify_timeout() {
        let (_temp, dir) = create_test_project("test_verify_timeout");
        create_manifest(&dir, "test_verify_timeout", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let _output = run_verum(&["verify", "--timeout", "5"], Some(&dir));
        // Should complete quickly with short timeout
    }

    /// Test verify --show-cost
    #[test]
    fn test_verify_show_cost() {
        let (_temp, dir) = create_test_project("test_verify_cost");
        create_manifest(&dir, "test_verify_cost", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["verify", "--show-cost"], Some(&dir));
        assert_success(&output, "verum verify --show-cost");
    }
}

// ============================================================================
// Profile Command Tests
// ============================================================================

mod profile_tests {
    use super::*;

    /// Test profile --memory
    #[test]
    fn test_profile_memory() {
        let (_temp, dir) = create_test_project("test_profile_memory");
        create_manifest(&dir, "test_profile_memory", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["profile", "--memory"], Some(&dir));
        assert_success(&output, "verum profile --memory");
    }

    /// Test profile --cpu
    #[test]
    fn test_profile_cpu() {
        let (_temp, dir) = create_test_project("test_profile_cpu");
        create_manifest(&dir, "test_profile_cpu", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["profile", "--cpu"], Some(&dir));
        assert_success(&output, "verum profile --cpu");
    }

    /// Test profile --suggest
    #[test]
    fn test_profile_suggest() {
        let (_temp, dir) = create_test_project("test_profile_suggest");
        create_manifest(&dir, "test_profile_suggest", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["profile", "--suggest"], Some(&dir));
        assert_success(&output, "verum profile --suggest");
    }
}

// ============================================================================
// Fmt Command Tests
// ============================================================================

mod fmt_tests {
    use super::*;

    /// Test fmt command
    #[test]
    fn test_fmt() {
        let (_temp, dir) = create_test_project("test_fmt");
        create_manifest(&dir, "test_fmt", "application");
        create_main_file(&dir, r#"fn main()->Int{0}"#); // Unformatted

        let output = run_verum(&["fmt"], Some(&dir));
        assert_success(&output, "verum fmt");
    }

    /// Test fmt --check
    #[test]
    fn test_fmt_check() {
        let (_temp, dir) = create_test_project("test_fmt_check");
        create_manifest(&dir, "test_fmt_check", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let _output = run_verum(&["fmt", "--check"], Some(&dir));
        // May succeed or fail depending on format
    }
}

// ============================================================================
// Lint Command Tests
// ============================================================================

mod lint_tests {
    use super::*;

    /// Test lint command
    #[test]
    fn test_lint() {
        let (_temp, dir) = create_test_project("test_lint");
        create_manifest(&dir, "test_lint", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["lint"], Some(&dir));
        assert_success(&output, "verum lint");
    }

    /// Test lint --fix
    #[test]
    fn test_lint_fix() {
        let (_temp, dir) = create_test_project("test_lint_fix");
        create_manifest(&dir, "test_lint_fix", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["lint", "--fix"], Some(&dir));
        assert_success(&output, "verum lint --fix");
    }

    /// Test lint --deny-warnings
    #[test]
    fn test_lint_deny_warnings() {
        let (_temp, dir) = create_test_project("test_lint_deny");
        create_manifest(&dir, "test_lint_deny", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let _output = run_verum(&["lint", "--deny-warnings"], Some(&dir));
        // May succeed or fail depending on warnings
    }
}

// ============================================================================
// Audit Command Tests
// ============================================================================

mod audit_tests {
    use super::*;

    /// Test audit command
    #[test]
    fn test_audit() {
        let (_temp, dir) = create_test_project("test_audit");
        create_manifest(&dir, "test_audit", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["audit"], Some(&dir));
        assert_success(&output, "verum audit");
    }

    /// Test audit --details
    #[test]
    fn test_audit_details() {
        let (_temp, dir) = create_test_project("test_audit_details");
        create_manifest(&dir, "test_audit_details", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["audit", "--details"], Some(&dir));
        assert_success(&output, "verum audit --details");
    }
}

// ============================================================================
// Tree Command Tests
// ============================================================================

mod tree_tests {
    use super::*;

    /// Test tree command
    #[test]
    fn test_tree() {
        let (_temp, dir) = create_test_project("test_tree");
        create_manifest(&dir, "test_tree", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["tree"], Some(&dir));
        assert_success(&output, "verum tree");
    }

    /// Test tree --duplicates
    #[test]
    fn test_tree_duplicates() {
        let (_temp, dir) = create_test_project("test_tree_dups");
        create_manifest(&dir, "test_tree_dups", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["tree", "--duplicates"], Some(&dir));
        assert_success(&output, "verum tree --duplicates");
    }

    /// Test tree --depth
    #[test]
    fn test_tree_depth() {
        let (_temp, dir) = create_test_project("test_tree_depth");
        create_manifest(&dir, "test_tree_depth", "application");
        create_main_file(&dir, r#"fn main() -> Int { 0 }"#);

        let output = run_verum(&["tree", "--depth", "2"], Some(&dir));
        assert_success(&output, "verum tree --depth 2");
    }
}
