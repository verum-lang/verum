//! Configurable fuzzing campaigns with comprehensive management
//!
//! Provides structured fuzzing campaigns with different goals:
//! - Exploration: Maximize coverage
//! - Bug hunting: Focus on crash discovery
//! - Differential: Find tier mismatches
//! - Stress: Long-running stability tests
//!
//! # Campaign Structure
//!
//! A campaign consists of:
//! - Target configuration
//! - Duration/iteration limits
//! - Focus areas (components, features)
//! - Success criteria
//!
//! # Advanced Features
//!
//! - **Seed Corpus Management**: Import, merge, minimize seed corpora
//! - **Energy Scheduling**: Prioritize inputs based on coverage potential
//! - **Parallel Coordination**: Multi-worker fuzzing with work stealing
//! - **Checkpoint/Resume**: Persist and restore campaign state
//!
//! # Usage
//!
//! ```rust,ignore
//! use verum_vfuzz::campaign::{CampaignConfig, CampaignRunner};
//!
//! let config = CampaignConfig::exploration()
//!     .with_workers(8)
//!     .with_duration(Duration::from_hours(24));
//!
//! let mut runner = CampaignRunner::new(config);
//! runner.run().await?;
//! ```

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Campaign type determining fuzzing strategy
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CampaignType {
    /// Maximize code coverage
    Exploration,
    /// Focus on finding crashes
    BugHunting,
    /// Compare execution across tiers
    Differential,
    /// Long-running stability test
    Stress,
    /// Regression testing (run known inputs)
    Regression,
    /// Focus on specific component
    Targeted(TargetComponent),
    /// Custom campaign
    Custom(String),
}

/// Target component for focused fuzzing
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TargetComponent {
    /// Lexer
    Lexer,
    /// Parser
    Parser,
    /// Type checker
    TypeChecker,
    /// CBGR memory system
    Cbgr,
    /// SMT verification
    Smt,
    /// Code generator
    Codegen,
    /// Async runtime
    AsyncRuntime,
    /// Standard library
    Stdlib,
}

/// Campaign configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignConfig {
    /// Campaign name
    pub name: String,
    /// Campaign type
    pub campaign_type: CampaignType,
    /// Maximum duration (None = unlimited)
    pub max_duration: Option<Duration>,
    /// Maximum iterations (0 = unlimited)
    pub max_iterations: usize,
    /// Number of parallel workers
    pub workers: usize,
    /// Seed for reproducibility
    pub seed: Option<u64>,
    /// Directory for crash artifacts
    pub crash_dir: PathBuf,
    /// Directory for corpus
    pub corpus_dir: PathBuf,
    /// Directory for seeds
    pub seed_dir: PathBuf,
    /// Generator configuration
    pub generator: GeneratorSettings,
    /// Mutator configuration
    pub mutator: MutatorSettings,
    /// Shrinking configuration
    pub shrinking: ShrinkingSettings,
    /// Coverage configuration
    pub coverage: CoverageSettings,
    /// Oracle configuration (for differential)
    pub oracle: OracleSettings,
    /// Reporting configuration
    pub reporting: ReportingSettings,
    /// Stop conditions
    pub stop_conditions: StopConditions,
}

impl Default for CampaignConfig {
    fn default() -> Self {
        Self {
            name: "default-campaign".to_string(),
            campaign_type: CampaignType::Exploration,
            max_duration: None,
            max_iterations: 0,
            workers: num_cpus(),
            seed: None,
            crash_dir: PathBuf::from("vcs/fuzz/crashes"),
            corpus_dir: PathBuf::from("vcs/fuzz/corpus"),
            seed_dir: PathBuf::from("vcs/fuzz/seeds"),
            generator: GeneratorSettings::default(),
            mutator: MutatorSettings::default(),
            shrinking: ShrinkingSettings::default(),
            coverage: CoverageSettings::default(),
            oracle: OracleSettings::default(),
            reporting: ReportingSettings::default(),
            stop_conditions: StopConditions::default(),
        }
    }
}

impl CampaignConfig {
    /// Create an exploration campaign
    pub fn exploration() -> Self {
        Self {
            name: "exploration".to_string(),
            campaign_type: CampaignType::Exploration,
            generator: GeneratorSettings {
                strategy: GeneratorStrategy::Mixed,
                generation_ratio: 0.3,
                mutation_ratio: 0.7,
                ..Default::default()
            },
            coverage: CoverageSettings {
                enabled: true,
                track_edges: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a bug hunting campaign
    pub fn bug_hunting() -> Self {
        Self {
            name: "bug-hunting".to_string(),
            campaign_type: CampaignType::BugHunting,
            generator: GeneratorSettings {
                strategy: GeneratorStrategy::EdgeCase,
                max_depth: 15,
                max_statements: 100,
                ..Default::default()
            },
            mutator: MutatorSettings {
                aggressive: true,
                max_mutations: 10,
                ..Default::default()
            },
            shrinking: ShrinkingSettings {
                enabled: true,
                max_iterations: 2000,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a differential testing campaign
    pub fn differential() -> Self {
        Self {
            name: "differential".to_string(),
            campaign_type: CampaignType::Differential,
            oracle: OracleSettings {
                enabled: true,
                reference_tier: 0,
                test_tiers: vec![3],
                compare_stdout: true,
                ..Default::default()
            },
            generator: GeneratorSettings {
                strategy: GeneratorStrategy::TypeAware,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a stress testing campaign
    pub fn stress() -> Self {
        Self {
            name: "stress".to_string(),
            campaign_type: CampaignType::Stress,
            max_duration: Some(Duration::from_secs(3600)), // 1 hour
            generator: GeneratorSettings {
                strategy: GeneratorStrategy::EdgeCase,
                max_depth: 20,
                max_statements: 200,
                ..Default::default()
            },
            stop_conditions: StopConditions {
                max_crashes: 0, // Don't stop on crashes
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Create a targeted campaign for a specific component
    pub fn targeted(component: TargetComponent) -> Self {
        let generator = match &component {
            TargetComponent::Lexer => GeneratorSettings {
                strategy: GeneratorStrategy::Lexer,
                ..Default::default()
            },
            TargetComponent::Parser => GeneratorSettings {
                strategy: GeneratorStrategy::Parser,
                ..Default::default()
            },
            TargetComponent::TypeChecker => GeneratorSettings {
                strategy: GeneratorStrategy::TypeAware,
                ..Default::default()
            },
            TargetComponent::Cbgr => GeneratorSettings {
                strategy: GeneratorStrategy::Cbgr,
                include_cbgr: true,
                ..Default::default()
            },
            TargetComponent::Smt => GeneratorSettings {
                strategy: GeneratorStrategy::Refinement,
                include_refinements: true,
                ..Default::default()
            },
            TargetComponent::AsyncRuntime => GeneratorSettings {
                strategy: GeneratorStrategy::Async,
                include_async: true,
                ..Default::default()
            },
            _ => GeneratorSettings::default(),
        };

        Self {
            name: format!("targeted-{:?}", component).to_lowercase(),
            campaign_type: CampaignType::Targeted(component),
            generator,
            ..Default::default()
        }
    }

    /// Load from JSON file
    pub fn load(path: &PathBuf) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Save to JSON file
    pub fn save(&self, path: &PathBuf) -> Result<(), std::io::Error> {
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)
    }
}

/// Generator settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorSettings {
    /// Generation strategy
    pub strategy: GeneratorStrategy,
    /// Maximum AST depth
    pub max_depth: usize,
    /// Maximum statements per function
    pub max_statements: usize,
    /// Maximum functions per program
    pub max_functions: usize,
    /// Maximum type definitions
    pub max_types: usize,
    /// Ratio of generation vs mutation (0.0 - 1.0)
    pub generation_ratio: f64,
    /// Ratio of mutation vs generation
    pub mutation_ratio: f64,
    /// Include async constructs
    pub include_async: bool,
    /// Include CBGR patterns
    pub include_cbgr: bool,
    /// Include refinement types
    pub include_refinements: bool,
    /// Include unsafe blocks
    pub include_unsafe: bool,
}

impl Default for GeneratorSettings {
    fn default() -> Self {
        Self {
            strategy: GeneratorStrategy::Mixed,
            max_depth: 10,
            max_statements: 50,
            max_functions: 10,
            max_types: 5,
            generation_ratio: 0.2,
            mutation_ratio: 0.8,
            include_async: true,
            include_cbgr: true,
            include_refinements: false,
            include_unsafe: false,
        }
    }
}

/// Generator strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeneratorStrategy {
    /// Lexer token focus
    Lexer,
    /// Parser construct focus
    Parser,
    /// Type-aware generation
    TypeAware,
    /// Edge case generation
    EdgeCase,
    /// Refinement types
    Refinement,
    /// Async patterns
    Async,
    /// CBGR patterns
    Cbgr,
    /// Mixed strategy
    Mixed,
}

/// Mutator settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatorSettings {
    /// Mutation rate (0.0 - 1.0)
    pub mutation_rate: f64,
    /// Maximum mutations per input
    pub max_mutations: usize,
    /// Enable aggressive mutations
    pub aggressive: bool,
    /// Enable structure-aware mutations
    pub structure_aware: bool,
    /// Enable token-level mutations
    pub token_level: bool,
    /// Enable byte-level mutations
    pub byte_level: bool,
}

impl Default for MutatorSettings {
    fn default() -> Self {
        Self {
            mutation_rate: 0.8,
            max_mutations: 5,
            aggressive: false,
            structure_aware: true,
            token_level: true,
            byte_level: false,
        }
    }
}

/// Shrinking settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShrinkingSettings {
    /// Enable automatic shrinking
    pub enabled: bool,
    /// Maximum shrink iterations
    pub max_iterations: usize,
    /// Timeout per shrink attempt (ms)
    pub timeout_ms: u64,
    /// Shrinking strategy
    pub strategy: ShrinkingStrategy,
}

impl Default for ShrinkingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_iterations: 1000,
            timeout_ms: 5000,
            strategy: ShrinkingStrategy::Combined,
        }
    }
}

/// Shrinking strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShrinkingStrategy {
    /// Binary search
    BinarySearch,
    /// Delta debugging
    DeltaDebugging,
    /// Line-by-line
    LineByLine,
    /// Token-by-token
    TokenByToken,
    /// Hierarchical (AST-aware)
    Hierarchical,
    /// Combined approach
    Combined,
}

/// Coverage settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageSettings {
    /// Enable coverage tracking
    pub enabled: bool,
    /// Track edge coverage
    pub track_edges: bool,
    /// Track source lines
    pub track_source: bool,
    /// Track call stacks (context sensitivity)
    pub track_context: bool,
    /// Coverage map size
    pub map_size: usize,
}

impl Default for CoverageSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            track_edges: true,
            track_source: false,
            track_context: false,
            map_size: 65536,
        }
    }
}

/// Oracle settings (for differential testing)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OracleSettings {
    /// Enable differential testing
    pub enabled: bool,
    /// Reference tier (usually 0)
    pub reference_tier: usize,
    /// Tiers to test against reference
    pub test_tiers: Vec<usize>,
    /// Compare stdout
    pub compare_stdout: bool,
    /// Float epsilon for comparison
    pub float_epsilon: f64,
    /// Strict error matching
    pub strict_errors: bool,
}

impl Default for OracleSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            reference_tier: 0,
            test_tiers: vec![3],
            compare_stdout: true,
            float_epsilon: 1e-10,
            strict_errors: false,
        }
    }
}

/// Reporting settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingSettings {
    /// Print progress
    pub verbose: bool,
    /// Progress interval (iterations)
    pub progress_interval: usize,
    /// Save interesting inputs
    pub save_interesting: bool,
    /// Generate final report
    pub final_report: bool,
    /// Report format
    pub format: ReportFormat,
}

impl Default for ReportingSettings {
    fn default() -> Self {
        Self {
            verbose: true,
            progress_interval: 1000,
            save_interesting: true,
            final_report: true,
            format: ReportFormat::Text,
        }
    }
}

/// Report format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReportFormat {
    /// Plain text
    Text,
    /// JSON
    Json,
    /// Markdown
    Markdown,
    /// HTML
    Html,
}

/// Stop conditions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopConditions {
    /// Stop after N crashes (0 = don't stop)
    pub max_crashes: usize,
    /// Stop after N unique crashes
    pub max_unique_crashes: usize,
    /// Stop when coverage plateaus
    pub stop_on_plateau: bool,
    /// Plateau detection window (iterations)
    pub plateau_window: usize,
    /// Plateau threshold (new edges)
    pub plateau_threshold: usize,
}

impl Default for StopConditions {
    fn default() -> Self {
        Self {
            max_crashes: 0,
            max_unique_crashes: 0,
            stop_on_plateau: false,
            plateau_window: 10000,
            plateau_threshold: 10,
        }
    }
}

/// Campaign execution state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignState {
    /// Campaign config
    pub config: CampaignConfig,
    /// Start time (unix timestamp)
    pub start_time: u64,
    /// End time (if finished)
    pub end_time: Option<u64>,
    /// Current iteration
    pub iterations: usize,
    /// Crashes found
    pub crashes: usize,
    /// Unique crashes
    pub unique_crashes: usize,
    /// Coverage percentage
    pub coverage_pct: f64,
    /// Corpus size
    pub corpus_size: usize,
    /// Current phase
    pub phase: CampaignPhase,
    /// Stop reason (if stopped)
    pub stop_reason: Option<StopReason>,
}

/// Campaign phase
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CampaignPhase {
    /// Initializing
    Init,
    /// Loading seeds
    LoadingSeeds,
    /// Main fuzzing loop
    Fuzzing,
    /// Shrinking crashes
    Shrinking,
    /// Generating reports
    Reporting,
    /// Finished
    Done,
}

/// Reason for stopping
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StopReason {
    /// Reached maximum iterations
    MaxIterations,
    /// Reached maximum duration
    MaxDuration,
    /// Reached crash limit
    CrashLimit,
    /// Coverage plateau
    CoveragePlateau,
    /// User interrupted
    UserInterrupt,
    /// Error occurred
    Error(String),
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_default_campaign() {
        let config = CampaignConfig::default();
        assert_eq!(config.name, "default-campaign");
        assert_eq!(config.campaign_type, CampaignType::Exploration);
    }

    #[test]
    fn test_exploration_campaign() {
        let config = CampaignConfig::exploration();
        assert_eq!(config.campaign_type, CampaignType::Exploration);
        assert!(config.coverage.enabled);
    }

    #[test]
    fn test_bug_hunting_campaign() {
        let config = CampaignConfig::bug_hunting();
        assert_eq!(config.campaign_type, CampaignType::BugHunting);
        assert!(config.mutator.aggressive);
    }

    #[test]
    fn test_differential_campaign() {
        let config = CampaignConfig::differential();
        assert_eq!(config.campaign_type, CampaignType::Differential);
        assert!(config.oracle.enabled);
    }

    #[test]
    fn test_targeted_campaign() {
        let config = CampaignConfig::targeted(TargetComponent::Lexer);
        assert!(matches!(
            config.campaign_type,
            CampaignType::Targeted(TargetComponent::Lexer)
        ));
    }

    #[test]
    fn test_save_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("campaign.json");

        let config = CampaignConfig::exploration();
        config.save(&path).unwrap();

        let loaded = CampaignConfig::load(&path).unwrap();
        assert_eq!(loaded.name, config.name);
        assert_eq!(loaded.campaign_type, config.campaign_type);
    }
}

// ============================================================================
// Seed Corpus Management
// ============================================================================

/// Manages the seed corpus for fuzzing
pub struct SeedCorpus {
    /// Corpus entries by hash
    entries: RwLock<HashMap<String, SeedEntry>>,
    /// Total size of corpus
    total_size: AtomicUsize,
    /// Directory for persistence
    directory: PathBuf,
    /// Configuration
    config: SeedCorpusConfig,
}

/// Configuration for seed corpus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedCorpusConfig {
    /// Maximum corpus size (entries)
    pub max_entries: usize,
    /// Maximum entry size (bytes)
    pub max_entry_size: usize,
    /// Enable deduplication
    pub deduplicate: bool,
    /// Enable minimization
    pub minimize: bool,
    /// Preferred seed formats
    pub formats: Vec<SeedFormat>,
}

impl Default for SeedCorpusConfig {
    fn default() -> Self {
        Self {
            max_entries: 100000,
            max_entry_size: 1024 * 1024, // 1MB
            deduplicate: true,
            minimize: true,
            formats: vec![SeedFormat::VerumSource, SeedFormat::Raw],
        }
    }
}

/// Seed format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SeedFormat {
    /// Raw Verum source code
    VerumSource,
    /// Pre-tokenized
    Tokenized,
    /// Pre-parsed AST
    Ast,
    /// Raw bytes
    Raw,
}

/// A seed entry in the corpus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedEntry {
    /// Unique hash
    pub hash: String,
    /// Seed content
    pub content: String,
    /// Size in bytes
    pub size: usize,
    /// Source of this seed
    pub source: SeedSource,
    /// Energy for scheduling
    pub energy: f64,
    /// Times selected
    pub selections: usize,
    /// Coverage bits (if computed)
    pub coverage_bits: Option<usize>,
    /// First discovered timestamp
    pub discovered_at: u64,
    /// Last selected timestamp
    pub last_selected: Option<u64>,
    /// Tags for categorization
    pub tags: Vec<String>,
}

/// Source of a seed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SeedSource {
    /// User-provided seed
    UserProvided,
    /// Generated by fuzzer
    Generated,
    /// Found via mutation
    Mutated { parent: String },
    /// From spec test
    SpecTest { path: String },
    /// From crash minimization
    CrashMinimized { crash_id: String },
    /// Imported from another corpus
    Imported { source: String },
}

impl SeedCorpus {
    /// Create a new seed corpus
    pub fn new(directory: PathBuf, config: SeedCorpusConfig) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            total_size: AtomicUsize::new(0),
            directory,
            config,
        }
    }

    /// Add a seed to the corpus
    pub fn add(&self, content: &str, source: SeedSource) -> Result<String, String> {
        if content.len() > self.config.max_entry_size {
            return Err("Seed too large".to_string());
        }

        let hash = Self::hash_content(content);

        // Check for duplicates
        if self.config.deduplicate {
            if let Ok(entries) = self.entries.read() {
                if entries.contains_key(&hash) {
                    return Err("Duplicate seed".to_string());
                }
            }
        }

        // Check capacity
        let current_count = self.entries.read().map(|e| e.len()).unwrap_or(0);
        if current_count >= self.config.max_entries {
            // Evict lowest energy seed
            self.evict_lowest_energy();
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let entry = SeedEntry {
            hash: hash.clone(),
            content: content.to_string(),
            size: content.len(),
            source,
            energy: 1.0,
            selections: 0,
            coverage_bits: None,
            discovered_at: now,
            last_selected: None,
            tags: Vec::new(),
        };

        if let Ok(mut entries) = self.entries.write() {
            entries.insert(hash.clone(), entry);
            self.total_size.fetch_add(content.len(), Ordering::Relaxed);
        }

        Ok(hash)
    }

    /// Get a seed by hash
    pub fn get(&self, hash: &str) -> Option<SeedEntry> {
        self.entries.read().ok()?.get(hash).cloned()
    }

    /// Remove a seed
    pub fn remove(&self, hash: &str) -> Option<SeedEntry> {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.remove(hash) {
                self.total_size.fetch_sub(entry.size, Ordering::Relaxed);
                return Some(entry);
            }
        }
        None
    }

    /// Get corpus size
    pub fn len(&self) -> usize {
        self.entries.read().map(|e| e.len()).unwrap_or(0)
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get total size in bytes
    pub fn total_bytes(&self) -> usize {
        self.total_size.load(Ordering::Relaxed)
    }

    /// Update energy for a seed
    pub fn update_energy(&self, hash: &str, delta: f64) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.get_mut(hash) {
                entry.energy = (entry.energy + delta).max(0.1);
            }
        }
    }

    /// Mark seed as selected
    pub fn mark_selected(&self, hash: &str) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some(entry) = entries.get_mut(hash) {
                entry.selections += 1;
                entry.last_selected = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                );
                // Energy decay
                entry.energy *= 0.99;
            }
        }
    }

    /// Load seeds from directory
    pub fn load_from_directory(&self, path: &PathBuf) -> Result<usize, std::io::Error> {
        let mut count = 0;

        if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let path = entry.path();

                if path.extension().map(|e| e == "vr").unwrap_or(false) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let source = SeedSource::Imported {
                            source: path.to_string_lossy().to_string(),
                        };
                        if self.add(&content, source).is_ok() {
                            count += 1;
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Save corpus to directory
    pub fn save_to_directory(&self, path: &PathBuf) -> Result<usize, std::io::Error> {
        std::fs::create_dir_all(path)?;

        let entries = self
            .entries
            .read()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Lock poisoned"))?;

        let mut count = 0;
        for (hash, entry) in entries.iter() {
            let file_path = path.join(format!("{}.vr", &hash[..16]));
            std::fs::write(&file_path, &entry.content)?;
            count += 1;
        }

        // Save metadata
        let meta_path = path.join("corpus.json");
        let meta = CorpusMetadata {
            count: entries.len(),
            total_bytes: self.total_bytes(),
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        let meta_json = serde_json::to_string_pretty(&meta)?;
        std::fs::write(meta_path, meta_json)?;

        Ok(count)
    }

    /// Merge another corpus into this one
    pub fn merge(&self, other: &SeedCorpus) -> usize {
        let mut merged = 0;

        if let Ok(other_entries) = other.entries.read() {
            for (_, entry) in other_entries.iter() {
                let source = SeedSource::Imported {
                    source: "merged".to_string(),
                };
                if self.add(&entry.content, source).is_ok() {
                    merged += 1;
                }
            }
        }

        merged
    }

    /// Minimize corpus (remove redundant seeds)
    pub fn minimize<F>(&self, coverage_fn: F) -> usize
    where
        F: Fn(&str) -> HashSet<usize>,
    {
        let entries: Vec<_> = self
            .entries
            .read()
            .ok()
            .map(|e| e.values().cloned().collect())
            .unwrap_or_default();

        // Compute coverage for each seed
        let mut coverages: Vec<(String, HashSet<usize>, usize)> = entries
            .iter()
            .map(|e| {
                let cov = coverage_fn(&e.content);
                (e.hash.clone(), cov, e.size)
            })
            .collect();

        // Greedy set cover to find minimal corpus
        let mut covered: HashSet<usize> = HashSet::new();
        let mut keep: HashSet<String> = HashSet::new();

        // Sort by coverage density (coverage / size)
        coverages.sort_by(|(_, cov_a, size_a), (_, cov_b, size_b)| {
            let density_a = cov_a.len() as f64 / *size_a as f64;
            let density_b = cov_b.len() as f64 / *size_b as f64;
            density_b.partial_cmp(&density_a).unwrap()
        });

        for (hash, cov, _) in coverages {
            let new_coverage: HashSet<_> = cov.difference(&covered).cloned().collect();
            if !new_coverage.is_empty() {
                covered.extend(new_coverage);
                keep.insert(hash);
            }
        }

        // Remove seeds not in keep set
        let mut removed = 0;
        if let Ok(mut entries_lock) = self.entries.write() {
            let to_remove: Vec<_> = entries_lock
                .keys()
                .filter(|h| !keep.contains(*h))
                .cloned()
                .collect();

            for hash in to_remove {
                if let Some(entry) = entries_lock.remove(&hash) {
                    self.total_size.fetch_sub(entry.size, Ordering::Relaxed);
                    removed += 1;
                }
            }
        }

        removed
    }

    /// Hash content
    fn hash_content(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = hasher.finalize();
        hex::encode(&hash[..16])
    }

    /// Evict lowest energy seed
    fn evict_lowest_energy(&self) {
        if let Ok(mut entries) = self.entries.write() {
            if let Some((hash, _)) = entries
                .iter()
                .min_by(|(_, a), (_, b)| a.energy.partial_cmp(&b.energy).unwrap())
            {
                let hash = hash.clone();
                if let Some(entry) = entries.remove(&hash) {
                    self.total_size.fetch_sub(entry.size, Ordering::Relaxed);
                }
            }
        }
    }
}

/// Corpus metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorpusMetadata {
    /// Entry count
    pub count: usize,
    /// Total bytes
    pub total_bytes: usize,
    /// Creation timestamp
    pub created_at: u64,
}

// ============================================================================
// Energy Scheduling
// ============================================================================

/// Energy-based input scheduler
///
/// Implements power schedules from AFL-style fuzzers:
/// - Explores assigns higher energy to inputs covering new edges
/// - Fast assigns energy based on execution speed
/// - COE (Coverage-Ordered Entry) prioritizes recent coverage gains
pub struct EnergyScheduler {
    /// Configuration
    config: EnergyConfig,
    /// Input queue with energy
    queue: RwLock<VecDeque<QueueEntry>>,
    /// Total energy
    total_energy: AtomicU64,
    /// Statistics
    stats: EnergyStats,
}

/// Configuration for energy scheduling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnergyConfig {
    /// Power schedule type
    pub schedule: PowerSchedule,
    /// Initial energy for new inputs
    pub initial_energy: f64,
    /// Energy decay per selection
    pub decay_rate: f64,
    /// Minimum energy
    pub min_energy: f64,
    /// Maximum energy
    pub max_energy: f64,
    /// Coverage boost factor
    pub coverage_boost: f64,
    /// Speed boost factor
    pub speed_boost: f64,
    /// Recency boost factor
    pub recency_boost: f64,
}

impl Default for EnergyConfig {
    fn default() -> Self {
        Self {
            schedule: PowerSchedule::Explore,
            initial_energy: 100.0,
            decay_rate: 0.99,
            min_energy: 1.0,
            max_energy: 10000.0,
            coverage_boost: 2.0,
            speed_boost: 1.5,
            recency_boost: 1.2,
        }
    }
}

/// Power schedule type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PowerSchedule {
    /// Explore: prioritize coverage
    Explore,
    /// Fast: prioritize speed
    Fast,
    /// COE: coverage-ordered entry
    Coe,
    /// Quad: quadratic decay
    Quad,
    /// Linear: linear decay
    Linear,
    /// Exploit: focus on productive inputs
    Exploit,
}

/// Entry in the scheduling queue
#[derive(Debug, Clone)]
pub struct QueueEntry {
    /// Input hash
    pub hash: String,
    /// Current energy
    pub energy: f64,
    /// Unique edges found
    pub unique_edges: usize,
    /// Execution time (nanoseconds)
    pub exec_time_ns: u64,
    /// Times selected
    pub selections: usize,
    /// Productive mutations (found new coverage)
    pub productive: usize,
    /// Last selected timestamp
    pub last_selected: u64,
}

/// Statistics for energy scheduling
#[derive(Debug, Default)]
pub struct EnergyStats {
    /// Total selections
    pub total_selections: AtomicUsize,
    /// Coverage-triggered boosts
    pub coverage_boosts: AtomicUsize,
    /// Speed-triggered boosts
    pub speed_boosts: AtomicUsize,
    /// Energy resets
    pub resets: AtomicUsize,
}

impl Default for EnergyScheduler {
    fn default() -> Self {
        Self::new(EnergyConfig::default())
    }
}

impl EnergyScheduler {
    /// Create a new energy scheduler
    pub fn new(config: EnergyConfig) -> Self {
        Self {
            config,
            queue: RwLock::new(VecDeque::new()),
            total_energy: AtomicU64::new(0),
            stats: EnergyStats::default(),
        }
    }

    /// Maximum queue size to prevent unbounded memory growth
    const MAX_QUEUE_SIZE: usize = 10_000;

    /// Add an input to the queue
    pub fn add(&self, hash: String, unique_edges: usize, exec_time_ns: u64) {
        let energy = self.calculate_initial_energy(unique_edges, exec_time_ns);

        let entry = QueueEntry {
            hash,
            energy,
            unique_edges,
            exec_time_ns,
            selections: 0,
            productive: 0,
            last_selected: 0,
        };

        if let Ok(mut queue) = self.queue.write() {
            queue.push_back(entry);

            // Prune if over limit to prevent unbounded memory growth
            if queue.len() > Self::MAX_QUEUE_SIZE {
                self.prune_low_energy_internal(&mut queue);
            }

            self.update_total_energy(&queue);
        }
    }

    /// Remove an entry by hash
    pub fn remove(&self, hash: &str) -> bool {
        if let Ok(mut queue) = self.queue.write() {
            let initial_len = queue.len();
            queue.retain(|e| e.hash != hash);
            let removed = queue.len() < initial_len;
            if removed {
                self.update_total_energy(&queue);
            }
            return removed;
        }
        false
    }

    /// Prune low-energy entries (keeps top entries by energy)
    pub fn prune_low_energy(&self, keep_count: usize) {
        if let Ok(mut queue) = self.queue.write() {
            if queue.len() <= keep_count {
                return;
            }
            // Convert to vec, sort, truncate, convert back
            let mut entries: Vec<_> = queue.drain(..).collect();
            entries.sort_by(|a, b| {
                b.energy
                    .partial_cmp(&a.energy)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            entries.truncate(keep_count);
            queue.extend(entries);
            self.update_total_energy(&queue);
        }
    }

    /// Internal pruning (called with lock held)
    fn prune_low_energy_internal(&self, queue: &mut VecDeque<QueueEntry>) {
        let keep_count = Self::MAX_QUEUE_SIZE * 9 / 10; // Keep 90%
        let mut entries: Vec<_> = queue.drain(..).collect();
        entries.sort_by(|a, b| {
            b.energy
                .partial_cmp(&a.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(keep_count);
        queue.extend(entries);
    }

    /// Reset queue state
    pub fn reset(&self) {
        if let Ok(mut queue) = self.queue.write() {
            queue.clear();
        }
        self.total_energy.store(0, Ordering::Relaxed);
    }

    /// Select an input for fuzzing
    pub fn select<R: rand::Rng>(&self, rng: &mut R) -> Option<String> {
        let mut queue = self.queue.write().ok()?;
        if queue.is_empty() {
            return None;
        }

        self.stats.total_selections.fetch_add(1, Ordering::Relaxed);

        let total = f64::from_bits(self.total_energy.load(Ordering::Relaxed));
        if total <= 0.0 {
            // Uniform random if no energy
            let idx = rng.random_range(0..queue.len());
            let entry = &mut queue[idx];
            entry.selections += 1;
            entry.last_selected = now_secs();
            return Some(entry.hash.clone());
        }

        // Weighted random selection
        let mut choice = rng.random::<f64>() * total;
        for entry in queue.iter_mut() {
            choice -= entry.energy;
            if choice <= 0.0 {
                entry.selections += 1;
                entry.last_selected = now_secs();
                entry.energy = self.decay_energy(entry.energy);
                return Some(entry.hash.clone());
            }
        }

        // Fallback to last
        queue.back().map(|e| e.hash.clone())
    }

    /// Mark an input as productive
    pub fn mark_productive(&self, hash: &str, new_edges: usize) {
        if let Ok(mut queue) = self.queue.write() {
            if let Some(entry) = queue.iter_mut().find(|e| e.hash == hash) {
                entry.productive += 1;
                entry.unique_edges += new_edges;
                entry.energy = self.boost_energy(entry.energy, new_edges);
                self.stats.coverage_boosts.fetch_add(1, Ordering::Relaxed);
            }
            self.update_total_energy(&queue);
        }
    }

    /// Calculate initial energy
    fn calculate_initial_energy(&self, unique_edges: usize, exec_time_ns: u64) -> f64 {
        let base = self.config.initial_energy;

        // Coverage bonus
        let coverage_bonus = (unique_edges as f64) * self.config.coverage_boost;

        // Speed bonus (faster = higher energy)
        let speed_bonus = if exec_time_ns > 0 {
            self.config.speed_boost * (1_000_000.0 / exec_time_ns as f64)
        } else {
            0.0
        };

        (base + coverage_bonus + speed_bonus).clamp(self.config.min_energy, self.config.max_energy)
    }

    /// Decay energy after selection
    fn decay_energy(&self, energy: f64) -> f64 {
        match self.config.schedule {
            PowerSchedule::Linear => (energy - 1.0).max(self.config.min_energy),
            PowerSchedule::Quad => (energy * 0.9).max(self.config.min_energy),
            _ => (energy * self.config.decay_rate).max(self.config.min_energy),
        }
    }

    /// Boost energy for productive inputs
    fn boost_energy(&self, energy: f64, new_edges: usize) -> f64 {
        let boost = match self.config.schedule {
            PowerSchedule::Explore => (new_edges as f64) * self.config.coverage_boost,
            PowerSchedule::Exploit => (new_edges as f64) * self.config.coverage_boost * 2.0,
            _ => (new_edges as f64) * self.config.coverage_boost,
        };

        (energy + boost).clamp(self.config.min_energy, self.config.max_energy)
    }

    /// Update total energy
    fn update_total_energy(&self, queue: &VecDeque<QueueEntry>) {
        let total: f64 = queue.iter().map(|e| e.energy).sum();
        self.total_energy.store(total.to_bits(), Ordering::Relaxed);
    }

    /// Get queue length
    pub fn len(&self) -> usize {
        self.queue.read().map(|q| q.len()).unwrap_or(0)
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Get statistics
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.stats.total_selections.load(Ordering::Relaxed),
            self.stats.coverage_boosts.load(Ordering::Relaxed),
            self.stats.speed_boosts.load(Ordering::Relaxed),
        )
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// ============================================================================
// Parallel Fuzzing Coordination
// ============================================================================

/// Coordinator for parallel fuzzing workers
pub struct ParallelCoordinator {
    /// Configuration
    config: ParallelConfig,
    /// Worker states
    workers: RwLock<Vec<WorkerState>>,
    /// Shared work queue
    work_queue: RwLock<VecDeque<WorkItem>>,
    /// Global statistics
    stats: Arc<ParallelStats>,
    /// Shutdown flag
    shutdown: AtomicBool,
}

/// Configuration for parallel fuzzing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelConfig {
    /// Number of workers
    pub num_workers: usize,
    /// Work stealing enabled
    pub work_stealing: bool,
    /// Sync interval (iterations)
    pub sync_interval: usize,
    /// Coverage sync enabled
    pub sync_coverage: bool,
    /// Corpus sync enabled
    pub sync_corpus: bool,
    /// Work item batch size
    pub batch_size: usize,
}

impl Default for ParallelConfig {
    fn default() -> Self {
        Self {
            num_workers: num_cpus(),
            work_stealing: true,
            sync_interval: 1000,
            sync_coverage: true,
            sync_corpus: true,
            batch_size: 100,
        }
    }
}

/// State of a fuzzing worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerState {
    /// Worker ID
    pub id: usize,
    /// Current phase
    pub phase: WorkerPhase,
    /// Iterations completed
    pub iterations: usize,
    /// Crashes found
    pub crashes: usize,
    /// New coverage found
    pub new_coverage: usize,
    /// Last heartbeat timestamp
    pub last_heartbeat: u64,
    /// Current input hash being processed
    pub current_input: Option<String>,
}

/// Worker phase
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkerPhase {
    /// Starting up
    Starting,
    /// Fuzzing
    Fuzzing,
    /// Syncing with coordinator
    Syncing,
    /// Shrinking a crash
    Shrinking,
    /// Idle (waiting for work)
    Idle,
    /// Stopped
    Stopped,
}

/// Work item for a worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    /// Work type
    pub work_type: WorkType,
    /// Input hash to process
    pub input_hash: String,
    /// Number of mutations to perform
    pub mutations: usize,
    /// Priority
    pub priority: u32,
}

/// Type of work
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WorkType {
    /// Mutate an existing input
    Mutate,
    /// Generate new random input
    Generate,
    /// Shrink a crash
    Shrink,
    /// Verify a crash
    Verify,
}

/// Statistics for parallel fuzzing
#[derive(Debug, Default)]
pub struct ParallelStats {
    /// Total iterations across all workers
    pub total_iterations: AtomicUsize,
    /// Total crashes found
    pub total_crashes: AtomicUsize,
    /// Work items distributed
    pub work_distributed: AtomicUsize,
    /// Work items stolen
    pub work_stolen: AtomicUsize,
    /// Sync operations
    pub syncs: AtomicUsize,
}

impl Default for ParallelCoordinator {
    fn default() -> Self {
        Self::new(ParallelConfig::default())
    }
}

impl ParallelCoordinator {
    /// Create a new coordinator
    pub fn new(config: ParallelConfig) -> Self {
        let workers = (0..config.num_workers)
            .map(|id| WorkerState {
                id,
                phase: WorkerPhase::Starting,
                iterations: 0,
                crashes: 0,
                new_coverage: 0,
                last_heartbeat: now_secs(),
                current_input: None,
            })
            .collect();

        Self {
            config,
            workers: RwLock::new(workers),
            work_queue: RwLock::new(VecDeque::new()),
            stats: Arc::new(ParallelStats::default()),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Enqueue work items
    pub fn enqueue_work(&self, items: Vec<WorkItem>) {
        if let Ok(mut queue) = self.work_queue.write() {
            for item in items {
                queue.push_back(item);
                self.stats.work_distributed.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Get work for a worker (batch)
    pub fn get_work(&self, worker_id: usize) -> Vec<WorkItem> {
        let mut result = Vec::new();

        if let Ok(mut queue) = self.work_queue.write() {
            for _ in 0..self.config.batch_size {
                if let Some(item) = queue.pop_front() {
                    result.push(item);
                } else {
                    break;
                }
            }
        }

        // If no work and stealing enabled, try to steal
        if result.is_empty() && self.config.work_stealing {
            if let Some(item) = self.steal_work(worker_id) {
                result.push(item);
                self.stats.work_stolen.fetch_add(1, Ordering::Relaxed);
            }
        }

        result
    }

    /// Steal work from another worker
    fn steal_work(&self, _stealer_id: usize) -> Option<WorkItem> {
        // Simple implementation: steal from main queue
        // In production, would steal from worker-local queues
        if let Ok(mut queue) = self.work_queue.write() {
            queue.pop_back()
        } else {
            None
        }
    }

    /// Update worker state
    pub fn update_worker(&self, id: usize, iterations: usize, crashes: usize, coverage: usize) {
        if let Ok(mut workers) = self.workers.write() {
            if let Some(worker) = workers.get_mut(id) {
                worker.iterations = iterations;
                worker.crashes = crashes;
                worker.new_coverage = coverage;
                worker.last_heartbeat = now_secs();
                worker.phase = WorkerPhase::Fuzzing;
            }
        }

        self.stats
            .total_iterations
            .fetch_add(iterations, Ordering::Relaxed);
        self.stats
            .total_crashes
            .fetch_add(crashes, Ordering::Relaxed);
    }

    /// Set worker phase
    pub fn set_worker_phase(&self, id: usize, phase: WorkerPhase) {
        if let Ok(mut workers) = self.workers.write() {
            if let Some(worker) = workers.get_mut(id) {
                worker.phase = phase;
                worker.last_heartbeat = now_secs();
            }
        }
    }

    /// Check if shutdown requested
    pub fn should_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Relaxed)
    }

    /// Request shutdown
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    /// Get worker states
    pub fn get_workers(&self) -> Vec<WorkerState> {
        self.workers
            .read()
            .ok()
            .map(|w| w.clone())
            .unwrap_or_default()
    }

    /// Get statistics
    pub fn get_stats(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.stats.total_iterations.load(Ordering::Relaxed),
            self.stats.total_crashes.load(Ordering::Relaxed),
            self.stats.work_distributed.load(Ordering::Relaxed),
            self.stats.work_stolen.load(Ordering::Relaxed),
            self.stats.syncs.load(Ordering::Relaxed),
        )
    }

    /// Get work queue length
    pub fn queue_len(&self) -> usize {
        self.work_queue.read().map(|q| q.len()).unwrap_or(0)
    }
}

// ============================================================================
// Campaign Checkpoint/Resume
// ============================================================================

/// Checkpoint for campaign resume
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CampaignCheckpoint {
    /// Campaign state
    pub state: CampaignState,
    /// Corpus snapshot
    pub corpus_hashes: Vec<String>,
    /// Coverage bitmap hash
    pub coverage_hash: String,
    /// Crash signatures
    pub crash_signatures: Vec<String>,
    /// Checkpoint timestamp
    pub timestamp: u64,
    /// Checkpoint version
    pub version: u32,
}

impl CampaignCheckpoint {
    /// Create a new checkpoint
    pub fn new(
        state: CampaignState,
        corpus_hashes: Vec<String>,
        coverage_hash: String,
        crash_signatures: Vec<String>,
    ) -> Self {
        Self {
            state,
            corpus_hashes,
            coverage_hash,
            crash_signatures,
            timestamp: now_secs(),
            version: 1,
        }
    }

    /// Save checkpoint to file
    pub fn save(&self, path: &PathBuf) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)
    }

    /// Load checkpoint from file
    pub fn load(path: &PathBuf) -> Result<Self, std::io::Error> {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_seed_corpus() {
        let dir = tempdir().unwrap();
        let config = SeedCorpusConfig::default();
        let corpus = SeedCorpus::new(dir.path().to_path_buf(), config);

        // Add seeds
        let hash1 = corpus
            .add("fn main() {}", SeedSource::UserProvided)
            .unwrap();
        let hash2 = corpus
            .add("fn foo() { 42 }", SeedSource::UserProvided)
            .unwrap();

        assert_eq!(corpus.len(), 2);
        assert!(corpus.get(&hash1).is_some());

        // Mark selected
        corpus.mark_selected(&hash1);
        let entry = corpus.get(&hash1).unwrap();
        assert_eq!(entry.selections, 1);
    }

    #[test]
    fn test_seed_corpus_dedup() {
        let dir = tempdir().unwrap();
        let config = SeedCorpusConfig::default();
        let corpus = SeedCorpus::new(dir.path().to_path_buf(), config);

        corpus
            .add("fn main() {}", SeedSource::UserProvided)
            .unwrap();
        let result = corpus.add("fn main() {}", SeedSource::UserProvided);

        assert!(result.is_err()); // Duplicate
        assert_eq!(corpus.len(), 1);
    }

    #[test]
    fn test_energy_scheduler() {
        let scheduler = EnergyScheduler::default();

        scheduler.add("input1".to_string(), 10, 1000);
        scheduler.add("input2".to_string(), 5, 500);

        assert_eq!(scheduler.len(), 2);

        // Selection should work
        let mut rng = rand::thread_rng();
        let selected = scheduler.select(&mut rng);
        assert!(selected.is_some());
    }

    #[test]
    fn test_energy_scheduler_productive() {
        let scheduler = EnergyScheduler::default();

        scheduler.add("input1".to_string(), 10, 1000);
        scheduler.mark_productive("input1", 5);

        let (_, boosts, _) = scheduler.stats();
        assert_eq!(boosts, 1);
    }

    #[test]
    fn test_parallel_coordinator() {
        let config = ParallelConfig {
            num_workers: 4,
            ..Default::default()
        };
        let coordinator = ParallelCoordinator::new(config);

        // Enqueue work
        coordinator.enqueue_work(vec![
            WorkItem {
                work_type: WorkType::Mutate,
                input_hash: "input1".to_string(),
                mutations: 5,
                priority: 1,
            },
            WorkItem {
                work_type: WorkType::Mutate,
                input_hash: "input2".to_string(),
                mutations: 5,
                priority: 1,
            },
        ]);

        assert_eq!(coordinator.queue_len(), 2);

        // Get work
        let work = coordinator.get_work(0);
        assert!(!work.is_empty());
    }

    #[test]
    fn test_checkpoint() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");

        let state = CampaignState {
            config: CampaignConfig::default(),
            start_time: now_secs(),
            end_time: None,
            iterations: 1000,
            crashes: 5,
            unique_crashes: 3,
            coverage_pct: 50.0,
            corpus_size: 100,
            phase: CampaignPhase::Fuzzing,
            stop_reason: None,
        };

        let checkpoint = CampaignCheckpoint::new(
            state,
            vec!["hash1".to_string(), "hash2".to_string()],
            "coverage_hash".to_string(),
            vec!["crash1".to_string()],
        );

        checkpoint.save(&path).unwrap();
        let loaded = CampaignCheckpoint::load(&path).unwrap();

        assert_eq!(loaded.state.iterations, 1000);
        assert_eq!(loaded.corpus_hashes.len(), 2);
    }
}
