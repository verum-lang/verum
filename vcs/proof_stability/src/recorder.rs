//! Proof recording for capturing and replaying successful proofs.
//!
//! The recorder captures all proof attempts including:
//! - The SMT-LIB formula
//! - Solver configuration and seed
//! - Proof result and timing
//! - Counterexamples (if any)

use crate::{
    ProofAttempt, ProofCategory, ProofId, ProofOutcome, StabilityError,
    cache::{ProofCache, compute_formula_hash},
    config::CacheConfig,
    solver::{DeterministicSolver, SolverOutput},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use verum_common::{List, Text};

/// Result of a single proof attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofResult {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category
    pub category: ProofCategory,
    /// Formula hash
    pub formula_hash: Text,
    /// Outcome
    pub outcome: ProofOutcome,
    /// Duration
    pub duration: Duration,
    /// Solver used
    pub solver: Text,
    /// Solver version
    pub solver_version: Text,
    /// Random seed
    pub seed: u64,
    /// Was this replayed from cache?
    pub from_cache: bool,
}

impl ProofResult {
    /// Create from a solver output.
    pub fn from_solver_output(
        proof_id: ProofId,
        category: ProofCategory,
        formula_hash: Text,
        output: &SolverOutput,
        solver: Text,
        solver_version: Text,
    ) -> Self {
        Self {
            proof_id,
            category,
            formula_hash,
            outcome: output.outcome.clone(),
            duration: output.duration,
            solver,
            solver_version,
            seed: output.seed,
            from_cache: false,
        }
    }

    /// Check if the proof verified successfully.
    pub fn is_verified(&self) -> bool {
        self.outcome.is_verified()
    }
}

/// A complete proof recording with formula and all attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofRecording {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category
    pub category: ProofCategory,
    /// Original SMT-LIB formula
    pub formula: Text,
    /// Formula hash for deduplication
    pub formula_hash: Text,
    /// All recorded attempts
    pub attempts: List<ProofResult>,
    /// Artifact path (if stored)
    pub artifact_path: Option<PathBuf>,
    /// Metadata
    pub metadata: HashMap<Text, Text>,
}

impl ProofRecording {
    /// Create a new recording.
    pub fn new(proof_id: ProofId, category: ProofCategory, formula: Text) -> Self {
        Self {
            proof_id,
            category,
            formula_hash: compute_formula_hash(&formula),
            formula,
            attempts: List::new(),
            artifact_path: None,
            metadata: HashMap::new(),
        }
    }

    /// Add an attempt to the recording.
    pub fn add_attempt(&mut self, result: ProofResult) {
        self.attempts.push(result);
    }

    /// Get the most common outcome.
    pub fn dominant_outcome(&self) -> Option<&ProofOutcome> {
        if self.attempts.is_empty() {
            return None;
        }

        // Count outcomes
        let mut verified = 0;
        let mut failed = 0;
        let mut unknown = 0;
        let mut timeout = 0;
        let mut error = 0;

        for attempt in &self.attempts {
            match &attempt.outcome {
                ProofOutcome::Verified => verified += 1,
                ProofOutcome::Failed { .. } => failed += 1,
                ProofOutcome::Unknown { .. } => unknown += 1,
                ProofOutcome::Timeout { .. } => timeout += 1,
                ProofOutcome::Error { .. } => error += 1,
            }
        }

        let max = verified.max(failed).max(unknown).max(timeout).max(error);
        self.attempts
            .iter()
            .find(|a| match &a.outcome {
                ProofOutcome::Verified => verified == max,
                ProofOutcome::Failed { .. } => failed == max,
                ProofOutcome::Unknown { .. } => unknown == max,
                ProofOutcome::Timeout { .. } => timeout == max,
                ProofOutcome::Error { .. } => error == max,
            })
            .map(|a| &a.outcome)
    }

    /// Calculate stability percentage.
    pub fn stability_percentage(&self) -> f64 {
        if self.attempts.len() < 2 {
            return 0.0;
        }

        // Count how many match the dominant outcome
        let dominant = self.dominant_outcome();
        if dominant.is_none() {
            return 0.0;
        }

        let matches = self
            .attempts
            .iter()
            .filter(|a| a.outcome.matches(dominant.unwrap()))
            .count();

        (matches as f64 / self.attempts.len() as f64) * 100.0
    }

    /// Get average duration.
    pub fn average_duration(&self) -> Duration {
        if self.attempts.is_empty() {
            return Duration::ZERO;
        }

        let total: Duration = self.attempts.iter().map(|a| a.duration).sum();
        total / self.attempts.len() as u32
    }
}

/// Proof recorder that captures and stores proof attempts.
pub struct ProofRecorder {
    /// Cache for storing recordings
    cache: ProofCache,
    /// Active recordings (not yet committed to cache)
    active_recordings: HashMap<Text, ProofRecording>,
    /// Solver wrapper
    solver: DeterministicSolver,
    /// Solver version
    solver_version: Text,
}

impl ProofRecorder {
    /// Create a new proof recorder.
    pub fn new(cache_config: CacheConfig, solver_config: crate::config::SolverConfig) -> Self {
        Self {
            cache: ProofCache::new(cache_config),
            active_recordings: HashMap::new(),
            solver: DeterministicSolver::new(solver_config),
            solver_version: "unknown".to_string().into(),
        }
    }

    /// Initialize the recorder (load cache, get solver version).
    pub async fn initialize(&mut self) -> Result<(), StabilityError> {
        self.cache.load()?;
        match self.solver.get_version("z3").await {
            Ok(version) => self.solver_version = version,
            Err(_) => {} // Keep default
        }
        Ok(())
    }

    /// Start a new recording for a proof.
    pub fn start_recording(
        &mut self,
        proof_id: ProofId,
        category: ProofCategory,
        formula: Text,
    ) -> Text {
        let recording = ProofRecording::new(proof_id, category, formula);
        let key = recording.formula_hash.clone();
        self.active_recordings.insert(key.clone(), recording);
        key
    }

    /// Record a proof attempt (run solver and store result).
    pub async fn record_attempt(
        &mut self,
        recording_key: &str,
        seed: u64,
    ) -> Result<ProofResult, StabilityError> {
        let recording_key_text: Text = recording_key.to_string().into();
        let recording = self
            .active_recordings
            .get(&recording_key_text)
            .ok_or_else(|| StabilityError::RecordingError("Recording not found".into()))?;

        let invocation = self.solver.create_invocation(seed);
        let output = self.solver.run(&recording.formula, &invocation).await?;

        let result = ProofResult::from_solver_output(
            recording.proof_id.clone(),
            recording.category,
            recording.formula_hash.clone(),
            &output,
            invocation.solver.clone(),
            self.solver_version.clone(),
        );

        // Update recording
        if let Some(rec) = self.active_recordings.get_mut(&recording_key_text) {
            rec.add_attempt(result.clone());
        }

        Ok(result)
    }

    /// Record multiple attempts with different seeds.
    pub async fn record_stability_test(
        &mut self,
        recording_key: &str,
        seeds: &[u64],
    ) -> Result<List<ProofResult>, StabilityError> {
        let mut results = List::new();

        for &seed in seeds {
            let result = self.record_attempt(recording_key, seed).await?;
            results.push(result);
        }

        Ok(results)
    }

    /// Finish a recording and commit to cache.
    pub fn finish_recording(
        &mut self,
        recording_key: &str,
    ) -> Result<ProofRecording, StabilityError> {
        let recording_key_text: Text = recording_key.to_string().into();
        let recording = self
            .active_recordings
            .remove(&recording_key_text)
            .ok_or_else(|| StabilityError::RecordingError("Recording not found".into()))?;

        // Add to cache
        for result in &recording.attempts {
            let attempt = ProofAttempt {
                proof_id: recording.proof_id.clone(),
                category: recording.category,
                seed: result.seed,
                solver: result.solver.clone(),
                solver_version: result.solver_version.clone(),
                outcome: result.outcome.clone(),
                duration: result.duration,
                timestamp: Utc::now(),
                metadata: HashMap::new(),
            };

            self.cache.insert_attempt(
                recording.proof_id.clone(),
                recording.category,
                recording.formula_hash.clone(),
                &attempt,
            )?;
        }

        Ok(recording)
    }

    /// Replay a proof from cache (verify cached result still holds).
    pub async fn replay_proof(
        &mut self,
        proof_id: &ProofId,
        formula_hash: &str,
        seed: Option<u64>,
    ) -> Result<ProofResult, StabilityError> {
        // Load formula from cache
        let formula = self.cache.load_artifact(proof_id, formula_hash)?;
        let category = self
            .cache
            .get(proof_id, formula_hash)
            .map(|e| e.category)
            .unwrap_or(ProofCategory::Mixed);

        let recording_key = self.start_recording(proof_id.clone(), category, formula);

        let effective_seed = seed.unwrap_or(42);
        let result = self.record_attempt(&recording_key, effective_seed).await?;

        let _ = self.finish_recording(&recording_key);

        Ok(result)
    }

    /// Get the cache for inspection.
    pub fn cache(&self) -> &ProofCache {
        &self.cache
    }

    /// Get mutable cache reference.
    pub fn cache_mut(&mut self) -> &mut ProofCache {
        &mut self.cache
    }

    /// Save the cache to disk.
    pub fn save(&self) -> Result<(), StabilityError> {
        self.cache.save()
    }
}

/// Utility to create a one-shot recording.
pub async fn record_single_proof(
    proof_id: ProofId,
    category: ProofCategory,
    formula: &str,
    solver_config: crate::config::SolverConfig,
    seed: u64,
) -> Result<ProofResult, StabilityError> {
    let solver = DeterministicSolver::new(solver_config);
    let invocation = solver.create_invocation(seed);
    let output = solver.run(formula, &invocation).await?;

    Ok(ProofResult::from_solver_output(
        proof_id,
        category,
        compute_formula_hash(formula),
        &output,
        invocation.solver,
        "unknown".to_string().into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proof_recording_stability() {
        let proof_id = ProofId::new("test.vr".into(), "main".into(), 10, "x > 0".into());
        let mut recording = ProofRecording::new(
            proof_id.clone(),
            ProofCategory::Arithmetic,
            "(assert (> x 0))".into(),
        );

        // Add consistent results
        for i in 0..5 {
            recording.add_attempt(ProofResult {
                proof_id: proof_id.clone(),
                category: ProofCategory::Arithmetic,
                formula_hash: "abc".into(),
                outcome: ProofOutcome::Verified,
                duration: Duration::from_millis(100),
                solver: "z3".into(),
                solver_version: "4.12".into(),
                seed: i as u64,
                from_cache: false,
            });
        }

        assert_eq!(recording.stability_percentage(), 100.0);
    }

    #[test]
    fn test_proof_recording_flaky() {
        let proof_id = ProofId::new("test.vr".into(), "main".into(), 10, "x > 0".into());
        let mut recording = ProofRecording::new(
            proof_id.clone(),
            ProofCategory::Quantifier,
            "(assert (forall x ...))".into(),
        );

        // Add inconsistent results
        recording.add_attempt(ProofResult {
            proof_id: proof_id.clone(),
            category: ProofCategory::Quantifier,
            formula_hash: "abc".into(),
            outcome: ProofOutcome::Verified,
            duration: Duration::from_millis(100),
            solver: "z3".into(),
            solver_version: "4.12".into(),
            seed: 1,
            from_cache: false,
        });

        recording.add_attempt(ProofResult {
            proof_id: proof_id.clone(),
            category: ProofCategory::Quantifier,
            formula_hash: "abc".into(),
            outcome: ProofOutcome::Timeout { timeout_ms: 30000 },
            duration: Duration::from_millis(30000),
            solver: "z3".into(),
            solver_version: "4.12".into(),
            seed: 2,
            from_cache: false,
        });

        assert!(recording.stability_percentage() < 100.0);
    }
}
