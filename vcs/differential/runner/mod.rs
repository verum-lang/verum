// VCS differential testing infrastructure - suppress clippy warnings for test tooling
#![allow(clippy::all)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_mut)]
#![allow(unused_assignments)]
#![allow(unreachable_code)]
#![allow(unreachable_patterns)]

//! Differential testing runner module
//!
//! This module provides comprehensive infrastructure for comparing outputs between
//! different execution tiers (Tier 0 interpreter, Tier 1 bytecode, Tier 2 JIT, Tier 3 AOT)
//! and across multiple implementations.
//!
//! # Architecture
//!
//! ```text
//! +---------------------------------------------------------------------------------+
//! |                    DIFFERENTIAL TESTING INFRASTRUCTURE                          |
//! +---------------------------------------------------------------------------------+
//! |                                                                                 |
//! |  +--------------------+    +--------------------+    +--------------------+     |
//! |  | DifferentialRunner |    |  CrossImplRunner   |    |   TestGenerator    |     |
//! |  +--------------------+    +--------------------+    +--------------------+     |
//! |           |                         |                         |                 |
//! |           v                         v                         v                 |
//! |  +--------------------+    +--------------------+    +--------------------+     |
//! |  |     Normalizer     |    | SemanticEquiv      |    | DivergenceReport   |     |
//! |  +--------------------+    +--------------------+    +--------------------+     |
//! |           |                         |                         |                 |
//! |           v                         v                         v                 |
//! |  +--------------------+    +--------------------+    +--------------------+     |
//! |  |  JSON Canonical    |    | Float Epsilon      |    | Performance        |     |
//! |  |  Address Strip     |    | Collection Order   |    | Memory Usage       |     |
//! |  |  Timestamp Norm    |    | NaN/Inf Handling   |    | Error Messages     |     |
//! |  +--------------------+    +--------------------+    +--------------------+     |
//! |                                                                                 |
//! +---------------------------------------------------------------------------------+
//! ```
//!
//! # Execution Tiers
//!
//! | Tier | Name        | Binary         | Description                    |
//! |------|-------------|----------------|--------------------------------|
//! | 0    | Interpreter | verum-interpret| Reference implementation       |
//! | 1    | Bytecode    | verum-bc       | Bytecode VM                    |
//! | 2    | JIT         | verum-jit      | Just-in-time compilation       |
//! | 3    | AOT         | verum-run      | Ahead-of-time compilation      |
//!
//! # Modules
//!
//! - [`differential`]: Core tier oracle for comparing across tiers
//! - [`normalizer`]: Output normalization for reliable comparison
//! - [`semantic_equiv`]: Semantic equivalence checking with tolerance
//! - [`divergence`]: Detailed divergence reporting and classification
//! - [`test_generator`]: Automatic test generation from divergences
//! - [`cross_impl`]: Cross-implementation testing framework
//! - [`vtest_integration`]: Integration with vtest test runner
//!
//! # Example
//!
//! ```rust,ignore
//! use vcs_differential_runner::{
//!     DifferentialRunner, CrossImplRunner, TestGenerator,
//!     NormalizationConfig, EquivalenceConfig, GeneratorConfig,
//!     TierSet, ComparisonMode,
//! };
//!
//! // Tier Oracle Testing - compare all tiers
//! let runner = DifferentialRunner::new()
//!     .with_interpreter("verum-interpret")
//!     .with_bytecode("verum-bc")
//!     .with_jit("verum-jit")
//!     .with_aot("verum-run")
//!     .with_timeout(30_000)
//!     .with_comparison_mode(ComparisonMode::AllPairs);
//!
//! let result = runner.run_differential(Path::new("test.vr"))?;
//! if !result.is_success() {
//!     for divergence in result.divergences() {
//!         println!("Divergence: {} vs {}", divergence.tier1, divergence.tier2);
//!     }
//! }
//!
//! // Selective tier comparison
//! let runner = DifferentialRunner::new()
//!     .with_tiers(TierSet::new().add(0).add(3))  // Only compare Tier 0 and Tier 3
//!     .with_reference_tier(0);
//!
//! // Cross-Implementation Testing
//! let cross_runner = CrossImplRunner::new(CrossImplConfig::default()
//!     .with_reference("interpreter", "verum-interpret")
//!     .with_alternative("aot", "verum-run")
//!     .with_alternative("jit", "verum-jit")
//!     .with_version_compatibility(true));
//!
//! let results = cross_runner.run_directory(Path::new("specs/"))?;
//!
//! // Test Generation from Divergences
//! let generator = TestGenerator::new(GeneratorConfig::default())
//!     .with_fuzzer_corpus(Path::new("corpus/"))
//!     .with_edge_case_generation(true)
//!     .with_stress_test_generation(true);
//!
//! for divergence in divergences {
//!     let tests = generator.generate_variants(&divergence)?;
//!     for test in tests {
//!         generator.write_test(&test)?;
//!     }
//! }
//! ```
//!
//! # Performance Targets
//!
//! | Operation              | Target       |
//! |------------------------|--------------|
//! | Simple test execution  | < 100ms      |
//! | Complex async test     | < 1s         |
//! | Fuzz iterations        | > 1000/sec   |
//! | Normalization          | < 1ms per KB |
//! | Report generation      | < 100ms      |

pub mod cross_impl;
pub mod differential;
pub mod divergence;
pub mod normalizer;
pub mod semantic_equiv;
pub mod test_generator;
pub mod vtest_integration;

// Re-export main types
pub use differential::{
    DiffResult, DifferentialFuzzer, DifferentialRunner, PropertyTest, TestMetadata, TestReport,
    TierOutput,
};

pub use normalizer::{NormalizationConfig, NormalizedValue, Normalizer};

pub use semantic_equiv::{
    DiffKind, DiffLocation, DiffSeverity, Difference, EquivalenceConfig, EquivalenceResult,
    EquivalenceRule, FloatComparisonResult, NaNHandling, SemanticEquivalenceChecker, TransformType,
    compare_floats, is_denormal,
};

pub use divergence::{
    Divergence, DivergenceClass, DivergenceReporter, DivergenceThresholds, ReportFormat, Tier,
    TierExecution, classify_divergence_with_thresholds, create_divergence,
    detect_determinism_violation,
};

pub use test_generator::{
    BatchResult, EdgeCaseGenerator, FuzzerCorpusGenerator, GeneratedTest, GeneratorConfig,
    StressTestGenerator, TestCategory, TestExpectation, TestGenerator,
};

pub use cross_impl::{
    BootstrapResult, BootstrapRunner, CompatIssue, CrossImplConfig, CrossImplResult,
    CrossImplRunner, ImplComparison, Implementation, ImplementationResult, VersionCompatConfig,
    VersionCompatResult, VersionCompatRunner, VersionCompatibility, standard_implementations,
};

pub use vtest_integration::{
    DifferentialExecutor, DifferentialSummary, DifferentialTestConfig, DifferentialTestResult,
    DivergenceInfo, TierSummary, TierTestResult, handle_differential_directive,
    is_differential_test,
};

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Set of tiers to execute tests on
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierSet {
    tiers: HashSet<u8>,
}

impl TierSet {
    /// Create a new empty tier set
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a tier set with all tiers (0, 1, 2, 3)
    pub fn all() -> Self {
        let mut tiers = HashSet::new();
        tiers.insert(0);
        tiers.insert(1);
        tiers.insert(2);
        tiers.insert(3);
        Self { tiers }
    }

    /// Create a tier set with only Tier 0 and Tier 3 (interpreter vs AOT)
    pub fn default_comparison() -> Self {
        let mut tiers = HashSet::new();
        tiers.insert(0);
        tiers.insert(3);
        Self { tiers }
    }

    /// Add a tier to the set
    pub fn add(mut self, tier: u8) -> Self {
        if tier <= 3 {
            self.tiers.insert(tier);
        }
        self
    }

    /// Remove a tier from the set
    pub fn remove(mut self, tier: u8) -> Self {
        self.tiers.remove(&tier);
        self
    }

    /// Check if a tier is in the set
    pub fn contains(&self, tier: u8) -> bool {
        self.tiers.contains(&tier)
    }

    /// Get all tiers as a sorted vector
    pub fn to_vec(&self) -> Vec<u8> {
        let mut v: Vec<_> = self.tiers.iter().copied().collect();
        v.sort();
        v
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.tiers.is_empty()
    }

    /// Get the number of tiers
    pub fn len(&self) -> usize {
        self.tiers.len()
    }

    /// Iterate over tiers
    pub fn iter(&self) -> impl Iterator<Item = &u8> {
        self.tiers.iter()
    }
}

impl FromIterator<u8> for TierSet {
    fn from_iter<I: IntoIterator<Item = u8>>(iter: I) -> Self {
        let tiers: HashSet<u8> = iter.into_iter().filter(|&t| t <= 3).collect();
        Self { tiers }
    }
}

/// Comparison mode for differential testing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonMode {
    /// Compare all tiers against a reference tier
    ReferenceComparison,
    /// Compare all pairs of tiers
    AllPairs,
    /// Compare adjacent tiers (0-1, 1-2, 2-3)
    Adjacent,
    /// Custom comparison (use comparison_pairs in config)
    Custom,
}

impl Default for ComparisonMode {
    fn default() -> Self {
        ComparisonMode::ReferenceComparison
    }
}

/// Performance thresholds for divergence detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceThresholds {
    /// Maximum acceptable time ratio between tiers (e.g., 10.0 means 10x slower)
    pub max_time_ratio: f64,
    /// Maximum acceptable memory ratio between tiers
    pub max_memory_ratio: f64,
    /// Minimum execution time (ms) before performance comparison applies
    pub min_duration_ms: u64,
    /// Minimum memory (bytes) before memory comparison applies
    pub min_memory_bytes: usize,
}

impl Default for PerformanceThresholds {
    fn default() -> Self {
        Self {
            max_time_ratio: 10.0,
            max_memory_ratio: 5.0,
            min_duration_ms: 10,
            min_memory_bytes: 1024,
        }
    }
}

/// Floating-point comparison configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatComparisonConfig {
    /// Absolute epsilon for small numbers
    pub absolute_epsilon: f64,
    /// Relative epsilon for large numbers
    pub relative_epsilon: f64,
    /// ULP (Units in Last Place) tolerance
    pub ulp_tolerance: u64,
    /// How to handle NaN comparisons
    pub nan_handling: NaNHandling,
    /// How to handle infinity comparisons
    pub infinity_handling: InfinityHandling,
    /// How to handle denormalized numbers
    pub denormal_handling: DenormalHandling,
}

impl Default for FloatComparisonConfig {
    fn default() -> Self {
        Self {
            absolute_epsilon: 1e-10,
            relative_epsilon: 1e-10,
            ulp_tolerance: 4,
            nan_handling: NaNHandling::TreatAsEqual,
            infinity_handling: InfinityHandling::ExactMatch,
            denormal_handling: DenormalHandling::TreatAsZero,
        }
    }
}

// NaNHandling is re-exported from semantic_equiv

/// How to handle infinity values in comparison
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InfinityHandling {
    /// Require exact match (+Inf == +Inf, -Inf == -Inf)
    ExactMatch,
    /// Treat very large values as infinity
    TreatLargeAsInf { threshold: u64 },
}

impl Default for InfinityHandling {
    fn default() -> Self {
        InfinityHandling::ExactMatch
    }
}

/// How to handle denormalized numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DenormalHandling {
    /// Treat denormalized numbers as zero
    TreatAsZero,
    /// Compare denormalized numbers exactly
    ExactMatch,
    /// Flush denormals to zero before comparison
    FlushToZero,
}

/// Collection comparison configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionComparisonConfig {
    /// Whether unordered collections (sets, maps) can differ in iteration order
    pub allow_unordered_iteration: bool,
    /// Whether to compare by value or by structural equality
    pub compare_by_value: bool,
    /// Maximum depth for nested collection comparison
    pub max_depth: usize,
    /// How to handle missing keys in maps
    pub missing_key_handling: MissingKeyHandling,
}

impl Default for CollectionComparisonConfig {
    fn default() -> Self {
        Self {
            allow_unordered_iteration: true,
            compare_by_value: true,
            max_depth: 100,
            missing_key_handling: MissingKeyHandling::ReportDifference,
        }
    }
}

/// How to handle missing keys in map comparison
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissingKeyHandling {
    /// Report as a difference
    ReportDifference,
    /// Treat missing as None/null
    TreatAsNull,
    /// Ignore missing keys (only compare common keys)
    IgnoreMissing,
}

/// Non-determinism handling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonDeterminismConfig {
    /// Patterns that indicate async/timing-dependent output
    pub async_markers: Vec<String>,
    /// Patterns that indicate hash-dependent output
    pub hash_markers: Vec<String>,
    /// Whether to allow async output reordering
    pub allow_async_reordering: bool,
    /// Whether to sort output lines within marked regions
    pub sort_marked_regions: bool,
    /// Maximum reorder distance (lines) for async output
    pub max_reorder_distance: usize,
}

impl Default for NonDeterminismConfig {
    fn default() -> Self {
        Self {
            async_markers: vec![
                "@async:".to_string(),
                "@concurrent:".to_string(),
                "@parallel:".to_string(),
            ],
            hash_markers: vec!["@unordered:".to_string(), "@hash-dependent:".to_string()],
            allow_async_reordering: false,
            sort_marked_regions: true,
            max_reorder_distance: 10,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tier_set_operations() {
        let set = TierSet::new().add(0).add(3);
        assert!(set.contains(0));
        assert!(set.contains(3));
        assert!(!set.contains(1));
        assert!(!set.contains(2));
        assert_eq!(set.len(), 2);
        assert_eq!(set.to_vec(), vec![0, 3]);
    }

    #[test]
    fn test_tier_set_all() {
        let set = TierSet::all();
        assert!(set.contains(0));
        assert!(set.contains(1));
        assert!(set.contains(2));
        assert!(set.contains(3));
        assert_eq!(set.len(), 4);
    }

    #[test]
    fn test_tier_set_from_iter() {
        let set: TierSet = vec![0, 3, 5, 10].into_iter().collect();
        assert!(set.contains(0));
        assert!(set.contains(3));
        assert!(!set.contains(5)); // Invalid tier, should be filtered
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_default_comparison_mode() {
        assert_eq!(
            ComparisonMode::default(),
            ComparisonMode::ReferenceComparison
        );
    }

    #[test]
    fn test_performance_thresholds() {
        let thresholds = PerformanceThresholds::default();
        assert_eq!(thresholds.max_time_ratio, 10.0);
        assert_eq!(thresholds.max_memory_ratio, 5.0);
    }
}
