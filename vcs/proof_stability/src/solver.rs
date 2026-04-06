//! Deterministic SMT solver invocation.
//!
//! This module provides infrastructure for invoking SMT solvers in a
//! deterministic manner with controlled random seeds, enabling
//! reproducible proof stability testing.

use crate::{ProofOutcome, StabilityError, config::SolverConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use verum_common::{List, Text};

/// Output from a solver invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverOutput {
    /// Outcome of the proof attempt
    pub outcome: ProofOutcome,
    /// Raw stdout from solver
    pub stdout: Text,
    /// Raw stderr from solver
    pub stderr: Text,
    /// Execution duration
    pub duration: Duration,
    /// Exit code
    pub exit_code: Option<i32>,
    /// Whether it timed out
    pub timed_out: bool,
    /// Random seed used
    pub seed: u64,
    /// Solver statistics (if available)
    pub statistics: HashMap<Text, Text>,
}

impl SolverOutput {
    /// Check if the proof verified successfully.
    pub fn is_verified(&self) -> bool {
        self.outcome.is_verified()
    }
}

/// A single solver invocation configuration.
#[derive(Debug, Clone)]
pub struct SolverInvocation {
    /// Solver name (z3, cvc5, etc.)
    pub solver: Text,
    /// Path to solver binary
    pub solver_path: PathBuf,
    /// Solver version (for recording)
    pub solver_version: Text,
    /// Random seed for deterministic solving
    pub seed: u64,
    /// Timeout in milliseconds
    pub timeout_ms: u64,
    /// Additional command-line arguments
    pub extra_args: List<Text>,
    /// Additional environment variables
    pub env: HashMap<Text, Text>,
}

impl SolverInvocation {
    /// Create a new Z3 invocation with deterministic settings.
    pub fn z3_deterministic(seed: u64, timeout_ms: u64) -> Self {
        Self {
            solver: "z3".to_string().into(),
            solver_path: PathBuf::from("z3"),
            solver_version: "unknown".to_string().into(),
            seed,
            timeout_ms,
            extra_args: vec![
                "-smt2".to_string().into(),
                "-in".to_string().into(),
                format!("sat.random_seed={}", seed).into(),
                format!("smt.random_seed={}", seed).into(),
                "smt.arith.random_initial_value=false".to_string().into(),
            ].into(),
            env: HashMap::new(),
        }
    }

    /// Create a new CVC5 invocation with deterministic settings.
    pub fn cvc5_deterministic(seed: u64, timeout_ms: u64) -> Self {
        Self {
            solver: "cvc5".to_string().into(),
            solver_path: PathBuf::from("cvc5"),
            solver_version: "unknown".to_string().into(),
            seed,
            timeout_ms,
            extra_args: vec![
                "--lang=smt2".to_string().into(),
                format!("--seed={}", seed).into(),
                "--reproducible".to_string().into(),
            ].into(),
            env: HashMap::new(),
        }
    }

    /// Get command-line arguments for the solver.
    fn args(&self) -> List<Text> {
        let mut args = self.extra_args.clone();

        // Add timeout (solver-specific)
        if self.solver.as_str() == "z3" {
            args.push(format!("-T:{}", self.timeout_ms / 1000).into());
        } else if self.solver.as_str() == "cvc5" {
            args.push(format!("--tlimit={}", self.timeout_ms).into());
        }

        args
    }
}

/// Deterministic solver wrapper for reproducible proofs.
pub struct DeterministicSolver {
    config: SolverConfig,
}

impl DeterministicSolver {
    /// Create a new deterministic solver from configuration.
    pub fn new(config: SolverConfig) -> Self {
        Self { config }
    }

    /// Create an invocation for the default solver.
    pub fn create_invocation(&self, seed: u64) -> SolverInvocation {
        self.create_invocation_for(&self.config.default_solver, seed)
    }

    /// Create an invocation for a specific solver.
    pub fn create_invocation_for(&self, solver: &str, seed: u64) -> SolverInvocation {
        let timeout_ms = self.config.default_timeout_ms;

        match solver.to_lowercase().as_str() {
            "z3" => {
                let mut inv = SolverInvocation::z3_deterministic(seed, timeout_ms);
                if let Some(path) = &self.config.z3_path {
                    inv.solver_path = path.clone();
                }
                // Apply custom options
                for (key, value) in &self.config.options {
                    inv.extra_args.push(format!("{}={}", key, value).into());
                }
                inv
            }
            "cvc5" => {
                let mut inv = SolverInvocation::cvc5_deterministic(seed, timeout_ms);
                if let Some(path) = &self.config.cvc5_path {
                    inv.solver_path = path.clone();
                }
                inv
            }
            _ => SolverInvocation::z3_deterministic(seed, timeout_ms),
        }
    }

    /// Run a proof with the solver.
    pub async fn run(
        &self,
        formula: &str,
        invocation: &SolverInvocation,
    ) -> Result<SolverOutput, StabilityError> {
        let start = Instant::now();
        let args = invocation.args();

        let mut cmd = Command::new(&invocation.solver_path);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Set environment variables
        for (key, value) in &invocation.env {
            cmd.env(key, value);
        }

        let mut child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StabilityError::SolverError(format!(
                    "Solver not found: {}",
                    invocation.solver_path.display()
                ).into())
            } else {
                StabilityError::SolverError(e.to_string().into())
            }
        })?;

        // Write formula to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(formula.as_bytes()).await?;
            drop(stdin);
        }

        // Wait with timeout
        let timeout_duration = Duration::from_millis(invocation.timeout_ms);
        match timeout(timeout_duration, child.wait_with_output()).await {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                Ok(SolverOutput {
                    outcome: parse_solver_output(&stdout, &stderr, &invocation.solver),
                    stdout: stdout.into(),
                    stderr: stderr.into(),
                    duration: start.elapsed(),
                    exit_code: output.status.code(),
                    timed_out: false,
                    seed: invocation.seed,
                    statistics: HashMap::new(),
                })
            }
            Ok(Err(e)) => Err(StabilityError::SolverError(e.to_string().into())),
            Err(_) => {
                // Timeout - process is killed automatically when child is dropped
                Ok(SolverOutput {
                    outcome: ProofOutcome::Timeout {
                        timeout_ms: invocation.timeout_ms,
                    },
                    stdout: String::new().into(),
                    stderr: String::new().into(),
                    duration: start.elapsed(),
                    exit_code: None,
                    timed_out: true,
                    seed: invocation.seed,
                    statistics: HashMap::new(),
                })
            }
        }
    }

    /// Run the same proof multiple times with different seeds.
    pub async fn run_stability_test(
        &self,
        formula: &str,
        seeds: &[u64],
    ) -> Result<List<SolverOutput>, StabilityError> {
        let mut outputs = List::new();

        for &seed in seeds {
            let invocation = self.create_invocation(seed);
            let output = self.run(formula, &invocation).await?;
            outputs.push(output);
        }

        Ok(outputs)
    }

    /// Get solver version.
    pub async fn get_version(&self, solver: &str) -> Result<Text, StabilityError> {
        let path = match solver.to_lowercase().as_str() {
            "z3" => self
                .config
                .z3_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("z3")),
            "cvc5" => self
                .config
                .cvc5_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("cvc5")),
            _ => {
                return Err(StabilityError::SolverError(format!(
                    "Unknown solver: {}",
                    solver
                ).into()));
            }
        };

        let output = Command::new(&path)
            .arg("--version")
            .output()
            .await
            .map_err(|e| StabilityError::SolverError(e.to_string().into()))?;

        let version = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("unknown")
            .to_string();

        Ok(version.into())
    }
}

/// Parse solver output to determine proof outcome.
fn parse_solver_output(stdout: &str, stderr: &str, _solver: &str) -> ProofOutcome {
    let stdout_lower = stdout.to_lowercase();
    let stderr_lower = stderr.to_lowercase();

    // Check for verification success (unsat for verification conditions)
    if stdout_lower.contains("unsat") && !stdout_lower.contains("unknown") {
        return ProofOutcome::Verified;
    }

    // Check for failure (sat means counterexample found)
    if stdout_lower.contains("sat") && !stdout_lower.contains("unsat") {
        // Try to extract counterexample
        let counterexample = if stdout.contains("(model") || stdout.contains("(define-fun") {
            // Simple model extraction
            let model_start = stdout.find("(model").or_else(|| stdout.find("(define-fun"));
            model_start.map(|start| stdout[start..].to_string().into())
        } else {
            None
        };

        return ProofOutcome::Failed { counterexample };
    }

    // Check for unknown
    if stdout_lower.contains("unknown") {
        let reason = if stderr_lower.contains("timeout") {
            Some("timeout".to_string().into())
        } else if stderr_lower.contains("resource") {
            Some("resource limit".to_string().into())
        } else if stdout.contains(":reason") {
            // Try to extract reason from SMT-LIB output
            stdout
                .lines()
                .find(|l| l.contains(":reason"))
                .map(|l| l.to_string().into())
        } else {
            None
        };

        return ProofOutcome::Unknown { reason };
    }

    // Check for errors
    if stderr_lower.contains("error") || stdout_lower.contains("error") {
        let message = if !stderr.is_empty() {
            stderr.lines().next().unwrap_or("Unknown error").to_string().into()
        } else {
            stdout
                .lines()
                .find(|l| l.to_lowercase().contains("error"))
                .unwrap_or("Unknown error")
                .to_string()
                .into()
        };

        return ProofOutcome::Error { message };
    }

    // Default to unknown
    ProofOutcome::Unknown {
        reason: Some("Could not parse solver output".to_string().into()),
    }
}

/// Check if a solver is available.
pub async fn is_solver_available(solver: &str) -> bool {
    let path = match solver.to_lowercase().as_str() {
        "z3" => "z3",
        "cvc5" => "cvc5",
        _ => return false,
    };

    Command::new(path)
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Generate SMT-LIB preamble with deterministic settings.
pub fn generate_smt_preamble(seed: u64) -> Text {
    format!(
        r#"; Deterministic proof stability preamble
(set-option :random-seed {seed})
(set-option :produce-models true)
(set-option :produce-unsat-cores false)
"#
    ).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_z3_invocation_args() {
        let inv = SolverInvocation::z3_deterministic(42, 30000);
        let args = inv.args();

        assert!(args.iter().any(|a| a.as_str() == "-smt2"));
        assert!(args.iter().any(|a| a.contains("random_seed=42")));
    }

    #[test]
    fn test_parse_unsat() {
        let outcome = parse_solver_output("unsat\n", "", "z3");
        assert!(outcome.is_verified());
    }

    #[test]
    fn test_parse_sat() {
        let outcome = parse_solver_output("sat\n(model\n  (define-fun x () Int 5)\n)\n", "", "z3");
        assert!(matches!(outcome, ProofOutcome::Failed { .. }));
    }

    #[test]
    fn test_parse_unknown() {
        let outcome = parse_solver_output("unknown\n", "", "z3");
        assert!(matches!(outcome, ProofOutcome::Unknown { .. }));
    }

    #[test]
    fn test_smt_preamble() {
        let preamble = generate_smt_preamble(42);
        assert!(preamble.contains("random-seed 42"));
    }
}
