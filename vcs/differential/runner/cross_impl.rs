//! Cross-implementation testing framework
//!
//! This module provides infrastructure for testing Verum code across
//! multiple implementations (reference interpreter, production compiler,
//! alternative backends, etc.) to ensure specification conformance.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::divergence::{
    Divergence, DivergenceClass, DivergenceReporter, ReportFormat, Tier, TierExecution,
    create_divergence,
};
use crate::normalizer::{NormalizationConfig, Normalizer};
use crate::semantic_equiv::{EquivalenceConfig, EquivalenceResult, SemanticEquivalenceChecker};

/// Implementation identifier
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Implementation {
    /// Name of the implementation
    pub name: String,
    /// Version string
    pub version: String,
    /// Path to the executable
    pub binary_path: PathBuf,
    /// Command line arguments
    pub args: Vec<String>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Whether this is the reference implementation
    pub is_reference: bool,
    /// Supported features
    pub features: Vec<String>,
    /// Platform constraints
    pub platforms: Vec<String>,
}

impl Implementation {
    /// Create a new implementation descriptor
    pub fn new(name: impl Into<String>, binary_path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            version: "0.0.0".to_string(),
            binary_path: binary_path.into(),
            args: vec![],
            env: HashMap::new(),
            is_reference: false,
            features: vec!["core".to_string()],
            platforms: vec!["any".to_string()],
        }
    }

    /// Set as reference implementation
    pub fn as_reference(mut self) -> Self {
        self.is_reference = true;
        self
    }

    /// Add command line arguments
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Add environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set version
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Check if implementation supports a feature
    pub fn supports(&self, feature: &str) -> bool {
        self.features.contains(&feature.to_string())
    }

    /// Check if implementation supports current platform
    pub fn supports_platform(&self, platform: &str) -> bool {
        self.platforms.contains(&"any".to_string())
            || self.platforms.contains(&platform.to_string())
    }
}

/// Result of running on a single implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplementationResult {
    pub implementation: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub duration: Duration,
    pub success: bool,
    pub memory_bytes: usize,
}

/// Cross-implementation test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossImplResult {
    /// Source file tested
    pub source_path: PathBuf,
    /// Results from each implementation
    pub results: HashMap<String, ImplementationResult>,
    /// Reference implementation name
    pub reference: String,
    /// Whether all implementations agree
    pub consensus: bool,
    /// Implementations that diverged from reference
    pub divergent: Vec<String>,
    /// Detailed comparison results
    pub comparisons: Vec<ImplComparison>,
}

/// Comparison between reference and another implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplComparison {
    pub implementation: String,
    pub equivalent: bool,
    pub differences: Vec<String>,
}

/// Cross-implementation test runner configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossImplConfig {
    /// Registered implementations
    pub implementations: Vec<Implementation>,
    /// Working directory
    pub work_dir: PathBuf,
    /// Default timeout
    pub timeout_ms: u64,
    /// Normalization config
    pub normalization: NormalizationConfig,
    /// Equivalence config
    pub equivalence: EquivalenceConfig,
    /// Whether to stop on first divergence
    pub fail_fast: bool,
    /// Whether to run in parallel
    pub parallel: bool,
    /// Number of parallel workers
    pub workers: usize,
    /// Report output directory
    pub report_dir: PathBuf,
}

impl Default for CrossImplConfig {
    fn default() -> Self {
        Self {
            implementations: vec![],
            work_dir: PathBuf::from("."),
            timeout_ms: 30_000,
            normalization: NormalizationConfig::semantic(),
            equivalence: EquivalenceConfig::default(),
            fail_fast: false,
            parallel: true,
            workers: num_cpus::get(),
            report_dir: PathBuf::from("cross_impl_reports"),
        }
    }
}

impl CrossImplConfig {
    /// Add an implementation
    pub fn with_implementation(mut self, impl_: Implementation) -> Self {
        self.implementations.push(impl_);
        self
    }

    /// Set reference implementation
    pub fn with_reference(mut self, name: &str, binary: impl Into<PathBuf>) -> Self {
        let impl_ = Implementation::new(name, binary).as_reference();
        self.implementations.push(impl_);
        self
    }

    /// Add alternative implementation
    pub fn with_alternative(mut self, name: &str, binary: impl Into<PathBuf>) -> Self {
        let impl_ = Implementation::new(name, binary);
        self.implementations.push(impl_);
        self
    }

    /// Get reference implementation
    pub fn reference(&self) -> Option<&Implementation> {
        self.implementations.iter().find(|i| i.is_reference)
    }
}

/// Cross-implementation test runner
pub struct CrossImplRunner {
    config: CrossImplConfig,
    normalizer: Normalizer,
    checker: SemanticEquivalenceChecker,
}

impl CrossImplRunner {
    /// Create a new runner
    pub fn new(config: CrossImplConfig) -> Self {
        let normalizer = Normalizer::new(config.normalization.clone());
        let checker = SemanticEquivalenceChecker::new(config.equivalence.clone());

        Self {
            config,
            normalizer,
            checker,
        }
    }

    /// Run a single test file across all implementations
    pub fn run(&self, source_path: &Path) -> Result<CrossImplResult> {
        let source_code = fs::read_to_string(source_path)
            .with_context(|| format!("Failed to read source: {}", source_path.display()))?;

        // Parse test metadata
        let metadata = self.parse_metadata(&source_code);

        // Filter implementations by features
        let applicable_impls: Vec<&Implementation> = self
            .config
            .implementations
            .iter()
            .filter(|impl_| self.is_applicable(impl_, &metadata))
            .collect();

        if applicable_impls.is_empty() {
            bail!("No applicable implementations for test");
        }

        // Find reference implementation
        let reference = applicable_impls
            .iter()
            .find(|i| i.is_reference)
            .ok_or_else(|| anyhow::anyhow!("No reference implementation found"))?;

        // Run on each implementation
        let mut results = HashMap::new();

        for impl_ in &applicable_impls {
            let result = self.run_implementation(impl_, source_path)?;
            results.insert(impl_.name.clone(), result);
        }

        // Compare results
        let ref_result = results
            .get(&reference.name)
            .ok_or_else(|| anyhow::anyhow!("Reference result missing"))?;

        let mut divergent = Vec::new();
        let mut comparisons = Vec::new();

        for impl_ in &applicable_impls {
            if impl_.is_reference {
                continue;
            }

            let impl_result = results
                .get(&impl_.name)
                .ok_or_else(|| anyhow::anyhow!("Result missing for {}", impl_.name))?;

            let comparison = self.compare_results(impl_, ref_result, impl_result);
            if !comparison.equivalent {
                divergent.push(impl_.name.clone());
            }
            comparisons.push(comparison);
        }

        Ok(CrossImplResult {
            source_path: source_path.to_path_buf(),
            results,
            reference: reference.name.clone(),
            consensus: divergent.is_empty(),
            divergent,
            comparisons,
        })
    }

    /// Run on a single implementation
    fn run_implementation(
        &self,
        impl_: &Implementation,
        source_path: &Path,
    ) -> Result<ImplementationResult> {
        let start = Instant::now();

        let mut cmd = Command::new(&impl_.binary_path);
        cmd.args(&impl_.args)
            .arg(source_path)
            .current_dir(&self.config.work_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for (key, value) in &impl_.env {
            cmd.env(key, value);
        }

        // Run with timeout
        let output = self.run_with_timeout(&mut cmd)?;
        let duration = start.elapsed();

        Ok(ImplementationResult {
            implementation: impl_.name.clone(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code(),
            duration,
            success: output.status.success(),
            memory_bytes: 0, // Would need platform-specific measurement
        })
    }

    /// Run command with timeout
    fn run_with_timeout(&self, cmd: &mut Command) -> Result<Output> {
        use std::io::Read;
        use wait_timeout::ChildExt;

        let mut child = cmd.spawn().context("Failed to spawn process")?;
        let timeout = Duration::from_millis(self.config.timeout_ms);

        match child.wait_timeout(timeout)? {
            Some(status) => {
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();

                if let Some(mut out) = child.stdout.take() {
                    out.read_to_end(&mut stdout)?;
                }
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_end(&mut stderr)?;
                }

                Ok(Output {
                    status,
                    stdout,
                    stderr,
                })
            }
            None => {
                child.kill()?;
                bail!("Process timed out after {}ms", self.config.timeout_ms);
            }
        }
    }

    /// Compare implementation result with reference
    fn compare_results(
        &self,
        impl_: &Implementation,
        reference: &ImplementationResult,
        result: &ImplementationResult,
    ) -> ImplComparison {
        let mut differences = Vec::new();

        // Compare exit codes
        if reference.exit_code != result.exit_code {
            differences.push(format!(
                "Exit code: reference={:?}, {}={:?}",
                reference.exit_code, impl_.name, result.exit_code
            ));
        }

        // Normalize and compare stdout
        let ref_stdout = self.normalizer.normalize(&reference.stdout);
        let impl_stdout = self.normalizer.normalize(&result.stdout);

        match self.checker.check(&ref_stdout, &impl_stdout) {
            EquivalenceResult::Equivalent => {}
            EquivalenceResult::Different(diffs) => {
                for diff in diffs {
                    differences.push(format!(
                        "Stdout differs at {}: expected '{}', got '{}'",
                        diff.location, diff.expected, diff.actual
                    ));
                }
            }
        }

        // Compare stderr (less strict)
        if !reference.stderr.is_empty() || !result.stderr.is_empty() {
            let ref_stderr = self.normalizer.normalize(&reference.stderr);
            let impl_stderr = self.normalizer.normalize(&result.stderr);

            if ref_stderr != impl_stderr {
                differences.push(format!(
                    "Stderr differs: reference='{}', {}='{}'",
                    truncate(&ref_stderr, 100),
                    impl_.name,
                    truncate(&impl_stderr, 100)
                ));
            }
        }

        ImplComparison {
            implementation: impl_.name.clone(),
            equivalent: differences.is_empty(),
            differences,
        }
    }

    /// Check if implementation is applicable for test
    fn is_applicable(&self, impl_: &Implementation, metadata: &TestMetadata) -> bool {
        // Check required features
        for feature in &metadata.required_features {
            if !impl_.supports(feature) {
                return false;
            }
        }

        // Check platform
        let current_platform = std::env::consts::OS;
        if !impl_.supports_platform(current_platform) {
            return false;
        }

        true
    }

    /// Parse test metadata from source
    fn parse_metadata(&self, source: &str) -> TestMetadata {
        let mut metadata = TestMetadata::default();

        for line in source.lines() {
            let line = line.trim();

            if line.starts_with("// @impl:") {
                let impls: Vec<String> = line
                    .trim_start_matches("// @impl:")
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                metadata.implementations = impls;
            }

            if line.starts_with("// @require:") {
                let features: Vec<String> = line
                    .trim_start_matches("// @require:")
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                metadata.required_features = features;
            }

            if line.starts_with("// @platform:") {
                let platforms: Vec<String> = line
                    .trim_start_matches("// @platform:")
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                metadata.platforms = platforms;
            }

            if line.starts_with("// @skip-impl:") {
                let skip: Vec<String> = line
                    .trim_start_matches("// @skip-impl:")
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                metadata.skip_implementations = skip;
            }
        }

        metadata
    }

    /// Run all tests in a directory
    pub fn run_directory(&self, dir: &Path) -> Result<Vec<CrossImplResult>> {
        let mut results = Vec::new();

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |ext| ext == "vr") {
                match self.run(&path) {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        eprintln!("Error running {}: {}", path.display(), e);
                        if self.config.fail_fast {
                            return Err(e);
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    /// Generate report for results
    pub fn generate_report(&self, results: &[CrossImplResult]) -> Result<PathBuf> {
        fs::create_dir_all(&self.config.report_dir)?;

        let report_path = self.config.report_dir.join("cross_impl_report.json");
        let content = serde_json::to_string_pretty(results)?;

        fs::write(&report_path, content)?;

        // Also generate summary
        let summary_path = self.config.report_dir.join("summary.txt");
        let mut summary = String::new();

        use std::fmt::Write;

        writeln!(summary, "Cross-Implementation Test Summary")?;
        writeln!(summary, "==================================")?;
        writeln!(summary)?;

        let total = results.len();
        let consensus = results.iter().filter(|r| r.consensus).count();
        let divergent = total - consensus;

        writeln!(summary, "Total tests: {}", total)?;
        writeln!(
            summary,
            "Consensus:   {} ({:.1}%)",
            consensus,
            100.0 * consensus as f64 / total as f64
        )?;
        writeln!(
            summary,
            "Divergent:   {} ({:.1}%)",
            divergent,
            100.0 * divergent as f64 / total as f64
        )?;
        writeln!(summary)?;

        if divergent > 0 {
            writeln!(summary, "Divergent Tests:")?;
            for result in results.iter().filter(|r| !r.consensus) {
                writeln!(
                    summary,
                    "  - {} (diverged: {:?})",
                    result.source_path.display(),
                    result.divergent
                )?;
            }
        }

        fs::write(&summary_path, summary)?;

        Ok(report_path)
    }
}

/// Test metadata parsed from source
#[derive(Debug, Clone, Default)]
struct TestMetadata {
    implementations: Vec<String>,
    required_features: Vec<String>,
    platforms: Vec<String>,
    skip_implementations: Vec<String>,
}

/// Truncate string
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

/// Standard implementations preset
pub fn standard_implementations() -> Vec<Implementation> {
    vec![
        Implementation::new("verum-interpreter", "verum-interpret")
            .as_reference()
            .with_version("0.1.0"),
        Implementation::new("verum-aot", "verum-run").with_version("0.1.0"),
        Implementation::new("verum-jit", "verum-jit")
            .with_args(vec!["--jit".to_string()])
            .with_version("0.1.0"),
        Implementation::new("verum-bytecode", "verum-bc").with_version("0.1.0"),
    ]
}

/// Version compatibility testing configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCompatConfig {
    /// Base version directory (contains version subdirectories)
    pub version_dir: PathBuf,
    /// Versions to test against
    pub versions: Vec<String>,
    /// Implementation to test
    pub implementation: String,
    /// Whether to test forward compatibility (new code on old runtime)
    pub forward_compat: bool,
    /// Whether to test backward compatibility (old code on new runtime)
    pub backward_compat: bool,
}

impl Default for VersionCompatConfig {
    fn default() -> Self {
        Self {
            version_dir: PathBuf::from("versions"),
            versions: vec![],
            implementation: "verum-interpreter".to_string(),
            forward_compat: true,
            backward_compat: true,
        }
    }
}

/// Version compatibility test runner
pub struct VersionCompatRunner {
    config: VersionCompatConfig,
    normalizer: Normalizer,
    checker: SemanticEquivalenceChecker,
}

impl VersionCompatRunner {
    /// Create a new version compatibility runner
    pub fn new(config: VersionCompatConfig) -> Self {
        Self {
            config,
            normalizer: Normalizer::new(NormalizationConfig::semantic()),
            checker: SemanticEquivalenceChecker::new(EquivalenceConfig::default()),
        }
    }

    /// Discover available versions
    pub fn discover_versions(&self) -> Result<Vec<String>> {
        let mut versions = Vec::new();

        if self.config.version_dir.exists() {
            for entry in fs::read_dir(&self.config.version_dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.is_dir() {
                    if let Some(name) = path.file_name() {
                        let version = name.to_string_lossy().to_string();
                        // Check if it looks like a version (semver pattern)
                        if version.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                            versions.push(version);
                        }
                    }
                }
            }
        }

        // Sort versions (simple lexicographic, could use semver)
        versions.sort();
        Ok(versions)
    }

    /// Run version compatibility tests
    pub fn run(&self, source_path: &Path) -> Result<VersionCompatResult> {
        let versions = if self.config.versions.is_empty() {
            self.discover_versions()?
        } else {
            self.config.versions.clone()
        };

        if versions.is_empty() {
            bail!("No versions found to test against");
        }

        let mut results = HashMap::new();
        let mut compatibility_matrix = HashMap::new();

        // Run on each version
        for version in &versions {
            let binary_path = self
                .config
                .version_dir
                .join(version)
                .join(&self.config.implementation);

            if !binary_path.exists() {
                continue;
            }

            let impl_ = Implementation::new(
                format!("{}-{}", self.config.implementation, version),
                binary_path,
            )
            .with_version(version.clone());

            match self.run_implementation(&impl_, source_path) {
                Ok(result) => {
                    results.insert(version.clone(), result);
                }
                Err(e) => {
                    eprintln!("Failed to run version {}: {}", version, e);
                }
            }
        }

        // Compare versions against each other
        let version_keys: Vec<_> = results.keys().cloned().collect();
        for i in 0..version_keys.len() {
            for j in (i + 1)..version_keys.len() {
                let v1 = &version_keys[i];
                let v2 = &version_keys[j];

                let r1 = &results[v1];
                let r2 = &results[v2];

                let compat = self.compare_version_results(r1, r2);
                compatibility_matrix.insert((v1.clone(), v2.clone()), compat);
            }
        }

        Ok(VersionCompatResult {
            source_path: source_path.to_path_buf(),
            versions_tested: version_keys,
            results,
            compatibility_matrix,
        })
    }

    fn run_implementation(
        &self,
        impl_: &Implementation,
        source_path: &Path,
    ) -> Result<ImplementationResult> {
        let start = Instant::now();

        let output = Command::new(&impl_.binary_path)
            .arg(source_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("Failed to run {}", impl_.name))?;

        let duration = start.elapsed();

        Ok(ImplementationResult {
            implementation: impl_.name.clone(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code(),
            duration,
            success: output.status.success(),
            memory_bytes: 0,
        })
    }

    fn compare_version_results(
        &self,
        r1: &ImplementationResult,
        r2: &ImplementationResult,
    ) -> VersionCompatibility {
        let mut issues = Vec::new();

        // Compare exit codes
        if r1.exit_code != r2.exit_code {
            issues.push(CompatIssue::ExitCodeMismatch {
                expected: r1.exit_code,
                actual: r2.exit_code,
            });
        }

        // Normalize and compare outputs
        let out1 = self.normalizer.normalize(&r1.stdout);
        let out2 = self.normalizer.normalize(&r2.stdout);

        match self.checker.check(&out1, &out2) {
            EquivalenceResult::Equivalent => {}
            EquivalenceResult::Different(diffs) => {
                for diff in diffs {
                    issues.push(CompatIssue::OutputMismatch {
                        location: format!("{}", diff.location),
                        expected: diff.expected.clone(),
                        actual: diff.actual.clone(),
                    });
                }
            }
        }

        VersionCompatibility {
            compatible: issues.is_empty(),
            issues,
        }
    }
}

/// Result of version compatibility testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCompatResult {
    pub source_path: PathBuf,
    pub versions_tested: Vec<String>,
    pub results: HashMap<String, ImplementationResult>,
    pub compatibility_matrix: HashMap<(String, String), VersionCompatibility>,
}

/// Compatibility status between two versions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionCompatibility {
    pub compatible: bool,
    pub issues: Vec<CompatIssue>,
}

/// Compatibility issue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompatIssue {
    ExitCodeMismatch {
        expected: Option<i32>,
        actual: Option<i32>,
    },
    OutputMismatch {
        location: String,
        expected: String,
        actual: String,
    },
    FeatureNotSupported {
        feature: String,
        version: String,
    },
    BehaviorChange {
        description: String,
    },
}

/// Bootstrap testing - compare Rust implementation with self-hosted
pub struct BootstrapRunner {
    /// Path to Rust bootstrap compiler
    rust_bootstrap: PathBuf,
    /// Path to self-hosted compiler
    self_hosted: PathBuf,
    normalizer: Normalizer,
    checker: SemanticEquivalenceChecker,
}

impl BootstrapRunner {
    /// Create a new bootstrap runner
    pub fn new(rust_bootstrap: PathBuf, self_hosted: PathBuf) -> Self {
        Self {
            rust_bootstrap,
            self_hosted,
            normalizer: Normalizer::new(NormalizationConfig::semantic()),
            checker: SemanticEquivalenceChecker::new(EquivalenceConfig::default()),
        }
    }

    /// Run bootstrap comparison on a source file
    pub fn compare(&self, source_path: &Path) -> Result<BootstrapResult> {
        let start = Instant::now();

        // Run Rust bootstrap
        let rust_output = Command::new(&self.rust_bootstrap)
            .arg(source_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("Failed to run Rust bootstrap")?;

        // Run self-hosted
        let self_output = Command::new(&self.self_hosted)
            .arg(source_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .context("Failed to run self-hosted compiler")?;

        let duration = start.elapsed();

        // Compare
        let rust_stdout = self
            .normalizer
            .normalize(&String::from_utf8_lossy(&rust_output.stdout));
        let self_stdout = self
            .normalizer
            .normalize(&String::from_utf8_lossy(&self_output.stdout));

        let equiv = self.checker.check(&rust_stdout, &self_stdout);

        Ok(BootstrapResult {
            source_path: source_path.to_path_buf(),
            rust_exit_code: rust_output.status.code(),
            self_exit_code: self_output.status.code(),
            rust_stdout: String::from_utf8_lossy(&rust_output.stdout).to_string(),
            self_stdout: String::from_utf8_lossy(&self_output.stdout).to_string(),
            equivalent: equiv.is_equivalent(),
            duration,
        })
    }
}

/// Result of bootstrap comparison
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapResult {
    pub source_path: PathBuf,
    pub rust_exit_code: Option<i32>,
    pub self_exit_code: Option<i32>,
    pub rust_stdout: String,
    pub self_stdout: String,
    pub equivalent: bool,
    pub duration: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_implementation_features() {
        let impl_ = Implementation::new("test", "/bin/test").with_version("1.0.0");

        assert!(impl_.supports("core"));
        assert!(!impl_.supports("advanced"));
    }

    #[test]
    fn test_implementation_platform() {
        let impl_ = Implementation::new("test", "/bin/test");

        assert!(impl_.supports_platform("linux"));
        assert!(impl_.supports_platform("macos"));
    }

    #[test]
    fn test_config_builder() {
        let config = CrossImplConfig::default()
            .with_reference("interpreter", "verum-interpret")
            .with_alternative("aot", "verum-run");

        assert_eq!(config.implementations.len(), 2);
        assert!(config.reference().is_some());
    }

    #[test]
    fn test_comparison_result() {
        let comparison = ImplComparison {
            implementation: "test".to_string(),
            equivalent: false,
            differences: vec!["exit code mismatch".to_string()],
        };

        assert!(!comparison.equivalent);
        assert_eq!(comparison.differences.len(), 1);
    }
}
