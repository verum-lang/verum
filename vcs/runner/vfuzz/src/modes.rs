//! Fuzzing modes for vfuzz
//!
//! This module implements different fuzzing strategies according to VCS Spec Section 19:
//!
//! - **Structure-aware fuzzing**: Generate syntactically valid programs
//! - **Differential fuzzing**: Compare Tier 0 vs Tier 3 execution
//! - **Property-based fuzzing**: Test invariants across all inputs
//! - **Coverage-guided fuzzing**: Evolve corpus based on coverage feedback
//!
//! Each mode can be used independently or combined for comprehensive testing.

use crate::coverage::{COVERAGE_MAP_SIZE, CoverageBitmap, CoverageTracker, GlobalCoverage};
use crate::generator::{Generator, GeneratorConfig, GeneratorKind};
use crate::mutator::{MutationStrategy, Mutator, MutatorConfig};
use crate::oracle::{DifferentialOracle, DifferentialResult, ExecutionTier, TierResult};
use crate::property::{FuzzProperty, FuzzPropertyResult, PropertyRunner};
use crate::shrink::{ShrinkConfig, ShrinkResult, Shrinker};
use crate::{FuzzConfig, FuzzStats, Issue, IssueKind, Tier};
use std::collections::HashMap;

use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Fuzzing mode enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FuzzMode {
    /// Generate and execute valid programs
    StructureAware,
    /// Compare Tier 0 vs Tier 3 execution
    Differential,
    /// Test invariant properties
    PropertyBased,
    /// Coverage-guided corpus evolution
    CoverageGuided,
    /// Combine all modes adaptively
    Adaptive,
}

impl Default for FuzzMode {
    fn default() -> Self {
        FuzzMode::Adaptive
    }
}

impl std::str::FromStr for FuzzMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "structure" | "structure-aware" | "structure_aware" => Ok(FuzzMode::StructureAware),
            "differential" | "diff" => Ok(FuzzMode::Differential),
            "property" | "property-based" | "property_based" => Ok(FuzzMode::PropertyBased),
            "coverage" | "coverage-guided" | "coverage_guided" => Ok(FuzzMode::CoverageGuided),
            "adaptive" | "combined" | "all" => Ok(FuzzMode::Adaptive),
            _ => Err(format!("Unknown fuzz mode: {}", s)),
        }
    }
}

/// Configuration for a specific fuzzing mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    /// The fuzzing mode to use
    pub mode: FuzzMode,
    /// Maximum iterations for this mode
    pub max_iterations: usize,
    /// Timeout per test in milliseconds
    pub timeout_ms: u64,
    /// Probability of mutation vs generation (0.0-1.0)
    pub mutation_probability: f64,
    /// Whether to minimize failing inputs
    pub minimize_failures: bool,
    /// Save interesting inputs to corpus
    pub save_interesting: bool,
    /// Random seed for reproducibility
    pub seed: Option<u64>,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            mode: FuzzMode::Adaptive,
            max_iterations: 0, // infinite
            timeout_ms: 10_000,
            mutation_probability: 0.8,
            minimize_failures: true,
            save_interesting: true,
            seed: None,
        }
    }
}

/// Result of a single fuzzing iteration
#[derive(Debug, Clone)]
pub struct IterationResult {
    /// Input program tested
    pub input: String,
    /// Whether the test passed
    pub passed: bool,
    /// Issue found (if any)
    pub issue: Option<Issue>,
    /// Whether the input was interesting (new coverage)
    pub interesting: bool,
    /// Execution time in nanoseconds
    pub duration_ns: u64,
    /// Mode that was used
    pub mode_used: FuzzMode,
}

/// Structure-aware fuzzing mode
///
/// Generates syntactically valid Verum programs and executes them.
/// Uses grammar-based generation and type-aware mutations.
pub struct StructureAwareFuzzer {
    config: ModeConfig,
    generator: Generator,
    mutator: Mutator,
    rng: ChaCha8Rng,
    corpus: Vec<String>,
}

impl StructureAwareFuzzer {
    /// Create a new structure-aware fuzzer
    pub fn new(config: ModeConfig) -> Self {
        let seed = config.seed.unwrap_or_else(rand::random);
        let rng = ChaCha8Rng::seed_from_u64(seed);

        let gen_config = GeneratorConfig {
            max_depth: 10,
            max_statements: 50,
            kind: GeneratorKind::TypeAware,
            ..Default::default()
        };

        let mut_config = MutatorConfig {
            mutation_rate: config.mutation_probability,
            ..Default::default()
        };

        Self {
            config,
            generator: Generator::new(gen_config),
            mutator: Mutator::new(mut_config),
            rng,
            corpus: Vec::new(),
        }
    }

    /// Add seed inputs to corpus
    pub fn add_seeds(&mut self, seeds: Vec<String>) {
        self.corpus.extend(seeds);
    }

    /// Run one fuzzing iteration
    pub fn iterate(&mut self) -> IterationResult {
        let start = Instant::now();

        // Decide whether to generate or mutate
        let input = if self.corpus.is_empty()
            || self.rng.random::<f64>() > self.config.mutation_probability
        {
            self.generator.generate(&mut self.rng)
        } else {
            let base = self
                .corpus
                .choose(&mut self.rng)
                .cloned()
                .unwrap_or_default();
            self.mutator.mutate(&base, &mut self.rng)
        };

        // Execute and check for issues
        let (passed, issue) = self.execute_and_check(&input);

        let duration_ns = start.elapsed().as_nanos() as u64;
        let interesting = self.is_interesting(&input);

        if interesting && self.config.save_interesting {
            self.corpus.push(input.clone());
        }

        IterationResult {
            input,
            passed,
            issue,
            interesting,
            duration_ns,
            mode_used: FuzzMode::StructureAware,
        }
    }

    /// Execute input and check for issues
    fn execute_and_check(&self, input: &str) -> (bool, Option<Issue>) {
        // Placeholder - real implementation would invoke compiler
        // Check for obvious crash patterns
        if input.contains("panic!") || input.contains("unreachable!") {
            return (
                false,
                Some(Issue::new(
                    input,
                    IssueKind::Crash(crate::CrashKind::Panic),
                    "Explicit panic in code",
                )),
            );
        }

        (true, None)
    }

    /// Check if input is interesting
    fn is_interesting(&self, input: &str) -> bool {
        // Simple heuristic - check for novel patterns
        let patterns = ["async", "await", "unsafe", "using", "contract"];
        patterns.iter().any(|p| input.contains(p)) && !self.corpus.iter().any(|c| c.contains(input))
    }
}

/// Differential fuzzing mode
///
/// Compares execution between Tier 0 (interpreter) and Tier 3 (native).
/// Any difference in output indicates a bug.
pub struct DifferentialFuzzer {
    config: ModeConfig,
    generator: Generator,
    mutator: Mutator,
    oracle: DifferentialOracle,
    rng: ChaCha8Rng,
    corpus: Vec<String>,
    shrinker: Shrinker,
}

impl DifferentialFuzzer {
    /// Create a new differential fuzzer
    pub fn new(config: ModeConfig) -> Self {
        let seed = config.seed.unwrap_or_else(rand::random);
        let rng = ChaCha8Rng::seed_from_u64(seed);

        let gen_config = GeneratorConfig {
            max_depth: 8,
            max_statements: 30,
            kind: GeneratorKind::TypeAware,
            ..Default::default()
        };

        let mut_config = MutatorConfig {
            mutation_rate: config.mutation_probability,
            ..Default::default()
        };

        let oracle_config = crate::oracle::DifferentialOracleConfig {
            timeout_ms: config.timeout_ms,
            compare_output: true,
            compare_side_effects: true,
            ..Default::default()
        };

        let shrink_config = ShrinkConfig {
            max_iterations: 1000,
            ..Default::default()
        };

        Self {
            config,
            generator: Generator::new(gen_config),
            mutator: Mutator::new(mut_config),
            oracle: DifferentialOracle::new(oracle_config),
            rng,
            corpus: Vec::new(),
            shrinker: Shrinker::new(shrink_config),
        }
    }

    /// Add seed inputs to corpus
    pub fn add_seeds(&mut self, seeds: Vec<String>) {
        self.corpus.extend(seeds);
    }

    /// Run one differential fuzzing iteration
    pub fn iterate(&mut self) -> IterationResult {
        let start = Instant::now();

        // Generate or mutate input
        let input = if self.corpus.is_empty()
            || self.rng.random::<f64>() > self.config.mutation_probability
        {
            self.generator.generate(&mut self.rng)
        } else {
            let base = self
                .corpus
                .choose(&mut self.rng)
                .cloned()
                .unwrap_or_default();
            self.mutator.mutate(&base, &mut self.rng)
        };

        // Run differential test - execute tiers and compare
        let tier_results: HashMap<ExecutionTier, TierResult> = HashMap::new(); // TODO: execute tiers
        let result = self.oracle.compare(&input, tier_results);

        let (passed, issue) = if result.consistent {
            (true, None)
        } else if !result.mismatches.is_empty() {
            let mismatch = &result.mismatches[0];
            let issue = Issue::new(
                &input,
                IssueKind::DifferentialMismatch,
                &format!("Mismatch: {:?} - {}", mismatch.kind, mismatch.details),
            );
            (false, Some(issue))
        } else {
            (true, None)
        };

        let duration_ns = start.elapsed().as_nanos() as u64;
        let interesting = !result.consistent;

        if interesting && self.config.save_interesting {
            self.corpus.push(input.clone());
        }

        IterationResult {
            input,
            passed,
            issue,
            interesting,
            duration_ns,
            mode_used: FuzzMode::Differential,
        }
    }

    /// Minimize a failing input
    pub fn minimize(&self, input: &str) -> Option<String> {
        if !self.config.minimize_failures {
            return None;
        }

        // For minimization, we need a mutable oracle reference
        // Return None for now since we can't borrow self.oracle mutably in closure
        // TODO: Implement proper minimization with mutable oracle access
        None
    }
}

/// Property-based fuzzing mode
///
/// Tests that certain invariants hold across all generated inputs.
/// Examples: idempotency, commutativity, associativity.
pub struct PropertyBasedFuzzer {
    config: ModeConfig,
    generator: Generator,
    mutator: Mutator,
    properties: Vec<Box<dyn FuzzProperty>>,
    rng: ChaCha8Rng,
    corpus: Vec<String>,
}

impl PropertyBasedFuzzer {
    /// Create a new property-based fuzzer
    pub fn new(config: ModeConfig) -> Self {
        let seed = config.seed.unwrap_or_else(rand::random);
        let rng = ChaCha8Rng::seed_from_u64(seed);

        let gen_config = GeneratorConfig {
            max_depth: 6,
            max_statements: 20,
            kind: GeneratorKind::Mixed,
            ..Default::default()
        };

        let mut_config = MutatorConfig {
            mutation_rate: config.mutation_probability,
            ..Default::default()
        };

        Self {
            config,
            generator: Generator::new(gen_config),
            mutator: Mutator::new(mut_config),
            properties: Vec::new(),
            rng,
            corpus: Vec::new(),
        }
    }

    /// Register a property to test
    pub fn register_property(&mut self, property: Box<dyn FuzzProperty>) {
        self.properties.push(property);
    }

    /// Register built-in compiler properties
    pub fn register_compiler_properties(&mut self) {
        // Add standard compiler properties
        self.register_property(Box::new(ParseIdempotencyProperty));
        self.register_property(Box::new(TypeCheckDeterminismProperty));
        self.register_property(Box::new(FormatterRoundtripProperty));
    }

    /// Add seed inputs to corpus
    pub fn add_seeds(&mut self, seeds: Vec<String>) {
        self.corpus.extend(seeds);
    }

    /// Run one property-based fuzzing iteration
    pub fn iterate(&mut self) -> IterationResult {
        let start = Instant::now();

        // Generate or mutate input
        let input = if self.corpus.is_empty()
            || self.rng.random::<f64>() > self.config.mutation_probability
        {
            self.generator.generate(&mut self.rng)
        } else {
            let base = self
                .corpus
                .choose(&mut self.rng)
                .cloned()
                .unwrap_or_default();
            self.mutator.mutate(&base, &mut self.rng)
        };

        // Test all properties
        let mut passed = true;
        let mut issue = None;

        for prop in &self.properties {
            match prop.check(&input) {
                FuzzPropertyResult::Pass => {}
                FuzzPropertyResult::Fail(reason) => {
                    passed = false;
                    issue = Some(Issue::new(
                        &input,
                        IssueKind::TypeUnsoundness,
                        &format!("Property '{}' failed: {}", prop.name(), reason),
                    ));
                    break;
                }
                FuzzPropertyResult::Skip => {}
            }
        }

        let duration_ns = start.elapsed().as_nanos() as u64;
        let interesting = !passed;

        IterationResult {
            input,
            passed,
            issue,
            interesting,
            duration_ns,
            mode_used: FuzzMode::PropertyBased,
        }
    }
}

/// Coverage-guided fuzzing mode
///
/// Evolves the corpus based on coverage feedback.
/// Prioritizes inputs that explore new code paths.
pub struct CoverageGuidedFuzzer {
    config: ModeConfig,
    generator: Generator,
    mutator: Mutator,
    coverage: CoverageTracker,
    rng: ChaCha8Rng,
    corpus: Vec<(String, u64)>, // (input, coverage_score)
    global_coverage: GlobalCoverage,
}

impl CoverageGuidedFuzzer {
    /// Create a new coverage-guided fuzzer
    pub fn new(config: ModeConfig) -> Self {
        let seed = config.seed.unwrap_or_else(rand::random);
        let rng = ChaCha8Rng::seed_from_u64(seed);

        let gen_config = GeneratorConfig {
            max_depth: 10,
            max_statements: 50,
            kind: GeneratorKind::EdgeCase,
            ..Default::default()
        };

        let mut_config = MutatorConfig {
            mutation_rate: config.mutation_probability,
            max_mutations: 5,
            ..Default::default()
        };

        Self {
            config,
            generator: Generator::new(gen_config),
            mutator: Mutator::new(mut_config),
            coverage: CoverageTracker::new(),
            rng,
            corpus: Vec::new(),
            global_coverage: GlobalCoverage::new(),
        }
    }

    /// Add seed inputs to corpus
    pub fn add_seeds(&mut self, seeds: Vec<String>) {
        for seed in seeds {
            // Calculate initial coverage for each seed
            let cov = self.calculate_coverage(&seed);
            self.corpus.push((seed, cov));
        }
    }

    /// Run one coverage-guided fuzzing iteration
    pub fn iterate(&mut self) -> IterationResult {
        let start = Instant::now();

        // Select input from corpus with priority to higher coverage
        let input = if self.corpus.is_empty() {
            self.generator.generate(&mut self.rng)
        } else if self.rng.random::<f64>() > self.config.mutation_probability {
            self.generator.generate(&mut self.rng)
        } else {
            // Use energy-based selection: favor inputs with higher coverage
            let total_score: u64 = self.corpus.iter().map(|(_, s)| *s).sum();
            let mut threshold = self.rng.gen_range(0..total_score.max(1));

            let base = self
                .corpus
                .iter()
                .find(|(_, score)| {
                    if *score >= threshold {
                        true
                    } else {
                        threshold -= score;
                        false
                    }
                })
                .map(|(input, _)| input.clone())
                .unwrap_or_else(|| self.corpus[0].0.clone());

            self.mutator.mutate(&base, &mut self.rng)
        };

        // Execute and measure coverage
        let new_coverage = self.calculate_coverage(&input);
        let is_new = self
            .coverage
            .bitmap()
            .has_new_coverage(&self.global_coverage);

        // Check for issues
        let (passed, issue) = self.check_for_issues(&input);

        let interesting = is_new || !passed;

        if interesting && self.config.save_interesting {
            self.corpus.push((input.clone(), new_coverage));
            self.global_coverage.update(self.coverage.bitmap());
        }

        let duration_ns = start.elapsed().as_nanos() as u64;

        IterationResult {
            input,
            passed,
            issue,
            interesting,
            duration_ns,
            mode_used: FuzzMode::CoverageGuided,
        }
    }

    /// Calculate coverage for an input
    fn calculate_coverage(&mut self, input: &str) -> u64 {
        self.coverage.reset();

        // Simulate execution with coverage tracking
        // Real implementation would instrument compiler execution
        let mut score = 0u64;

        // Track coverage based on source patterns (simplified)
        let patterns = [
            ("fn ", 10),
            ("if ", 5),
            ("match ", 8),
            ("for ", 6),
            ("while ", 6),
            ("async ", 12),
            ("unsafe ", 15),
            ("using ", 8),
            ("contract", 20),
            ("<", 3),
            (">", 3),
            ("&", 4),
            ("&mut ", 6),
            ("&checked ", 10),
            ("&unsafe ", 12),
        ];

        for (pattern, weight) in patterns {
            let count = input.matches(pattern).count() as u64;
            score += count * weight;

            // Mark coverage bits by visiting locations
            for i in 0..count.min(8) {
                let location =
                    ((pattern.as_ptr() as usize + i as usize) % COVERAGE_MAP_SIZE) as u32;
                self.coverage.visit_location(location);
            }
        }

        score
    }

    /// Check for issues in input
    fn check_for_issues(&self, input: &str) -> (bool, Option<Issue>) {
        // Check for crash patterns
        if input.contains("panic!") || input.contains("unreachable!") {
            return (
                false,
                Some(Issue::new(
                    input,
                    IssueKind::Crash(crate::CrashKind::Panic),
                    "Explicit panic in code",
                )),
            );
        }

        (true, None)
    }

    /// Get current coverage percentage
    pub fn coverage_percentage(&self) -> f64 {
        self.global_coverage.coverage_pct()
    }

    /// Get corpus size
    pub fn corpus_size(&self) -> usize {
        self.corpus.len()
    }
}

/// Adaptive fuzzer that combines all modes
pub struct AdaptiveFuzzer {
    config: ModeConfig,
    structure_aware: StructureAwareFuzzer,
    differential: DifferentialFuzzer,
    property_based: PropertyBasedFuzzer,
    coverage_guided: CoverageGuidedFuzzer,
    rng: ChaCha8Rng,
    mode_weights: [f64; 4],
    mode_successes: [usize; 4],
    mode_trials: [usize; 4],
}

impl AdaptiveFuzzer {
    /// Create a new adaptive fuzzer
    pub fn new(config: ModeConfig) -> Self {
        let seed = config.seed.unwrap_or_else(rand::random);

        Self {
            structure_aware: StructureAwareFuzzer::new(ModeConfig {
                seed: Some(seed),
                ..config.clone()
            }),
            differential: DifferentialFuzzer::new(ModeConfig {
                seed: Some(seed.wrapping_add(1)),
                ..config.clone()
            }),
            property_based: PropertyBasedFuzzer::new(ModeConfig {
                seed: Some(seed.wrapping_add(2)),
                ..config.clone()
            }),
            coverage_guided: CoverageGuidedFuzzer::new(ModeConfig {
                seed: Some(seed.wrapping_add(3)),
                ..config.clone()
            }),
            rng: ChaCha8Rng::seed_from_u64(seed),
            mode_weights: [0.25, 0.25, 0.25, 0.25],
            mode_successes: [0; 4],
            mode_trials: [0; 4],
            config,
        }
    }

    /// Add seed inputs to all sub-fuzzers
    pub fn add_seeds(&mut self, seeds: Vec<String>) {
        self.structure_aware.add_seeds(seeds.clone());
        self.differential.add_seeds(seeds.clone());
        self.property_based.add_seeds(seeds.clone());
        self.coverage_guided.add_seeds(seeds);
    }

    /// Register compiler properties for property-based testing
    pub fn register_compiler_properties(&mut self) {
        self.property_based.register_compiler_properties();
    }

    /// Run one adaptive fuzzing iteration
    pub fn iterate(&mut self) -> IterationResult {
        // Select mode based on adaptive weights
        let mode_idx = self.select_mode();

        let result = match mode_idx {
            0 => self.structure_aware.iterate(),
            1 => self.differential.iterate(),
            2 => self.property_based.iterate(),
            _ => self.coverage_guided.iterate(),
        };

        // Update statistics for adaptive scheduling
        self.mode_trials[mode_idx] += 1;
        if result.interesting || result.issue.is_some() {
            self.mode_successes[mode_idx] += 1;
        }

        // Periodically update weights
        if self.mode_trials.iter().sum::<usize>() % 100 == 0 {
            self.update_weights();
        }

        result
    }

    /// Select mode based on adaptive weights (UCB1 algorithm)
    fn select_mode(&mut self) -> usize {
        let total_trials: usize = self.mode_trials.iter().sum();

        if total_trials < 40 {
            // Initial exploration: try each mode at least 10 times
            return (total_trials / 10) % 4;
        }

        // UCB1 selection
        let mut best_idx = 0;
        let mut best_ucb = f64::NEG_INFINITY;

        for i in 0..4 {
            let trials = self.mode_trials[i] as f64;
            let successes = self.mode_successes[i] as f64;

            if trials == 0.0 {
                return i;
            }

            let avg = successes / trials;
            let exploration = (2.0 * (total_trials as f64).ln() / trials).sqrt();
            let ucb = avg + exploration;

            if ucb > best_ucb {
                best_ucb = ucb;
                best_idx = i;
            }
        }

        best_idx
    }

    /// Update mode weights based on performance
    fn update_weights(&mut self) {
        let total: f64 = self.mode_successes.iter().map(|&s| s as f64 + 1.0).sum();

        for i in 0..4 {
            self.mode_weights[i] = (self.mode_successes[i] as f64 + 1.0) / total;
        }
    }

    /// Get current coverage percentage
    pub fn coverage_percentage(&self) -> f64 {
        self.coverage_guided.coverage_percentage()
    }

    /// Get mode statistics
    pub fn mode_stats(&self) -> Vec<(FuzzMode, usize, usize)> {
        vec![
            (
                FuzzMode::StructureAware,
                self.mode_trials[0],
                self.mode_successes[0],
            ),
            (
                FuzzMode::Differential,
                self.mode_trials[1],
                self.mode_successes[1],
            ),
            (
                FuzzMode::PropertyBased,
                self.mode_trials[2],
                self.mode_successes[2],
            ),
            (
                FuzzMode::CoverageGuided,
                self.mode_trials[3],
                self.mode_successes[3],
            ),
        ]
    }
}

// ============================================================================
// Built-in Properties
// ============================================================================

/// Property: Parsing is idempotent (parse(format(parse(x))) == parse(x))
struct ParseIdempotencyProperty;

impl FuzzProperty for ParseIdempotencyProperty {
    fn name(&self) -> &str {
        "parse_idempotency"
    }

    fn check(&self, input: &str) -> FuzzPropertyResult {
        // Placeholder - real implementation would use parser
        // Check basic structural validity
        let balanced = input.matches('{').count() == input.matches('}').count()
            && input.matches('(').count() == input.matches(')').count()
            && input.matches('[').count() == input.matches(']').count();

        if balanced {
            FuzzPropertyResult::Pass
        } else {
            FuzzPropertyResult::Fail("Unbalanced brackets".to_string())
        }
    }
}

/// Property: Type checking is deterministic
struct TypeCheckDeterminismProperty;

impl FuzzProperty for TypeCheckDeterminismProperty {
    fn name(&self) -> &str {
        "typecheck_determinism"
    }

    fn check(&self, input: &str) -> FuzzPropertyResult {
        // Placeholder - real implementation would run type checker twice
        // For now, always pass
        FuzzPropertyResult::Pass
    }
}

/// Property: Formatting round-trips (format(parse(x)) parses equivalently)
struct FormatterRoundtripProperty;

impl FuzzProperty for FormatterRoundtripProperty {
    fn name(&self) -> &str {
        "formatter_roundtrip"
    }

    fn check(&self, input: &str) -> FuzzPropertyResult {
        // Placeholder - real implementation would use formatter
        FuzzPropertyResult::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fuzz_mode_parsing() {
        assert_eq!(
            "differential".parse::<FuzzMode>().unwrap(),
            FuzzMode::Differential
        );
        assert_eq!(
            "coverage".parse::<FuzzMode>().unwrap(),
            FuzzMode::CoverageGuided
        );
        assert_eq!(
            "property".parse::<FuzzMode>().unwrap(),
            FuzzMode::PropertyBased
        );
    }

    #[test]
    fn test_structure_aware_fuzzer() {
        let config = ModeConfig {
            seed: Some(42),
            ..Default::default()
        };
        let mut fuzzer = StructureAwareFuzzer::new(config);

        let result = fuzzer.iterate();
        assert!(!result.input.is_empty());
    }

    #[test]
    fn test_coverage_guided_fuzzer() {
        let config = ModeConfig {
            seed: Some(42),
            ..Default::default()
        };
        let mut fuzzer = CoverageGuidedFuzzer::new(config);

        // Add some seeds
        fuzzer.add_seeds(vec![
            "fn main() { }".to_string(),
            "fn test() { let x = 1; }".to_string(),
        ]);

        for _ in 0..10 {
            let result = fuzzer.iterate();
            assert!(!result.input.is_empty());
        }

        assert!(fuzzer.corpus_size() >= 2);
    }

    #[test]
    fn test_adaptive_fuzzer() {
        let config = ModeConfig {
            seed: Some(42),
            ..Default::default()
        };
        let mut fuzzer = AdaptiveFuzzer::new(config);

        for _ in 0..50 {
            let result = fuzzer.iterate();
            assert!(!result.input.is_empty());
        }

        let stats = fuzzer.mode_stats();
        // Each mode should have been tried
        assert!(stats.iter().all(|(_, trials, _)| *trials > 0));
    }

    #[test]
    fn test_parse_idempotency_property() {
        let prop = ParseIdempotencyProperty;

        assert!(matches!(
            prop.check("fn main() { }"),
            FuzzPropertyResult::Pass
        ));

        assert!(matches!(
            prop.check("fn main() { { }"),
            FuzzPropertyResult::Fail(_)
        ));
    }
}
