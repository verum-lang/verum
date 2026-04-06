//! Proof cache for recording and replaying proof attempts.
//!
//! The cache stores proof results, SMT-LIB files, and metadata to enable:
//! - Fast proof replay without re-running the solver
//! - Historical analysis of proof stability
//! - Regression detection across compiler versions

use crate::{
    ProofAttempt, ProofCategory, ProofId, ProofOutcome, StabilityError, config::CacheConfig,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use verum_common::{List, Text};

/// A cached proof entry with all relevant metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofCacheEntry {
    /// Proof identifier
    pub proof_id: ProofId,
    /// Category of proof
    pub category: ProofCategory,
    /// Hash of the SMT-LIB formula
    pub formula_hash: Text,
    /// All recorded attempts
    pub attempts: List<CachedAttempt>,
    /// First recorded timestamp
    pub first_seen: DateTime<Utc>,
    /// Last recorded timestamp
    pub last_seen: DateTime<Utc>,
    /// Stability status based on cached attempts
    pub stability_status: crate::StabilityStatus,
    /// Computed stability percentage
    pub stability_percentage: f64,
}

/// A single cached proof attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedAttempt {
    /// Solver used
    pub solver: Text,
    /// Solver version
    pub solver_version: Text,
    /// Random seed
    pub seed: u64,
    /// Outcome
    pub outcome: ProofOutcome,
    /// Duration
    pub duration: Duration,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Compiler version (for regression tracking)
    pub compiler_version: Option<Text>,
}

impl From<&ProofAttempt> for CachedAttempt {
    fn from(attempt: &ProofAttempt) -> Self {
        Self {
            solver: attempt.solver.clone(),
            solver_version: attempt.solver_version.clone(),
            seed: attempt.seed,
            outcome: attempt.outcome.clone(),
            duration: attempt.duration,
            timestamp: attempt.timestamp,
            compiler_version: {
                let key: Text = "compiler_version".to_string().into();
                attempt.metadata.get(&key).cloned()
            },
        }
    }
}

/// Proof cache implementation.
pub struct ProofCache {
    config: CacheConfig,
    /// In-memory cache
    entries: HashMap<Text, ProofCacheEntry>,
    /// Whether the cache has been modified
    dirty: bool,
}

impl ProofCache {
    /// Create a new proof cache with the given configuration.
    pub fn new(config: CacheConfig) -> Self {
        Self {
            config,
            entries: HashMap::new(),
            dirty: false,
        }
    }

    /// Load the cache from disk.
    pub fn load(&mut self) -> Result<(), StabilityError> {
        if !self.config.enabled {
            return Ok(());
        }

        let cache_dir = &self.config.cache_dir;
        if !cache_dir.exists() {
            fs::create_dir_all(cache_dir)?;
            return Ok(());
        }

        let index_path = cache_dir.join("index.json");
        if index_path.exists() {
            let content = fs::read_to_string(&index_path)?;
            let entries: Vec<ProofCacheEntry> = serde_json::from_str(&content)
                .map_err(|e| StabilityError::CacheError(e.to_string().into()))?;

            for entry in entries {
                let key = Self::entry_key(&entry.proof_id, &entry.formula_hash);
                self.entries.insert(key, entry);
            }
        }

        Ok(())
    }

    /// Save the cache to disk.
    pub fn save(&self) -> Result<(), StabilityError> {
        if !self.config.enabled || !self.dirty {
            return Ok(());
        }

        let cache_dir = &self.config.cache_dir;
        fs::create_dir_all(cache_dir)?;

        let entries: Vec<&ProofCacheEntry> = self.entries.values().collect();
        let content = serde_json::to_string_pretty(&entries)
            .map_err(|e| StabilityError::SerializationError(e.to_string().into()))?;

        let index_path = cache_dir.join("index.json");
        fs::write(&index_path, content)?;

        Ok(())
    }

    /// Get a cache entry by proof ID and formula hash.
    pub fn get(&self, proof_id: &ProofId, formula_hash: &str) -> Option<&ProofCacheEntry> {
        let key = Self::entry_key(proof_id, formula_hash);
        self.entries.get(&key)
    }

    /// Insert or update a cache entry with a new attempt.
    pub fn insert_attempt(
        &mut self,
        proof_id: ProofId,
        category: ProofCategory,
        formula_hash: Text,
        attempt: &ProofAttempt,
    ) -> Result<(), StabilityError> {
        if !self.config.enabled {
            return Ok(());
        }

        let key = Self::entry_key(&proof_id, &formula_hash);
        let now = Utc::now();
        let cached_attempt = CachedAttempt::from(attempt);

        if let Some(entry) = self.entries.get_mut(&key) {
            entry.attempts.push(cached_attempt);
            entry.last_seen = now;
            // Recompute stability
            self.recompute_stability(&key);
        } else {
            let entry = ProofCacheEntry {
                proof_id,
                category,
                formula_hash: formula_hash.clone(),
                attempts: vec![cached_attempt].into(),
                first_seen: now,
                last_seen: now,
                stability_status: crate::StabilityStatus::Unknown,
                stability_percentage: 0.0,
            };
            self.entries.insert(key, entry);
        }

        self.dirty = true;
        Ok(())
    }

    /// Store an SMT-LIB artifact file.
    pub fn store_artifact(
        &self,
        proof_id: &ProofId,
        formula_hash: &str,
        content: &str,
    ) -> Result<PathBuf, StabilityError> {
        if !self.config.enabled || !self.config.store_artifacts {
            return Err(StabilityError::CacheError(
                "Artifact storage disabled".to_string().into(),
            ));
        }

        let artifacts_dir = self.config.cache_dir.join("artifacts");
        fs::create_dir_all(&artifacts_dir)?;

        let filename = format!("{}_{}.smt2", formula_hash, proof_id.id);
        let path = artifacts_dir.join(&filename);

        let content = if self.config.compression_level > 0 {
            // Simple compression placeholder - would use flate2 in production
            content.to_string()
        } else {
            content.to_string()
        };

        fs::write(&path, content)?;
        Ok(path)
    }

    /// Load an SMT-LIB artifact file.
    pub fn load_artifact(
        &self,
        proof_id: &ProofId,
        formula_hash: &str,
    ) -> Result<Text, StabilityError> {
        let artifacts_dir = self.config.cache_dir.join("artifacts");
        let filename = format!("{}_{}.smt2", formula_hash, proof_id.id);
        let path = artifacts_dir.join(&filename);

        if !path.exists() {
            return Err(StabilityError::CacheError(format!(
                "Artifact not found: {}",
                path.display()
            ).into()));
        }

        let content = fs::read_to_string(&path)?;
        Ok(content.into())
    }

    /// Get all entries for a specific proof category.
    pub fn get_by_category(&self, category: ProofCategory) -> List<&ProofCacheEntry> {
        self.entries
            .values()
            .filter(|e| e.category == category)
            .collect()
    }

    /// Get all flaky proofs.
    pub fn get_flaky_proofs(&self) -> List<&ProofCacheEntry> {
        self.entries
            .values()
            .filter(|e| e.stability_status == crate::StabilityStatus::Flaky)
            .collect()
    }

    /// Get stability statistics.
    pub fn get_statistics(&self) -> CacheStatistics {
        let total = self.entries.len();
        let mut stable = 0;
        let mut flaky = 0;
        let mut unknown = 0;
        let mut by_category: HashMap<ProofCategory, CategoryStats> = HashMap::new();

        for entry in self.entries.values() {
            match entry.stability_status {
                crate::StabilityStatus::Stable => stable += 1,
                crate::StabilityStatus::Flaky | crate::StabilityStatus::TimeoutUnstable => {
                    flaky += 1
                }
                crate::StabilityStatus::Unknown => unknown += 1,
            }

            let cat_stats = by_category.entry(entry.category).or_default();
            cat_stats.total += 1;
            if entry.stability_status == crate::StabilityStatus::Stable {
                cat_stats.stable += 1;
            } else if entry.stability_status == crate::StabilityStatus::Flaky {
                cat_stats.flaky += 1;
            }
        }

        CacheStatistics {
            total_proofs: total,
            stable_proofs: stable,
            flaky_proofs: flaky,
            unknown_proofs: unknown,
            stability_percentage: if total > 0 {
                (stable as f64 / total as f64) * 100.0
            } else {
                100.0
            },
            by_category,
        }
    }

    /// Clean expired entries based on TTL.
    pub fn clean_expired(&mut self) -> usize {
        if self.config.ttl_seconds == 0 {
            return 0;
        }

        let now = Utc::now();
        let ttl = chrono::Duration::seconds(self.config.ttl_seconds as i64);
        let cutoff = now - ttl;

        let initial_count = self.entries.len();
        self.entries.retain(|_, entry| entry.last_seen > cutoff);
        let removed = initial_count - self.entries.len();

        if removed > 0 {
            self.dirty = true;
        }

        removed
    }

    /// Clear all cached entries.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.dirty = true;
    }

    /// Get cache size in bytes (approximate).
    pub fn size_bytes(&self) -> u64 {
        // Rough estimate based on serialization
        self.entries
            .values()
            .map(|e| {
                // Estimate size per entry
                let attempts_size = e.attempts.len() * 200; // ~200 bytes per attempt
                let metadata_size = 500; // Base metadata
                (attempts_size + metadata_size) as u64
            })
            .sum()
    }

    /// Generate a cache key from proof ID and formula hash.
    fn entry_key(proof_id: &ProofId, formula_hash: &str) -> Text {
        format!(
            "{}:{}:{}",
            proof_id.source_path, proof_id.line, formula_hash
        ).into()
    }

    /// Recompute stability status for an entry.
    fn recompute_stability(&mut self, key: &str) {
        let key_text: Text = key.to_string().into();
        if let Some(entry) = self.entries.get_mut(&key_text) {
            let attempts = &entry.attempts;
            if attempts.len() < 2 {
                entry.stability_status = crate::StabilityStatus::Unknown;
                entry.stability_percentage = 0.0;
                return;
            }

            // Count outcome types
            let mut verified = 0;
            let mut failed = 0;
            let mut timeouts = 0;
            let mut _errors = 0;

            for attempt in attempts {
                match &attempt.outcome {
                    ProofOutcome::Verified => verified += 1,
                    ProofOutcome::Failed { .. } => failed += 1,
                    ProofOutcome::Timeout { .. } => timeouts += 1,
                    ProofOutcome::Unknown { .. } => {}
                    ProofOutcome::Error { .. } => _errors += 1,
                }
            }

            let total = attempts.len();
            let dominant = verified.max(failed);
            let consistency = dominant as f64 / total as f64;

            entry.stability_percentage = consistency * 100.0;

            if timeouts > total / 2 {
                entry.stability_status = crate::StabilityStatus::TimeoutUnstable;
            } else if consistency >= 0.95 {
                entry.stability_status = crate::StabilityStatus::Stable;
            } else if consistency < 0.80 {
                entry.stability_status = crate::StabilityStatus::Flaky;
            } else {
                entry.stability_status = crate::StabilityStatus::Unknown;
            }
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStatistics {
    pub total_proofs: usize,
    pub stable_proofs: usize,
    pub flaky_proofs: usize,
    pub unknown_proofs: usize,
    pub stability_percentage: f64,
    pub by_category: HashMap<ProofCategory, CategoryStats>,
}

/// Per-category statistics.
#[derive(Debug, Clone, Default)]
pub struct CategoryStats {
    pub total: usize,
    pub stable: usize,
    pub flaky: usize,
}

impl CategoryStats {
    pub fn stability_percentage(&self) -> f64 {
        if self.total > 0 {
            (self.stable as f64 / self.total as f64) * 100.0
        } else {
            100.0
        }
    }
}

/// Compute SHA256 hash of SMT-LIB formula content.
pub fn compute_formula_hash(formula: &str) -> Text {
    let mut hasher = Sha256::new();
    hasher.update(formula.as_bytes());
    let hash = hasher.finalize();
    crate::hex::encode(hash).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_config() -> CacheConfig {
        CacheConfig {
            enabled: true,
            cache_dir: tempdir().unwrap().keep(),
            max_size_mb: 100,
            ttl_seconds: 0,
            store_artifacts: true,
            store_counterexamples: true,
            compression_level: 0,
        }
    }

    #[test]
    fn test_cache_insert_and_get() {
        let mut cache = ProofCache::new(test_config());

        let proof_id = ProofId::new(
            "test.vr".to_string().into(),
            "main".to_string().into(),
            10,
            "x > 0".to_string().into(),
        );
        let formula_hash: Text = "abc123".to_string().into();

        let attempt = ProofAttempt {
            proof_id: proof_id.clone(),
            category: ProofCategory::Arithmetic,
            seed: 42,
            solver: "z3".to_string().into(),
            solver_version: "4.12.0".to_string().into(),
            outcome: ProofOutcome::Verified,
            duration: Duration::from_millis(100),
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        };

        cache
            .insert_attempt(
                proof_id.clone(),
                ProofCategory::Arithmetic,
                formula_hash.clone(),
                &attempt,
            )
            .unwrap();

        let entry = cache.get(&proof_id, &formula_hash).unwrap();
        assert_eq!(entry.attempts.len(), 1);
        assert!(entry.attempts[0].outcome.is_verified());
    }

    #[test]
    fn test_formula_hash() {
        let hash1 = compute_formula_hash("(assert (> x 0))");
        let hash2 = compute_formula_hash("(assert (> x 0))");
        let hash3 = compute_formula_hash("(assert (> y 0))");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_cache_statistics() {
        let cache = ProofCache::new(test_config());
        let stats = cache.get_statistics();
        assert_eq!(stats.total_proofs, 0);
        assert_eq!(stats.stability_percentage, 100.0);
    }
}
