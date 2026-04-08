//! Pattern generator for fuzz testing
//!
//! This module provides random pattern generation with Arbitrary trait
//! implementations for property-based testing. It supports:
//!
//! - All Verum pattern kinds (wildcard, identifier, literal, tuple, etc.)
//! - Constructor patterns for enums
//! - Guard patterns
//! - Or-patterns
//! - Shrinking for minimal counterexamples
//!
//! # Usage
//!
//! ```rust,no_run
//! use verum_fuzz::generators::pattern_generator::{PatternGenerator, ArbitraryPattern};
//! use rand::rng;
//!
//! let generator = PatternGenerator::new(Default::default());
//! let pattern = generator.generate(&mut rng());
//! ```

use super::config::GeneratorConfig;
use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use std::fmt;

/// Generated pattern with source representation
#[derive(Clone)]
pub struct ArbitraryPattern {
    /// Source code representation
    pub source: String,
    /// Pattern kind for shrinking
    pub kind: PatternKind,
    /// Depth of this pattern
    pub depth: usize,
    /// Estimated complexity score
    pub complexity: usize,
}

impl fmt::Debug for ArbitraryPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArbitraryPattern")
            .field("source", &self.source)
            .field("kind", &self.kind)
            .field("depth", &self.depth)
            .finish()
    }
}

impl fmt::Display for ArbitraryPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl ArbitraryPattern {
    /// Create a new pattern
    pub fn new(source: String, kind: PatternKind, depth: usize) -> Self {
        let complexity = Self::calculate_complexity(&source, depth);
        Self {
            source,
            kind,
            depth,
            complexity,
        }
    }

    /// Calculate complexity score for a pattern
    fn calculate_complexity(source: &str, depth: usize) -> usize {
        let mut score = depth * 5;
        score += source.len();
        score += source.matches('(').count() * 3;
        score += source.matches('[').count() * 3;
        score += source.matches('|').count() * 4;
        score += source.matches("if ").count() * 5;
        score += source.matches('@').count() * 3;
        score
    }

    /// Get all variable names bound by this pattern
    pub fn bound_names(&self) -> Vec<String> {
        match &self.kind {
            PatternKind::Wildcard => Vec::new(),
            PatternKind::Identifier(name) => vec![name.clone()],
            PatternKind::Literal(_) => Vec::new(),
            PatternKind::Tuple(patterns) => patterns.iter().flat_map(|p| p.bound_names()).collect(),
            PatternKind::List(patterns, rest) => {
                let mut names: Vec<_> = patterns.iter().flat_map(|p| p.bound_names()).collect();
                if let Some(rest_pattern) = rest {
                    names.extend(rest_pattern.bound_names());
                }
                names
            }
            PatternKind::Constructor { fields, .. } => {
                fields.iter().flat_map(|(_, p)| p.bound_names()).collect()
            }
            PatternKind::Or(left, right) => {
                // In an or-pattern, both sides must bind the same names
                left.bound_names()
            }
            PatternKind::Guard { pattern, .. } => pattern.bound_names(),
            PatternKind::Named { name, pattern } => {
                let mut names = vec![name.clone()];
                names.extend(pattern.bound_names());
                names
            }
            PatternKind::Range { .. } => Vec::new(),
            PatternKind::Rest(name) => name.clone().map(|n| vec![n]).unwrap_or_default(),
        }
    }

    /// Check if this pattern is irrefutable (always matches)
    pub fn is_irrefutable(&self) -> bool {
        match &self.kind {
            PatternKind::Wildcard => true,
            PatternKind::Identifier(_) => true,
            PatternKind::Literal(_) => false,
            PatternKind::Tuple(patterns) => patterns.iter().all(|p| p.is_irrefutable()),
            PatternKind::List(_, _) => false,
            PatternKind::Constructor { .. } => false,
            PatternKind::Or(left, right) => left.is_irrefutable() || right.is_irrefutable(),
            PatternKind::Guard { .. } => false,
            PatternKind::Named { pattern, .. } => pattern.is_irrefutable(),
            PatternKind::Range { .. } => false,
            PatternKind::Rest(_) => true,
        }
    }

    /// Generate shrunk versions of this pattern
    pub fn shrink(&self) -> Vec<ArbitraryPattern> {
        let mut shrunk = Vec::new();

        match &self.kind {
            PatternKind::Tuple(patterns) => {
                // Try individual patterns
                for pattern in patterns {
                    if pattern.complexity < self.complexity {
                        shrunk.push(pattern.clone());
                    }
                }

                // Try with fewer elements
                if patterns.len() > 2 {
                    let simpler: Vec<_> = patterns[..2].to_vec();
                    let source = format!(
                        "({})",
                        simpler
                            .iter()
                            .map(|p| p.source.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    shrunk.push(ArbitraryPattern::new(
                        source,
                        PatternKind::Tuple(simpler),
                        self.depth,
                    ));
                }
            }

            PatternKind::List(patterns, rest) => {
                // Try individual patterns
                for pattern in patterns {
                    if pattern.complexity < self.complexity {
                        shrunk.push(pattern.clone());
                    }
                }

                // Try without rest pattern
                if rest.is_some() {
                    let source = format!(
                        "[{}]",
                        patterns
                            .iter()
                            .map(|p| p.source.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    shrunk.push(ArbitraryPattern::new(
                        source,
                        PatternKind::List(patterns.clone(), None),
                        self.depth,
                    ));
                }

                // Try with fewer elements
                if patterns.len() > 1 {
                    let simpler: Vec<_> = patterns[..1].to_vec();
                    let rest_str = rest
                        .as_ref()
                        .map(|r| format!(", ..{}", r.source))
                        .unwrap_or_default();
                    let source = format!(
                        "[{}{}]",
                        simpler
                            .iter()
                            .map(|p| p.source.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                        rest_str
                    );
                    shrunk.push(ArbitraryPattern::new(
                        source,
                        PatternKind::List(simpler, rest.clone()),
                        self.depth,
                    ));
                }
            }

            PatternKind::Constructor { name, fields } => {
                // Try with fewer fields
                if fields.len() > 1 {
                    for i in 0..fields.len() {
                        let mut new_fields = fields.clone();
                        new_fields.remove(i);
                        let fields_str = new_fields
                            .iter()
                            .map(|(n, p)| format!("{}: {}", n, p.source))
                            .collect::<Vec<_>>()
                            .join(", ");
                        let source = format!("{}{{ {} }}", name, fields_str);
                        shrunk.push(ArbitraryPattern::new(
                            source,
                            PatternKind::Constructor {
                                name: name.clone(),
                                fields: new_fields,
                            },
                            self.depth,
                        ));
                    }
                }

                // Try just the constructor name
                if !fields.is_empty() {
                    shrunk.push(ArbitraryPattern::new(
                        name.clone(),
                        PatternKind::Identifier(name.clone()),
                        0,
                    ));
                }
            }

            PatternKind::Or(left, right) => {
                // Try just the left pattern
                if left.complexity < self.complexity {
                    shrunk.push(left.as_ref().clone());
                }
                // Try just the right pattern
                if right.complexity < self.complexity {
                    shrunk.push(right.as_ref().clone());
                }
            }

            PatternKind::Guard { pattern, .. } => {
                // Try without the guard
                if pattern.complexity < self.complexity {
                    shrunk.push(pattern.as_ref().clone());
                }
            }

            PatternKind::Named { pattern, .. } => {
                // Try just the inner pattern
                if pattern.complexity < self.complexity {
                    shrunk.push(pattern.as_ref().clone());
                }
            }

            _ => {
                // For simple patterns, try simpler variants
                if !matches!(self.kind, PatternKind::Wildcard) {
                    shrunk.push(ArbitraryPattern::new(
                        "_".to_string(),
                        PatternKind::Wildcard,
                        0,
                    ));
                }
            }
        }

        // Filter to keep only simpler patterns
        shrunk.retain(|s| s.complexity < self.complexity);
        shrunk
    }
}

/// Pattern kind for structured representation
#[derive(Debug, Clone)]
pub enum PatternKind {
    /// Wildcard pattern: _
    Wildcard,

    /// Identifier pattern: x, foo
    Identifier(String),

    /// Literal pattern: 42, "hello", true
    Literal(LiteralPattern),

    /// Tuple pattern: (a, b, c)
    Tuple(Vec<ArbitraryPattern>),

    /// List pattern: [a, b, c] or [head, ..tail]
    List(Vec<ArbitraryPattern>, Option<Box<ArbitraryPattern>>),

    /// Constructor pattern: Some(x), Point { x, y }
    Constructor {
        name: String,
        fields: Vec<(String, ArbitraryPattern)>,
    },

    /// Or pattern: a | b
    Or(Box<ArbitraryPattern>, Box<ArbitraryPattern>),

    /// Guard pattern: x if condition
    Guard {
        pattern: Box<ArbitraryPattern>,
        condition: String,
    },

    /// Named pattern: name @ pattern
    Named {
        name: String,
        pattern: Box<ArbitraryPattern>,
    },

    /// Range pattern: 0..10, 'a'..'z'
    Range {
        start: Option<String>,
        end: Option<String>,
        inclusive: bool,
    },

    /// Rest pattern: ..rest or ..
    Rest(Option<String>),
}

/// Literal pattern values
#[derive(Debug, Clone)]
pub enum LiteralPattern {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    Text(String),
}

impl LiteralPattern {
    pub fn to_source(&self) -> String {
        match self {
            LiteralPattern::Int(n) => n.to_string(),
            LiteralPattern::Float(f) => format!("{:.2}", f),
            LiteralPattern::Bool(b) => b.to_string(),
            LiteralPattern::Char(c) => format!("'{}'", c),
            LiteralPattern::Text(s) => format!("\"{}\"", s),
        }
    }
}

/// Pattern generator
pub struct PatternGenerator {
    config: GeneratorConfig,
    pattern_dist: WeightedIndex<u32>,
    var_counter: std::cell::RefCell<usize>,
}

impl PatternGenerator {
    /// Create a new pattern generator with the given configuration
    pub fn new(config: GeneratorConfig) -> Self {
        let weights = config.weights.patterns.as_vec();
        let pattern_dist = WeightedIndex::new(&weights).unwrap();
        Self {
            config,
            pattern_dist,
            var_counter: std::cell::RefCell::new(0),
        }
    }

    /// Generate a fresh variable name
    fn fresh_var(&self) -> String {
        let mut counter = self.var_counter.borrow_mut();
        *counter += 1;
        format!("p_{}", *counter)
    }

    /// Reset variable counter (for independent test runs)
    pub fn reset_counter(&self) {
        *self.var_counter.borrow_mut() = 0;
    }

    /// Generate a random pattern
    pub fn generate<R: Rng>(&self, rng: &mut R) -> ArbitraryPattern {
        self.generate_pattern(rng, 0)
    }

    /// Generate a binding pattern (identifier or tuple of identifiers)
    pub fn generate_binding_pattern<R: Rng>(&self, rng: &mut R) -> ArbitraryPattern {
        match rng.random_range(0..3) {
            0 => self.generate_identifier(rng),
            1 if self.config.features.pattern_matching => {
                // Tuple of identifiers
                let num_elems = rng.random_range(2..=3);
                let patterns: Vec<_> = (0..num_elems)
                    .map(|_| self.generate_identifier(rng))
                    .collect();
                let source = format!(
                    "({})",
                    patterns
                        .iter()
                        .map(|p| p.source.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                ArbitraryPattern::new(source, PatternKind::Tuple(patterns), 1)
            }
            _ => self.generate_identifier(rng),
        }
    }

    /// Generate a pattern at a given depth
    fn generate_pattern<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        // At max depth, only generate simple patterns
        if depth >= 3 {
            return self.generate_simple_pattern(rng);
        }

        match self.pattern_dist.sample(rng) {
            0 => self.generate_wildcard(),
            1 => self.generate_identifier(rng),
            2 => self.generate_literal(rng),
            3 => self.generate_tuple(rng, depth),
            4 => self.generate_list(rng, depth),
            5 => self.generate_constructor(rng, depth),
            6 if self.config.features.pattern_matching => self.generate_or_pattern(rng, depth),
            7 if self.config.features.pattern_matching => self.generate_guard_pattern(rng, depth),
            _ => self.generate_simple_pattern(rng),
        }
    }

    /// Generate a simple pattern (no recursion)
    fn generate_simple_pattern<R: Rng>(&self, rng: &mut R) -> ArbitraryPattern {
        match rng.random_range(0..3) {
            0 => self.generate_wildcard(),
            1 => self.generate_identifier(rng),
            _ => self.generate_literal(rng),
        }
    }

    /// Generate a wildcard pattern
    pub fn generate_wildcard(&self) -> ArbitraryPattern {
        ArbitraryPattern::new("_".to_string(), PatternKind::Wildcard, 0)
    }

    /// Generate an identifier pattern
    fn generate_identifier<R: Rng>(&self, rng: &mut R) -> ArbitraryPattern {
        let name = if rng.random_bool(0.3) {
            // Use a common name
            let names = ["x", "y", "z", "n", "m", "a", "b", "value", "item", "elem"];
            (*names.choose(rng).unwrap()).to_string()
        } else {
            self.fresh_var()
        };

        ArbitraryPattern::new(name.clone(), PatternKind::Identifier(name), 0)
    }

    /// Generate a literal pattern
    fn generate_literal<R: Rng>(&self, rng: &mut R) -> ArbitraryPattern {
        let literal = match rng.random_range(0..5) {
            0 => LiteralPattern::Int(rng.random_range(-100..100)),
            1 => LiteralPattern::Float(rng.random_range(-100.0..100.0)),
            2 => LiteralPattern::Bool(rng.random_bool(0.5)),
            3 => {
                let chars: Vec<char> = ('a'..='z').collect();
                LiteralPattern::Char(*chars.choose(rng).unwrap())
            }
            _ => {
                let strings = ["hello", "world", "test", "foo", "bar"];
                LiteralPattern::Text((*strings.choose(rng).unwrap()).to_string())
            }
        };

        let source = literal.to_source();
        ArbitraryPattern::new(source, PatternKind::Literal(literal), 0)
    }

    /// Generate a tuple pattern
    fn generate_tuple<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        let num_elems = rng.random_range(2..=4);
        let patterns: Vec<_> = (0..num_elems)
            .map(|_| self.generate_pattern(rng, depth + 1))
            .collect();

        let source = format!(
            "({})",
            patterns
                .iter()
                .map(|p| p.source.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );

        ArbitraryPattern::new(source, PatternKind::Tuple(patterns), depth + 1)
    }

    /// Generate a list pattern
    fn generate_list<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        let num_elems = rng.random_range(0..=3);
        let patterns: Vec<_> = (0..num_elems)
            .map(|_| self.generate_pattern(rng, depth + 1))
            .collect();

        let has_rest = rng.random_bool(0.3);
        let rest = if has_rest {
            let rest_name = if rng.random_bool(0.5) {
                Some(self.fresh_var())
            } else {
                None
            };
            Some(Box::new(ArbitraryPattern::new(
                rest_name
                    .clone()
                    .map(|n| format!("..{}", n))
                    .unwrap_or_else(|| "..".to_string()),
                PatternKind::Rest(rest_name),
                0,
            )))
        } else {
            None
        };

        let patterns_str = patterns
            .iter()
            .map(|p| p.source.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let rest_str = rest
            .as_ref()
            .map(|r| {
                if patterns.is_empty() {
                    r.source.clone()
                } else {
                    format!(", {}", r.source)
                }
            })
            .unwrap_or_default();

        let source = format!("[{}{}]", patterns_str, rest_str);

        ArbitraryPattern::new(source, PatternKind::List(patterns, rest), depth + 1)
    }

    /// Generate a constructor pattern
    fn generate_constructor<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        let constructors = [
            ("Some", vec!["value"]),
            ("None", vec![]),
            ("Ok", vec!["value"]),
            ("Err", vec!["error"]),
            ("Point", vec!["x", "y"]),
            ("Pair", vec!["first", "second"]),
            ("Node", vec!["value", "next"]),
            ("Leaf", vec![]),
        ];

        let (name, field_names) = constructors.choose(rng).unwrap();

        let fields: Vec<(String, ArbitraryPattern)> = if field_names.is_empty() {
            Vec::new()
        } else if rng.random_bool(0.5) {
            // Struct-like: Point { x: 0, y: 0 }
            field_names
                .iter()
                .map(|&f| (f.to_string(), self.generate_pattern(rng, depth + 1)))
                .collect()
        } else if field_names.len() == 1 {
            // Single value: Some(x)
            vec![("0".to_string(), self.generate_pattern(rng, depth + 1))]
        } else {
            Vec::new()
        };

        let source = if fields.is_empty() {
            name.to_string()
        } else if fields.len() == 1 && fields[0].0 == "0" {
            format!("{}({})", name, fields[0].1.source)
        } else {
            let fields_str = fields
                .iter()
                .map(|(n, p)| format!("{}: {}", n, p.source))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} {{ {} }}", name, fields_str)
        };

        ArbitraryPattern::new(
            source,
            PatternKind::Constructor {
                name: name.to_string(),
                fields,
            },
            depth + 1,
        )
    }

    /// Generate an or-pattern
    fn generate_or_pattern<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        // Or-patterns should use simple patterns or literals to avoid complexity
        let left = if rng.random_bool(0.7) {
            self.generate_literal(rng)
        } else {
            self.generate_pattern(rng, depth + 1)
        };

        let right = if rng.random_bool(0.7) {
            self.generate_literal(rng)
        } else {
            self.generate_pattern(rng, depth + 1)
        };

        let source = format!("{} | {}", left.source, right.source);

        ArbitraryPattern::new(
            source,
            PatternKind::Or(Box::new(left), Box::new(right)),
            depth + 1,
        )
    }

    /// Generate a guard pattern
    fn generate_guard_pattern<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        let pattern = self.generate_pattern(rng, depth + 1);

        // Generate a simple boolean condition
        let conditions = [
            "x > 0",
            "x < 100",
            "x != 0",
            "x == y",
            "!is_empty",
            "len > 0",
            "value.is_some()",
            "n % 2 == 0",
        ];
        let condition = (*conditions.choose(rng).unwrap()).to_string();

        let source = format!("{} if {}", pattern.source, condition);

        ArbitraryPattern::new(
            source,
            PatternKind::Guard {
                pattern: Box::new(pattern),
                condition,
            },
            depth + 1,
        )
    }

    /// Generate a named pattern (name @ pattern)
    pub fn generate_named_pattern<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryPattern {
        let name = self.fresh_var();
        let pattern = self.generate_pattern(rng, depth + 1);

        let source = format!("{} @ {}", name, pattern.source);

        ArbitraryPattern::new(
            source,
            PatternKind::Named {
                name,
                pattern: Box::new(pattern),
            },
            depth + 1,
        )
    }

    /// Generate a range pattern
    pub fn generate_range_pattern<R: Rng>(&self, rng: &mut R) -> ArbitraryPattern {
        let is_inclusive = rng.random_bool(0.5);

        // Generate either int or char range
        let (start, end, source) = if rng.random_bool(0.7) {
            // Int range
            let s = rng.random_range(0..50);
            let e = rng.random_range(s + 1..100);
            let op = if is_inclusive { "..=" } else { ".." };
            (
                Some(s.to_string()),
                Some(e.to_string()),
                format!("{}{}{}", s, op, e),
            )
        } else {
            // Char range
            let chars: Vec<char> = ('a'..='z').collect();
            let s_idx = rng.random_range(0..13);
            let e_idx = rng.random_range(s_idx + 1..26);
            let s = chars[s_idx];
            let e = chars[e_idx];
            let op = if is_inclusive { "..=" } else { ".." };
            (
                Some(format!("'{}'", s)),
                Some(format!("'{}'", e)),
                format!("'{}'{}'{}'", s, op, e),
            )
        };

        ArbitraryPattern::new(
            source,
            PatternKind::Range {
                start,
                end,
                inclusive: is_inclusive,
            },
            0,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generate_pattern() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let pattern = generator.generate(&mut rng);
            assert!(!pattern.source.is_empty());
        }
    }

    #[test]
    fn test_generate_wildcard() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);

        let pattern = generator.generate_wildcard();
        assert_eq!(pattern.source, "_");
        assert!(pattern.is_irrefutable());
    }

    #[test]
    fn test_generate_identifier() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_identifier(&mut rng);
        assert!(!pattern.source.is_empty());
        assert!(pattern.is_irrefutable());
        assert_eq!(pattern.bound_names().len(), 1);
    }

    #[test]
    fn test_generate_tuple() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_tuple(&mut rng, 0);
        assert!(pattern.source.starts_with("("));
        assert!(pattern.source.ends_with(")"));
    }

    #[test]
    fn test_generate_list() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_list(&mut rng, 0);
        assert!(pattern.source.starts_with("["));
        assert!(pattern.source.ends_with("]"));
    }

    #[test]
    fn test_generate_constructor() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let pattern = generator.generate_constructor(&mut rng, 0);
            assert!(!pattern.source.is_empty());
        }
    }

    #[test]
    fn test_bound_names() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_tuple(&mut rng, 0);
        let names = pattern.bound_names();
        // Tuple should bind at least one variable
        assert!(!names.is_empty() || pattern.source.contains("_"));
    }

    #[test]
    fn test_shrinking() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_tuple(&mut rng, 0);
        let shrunk = pattern.shrink();

        for s in shrunk {
            assert!(s.complexity <= pattern.complexity);
        }
    }

    #[test]
    fn test_or_pattern() {
        let mut config = GeneratorConfig::default();
        config.features.pattern_matching = true;
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_or_pattern(&mut rng, 0);
        assert!(pattern.source.contains("|"));
    }

    #[test]
    fn test_guard_pattern() {
        let mut config = GeneratorConfig::default();
        config.features.pattern_matching = true;
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_guard_pattern(&mut rng, 0);
        assert!(pattern.source.contains("if "));
    }

    #[test]
    fn test_range_pattern() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let pattern = generator.generate_range_pattern(&mut rng);
        assert!(pattern.source.contains(".."));
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        generator.reset_counter();
        let pattern1 = generator.generate(&mut rng1);
        generator.reset_counter();
        let pattern2 = generator.generate(&mut rng2);

        assert_eq!(pattern1.source, pattern2.source);
    }

    #[test]
    fn test_irrefutability() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        // Wildcard is always irrefutable
        assert!(generator.generate_wildcard().is_irrefutable());

        // Identifier is always irrefutable
        assert!(generator.generate_identifier(&mut rng).is_irrefutable());

        // Literal is never irrefutable
        assert!(!generator.generate_literal(&mut rng).is_irrefutable());
    }

    #[test]
    fn test_binding_pattern() {
        let config = GeneratorConfig::default();
        let generator = PatternGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let pattern = generator.generate_binding_pattern(&mut rng);
            // Binding patterns should be irrefutable
            assert!(pattern.is_irrefutable());
        }
    }
}
