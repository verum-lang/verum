//! VCS Proof Stability Testing Infrastructure
//!
//! This crate provides infrastructure for testing the stability of SMT proofs
//! in the Verum verification system. It implements:
//!
//! - Deterministic SMT solver invocation with seed control
//! - Proof recording, caching, and replay
//! - Stability metrics and flaky proof detection
//! - Proof regression detection
//!
//! # Architecture
//!
//! ```text
//! +------------------------------------------------------------------+
//! |                    PROOF STABILITY ARCHITECTURE                   |
//! +------------------------------------------------------------------+
//! |                                                                    |
//! |    CLI Layer (main.rs)                                            |
//! |         |                                                          |
//! |         v                                                          |
//! |    +------------------------------------------------------+       |
//! |    |              ProofStabilityRunner                     |       |
//! |    |  - Orchestrates proof discovery and execution        |       |
//! |    |  - Manages parallel execution with seed control       |       |
//! |    |  - Coordinates stability analysis                     |       |
//! |    +---------------------------+----------------------------+       |
//! |                                |                                   |
//! |         +----------+-----------+----------+-----------+            |
//! |         |          |           |          |           |            |
//! |         v          v           v          v           v            |
//! |    +--------+ +--------+ +---------+ +---------+ +--------+       |
//! |    | Config | | Cache  | | Metrics | | Recorder| | Report |       |
//! |    +--------+ +--------+ +---------+ +---------+ +--------+       |
//! |                                                                    |
//! +------------------------------------------------------------------+
//! ```
//!
//! # Test Categories
//!
//! - **Arithmetic proofs**: Linear and non-linear arithmetic (stable)
//! - **Quantifier proofs**: Forall/exists reasoning (potentially unstable)
//! - **Array proofs**: Memory and array theory (memory-related)
//! - **Recursive proofs**: Termination and well-foundedness
//! - **Timeout proofs**: Tests that push solver limits
//!
//! # Example Usage
//!
//! ```rust,ignore
//! use proof_stability::{ProofStabilityRunner, StabilityConfig};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = StabilityConfig::default();
//!     let runner = ProofStabilityRunner::new(config);
//!
//!     let results = runner.run_stability_tests("specs/**/*.vr").await.unwrap();
//!     println!("Stable: {}%, Flaky: {}", results.stability_percentage, results.flaky_count);
//! }
//! ```

// VCS proof stability infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(clippy::pedantic)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]

pub mod cache;
pub mod config;
pub mod metrics;
pub mod recorder;
pub mod regression;
pub mod report;
pub mod runner;
pub mod solver;
pub mod vtest_integration;

pub use cache::{ProofCache, ProofCacheEntry};
pub use config::{SolverConfig, StabilityConfig, StabilityThresholds};
pub use metrics::{FlakyProofInfo, ProofMetrics, StabilityMetrics};
pub use recorder::{ProofRecorder, ProofRecording, ProofResult};
pub use regression::{ProofRegression, RegressionDetector, RegressionReport};
pub use report::{StabilityReport, StabilityReportFormat};
pub use runner::{ProofStabilityRunner, StabilityRunSummary};
pub use solver::{DeterministicSolver, SolverInvocation, SolverOutput};
pub use vtest_integration::{StabilityDirective, VTestIntegrationConfig, VTestStabilitySummary};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;
use verum_common::Text;

/// Error type for proof stability operations.
#[derive(Debug, Error)]
pub enum StabilityError {
    #[error("Cache error: {0}")]
    CacheError(Text),

    #[error("Solver error: {0}")]
    SolverError(Text),

    #[error("Recording error: {0}")]
    RecordingError(Text),

    #[error("Regression error: {0}")]
    RegressionError(Text),

    #[error("Configuration error: {0}")]
    ConfigError(Text),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(Text),

    #[error("Timeout after {0}ms")]
    Timeout(u64),
}

/// Unique identifier for a proof obligation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProofId {
    /// Unique UUID for this proof
    pub id: Uuid,
    /// Source file path
    pub source_path: Text,
    /// Function or scope name
    pub scope: Text,
    /// Line number in source
    pub line: usize,
    /// Proof obligation description
    pub description: Text,
}

impl ProofId {
    /// Create a new proof ID.
    pub fn new(source_path: Text, scope: Text, line: usize, description: Text) -> Self {
        Self {
            id: Uuid::new_v4(),
            source_path,
            scope,
            line,
            description,
        }
    }

    /// Create a deterministic proof ID based on content hash.
    pub fn deterministic(source_path: &str, scope: &str, line: usize, formula: &str) -> Self {
        // Create a deterministic UUID from the content hash
        let mut hasher = Sha256::new();
        hasher.update(source_path.as_bytes());
        hasher.update(scope.as_bytes());
        hasher.update(line.to_string().as_bytes());
        hasher.update(formula.as_bytes());
        let hash = hasher.finalize();

        // Use first 16 bytes of SHA256 as UUID bytes
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&hash[..16]);
        let id = Uuid::from_bytes(bytes);

        Self {
            id,
            source_path: source_path.to_string().into(),
            scope: scope.to_string().into(),
            line,
            description: format!("Proof at {}:{}:{}", source_path, scope, line).into(),
        }
    }
}

/// Category of proof for stability analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProofCategory {
    /// Linear and non-linear arithmetic (typically stable)
    Arithmetic,
    /// Quantifier reasoning (potentially unstable)
    Quantifier,
    /// Array and memory theory
    Array,
    /// Recursive function and termination proofs
    Recursive,
    /// Bit-vector operations
    BitVector,
    /// String operations
    String,
    /// Mixed or unknown category
    Mixed,
}

impl ProofCategory {
    /// Get expected stability for this category.
    pub fn expected_stability(&self) -> f64 {
        match self {
            Self::Arithmetic => 0.99, // Very stable
            Self::BitVector => 0.98,  // Very stable
            Self::Array => 0.95,      // Generally stable
            Self::String => 0.90,     // Moderately stable
            Self::Recursive => 0.85,  // Can be unstable
            Self::Quantifier => 0.80, // Often unstable
            Self::Mixed => 0.85,      // Unknown, assume moderate
        }
    }

    /// Parse category from string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "arithmetic" | "arith" => Some(Self::Arithmetic),
            "quantifier" | "quant" => Some(Self::Quantifier),
            "array" | "memory" => Some(Self::Array),
            "recursive" | "termination" => Some(Self::Recursive),
            "bitvector" | "bv" => Some(Self::BitVector),
            "string" | "str" => Some(Self::String),
            "mixed" | "unknown" => Some(Self::Mixed),
            _ => None,
        }
    }
}

impl std::fmt::Display for ProofCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Arithmetic => "arithmetic",
            Self::Quantifier => "quantifier",
            Self::Array => "array",
            Self::Recursive => "recursive",
            Self::BitVector => "bitvector",
            Self::String => "string",
            Self::Mixed => "mixed",
        };
        write!(f, "{}", s)
    }
}

/// Outcome of a proof attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofOutcome {
    /// Proof verified successfully
    Verified,
    /// Proof failed (counterexample found)
    Failed { counterexample: Option<Text> },
    /// Solver returned unknown
    Unknown { reason: Option<Text> },
    /// Solver timed out
    Timeout { timeout_ms: u64 },
    /// Solver crashed or errored
    Error { message: Text },
}

impl ProofOutcome {
    /// Check if this is a successful verification.
    pub fn is_verified(&self) -> bool {
        matches!(self, Self::Verified)
    }

    /// Check if this outcome matches another (for stability comparison).
    pub fn matches(&self, other: &Self) -> bool {
        use ProofOutcome::*;
        match (self, other) {
            (Verified, Verified) => true,
            (Failed { .. }, Failed { .. }) => true,
            (Unknown { .. }, Unknown { .. }) => true,
            (Timeout { .. }, Timeout { .. }) => true,
            (Error { .. }, Error { .. }) => true,
            _ => false,
        }
    }
}

/// A single proof attempt with all metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofAttempt {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category of proof
    pub category: ProofCategory,
    /// Random seed used
    pub seed: u64,
    /// Solver used
    pub solver: Text,
    /// Solver version
    pub solver_version: Text,
    /// Outcome of the attempt
    pub outcome: ProofOutcome,
    /// Duration of the attempt
    pub duration: Duration,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Additional metadata
    pub metadata: std::collections::HashMap<Text, Text>,
}

impl ProofAttempt {
    /// Create a fingerprint for this proof attempt (excluding variable parts).
    pub fn fingerprint(&self) -> Text {
        let mut hasher = Sha256::new();
        hasher.update(self.proof_id.source_path.as_bytes());
        hasher.update(self.proof_id.scope.as_bytes());
        hasher.update(self.proof_id.line.to_string().as_bytes());
        hasher.update(self.solver.as_bytes());

        let hash = hasher.finalize();
        hex::encode(hash).into()
    }
}

/// Stability status of a proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StabilityStatus {
    /// Proof is stable (consistent across runs)
    Stable,
    /// Proof is flaky (inconsistent outcomes)
    Flaky,
    /// Proof stability is unknown (insufficient data)
    Unknown,
    /// Proof is unstable due to timeouts
    TimeoutUnstable,
}

impl std::fmt::Display for StabilityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Stable => "stable",
            Self::Flaky => "flaky",
            Self::Unknown => "unknown",
            Self::TimeoutUnstable => "timeout-unstable",
        };
        write!(f, "{}", s)
    }
}

/// Helper for hex encoding
mod hex {
    use verum_common::Text;
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proof_id_deterministic() {
        let id1 = ProofId::deterministic("test.vr", "main", 10, "x > 0");
        let id2 = ProofId::deterministic("test.vr", "main", 10, "x > 0");
        assert_eq!(id1.id, id2.id);

        let id3 = ProofId::deterministic("test.vr", "main", 11, "x > 0");
        assert_ne!(id1.id, id3.id);
    }

    #[test]
    fn test_proof_category_stability() {
        assert!(
            ProofCategory::Arithmetic.expected_stability()
                > ProofCategory::Quantifier.expected_stability()
        );
    }

    #[test]
    fn test_proof_outcome_matches() {
        assert!(ProofOutcome::Verified.matches(&ProofOutcome::Verified));
        assert!(!ProofOutcome::Verified.matches(&ProofOutcome::Failed {
            counterexample: None
        }));

        let fail1 = ProofOutcome::Failed {
            counterexample: Some("x=5".into()),
        };
        let fail2 = ProofOutcome::Failed {
            counterexample: Some("x=10".into()),
        };
        assert!(fail1.matches(&fail2)); // Different counterexamples still "match"
    }

    #[test]
    fn test_stability_status_display() {
        assert_eq!(format!("{}", StabilityStatus::Stable), "stable");
        assert_eq!(format!("{}", StabilityStatus::Flaky), "flaky");
    }
}
