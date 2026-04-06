//! Specialized program generators for Verum fuzzing
//!
//! This module provides grammar-aware generators that produce syntactically
//! valid and semantically meaningful Verum programs for different testing scenarios.
//!
//! # Generator Types
//!
//! - [`lexer`]: Generates valid lexer tokens and edge cases
//! - [`parser`]: Generates syntactically valid programs
//! - [`refinement`]: Generates programs with refinement types
//! - [`async_gen`]: Generates async/concurrent programs
//! - [`cbgr`]: Generates memory-intensive CBGR patterns
//! - [`mutation`]: Mutates existing programs

pub mod async_gen;
pub mod cbgr;
pub mod lexer;
pub mod parser;
pub mod refinement;

use rand::prelude::*;
use serde::{Deserialize, Serialize};

/// Common configuration for all generators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Maximum AST depth
    pub max_depth: usize,
    /// Maximum statements per block
    pub max_statements: usize,
    /// Maximum number of functions
    pub max_functions: usize,
    /// Maximum number of type definitions
    pub max_types: usize,
    /// Include async constructs
    pub include_async: bool,
    /// Include CBGR references
    pub include_cbgr: bool,
    /// Include refinement types
    pub include_refinements: bool,
    /// Include unsafe blocks
    pub include_unsafe: bool,
    /// Probability of generating invalid syntax (0.0 - 1.0)
    pub invalid_syntax_prob: f64,
    /// Random seed (None for random)
    pub seed: Option<u64>,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            max_depth: 10,
            max_statements: 50,
            max_functions: 10,
            max_types: 5,
            include_async: true,
            include_cbgr: true,
            include_refinements: false,
            include_unsafe: false,
            invalid_syntax_prob: 0.0,
            seed: None,
        }
    }
}

/// Trait for all generators
pub trait Generate {
    /// Generate a random program
    fn generate<R: Rng>(&mut self, rng: &mut R) -> String;

    /// Get generator name
    fn name(&self) -> &'static str;

    /// Get generator description
    fn description(&self) -> &'static str;
}

/// Unified generator that combines all strategies
pub struct UnifiedGenerator {
    config: GeneratorConfig,
    lexer: lexer::LexerGenerator,
    parser: parser::ParserGenerator,
    refinement: refinement::RefinementGenerator,
    async_gen: async_gen::AsyncGenerator,
    cbgr: cbgr::CbgrGenerator,
}

impl UnifiedGenerator {
    /// Create a new unified generator
    pub fn new(config: GeneratorConfig) -> Self {
        Self {
            lexer: lexer::LexerGenerator::new(config.clone()),
            parser: parser::ParserGenerator::new(config.clone()),
            refinement: refinement::RefinementGenerator::new(config.clone()),
            async_gen: async_gen::AsyncGenerator::new(config.clone()),
            cbgr: cbgr::CbgrGenerator::new(config.clone()),
            config,
        }
    }

    /// Generate a program using a randomly selected strategy
    pub fn generate<R: Rng>(&mut self, rng: &mut R) -> String {
        match rng.random_range(0..5) {
            0 => self.lexer.generate(rng),
            1 => self.parser.generate(rng),
            2 if self.config.include_refinements => self.refinement.generate(rng),
            3 if self.config.include_async => self.async_gen.generate(rng),
            4 if self.config.include_cbgr => self.cbgr.generate(rng),
            _ => self.parser.generate(rng),
        }
    }

    /// Generate using a specific strategy
    pub fn generate_with<R: Rng>(&mut self, strategy: GeneratorStrategy, rng: &mut R) -> String {
        match strategy {
            GeneratorStrategy::Lexer => self.lexer.generate(rng),
            GeneratorStrategy::Parser => self.parser.generate(rng),
            GeneratorStrategy::Refinement => self.refinement.generate(rng),
            GeneratorStrategy::Async => self.async_gen.generate(rng),
            GeneratorStrategy::Cbgr => self.cbgr.generate(rng),
        }
    }
}

/// Strategy for program generation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeneratorStrategy {
    /// Focus on lexer token edge cases
    Lexer,
    /// Focus on parser constructs
    Parser,
    /// Focus on refinement types
    Refinement,
    /// Focus on async constructs
    Async,
    /// Focus on CBGR patterns
    Cbgr,
}

/// Helper to generate random identifiers
pub fn random_identifier<R: Rng>(rng: &mut R) -> String {
    let prefixes = [
        "x", "y", "z", "val", "var", "tmp", "result", "data", "item", "elem",
    ];
    let prefix = prefixes[rng.random_range(0..prefixes.len())];
    format!("{}_{}", prefix, rng.random_range(0..1000))
}

/// Helper to generate random type names
pub fn random_type_name<R: Rng>(rng: &mut R) -> String {
    let types = [
        "MyStruct", "Config", "State", "Handler", "Builder", "Context", "Node", "Tree", "Graph",
        "Cache",
    ];
    let base = types[rng.random_range(0..types.len())];
    format!("{}{}", base, rng.random_range(0..100))
}

/// Helper to generate random function names
pub fn random_function_name<R: Rng>(rng: &mut R) -> String {
    let prefixes = [
        "compute", "process", "handle", "get", "set", "update", "create", "build",
    ];
    let prefix = prefixes[rng.random_range(0..prefixes.len())];
    format!("{}_{}", prefix, rng.random_range(0..100))
}

/// Generate a random primitive type
pub fn random_primitive_type<R: Rng>(rng: &mut R) -> &'static str {
    let types = ["Int", "Float", "Bool", "Text", "Unit"];
    types[rng.random_range(0..types.len())]
}

/// Generate a random type (including compound types)
pub fn random_type<R: Rng>(rng: &mut R, depth: usize) -> String {
    if depth > 2 {
        return random_primitive_type(rng).to_string();
    }

    match rng.random_range(0..10) {
        0..=4 => random_primitive_type(rng).to_string(),
        5 => format!("List<{}>", random_type(rng, depth + 1)),
        6 => format!("Maybe<{}>", random_type(rng, depth + 1)),
        7 => format!(
            "Map<{}, {}>",
            random_primitive_type(rng),
            random_type(rng, depth + 1)
        ),
        8 => format!("Set<{}>", random_primitive_type(rng)),
        _ => format!(
            "({}, {})",
            random_type(rng, depth + 1),
            random_type(rng, depth + 1)
        ),
    }
}

/// Generate a random string literal
pub fn random_string<R: Rng>(rng: &mut R, max_len: usize) -> String {
    let len = rng.random_range(0..=max_len);
    let chars: Vec<char> = (0..len)
        .map(|_| {
            let c = rng.random_range(0..62);
            match c {
                0..=25 => (b'a' + c as u8) as char,
                26..=51 => (b'A' + (c - 26) as u8) as char,
                _ => (b'0' + (c - 52) as u8) as char,
            }
        })
        .collect();
    chars.into_iter().collect()
}

/// Generate indentation
pub fn indent(level: usize) -> String {
    "    ".repeat(level)
}
