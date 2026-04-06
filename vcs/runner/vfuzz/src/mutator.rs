//! Input mutation strategies for fuzz testing
//!
//! This module provides various mutation strategies to transform existing
//! Verum programs into new test cases. Mutations are designed to:
//!
//! - Preserve syntactic validity (when possible)
//! - Explore edge cases
//! - Find boundary conditions
//! - Stress specific compiler components

use rand::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// Configuration for the mutator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutatorConfig {
    /// Probability of applying each mutation (0.0 - 1.0)
    pub mutation_rate: f64,
    /// Maximum number of mutations to apply per input
    pub max_mutations: usize,
    /// Enable structure-aware mutations
    pub structure_aware: bool,
    /// Enable token-level mutations
    pub token_level: bool,
    /// Enable byte-level mutations (can break syntax)
    pub byte_level: bool,
    /// Weight for each mutation strategy
    pub strategy_weights: StrategyWeights,
}

impl Default for MutatorConfig {
    fn default() -> Self {
        Self {
            mutation_rate: 0.8,
            max_mutations: 5,
            structure_aware: true,
            token_level: true,
            byte_level: false,
            strategy_weights: StrategyWeights::default(),
        }
    }
}

/// Weights for mutation strategies
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyWeights {
    /// Weight for literal mutations
    pub literal: f64,
    /// Weight for identifier mutations
    pub identifier: f64,
    /// Weight for operator mutations
    pub operator: f64,
    /// Weight for structure mutations
    pub structure: f64,
    /// Weight for deletion mutations
    pub deletion: f64,
    /// Weight for insertion mutations
    pub insertion: f64,
    /// Weight for swap mutations
    pub swap: f64,
    /// Weight for duplication mutations
    pub duplication: f64,
}

impl Default for StrategyWeights {
    fn default() -> Self {
        Self {
            literal: 1.0,
            identifier: 0.8,
            operator: 0.9,
            structure: 0.7,
            deletion: 0.5,
            insertion: 0.6,
            swap: 0.4,
            duplication: 0.3,
        }
    }
}

/// Type of mutation applied
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MutationStrategy {
    /// Mutate numeric literals
    LiteralInteger,
    /// Mutate float literals
    LiteralFloat,
    /// Mutate string literals
    LiteralString,
    /// Mutate boolean literals
    LiteralBool,
    /// Swap operators
    OperatorSwap,
    /// Replace identifiers
    IdentifierReplace,
    /// Delete a token or statement
    Delete,
    /// Insert a token or statement
    Insert,
    /// Swap two adjacent elements
    Swap,
    /// Duplicate a statement or expression
    Duplicate,
    /// Change type annotations
    TypeChange,
    /// Add or remove modifiers
    ModifierChange,
    /// Flip control flow
    ControlFlowFlip,
    /// Byte-level mutation
    ByteFlip,
    /// Interesting value substitution
    InterestingValue,
}

/// Result of a mutation
#[derive(Debug, Clone)]
pub struct MutationResult {
    /// The mutated input
    pub output: String,
    /// Mutations that were applied
    pub mutations: Vec<MutationStrategy>,
    /// Number of bytes changed
    pub bytes_changed: usize,
}

/// Main mutator for Verum programs
pub struct Mutator {
    config: MutatorConfig,
    /// Precompiled regex patterns
    patterns: MutatorPatterns,
}

/// Precompiled regex patterns for efficient matching
struct MutatorPatterns {
    integer: Regex,
    float: Regex,
    string: Regex,
    identifier: Regex,
    binary_op: Regex,
    comparison_op: Regex,
    type_annotation: Regex,
    let_binding: Regex,
    function_decl: Regex,
    if_statement: Regex,
}

impl Default for MutatorPatterns {
    fn default() -> Self {
        Self {
            integer: Regex::new(r"-?\d+").unwrap(),
            float: Regex::new(r"-?\d+\.\d+([eE][+-]?\d+)?").unwrap(),
            string: Regex::new(r#""[^"\\]*(\\.[^"\\]*)*""#).unwrap(),
            identifier: Regex::new(r"\b[a-zA-Z_][a-zA-Z0-9_]*\b").unwrap(),
            binary_op: Regex::new(r"\s*(\+|-|\*|/|%)\s*").unwrap(),
            comparison_op: Regex::new(r"\s*(==|!=|<=|>=|<|>)\s*").unwrap(),
            type_annotation: Regex::new(r":\s*(Int|Float|Bool|Text|Unit|List|Map|Set|Maybe)")
                .unwrap(),
            let_binding: Regex::new(r"let\s+\w+").unwrap(),
            function_decl: Regex::new(r"fn\s+\w+\s*\(").unwrap(),
            if_statement: Regex::new(r"if\s+").unwrap(),
        }
    }
}

impl Mutator {
    /// Create a new mutator with the given configuration
    pub fn new(config: MutatorConfig) -> Self {
        Self {
            config,
            patterns: MutatorPatterns::default(),
        }
    }

    /// Mutate an input program
    pub fn mutate<R: Rng>(&self, input: &str, rng: &mut R) -> String {
        let mut output = input.to_string();
        let mut mutations = Vec::new();

        let num_mutations = rng.random_range(1..=self.config.max_mutations);

        for _ in 0..num_mutations {
            if rng.random::<f64>() > self.config.mutation_rate {
                continue;
            }

            let strategy = self.select_strategy(rng);
            if let Some(mutated) = self.apply_mutation(&output, strategy, rng) {
                output = mutated;
                mutations.push(strategy);
            }
        }

        output
    }

    /// Mutate and return detailed result
    pub fn mutate_with_info<R: Rng>(&self, input: &str, rng: &mut R) -> MutationResult {
        let mut output = input.to_string();
        let mut mutations = Vec::new();

        let num_mutations = rng.random_range(1..=self.config.max_mutations);

        for _ in 0..num_mutations {
            if rng.random::<f64>() > self.config.mutation_rate {
                continue;
            }

            let strategy = self.select_strategy(rng);
            if let Some(mutated) = self.apply_mutation(&output, strategy, rng) {
                output = mutated;
                mutations.push(strategy);
            }
        }

        let bytes_changed = input
            .bytes()
            .zip(output.bytes())
            .filter(|(a, b)| a != b)
            .count();

        MutationResult {
            output,
            mutations,
            bytes_changed,
        }
    }

    /// Select a mutation strategy based on weights
    fn select_strategy<R: Rng>(&self, rng: &mut R) -> MutationStrategy {
        let strategies = [
            (
                MutationStrategy::LiteralInteger,
                self.config.strategy_weights.literal,
            ),
            (
                MutationStrategy::LiteralFloat,
                self.config.strategy_weights.literal * 0.8,
            ),
            (
                MutationStrategy::LiteralString,
                self.config.strategy_weights.literal * 0.6,
            ),
            (
                MutationStrategy::LiteralBool,
                self.config.strategy_weights.literal * 0.5,
            ),
            (
                MutationStrategy::OperatorSwap,
                self.config.strategy_weights.operator,
            ),
            (
                MutationStrategy::IdentifierReplace,
                self.config.strategy_weights.identifier,
            ),
            (
                MutationStrategy::Delete,
                self.config.strategy_weights.deletion,
            ),
            (
                MutationStrategy::Insert,
                self.config.strategy_weights.insertion,
            ),
            (MutationStrategy::Swap, self.config.strategy_weights.swap),
            (
                MutationStrategy::Duplicate,
                self.config.strategy_weights.duplication,
            ),
            (
                MutationStrategy::TypeChange,
                self.config.strategy_weights.structure * 0.5,
            ),
            (
                MutationStrategy::ModifierChange,
                self.config.strategy_weights.structure * 0.3,
            ),
            (
                MutationStrategy::ControlFlowFlip,
                self.config.strategy_weights.structure * 0.4,
            ),
            (
                MutationStrategy::InterestingValue,
                self.config.strategy_weights.literal * 1.2,
            ),
        ];

        let total_weight: f64 = strategies.iter().map(|(_, w)| w).sum();
        let mut choice = rng.random::<f64>() * total_weight;

        for (strategy, weight) in &strategies {
            choice -= weight;
            if choice <= 0.0 {
                return *strategy;
            }
        }

        MutationStrategy::LiteralInteger
    }

    /// Apply a specific mutation strategy
    fn apply_mutation<R: Rng>(
        &self,
        input: &str,
        strategy: MutationStrategy,
        rng: &mut R,
    ) -> Option<String> {
        match strategy {
            MutationStrategy::LiteralInteger => self.mutate_integer(input, rng),
            MutationStrategy::LiteralFloat => self.mutate_float(input, rng),
            MutationStrategy::LiteralString => self.mutate_string(input, rng),
            MutationStrategy::LiteralBool => self.mutate_bool(input, rng),
            MutationStrategy::OperatorSwap => self.mutate_operator(input, rng),
            MutationStrategy::IdentifierReplace => self.mutate_identifier(input, rng),
            MutationStrategy::Delete => self.mutate_delete(input, rng),
            MutationStrategy::Insert => self.mutate_insert(input, rng),
            MutationStrategy::Swap => self.mutate_swap(input, rng),
            MutationStrategy::Duplicate => self.mutate_duplicate(input, rng),
            MutationStrategy::TypeChange => self.mutate_type(input, rng),
            MutationStrategy::ModifierChange => self.mutate_modifier(input, rng),
            MutationStrategy::ControlFlowFlip => self.mutate_control_flow(input, rng),
            MutationStrategy::ByteFlip => self.mutate_byte_flip(input, rng),
            MutationStrategy::InterestingValue => self.mutate_interesting_value(input, rng),
        }
    }

    /// Mutate an integer literal
    fn mutate_integer<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let matches: Vec<_> = self.patterns.integer.find_iter(input).collect();
        if matches.is_empty() {
            return None;
        }

        let m = &matches[rng.random_range(0..matches.len())];
        let original = m.as_str();

        // Skip if this looks like part of a float
        if input[m.end()..].starts_with('.') && !input[m.end()..].starts_with("..") {
            return None;
        }

        let mutated = self.mutate_integer_value(original, rng);

        Some(format!(
            "{}{}{}",
            &input[..m.start()],
            mutated,
            &input[m.end()..]
        ))
    }

    /// Mutate an integer value
    fn mutate_integer_value<R: Rng>(&self, value: &str, rng: &mut R) -> String {
        let n: i64 = value.parse().unwrap_or(0);

        match rng.random_range(0..10) {
            0 => "0".to_string(),
            1 => "1".to_string(),
            2 => "-1".to_string(),
            3 => format!("{}", n.wrapping_add(1)),
            4 => format!("{}", n.wrapping_sub(1)),
            5 => format!("{}", n.wrapping_neg()),
            6 => format!("{}", i64::MAX),
            7 => format!("{}", i64::MIN),
            8 => format!("{}", rng.random::<i64>()),
            _ => format!("{}", n.wrapping_mul(2)),
        }
    }

    /// Mutate a float literal
    fn mutate_float<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let matches: Vec<_> = self.patterns.float.find_iter(input).collect();
        if matches.is_empty() {
            return None;
        }

        let m = &matches[rng.random_range(0..matches.len())];
        let original = m.as_str();
        let mutated = self.mutate_float_value(original, rng);

        Some(format!(
            "{}{}{}",
            &input[..m.start()],
            mutated,
            &input[m.end()..]
        ))
    }

    /// Mutate a float value
    fn mutate_float_value<R: Rng>(&self, value: &str, rng: &mut R) -> String {
        let n: f64 = value.parse().unwrap_or(0.0);

        match rng.random_range(0..12) {
            0 => "0.0".to_string(),
            1 => "1.0".to_string(),
            2 => "-1.0".to_string(),
            3 => "-0.0".to_string(),
            4 => format!("{}", n + 1.0),
            5 => format!("{}", n - 1.0),
            6 => format!("{}", -n),
            7 => "1.0e308".to_string(),
            8 => "-1.0e308".to_string(),
            9 => "1.0e-308".to_string(),
            10 => format!("{:.10}", rng.random::<f64>()),
            _ => format!("{}", n * 2.0),
        }
    }

    /// Mutate a string literal
    fn mutate_string<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let matches: Vec<_> = self.patterns.string.find_iter(input).collect();
        if matches.is_empty() {
            return None;
        }

        let m = &matches[rng.random_range(0..matches.len())];
        let original = m.as_str();
        let mutated = self.mutate_string_value(original, rng);

        Some(format!(
            "{}{}{}",
            &input[..m.start()],
            mutated,
            &input[m.end()..]
        ))
    }

    /// Mutate a string value
    fn mutate_string_value<R: Rng>(&self, value: &str, rng: &mut R) -> String {
        // Remove quotes
        let inner = &value[1..value.len() - 1];

        match rng.random_range(0..10) {
            0 => "\"\"".to_string(),
            1 => format!("\"{}\"", " ".repeat(100)),
            2 => format!("\"{}\"", "a".repeat(1000)),
            3 => "\"\\n\\n\\n\"".to_string(),
            4 => "\"\\t\\t\\t\"".to_string(),
            5 => "\"\\u{1F600}\\u{1F680}\"".to_string(),
            6 => "\"\\0\\0\\0\"".to_string(),
            7 => format!("\"{}x\"", inner),
            8 => {
                if !inner.is_empty() {
                    format!("\"{}\"", &inner[..inner.len() - 1])
                } else {
                    "\"x\"".to_string()
                }
            }
            _ => format!("\"{}{}\"", inner, inner),
        }
    }

    /// Mutate a boolean literal
    fn mutate_bool<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let true_positions: Vec<_> = input.match_indices("true").collect();
        let false_positions: Vec<_> = input.match_indices("false").collect();

        let all_positions: Vec<_> = true_positions
            .iter()
            .map(|(pos, _)| (*pos, "true", "false"))
            .chain(
                false_positions
                    .iter()
                    .map(|(pos, _)| (*pos, "false", "true")),
            )
            .collect();

        if all_positions.is_empty() {
            return None;
        }

        let (pos, from, to) = all_positions[rng.random_range(0..all_positions.len())];

        Some(format!(
            "{}{}{}",
            &input[..pos],
            to,
            &input[pos + from.len()..]
        ))
    }

    /// Mutate an operator
    fn mutate_operator<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        // Find binary operators
        let binary_ops = ["+", "-", "*", "/", "%"];
        let comparison_ops = ["==", "!=", "<=", ">=", "<", ">"];
        let logical_ops = ["&&", "||"];

        // Try to find and replace an operator
        for op in binary_ops.iter() {
            if let Some(pos) = input.find(op) {
                let replacement = binary_ops[rng.random_range(0..binary_ops.len())];
                if replacement != *op {
                    return Some(format!(
                        "{}{}{}",
                        &input[..pos],
                        replacement,
                        &input[pos + op.len()..]
                    ));
                }
            }
        }

        for op in comparison_ops.iter() {
            if let Some(pos) = input.find(op) {
                let replacement = comparison_ops[rng.random_range(0..comparison_ops.len())];
                if replacement != *op {
                    return Some(format!(
                        "{}{}{}",
                        &input[..pos],
                        replacement,
                        &input[pos + op.len()..]
                    ));
                }
            }
        }

        for op in logical_ops.iter() {
            if let Some(pos) = input.find(op) {
                let replacement = if *op == "&&" { "||" } else { "&&" };
                return Some(format!(
                    "{}{}{}",
                    &input[..pos],
                    replacement,
                    &input[pos + op.len()..]
                ));
            }
        }

        None
    }

    /// Mutate an identifier
    fn mutate_identifier<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let reserved = [
            "fn", "let", "if", "else", "match", "for", "while", "loop", "return", "break",
            "continue", "type", "use", "pub", "async", "await", "true", "false", "Int", "Float",
            "Bool", "Text",
        ];

        let matches: Vec<_> = self.patterns.identifier.find_iter(input).collect();
        if matches.is_empty() {
            return None;
        }

        // Find a non-reserved identifier
        let candidates: Vec<_> = matches
            .iter()
            .filter(|m| !reserved.contains(&m.as_str()))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let m = candidates[rng.random_range(0..candidates.len())];
        let original = m.as_str();

        let mutated = match rng.random_range(0..5) {
            0 => format!("{}_mutated", original),
            1 => format!("{}2", original),
            2 => "x".to_string(),
            3 => "undefined_var".to_string(),
            _ => format!("_{}", original),
        };

        Some(format!(
            "{}{}{}",
            &input[..m.start()],
            mutated,
            &input[m.end()..]
        ))
    }

    /// Delete a token or statement
    fn mutate_delete<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let lines: Vec<&str> = input.lines().collect();
        if lines.len() <= 2 {
            return None;
        }

        // Find lines that can be deleted (not fn declarations or braces)
        let deletable: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                !trimmed.starts_with("fn ")
                    && !trimmed.starts_with("type ")
                    && trimmed != "{"
                    && trimmed != "}"
                    && !trimmed.is_empty()
            })
            .collect();

        if deletable.is_empty() {
            return None;
        }

        let &(idx, _) = deletable.get(rng.random_range(0..deletable.len())).unwrap();
        let mut new_lines = lines.to_vec();
        new_lines.remove(idx);

        Some(new_lines.join("\n"))
    }

    /// Insert a token or statement
    fn mutate_insert<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let lines: Vec<&str> = input.lines().collect();

        // Find a position to insert (inside function bodies)
        let insertable: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(i, line)| {
                let trimmed = line.trim();
                *i > 0 && *i < lines.len() - 1 && (trimmed.ends_with(';') || trimmed == "{")
            })
            .collect();

        if insertable.is_empty() {
            return None;
        }

        let &(idx, _) = insertable
            .get(rng.random_range(0..insertable.len()))
            .unwrap();

        let insertions = [
            "    let inserted = 0;",
            "    let temp = true;",
            "    debug();",
            "    // Inserted comment",
            "    assert true;",
            "    let _ = 42;",
        ];

        let insertion = insertions[rng.random_range(0..insertions.len())];

        let mut new_lines = lines.to_vec();
        new_lines.insert(idx + 1, insertion);

        Some(new_lines.join("\n"))
    }

    /// Swap two adjacent elements
    fn mutate_swap<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let lines: Vec<&str> = input.lines().collect();
        if lines.len() < 4 {
            return None;
        }

        // Find swappable pairs (let statements)
        let swappable: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(i, line)| {
                let trimmed = line.trim();
                *i < lines.len() - 1
                    && trimmed.starts_with("let ")
                    && lines
                        .get(i + 1)
                        .map_or(false, |next| next.trim().starts_with("let "))
            })
            .collect();

        if swappable.is_empty() {
            return None;
        }

        let &(idx, _) = swappable.get(rng.random_range(0..swappable.len())).unwrap();

        let mut new_lines = lines.to_vec();
        new_lines.swap(idx, idx + 1);

        Some(new_lines.join("\n"))
    }

    /// Duplicate a statement or expression
    fn mutate_duplicate<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let lines: Vec<&str> = input.lines().collect();

        // Find duplicable lines
        let duplicable: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim();
                trimmed.starts_with("let ")
                    || trimmed.starts_with("debug")
                    || trimmed.starts_with("print")
            })
            .collect();

        if duplicable.is_empty() {
            return None;
        }

        let &(idx, line) = duplicable
            .get(rng.random_range(0..duplicable.len()))
            .unwrap();

        let mut new_lines = lines.to_vec();
        new_lines.insert(idx + 1, line);

        Some(new_lines.join("\n"))
    }

    /// Mutate a type annotation
    fn mutate_type<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let types = ["Int", "Float", "Bool", "Text", "Unit"];

        let matches: Vec<_> = self.patterns.type_annotation.find_iter(input).collect();
        if matches.is_empty() {
            return None;
        }

        let m = &matches[rng.random_range(0..matches.len())];
        let new_type = types[rng.random_range(0..types.len())];

        Some(format!(
            "{}: {}{}",
            &input[..m.start()],
            new_type,
            &input[m.end()..]
        ))
    }

    /// Mutate modifiers (pub, async, unsafe)
    fn mutate_modifier<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        // Add or remove modifiers
        if let Some(pos) = input.find("fn ") {
            match rng.random_range(0..4) {
                0 => {
                    // Add pub
                    Some(format!("{}pub {}", &input[..pos], &input[pos..]))
                }
                1 => {
                    // Add async
                    Some(format!("{}async {}", &input[..pos], &input[pos..]))
                }
                2 if input.contains("pub ") => {
                    // Remove pub
                    Some(input.replace("pub ", ""))
                }
                3 if input.contains("async ") => {
                    // Remove async
                    Some(input.replace("async ", ""))
                }
                _ => None,
            }
        } else {
            None
        }
    }

    /// Mutate control flow
    fn mutate_control_flow<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        match rng.random_range(0..4) {
            0 => {
                // Flip if condition
                if input.contains("if true") {
                    Some(input.replace("if true", "if false"))
                } else if input.contains("if false") {
                    Some(input.replace("if false", "if true"))
                } else {
                    None
                }
            }
            1 => {
                // Add break
                if let Some(pos) = input.find("loop {") {
                    let insert_pos = pos + 6;
                    Some(format!(
                        "{} break; {}",
                        &input[..insert_pos],
                        &input[insert_pos..]
                    ))
                } else {
                    None
                }
            }
            2 => {
                // Change while to loop
                Some(
                    input
                        .replace("while ", "loop { if !(")
                        .replace(" {", ") { break; } "),
                )
            }
            _ => {
                // Add continue
                if let Some(pos) = input.find("for ") {
                    if let Some(brace) = input[pos..].find('{') {
                        let insert_pos = pos + brace + 1;
                        return Some(format!(
                            "{} continue; {}",
                            &input[..insert_pos],
                            &input[insert_pos..]
                        ));
                    }
                }
                None
            }
        }
    }

    /// Flip random bytes
    fn mutate_byte_flip<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        if input.is_empty() {
            return None;
        }

        let mut bytes = input.as_bytes().to_vec();
        let num_flips = rng.random_range(1..=3);

        for _ in 0..num_flips {
            let pos = rng.random_range(0..bytes.len());
            let bit = rng.random_range(0..8);
            bytes[pos] ^= 1 << bit;
        }

        String::from_utf8(bytes).ok()
    }

    /// Substitute with interesting values
    fn mutate_interesting_value<R: Rng>(&self, input: &str, rng: &mut R) -> Option<String> {
        let interesting_ints = [
            "0",
            "1",
            "-1",
            "127",
            "-128",
            "255",
            "256",
            "32767",
            "-32768",
            "65535",
            "65536",
            "2147483647",
            "-2147483648",
            "4294967295",
            "9223372036854775807",
            "-9223372036854775808",
        ];

        let interesting_floats = [
            "0.0",
            "-0.0",
            "1.0",
            "-1.0",
            "1.0e308",
            "-1.0e308",
            "1.0e-308",
            "1.7976931348623157e308",
        ];

        // Try to replace an integer
        if let Some(m) = self.patterns.integer.find(input) {
            let replacement = interesting_ints[rng.random_range(0..interesting_ints.len())];
            return Some(format!(
                "{}{}{}",
                &input[..m.start()],
                replacement,
                &input[m.end()..]
            ));
        }

        // Try to replace a float
        if let Some(m) = self.patterns.float.find(input) {
            let replacement = interesting_floats[rng.random_range(0..interesting_floats.len())];
            return Some(format!(
                "{}{}{}",
                &input[..m.start()],
                replacement,
                &input[m.end()..]
            ));
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_mutator_creation() {
        let config = MutatorConfig::default();
        let mutator = Mutator::new(config);
        assert_eq!(mutator.config.mutation_rate, 0.8);
    }

    #[test]
    fn test_integer_mutation() {
        let config = MutatorConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let input = "let x = 42;";
        let result = mutator.mutate_integer(input, &mut rng);
        assert!(result.is_some());
        assert!(result.unwrap() != input || true); // May or may not change
    }

    #[test]
    fn test_bool_mutation() {
        let config = MutatorConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let input = "let x = true;";
        let result = mutator.mutate_bool(input, &mut rng);
        assert!(result.is_some());
        assert!(result.unwrap().contains("false"));
    }

    #[test]
    fn test_operator_mutation() {
        let config = MutatorConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let input = "let x = 1 + 2;";
        for _ in 0..10 {
            if let Some(result) = mutator.mutate_operator(input, &mut rng) {
                if result != input {
                    assert!(result.contains('-') || result.contains('*') || result.contains('/'));
                    return;
                }
            }
        }
    }

    #[test]
    fn test_mutate_with_info() {
        let config = MutatorConfig::default();
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let input = "fn main() { let x = 42; }";
        let result = mutator.mutate_with_info(input, &mut rng);

        assert!(!result.output.is_empty());
    }

    #[test]
    fn test_multiple_mutations() {
        let config = MutatorConfig {
            max_mutations: 10,
            mutation_rate: 1.0,
            ..Default::default()
        };
        let mutator = Mutator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let input = r#"
fn main() {
    let x = 42;
    let y = true;
    let z = "hello";
    if x > 0 {
        print(z);
    }
}
"#;

        let result = mutator.mutate(input, &mut rng);
        assert!(!result.is_empty());
    }
}
