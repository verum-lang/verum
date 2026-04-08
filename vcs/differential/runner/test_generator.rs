//! Automatic test generation from divergences
//!
//! This module generates minimized, reproducible test cases from discovered
//! divergences. It implements:
//!
//! - Delta debugging for test minimization
//! - Test case augmentation for edge case discovery
//! - Regression test suite generation
//! - Categorized test organization

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::divergence::{Divergence, DivergenceClass, Tier};
use crate::semantic_equiv::{DiffKind, Difference};

/// Test generation configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Output directory for generated tests
    pub output_dir: PathBuf,
    /// Minimum source lines to keep during minimization
    pub min_lines: usize,
    /// Maximum minimization iterations
    pub max_iterations: usize,
    /// Whether to generate mutation variants
    pub generate_mutations: bool,
    /// Number of mutations to generate per divergence
    pub mutation_count: usize,
    /// Whether to add test annotations
    pub add_annotations: bool,
    /// Whether to include original source as comment
    pub include_original: bool,
    /// Tags to add to generated tests
    pub default_tags: Vec<String>,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("generated_tests"),
            min_lines: 3,
            max_iterations: 100,
            generate_mutations: true,
            mutation_count: 5,
            add_annotations: true,
            include_original: true,
            default_tags: vec!["generated".to_string(), "regression".to_string()],
        }
    }
}

/// Generated test case
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedTest {
    /// Test name/identifier
    pub name: String,
    /// Test source code
    pub source: String,
    /// Category of the test
    pub category: TestCategory,
    /// Original divergence ID
    pub divergence_id: String,
    /// Tiers to compare
    pub tiers: Vec<Tier>,
    /// Tags for filtering
    pub tags: Vec<String>,
    /// Description of what this test checks
    pub description: String,
    /// Expected behavior
    pub expectation: TestExpectation,
}

/// Test category for organization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TestCategory {
    /// Arithmetic operations
    Arithmetic,
    /// String operations
    String,
    /// Collection operations
    Collection,
    /// Async semantics
    Async,
    /// Memory operations
    Memory,
    /// Floating-point precision
    FloatPrecision,
    /// Error handling
    ErrorHandling,
    /// Pattern matching
    PatternMatching,
    /// Generic types
    Generics,
    /// Control flow
    ControlFlow,
    /// Other/uncategorized
    Other,
}

impl TestCategory {
    /// Get directory name for this category
    pub fn dir_name(&self) -> &'static str {
        match self {
            TestCategory::Arithmetic => "arithmetic",
            TestCategory::String => "string_ops",
            TestCategory::Collection => "collections",
            TestCategory::Async => "async",
            TestCategory::Memory => "memory",
            TestCategory::FloatPrecision => "float_precision",
            TestCategory::ErrorHandling => "error_handling",
            TestCategory::PatternMatching => "pattern_matching",
            TestCategory::Generics => "generics",
            TestCategory::ControlFlow => "control_flow",
            TestCategory::Other => "other",
        }
    }

    /// Infer category from source code
    pub fn infer(source: &str) -> Self {
        let lower = source.to_lowercase();

        if lower.contains("async") || lower.contains("await") || lower.contains("spawn") {
            return TestCategory::Async;
        }

        if lower.contains("list")
            || lower.contains("map")
            || lower.contains("set")
            || lower.contains("vec")
            || lower.contains("hash")
        {
            return TestCategory::Collection;
        }

        if lower.contains("text") || lower.contains("string") || lower.contains("char") {
            return TestCategory::String;
        }

        if lower.contains("box::")
            || lower.contains("heap::")
            || lower.contains("&mut")
            || lower.contains("drop")
            || lower.contains("cbgr")
        {
            return TestCategory::Memory;
        }

        if lower.contains("result")
            || lower.contains("maybe")
            || lower.contains("err")
            || lower.contains("panic")
        {
            return TestCategory::ErrorHandling;
        }

        if lower.contains("match") || lower.contains("if let") || lower.contains("=>") {
            return TestCategory::PatternMatching;
        }

        if lower.contains("<t>") || lower.contains("impl<") || lower.contains("where") {
            return TestCategory::Generics;
        }

        if lower.contains("if ")
            || lower.contains("loop")
            || lower.contains("while")
            || lower.contains("for ")
        {
            return TestCategory::ControlFlow;
        }

        if lower.contains("float")
            || lower.contains(".0")
            || lower.contains("nan")
            || lower.contains("inf")
        {
            return TestCategory::FloatPrecision;
        }

        if lower.contains('+')
            || lower.contains('-')
            || lower.contains('*')
            || lower.contains('/')
            || lower.contains('%')
        {
            return TestCategory::Arithmetic;
        }

        TestCategory::Other
    }
}

/// Expected test behavior
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestExpectation {
    /// Tiers should produce identical output
    IdenticalOutput,
    /// Tiers should produce semantically equivalent output
    SemanticEquivalence,
    /// One tier should crash/panic
    ExpectCrash { tier: Tier },
    /// Specific exit code expected
    ExitCode { tier: Tier, code: i32 },
    /// Output should contain specific text
    OutputContains { text: String },
    /// Custom expectation description
    Custom { description: String },
}

/// Test generator
pub struct TestGenerator {
    config: GeneratorConfig,
    /// Function to check if source still triggers divergence
    /// In production, this would run actual differential tests
    divergence_checker: Option<Box<dyn Fn(&str) -> bool + Send + Sync>>,
}

impl TestGenerator {
    /// Create a new test generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            config,
            divergence_checker: None,
        }
    }

    /// Set the divergence checker function
    pub fn with_checker<F>(mut self, checker: F) -> Self
    where
        F: Fn(&str) -> bool + Send + Sync + 'static,
    {
        self.divergence_checker = Some(Box::new(checker));
        self
    }

    /// Generate test case from a divergence
    pub fn generate(&self, divergence: &Divergence) -> Result<GeneratedTest> {
        // Minimize the test case
        let minimized = self.minimize(&divergence.source_code)?;

        // Infer category
        let category = TestCategory::infer(&minimized);

        // Determine expectation
        let expectation = self.infer_expectation(divergence);

        // Generate name
        let name = format!("regression_{}_{}", category.dir_name(), &divergence.id[..8]);

        // Collect tags
        let mut tags = self.config.default_tags.clone();
        tags.extend(divergence.tags.iter().cloned());
        tags.push(format!("{}", divergence.classification));

        // Generate description
        let description = format!(
            "Regression test for {} divergence between {} and {} ({})",
            divergence.classification, divergence.tier1, divergence.tier2, divergence.id
        );

        Ok(GeneratedTest {
            name,
            source: minimized,
            category,
            divergence_id: divergence.id.clone(),
            tiers: vec![divergence.tier1, divergence.tier2],
            tags,
            description,
            expectation,
        })
    }

    /// Generate multiple test variants from a divergence
    pub fn generate_variants(&self, divergence: &Divergence) -> Result<Vec<GeneratedTest>> {
        let mut tests = vec![self.generate(divergence)?];

        if self.config.generate_mutations {
            let mutations = self.generate_mutations(&divergence.source_code);

            for (i, mutation) in mutations
                .into_iter()
                .take(self.config.mutation_count)
                .enumerate()
            {
                // Check if mutation still triggers divergence
                let triggers = self
                    .divergence_checker
                    .as_ref()
                    .map_or(true, |check| check(&mutation));

                if triggers {
                    let category = TestCategory::infer(&mutation);
                    let name = format!(
                        "mutation_{}_{}_{}",
                        category.dir_name(),
                        &divergence.id[..8],
                        i
                    );

                    tests.push(GeneratedTest {
                        name,
                        source: mutation,
                        category,
                        divergence_id: divergence.id.clone(),
                        tiers: vec![divergence.tier1, divergence.tier2],
                        tags: vec![
                            "generated".to_string(),
                            "mutation".to_string(),
                            format!("{}", divergence.classification),
                        ],
                        description: format!(
                            "Mutation variant {} for divergence {}",
                            i, divergence.id
                        ),
                        expectation: self.infer_expectation(divergence),
                    });
                }
            }
        }

        Ok(tests)
    }

    /// Minimize source code while preserving divergence
    fn minimize(&self, source: &str) -> Result<String> {
        let lines: Vec<&str> = source.lines().collect();

        if lines.len() <= self.config.min_lines {
            return Ok(source.to_string());
        }

        let mut current = lines.clone();

        // Delta debugging: try removing each line
        for _ in 0..self.config.max_iterations {
            let mut made_progress = false;

            for i in (0..current.len()).rev() {
                if current.len() <= self.config.min_lines {
                    break;
                }

                // Skip lines that look essential
                let line = current[i].trim();
                if line.starts_with("fn main") || line == "}" || line.is_empty() {
                    continue;
                }

                // Try removing this line
                let candidate: Vec<&str> = current
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, &l)| l)
                    .collect();

                let candidate_str = candidate.join("\n");

                // Check if still triggers divergence
                let still_triggers = self
                    .divergence_checker
                    .as_ref()
                    .map_or(true, |check| check(&candidate_str));

                if still_triggers {
                    current = candidate;
                    made_progress = true;
                    break;
                }
            }

            if !made_progress {
                break;
            }
        }

        Ok(current.join("\n"))
    }

    /// Generate mutations of the source code
    fn generate_mutations(&self, source: &str) -> Vec<String> {
        let mut mutations = Vec::new();

        // Mutation strategies

        // 1. Replace integer literals with edge cases
        let int_pattern = regex::Regex::new(r"\b(\d+)\b").unwrap();
        for edge_value in [
            "0",
            "1",
            "-1",
            "255",
            "256",
            "65535",
            "65536",
            "2147483647",
            "-2147483648",
        ] {
            let mutated = int_pattern.replace_all(source, edge_value).to_string();
            if mutated != source {
                mutations.push(mutated);
            }
        }

        // 2. Replace float literals with edge cases
        let float_pattern = regex::Regex::new(r"\b(\d+\.\d+)\b").unwrap();
        for edge_value in [
            "0.0",
            "1.0",
            "-1.0",
            "0.1",
            "1e-10",
            "1e10",
            "1.7976931348623157e308",
        ] {
            let mutated = float_pattern.replace_all(source, edge_value).to_string();
            if mutated != source {
                mutations.push(mutated);
            }
        }

        // 3. Swap comparison operators
        let ops = [("<", ">"), ("<=", ">="), ("==", "!=")];
        for (from, to) in &ops {
            let mutated = source.replace(from, to);
            if mutated != source {
                mutations.push(mutated);
            }
        }

        // 4. Change arithmetic operators
        let arith_ops = [("+", "-"), ("*", "/"), ("%", "/")];
        for (from, to) in &arith_ops {
            // Only replace once to avoid invalid code
            if let Some(pos) = source.find(from) {
                let mut mutated = source.to_string();
                mutated.replace_range(pos..pos + from.len(), to);
                if mutated != source {
                    mutations.push(mutated);
                }
            }
        }

        // 5. Replace string literals with edge cases
        let string_pattern = regex::Regex::new(r#""([^"]*)""#).unwrap();
        for edge_value in [
            "",
            " ",
            "\n",
            "\t",
            "\u{0}",
            "\u{FEFF}",
            "Hello, \u{4E16}\u{754C}!",
        ] {
            let escaped = format!("\"{}\"", edge_value);
            let mutated = string_pattern.replace(source, &escaped).to_string();
            if mutated != source {
                mutations.push(mutated);
            }
        }

        // 6. Add/remove empty iterations
        if source.contains("for ") || source.contains("while ") {
            // Try with 0 iterations
            let mutated = source.replace("1..10", "0..0").replace("0..5", "0..0");
            if mutated != source {
                mutations.push(mutated);
            }
        }

        // 7. Replace true/false
        let bool_mutations = [
            source.replace("true", "false"),
            source.replace("false", "true"),
        ];
        for m in bool_mutations {
            if m != source {
                mutations.push(m);
            }
        }

        mutations
    }

    /// Infer expected behavior from divergence
    fn infer_expectation(&self, divergence: &Divergence) -> TestExpectation {
        match divergence.classification {
            DivergenceClass::Crash => {
                // Determine which tier crashes
                if !divergence.execution1.success {
                    TestExpectation::ExpectCrash {
                        tier: divergence.tier1,
                    }
                } else {
                    TestExpectation::ExpectCrash {
                        tier: divergence.tier2,
                    }
                }
            }
            DivergenceClass::ExitCode => {
                // Both should match the tier1 exit code
                if let Some(code) = divergence.execution1.exit_code {
                    TestExpectation::ExitCode {
                        tier: divergence.tier1,
                        code,
                    }
                } else {
                    TestExpectation::IdenticalOutput
                }
            }
            DivergenceClass::FloatPrecision => TestExpectation::SemanticEquivalence,
            DivergenceClass::Ordering => TestExpectation::SemanticEquivalence,
            _ => TestExpectation::IdenticalOutput,
        }
    }

    /// Write generated test to file
    pub fn write_test(&self, test: &GeneratedTest) -> Result<PathBuf> {
        // Create category subdirectory
        let category_dir = self.config.output_dir.join(test.category.dir_name());
        fs::create_dir_all(&category_dir)?;

        let filename = format!("{}.vr", test.name);
        let path = category_dir.join(&filename);

        let mut file = File::create(&path)?;

        // Write annotations
        if self.config.add_annotations {
            writeln!(file, "// @test: differential")?;
            writeln!(
                file,
                "// @tier: {}",
                test.tiers
                    .iter()
                    .map(|t| match t {
                        Tier::Interpreter => "0",
                        Tier::Bytecode => "1",
                        Tier::Jit => "2",
                        Tier::Aot => "3",
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
            writeln!(file, "// @level: L1")?;
            writeln!(file, "// @tags: {}", test.tags.join(", "))?;
            writeln!(file)?;
        }

        // Write description
        writeln!(file, "/// {}", test.description)?;
        writeln!(
            file,
            "/// Generated from divergence: {}",
            test.divergence_id
        )?;
        writeln!(file)?;

        // Write expectation comment
        match &test.expectation {
            TestExpectation::IdenticalOutput => {
                writeln!(file, "// Expectation: All tiers produce identical output")?;
            }
            TestExpectation::SemanticEquivalence => {
                writeln!(
                    file,
                    "// Expectation: All tiers produce semantically equivalent output"
                )?;
            }
            TestExpectation::ExpectCrash { tier } => {
                writeln!(file, "// Expectation: {} should crash/panic", tier)?;
            }
            TestExpectation::ExitCode { tier, code } => {
                writeln!(
                    file,
                    "// Expectation: {} should exit with code {}",
                    tier, code
                )?;
            }
            TestExpectation::OutputContains { text } => {
                writeln!(file, "// Expectation: Output should contain '{}'", text)?;
            }
            TestExpectation::Custom { description } => {
                writeln!(file, "// Expectation: {}", description)?;
            }
        }
        writeln!(file)?;

        // Write source code
        write!(file, "{}", test.source)?;

        Ok(path)
    }

    /// Write all tests from a batch of divergences
    pub fn write_batch(&self, divergences: &[Divergence]) -> Result<BatchResult> {
        let mut written = Vec::new();
        let mut failed = Vec::new();

        for divergence in divergences {
            match self.generate_variants(divergence) {
                Ok(tests) => {
                    for test in tests {
                        match self.write_test(&test) {
                            Ok(path) => written.push((test.name.clone(), path)),
                            Err(e) => failed.push((divergence.id.clone(), e.to_string())),
                        }
                    }
                }
                Err(e) => failed.push((divergence.id.clone(), e.to_string())),
            }
        }

        Ok(BatchResult { written, failed })
    }

    /// Generate a test suite summary file
    pub fn write_summary(&self, tests: &[GeneratedTest]) -> Result<PathBuf> {
        let path = self.config.output_dir.join("test_suite.toml");
        let mut file = File::create(&path)?;

        writeln!(file, "# Generated Test Suite")?;
        writeln!(
            file,
            "# Auto-generated from differential testing divergences"
        )?;
        writeln!(file)?;
        writeln!(file, "[suite]")?;
        writeln!(file, "name = \"differential_regression\"")?;
        writeln!(
            file,
            "description = \"Regression tests from differential testing\""
        )?;
        writeln!(file, "test_count = {}", tests.len())?;
        writeln!(file)?;

        // Group by category
        let mut by_category: std::collections::HashMap<TestCategory, Vec<&GeneratedTest>> =
            std::collections::HashMap::new();

        for test in tests {
            by_category.entry(test.category).or_default().push(test);
        }

        for (category, cat_tests) in &by_category {
            writeln!(file, "[[categories]]")?;
            writeln!(file, "name = \"{}\"", category.dir_name())?;
            writeln!(file, "count = {}", cat_tests.len())?;
            writeln!(
                file,
                "tests = [{}]",
                cat_tests
                    .iter()
                    .map(|t| format!("\"{}\"", t.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
            writeln!(file)?;
        }

        Ok(path)
    }
}

/// Result of batch test generation
#[derive(Debug)]
pub struct BatchResult {
    /// Successfully written tests: (name, path)
    pub written: Vec<(String, PathBuf)>,
    /// Failed generations: (divergence_id, error)
    pub failed: Vec<(String, String)>,
}

impl BatchResult {
    pub fn success_count(&self) -> usize {
        self.written.len()
    }

    pub fn failure_count(&self) -> usize {
        self.failed.len()
    }

    pub fn total(&self) -> usize {
        self.success_count() + self.failure_count()
    }
}

/// Fuzzer corpus integration for test generation
pub struct FuzzerCorpusGenerator {
    /// Path to fuzzer corpus directory
    corpus_dir: PathBuf,
    /// Output directory for generated tests
    output_dir: PathBuf,
    /// Categories of interest from corpus
    categories: Vec<TestCategory>,
}

impl FuzzerCorpusGenerator {
    /// Create a new corpus generator
    pub fn new(corpus_dir: PathBuf, output_dir: PathBuf) -> Self {
        Self {
            corpus_dir,
            output_dir,
            categories: vec![],
        }
    }

    /// Filter to specific categories
    pub fn with_categories(mut self, categories: Vec<TestCategory>) -> Self {
        self.categories = categories;
        self
    }

    /// Import tests from fuzzer corpus
    pub fn import_corpus(&self) -> Result<Vec<GeneratedTest>> {
        let mut tests = Vec::new();

        if !self.corpus_dir.exists() {
            return Ok(tests);
        }

        for entry in fs::read_dir(&self.corpus_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map_or(false, |e| e == "vr") {
                let source = fs::read_to_string(&path)?;
                let category = TestCategory::infer(&source);

                // Filter by category if specified
                if !self.categories.is_empty() && !self.categories.contains(&category) {
                    continue;
                }

                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("corpus_{}", tests.len()));

                tests.push(GeneratedTest {
                    name: format!("corpus_{}", name),
                    source,
                    category,
                    divergence_id: format!("corpus:{}", name),
                    tiers: vec![Tier::Interpreter, Tier::Aot],
                    tags: vec!["corpus".to_string(), "fuzzer".to_string()],
                    description: format!("Imported from fuzzer corpus: {}", name),
                    expectation: TestExpectation::IdenticalOutput,
                });
            }
        }

        Ok(tests)
    }

    /// Generate tests from corpus and write to output directory
    pub fn generate(&self) -> Result<BatchResult> {
        let tests = self.import_corpus()?;
        let generator = TestGenerator::new(GeneratorConfig {
            output_dir: self.output_dir.clone(),
            ..Default::default()
        });

        let mut written = Vec::new();
        let mut failed = Vec::new();

        for test in tests {
            match generator.write_test(&test) {
                Ok(path) => written.push((test.name.clone(), path)),
                Err(e) => failed.push((test.name.clone(), e.to_string())),
            }
        }

        Ok(BatchResult { written, failed })
    }
}

/// Edge case test generator
pub struct EdgeCaseGenerator {
    output_dir: PathBuf,
}

impl EdgeCaseGenerator {
    /// Create a new edge case generator
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    /// Generate arithmetic edge case tests
    pub fn generate_arithmetic_edge_cases(&self) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        // Integer overflow edge cases
        let overflow_cases = [
            (
                "max_i64_add",
                "fn main() { print(9223372036854775807 + 1) }",
            ),
            (
                "min_i64_sub",
                "fn main() { print(-9223372036854775808 - 1) }",
            ),
            (
                "max_i64_mul",
                "fn main() { print(9223372036854775807 * 2) }",
            ),
            ("zero_division", "fn main() { print(1 / 0) }"),
            ("modulo_zero", "fn main() { print(5 % 0) }"),
            (
                "neg_overflow",
                "fn main() { print(-(-9223372036854775808)) }",
            ),
        ];

        for (name, source) in overflow_cases {
            tests.push(GeneratedTest {
                name: format!("edge_arithmetic_{}", name),
                source: source.to_string(),
                category: TestCategory::Arithmetic,
                divergence_id: format!("edge:arithmetic:{}", name),
                tiers: vec![Tier::Interpreter, Tier::Aot],
                tags: vec![
                    "edge_case".to_string(),
                    "arithmetic".to_string(),
                    "overflow".to_string(),
                ],
                description: format!("Edge case: {}", name),
                expectation: TestExpectation::IdenticalOutput,
            });
        }

        // Float precision edge cases
        let float_cases = [
            ("float_denormal", "fn main() { print(5e-324) }"),
            (
                "float_min_positive",
                "fn main() { print(2.2250738585072014e-308) }",
            ),
            ("float_max", "fn main() { print(1.7976931348623157e308) }"),
            (
                "float_epsilon",
                "fn main() { print(1.0 + 2.220446049250313e-16) }",
            ),
            ("float_nan_add", "fn main() { print(0.0 / 0.0 + 1.0) }"),
            (
                "float_inf_sub",
                "fn main() { print(1.0 / 0.0 - 1.0 / 0.0) }",
            ),
            ("float_zero_neg", "fn main() { print(-0.0) }"),
            ("float_denormal_arith", "fn main() { print(5e-324 * 2.0) }"),
        ];

        for (name, source) in float_cases {
            tests.push(GeneratedTest {
                name: format!("edge_float_{}", name),
                source: source.to_string(),
                category: TestCategory::FloatPrecision,
                divergence_id: format!("edge:float:{}", name),
                tiers: vec![Tier::Interpreter, Tier::Aot],
                tags: vec!["edge_case".to_string(), "float".to_string()],
                description: format!("Float edge case: {}", name),
                expectation: TestExpectation::SemanticEquivalence,
            });
        }

        tests
    }

    /// Generate string encoding edge cases
    pub fn generate_string_edge_cases(&self) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        let string_cases = [
            ("empty_string", r#"fn main() { print("") }"#),
            ("null_char", r#"fn main() { print("\0") }"#),
            ("bom", r#"fn main() { print("\u{FEFF}") }"#),
            ("replacement_char", r#"fn main() { print("\u{FFFD}") }"#),
            ("emoji", r#"fn main() { print("\u{1F600}") }"#),
            ("combining_char", r#"fn main() { print("e\u{0301}") }"#), // e + combining acute
            (
                "rtl_override",
                r#"fn main() { print("\u{202E}abc\u{202C}") }"#,
            ),
            ("zero_width_joiner", r#"fn main() { print("\u{200D}") }"#),
            ("surrogate_escape", r#"fn main() { print("\u{D800}") }"#),
            ("max_codepoint", r#"fn main() { print("\u{10FFFF}") }"#),
            (
                "long_string",
                &format!(r#"fn main() {{ print("{}") }}"#, "a".repeat(10000)),
            ),
        ];

        for (name, source) in string_cases {
            tests.push(GeneratedTest {
                name: format!("edge_string_{}", name),
                source: source.to_string(),
                category: TestCategory::String,
                divergence_id: format!("edge:string:{}", name),
                tiers: vec![Tier::Interpreter, Tier::Aot],
                tags: vec![
                    "edge_case".to_string(),
                    "string".to_string(),
                    "encoding".to_string(),
                ],
                description: format!("String encoding edge case: {}", name),
                expectation: TestExpectation::IdenticalOutput,
            });
        }

        tests
    }

    /// Generate memory-related edge cases
    pub fn generate_memory_edge_cases(&self) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        // Build deep nesting case dynamically
        let deep_nesting_source = format!(
            "fn main() {{ {} }}",
            (0..100).map(|_| "let x = Box::new(").collect::<String>() + "42" + &")".repeat(100)
        );

        let memory_cases: Vec<(&str, String)> = vec![
            ("deep_nesting", deep_nesting_source),
            ("large_allocation", "fn main() { let arr = List::new(); for i in 0..1000000 { arr.push(i) } print(arr.len()) }".to_string()),
            ("cyclic_reference", "fn main() { struct Node { next: Maybe<Shared<Node>> }; let n = Shared::new(Node { next: None }); n.next = Some(n.clone()); }".to_string()),
        ];

        for (name, source) in memory_cases {
            tests.push(GeneratedTest {
                name: format!("edge_memory_{}", name),
                source,
                category: TestCategory::Memory,
                divergence_id: format!("edge:memory:{}", name),
                tiers: vec![Tier::Interpreter, Tier::Aot],
                tags: vec!["edge_case".to_string(), "memory".to_string()],
                description: format!("Memory edge case: {}", name),
                expectation: TestExpectation::IdenticalOutput,
            });
        }

        tests
    }

    /// Generate all edge case tests
    pub fn generate_all(&self) -> Vec<GeneratedTest> {
        let mut all = Vec::new();
        all.extend(self.generate_arithmetic_edge_cases());
        all.extend(self.generate_string_edge_cases());
        all.extend(self.generate_memory_edge_cases());
        all
    }

    /// Write all edge case tests to output directory
    pub fn write_all(&self) -> Result<BatchResult> {
        let tests = self.generate_all();
        let generator = TestGenerator::new(GeneratorConfig {
            output_dir: self.output_dir.clone(),
            ..Default::default()
        });

        let mut written = Vec::new();
        let mut failed = Vec::new();

        for test in tests {
            match generator.write_test(&test) {
                Ok(path) => written.push((test.name.clone(), path)),
                Err(e) => failed.push((test.name.clone(), e.to_string())),
            }
        }

        Ok(BatchResult { written, failed })
    }
}

/// Stress test generator for performance and memory testing
pub struct StressTestGenerator {
    output_dir: PathBuf,
    /// Maximum iterations for stress loops
    max_iterations: usize,
    /// Maximum depth for recursive structures
    max_depth: usize,
}

impl StressTestGenerator {
    /// Create a new stress test generator
    pub fn new(output_dir: PathBuf) -> Self {
        Self {
            output_dir,
            max_iterations: 100000,
            max_depth: 1000,
        }
    }

    /// Set maximum iterations
    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    /// Set maximum depth
    pub fn with_max_depth(mut self, max: usize) -> Self {
        self.max_depth = max;
        self
    }

    /// Generate loop stress tests
    pub fn generate_loop_stress(&self) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        // Simple counting loop
        tests.push(GeneratedTest {
            name: "stress_loop_count".to_string(),
            source: format!(
                "fn main() {{ let mut sum = 0; for i in 0..{} {{ sum = sum + 1 }}; print(sum) }}",
                self.max_iterations
            ),
            category: TestCategory::ControlFlow,
            divergence_id: "stress:loop:count".to_string(),
            tiers: vec![Tier::Interpreter, Tier::Aot],
            tags: vec![
                "stress".to_string(),
                "loop".to_string(),
                "performance".to_string(),
            ],
            description: format!("Stress test: {} iterations", self.max_iterations),
            expectation: TestExpectation::IdenticalOutput,
        });

        // Nested loop
        let nested_size = (self.max_iterations as f64).sqrt() as usize;
        tests.push(GeneratedTest {
            name: "stress_loop_nested".to_string(),
            source: format!(
                "fn main() {{ let mut sum = 0; for i in 0..{} {{ for j in 0..{} {{ sum = sum + 1 }} }}; print(sum) }}",
                nested_size, nested_size
            ),
            category: TestCategory::ControlFlow,
            divergence_id: "stress:loop:nested".to_string(),
            tiers: vec![Tier::Interpreter, Tier::Aot],
            tags: vec!["stress".to_string(), "loop".to_string(), "nested".to_string()],
            description: format!("Stress test: {}x{} nested iterations", nested_size, nested_size),
            expectation: TestExpectation::IdenticalOutput,
        });

        tests
    }

    /// Generate recursion stress tests
    pub fn generate_recursion_stress(&self) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        // Deep recursion (might hit stack limits)
        tests.push(GeneratedTest {
            name: "stress_recursion_deep".to_string(),
            source: format!(
                "fn recurse(n: Int) -> Int {{ if n <= 0 {{ 0 }} else {{ 1 + recurse(n - 1) }} }}\nfn main() {{ print(recurse({})) }}",
                self.max_depth
            ),
            category: TestCategory::ControlFlow,
            divergence_id: "stress:recursion:deep".to_string(),
            tiers: vec![Tier::Interpreter, Tier::Aot],
            tags: vec!["stress".to_string(), "recursion".to_string()],
            description: format!("Stress test: {} recursion depth", self.max_depth),
            expectation: TestExpectation::IdenticalOutput,
        });

        // Tail recursion (should be optimized)
        tests.push(GeneratedTest {
            name: "stress_recursion_tail".to_string(),
            source: format!(
                "fn count(n: Int, acc: Int) -> Int {{ if n <= 0 {{ acc }} else {{ count(n - 1, acc + 1) }} }}\nfn main() {{ print(count({}, 0)) }}",
                self.max_iterations
            ),
            category: TestCategory::ControlFlow,
            divergence_id: "stress:recursion:tail".to_string(),
            tiers: vec![Tier::Interpreter, Tier::Aot],
            tags: vec!["stress".to_string(), "recursion".to_string(), "tail_call".to_string()],
            description: format!("Stress test: {} tail recursive calls", self.max_iterations),
            expectation: TestExpectation::IdenticalOutput,
        });

        tests
    }

    /// Generate allocation stress tests
    pub fn generate_allocation_stress(&self) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        // Many small allocations
        tests.push(GeneratedTest {
            name: "stress_alloc_many_small".to_string(),
            source: format!(
                "fn main() {{ let list = List::new(); for i in 0..{} {{ list.push(i) }}; print(list.len()) }}",
                self.max_iterations
            ),
            category: TestCategory::Memory,
            divergence_id: "stress:alloc:many_small".to_string(),
            tiers: vec![Tier::Interpreter, Tier::Aot],
            tags: vec!["stress".to_string(), "allocation".to_string(), "memory".to_string()],
            description: format!("Stress test: {} allocations", self.max_iterations),
            expectation: TestExpectation::IdenticalOutput,
        });

        // Churn (allocate and deallocate)
        tests.push(GeneratedTest {
            name: "stress_alloc_churn".to_string(),
            source: format!(
                "fn main() {{ for i in 0..{} {{ let v = List::with_capacity(100); for j in 0..100 {{ v.push(j) }} }}; print(\"done\") }}",
                self.max_iterations / 100
            ),
            category: TestCategory::Memory,
            divergence_id: "stress:alloc:churn".to_string(),
            tiers: vec![Tier::Interpreter, Tier::Aot],
            tags: vec!["stress".to_string(), "allocation".to_string(), "gc".to_string()],
            description: "Stress test: allocation churn".to_string(),
            expectation: TestExpectation::IdenticalOutput,
        });

        tests
    }

    /// Generate all stress tests
    pub fn generate_all(&self) -> Vec<GeneratedTest> {
        let mut all = Vec::new();
        all.extend(self.generate_loop_stress());
        all.extend(self.generate_recursion_stress());
        all.extend(self.generate_allocation_stress());
        all
    }

    /// Write all stress tests to output directory
    pub fn write_all(&self) -> Result<BatchResult> {
        let tests = self.generate_all();
        let generator = TestGenerator::new(GeneratorConfig {
            output_dir: self.output_dir.clone(),
            ..Default::default()
        });

        let mut written = Vec::new();
        let mut failed = Vec::new();

        for test in tests {
            match generator.write_test(&test) {
                Ok(path) => written.push((test.name.clone(), path)),
                Err(e) => failed.push((test.name.clone(), e.to_string())),
            }
        }

        Ok(BatchResult { written, failed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_inference() {
        assert_eq!(
            TestCategory::infer("async fn test() { await foo(); }"),
            TestCategory::Async
        );

        assert_eq!(
            TestCategory::infer("let list = List::new();"),
            TestCategory::Collection
        );

        assert_eq!(
            TestCategory::infer("let s = \"hello\".to_text();"),
            TestCategory::String
        );

        assert_eq!(
            TestCategory::infer("let b = Box::new(42);"),
            TestCategory::Memory
        );

        assert_eq!(
            TestCategory::infer("let x = 1 + 2 * 3;"),
            TestCategory::Arithmetic
        );
    }

    #[test]
    fn test_category_dir_name() {
        assert_eq!(TestCategory::Arithmetic.dir_name(), "arithmetic");
        assert_eq!(TestCategory::Async.dir_name(), "async");
        assert_eq!(TestCategory::Memory.dir_name(), "memory");
    }

    #[test]
    fn test_mutations() {
        let generator = TestGenerator::new(GeneratorConfig::default());
        let source = "let x = 42; let y = 1.5; if true { x + y }";

        let mutations = generator.generate_mutations(source);
        assert!(!mutations.is_empty());

        // Should contain integer edge cases
        assert!(
            mutations
                .iter()
                .any(|m| m.contains("0") && !m.contains("42"))
        );

        // Should contain boolean swap
        assert!(mutations.iter().any(|m| m.contains("false")));
    }

    #[test]
    fn test_batch_result() {
        let result = BatchResult {
            written: vec![("test1".to_string(), PathBuf::from("test1.vr"))],
            failed: vec![("div1".to_string(), "error".to_string())],
        };

        assert_eq!(result.success_count(), 1);
        assert_eq!(result.failure_count(), 1);
        assert_eq!(result.total(), 2);
    }
}
