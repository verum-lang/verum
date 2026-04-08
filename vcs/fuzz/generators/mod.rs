//! Fuzz test generators for Verum
//!
//! This module provides various program generators for fuzz testing
//! the Verum compiler, runtime, and type system.
//!
//! # Generator Types
//!
//! - **Grammar Generator**: Generates syntactically valid programs
//! - **Type-Aware Generator**: Generates type-correct programs
//! - **Refinement Generator**: Generates programs with refinement types
//! - **Async Generator**: Generates async/await programs
//! - **CBGR Stress Generator**: Memory-intensive CBGR tests
//! - **Mutation Fuzzer**: Mutates existing programs
//! - **Syntax Fuzzer**: Generates random syntactically valid programs
//! - **Type Fuzzer**: Generates guaranteed type-correct programs
//! - **Semantic Fuzzer**: Generates semantically interesting programs
//!
//! # Arbitrary Trait Generators (VCS Spec Section 19)
//!
//! These generators provide Arbitrary trait implementations for property-based testing:
//!
//! - **config**: Generator configuration with depth/complexity limits
//! - **expr_generator**: Random expression generator with shrinking
//! - **stmt_generator**: Random statement generator with shrinking
//! - **pattern_generator**: Random pattern generator with shrinking
//! - **type_generator**: Random type generator with shrinking
//! - **program_generator**: Complete program generator with shrinking
//! - **mutation**: Advanced mutation strategies for property testing

// Original generators
pub mod async_generator;
pub mod cbgr_stress_generator;
pub mod grammar_generator;
pub mod mutation_fuzzer;
pub mod refinement_generator;
pub mod type_aware_generator;

// New specialized fuzzers
pub mod semantic_fuzzer;
pub mod syntax_fuzzer;
pub mod type_fuzzer;

// New Arbitrary-based generators (Spec section 19)
pub mod config;
pub mod expr_generator;
pub mod mutation;
pub mod pattern_generator;
pub mod program_generator;
pub mod stmt_generator;
pub mod type_generator;

// Original exports
pub use async_generator::{AsyncConfig, AsyncGenerator};
pub use cbgr_stress_generator::{CbgrStressConfig, CbgrStressGenerator};
pub use grammar_generator::{GrammarConfig, GrammarGenerator, GrammarGeneratorBuilder};
pub use mutation_fuzzer::{MutationConfig, MutationFuzzer, MutationResult, MutationType};
pub use refinement_generator::{RefinementConfig, RefinementGenerator, RefinementPredicate};
pub use type_aware_generator::{RefTier, TypeAwareConfig, TypeAwareGenerator, VerumType};

// New specialized fuzzer exports
pub use semantic_fuzzer::{SemanticCategory, SemanticFuzzer, SemanticFuzzerConfig};
pub use syntax_fuzzer::{SyntaxFuzzer, SyntaxFuzzerConfig};
pub use type_fuzzer::{RefTier as TypeFuzzerRefTier, TypeFuzzer, TypeFuzzerConfig, VType};

// New Arbitrary-based exports
pub use config::{
    ComplexityLimits, ExpressionWeights, FeatureToggles, GenerationWeights, GeneratorConfig,
    GeneratorConfigBuilder, PatternWeights, ShrinkingConfig, StatementWeights, TypeWeights,
};
pub use expr_generator::{
    ArbitraryExpr, BinaryOp, ExprGenerator, ExprKind, LiteralValue, RefTier as ExprRefTier, UnaryOp,
};
pub use mutation::{
    AppliedMutation, MutationConfig as ArbitraryMutationConfig,
    MutationResult as ArbitraryMutationResult, MutationStrategy, Mutator, StrategyWeights,
};
pub use pattern_generator::{ArbitraryPattern, LiteralPattern, PatternGenerator, PatternKind};
pub use program_generator::{
    ArbitraryProgram, FunctionBody, FunctionDef, ImportDef,
    ProgramGenerator as ArbitraryProgramGenerator, ProgramStructure, TypeDef, TypeDefKind,
};
pub use stmt_generator::{ArbitraryStmt, StmtGenerator, StmtKind};
pub use type_generator::{
    ArbitraryType, PrimitiveType, RefTier as TypeRefTier, TypeGenerator, TypeKind,
};

use rand::Rng;

/// Unified generator interface
pub trait ProgramGenerator {
    /// Generate a random program
    fn generate<R: Rng>(&self, rng: &mut R) -> String;

    /// Get the generator name
    fn name(&self) -> &'static str;

    /// Get generator description
    fn description(&self) -> &'static str;
}

impl ProgramGenerator for GrammarGenerator {
    fn generate<R: Rng>(&self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "Grammar Generator"
    }

    fn description(&self) -> &'static str {
        "Generates syntactically valid Verum programs using grammar rules"
    }
}

impl ProgramGenerator for TypeAwareGenerator {
    fn generate<R: Rng>(&self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "Type-Aware Generator"
    }

    fn description(&self) -> &'static str {
        "Generates type-correct Verum programs with consistent type annotations"
    }
}

impl ProgramGenerator for RefinementGenerator {
    fn generate<R: Rng>(&self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "Refinement Generator"
    }

    fn description(&self) -> &'static str {
        "Generates programs with refinement types and verification annotations"
    }
}

impl ProgramGenerator for AsyncGenerator {
    fn generate<R: Rng>(&self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "Async Generator"
    }

    fn description(&self) -> &'static str {
        "Generates async/await programs with concurrency patterns"
    }
}

impl ProgramGenerator for CbgrStressGenerator {
    fn generate<R: Rng>(&self, rng: &mut R) -> String {
        self.generate_program(rng)
    }

    fn name(&self) -> &'static str {
        "CBGR Stress Generator"
    }

    fn description(&self) -> &'static str {
        "Generates memory-intensive programs to stress-test CBGR"
    }
}

/// Combined generator that randomly selects from all generators
pub struct CombinedGenerator {
    grammar: GrammarGenerator,
    type_aware: TypeAwareGenerator,
    refinement: RefinementGenerator,
    async_gen: AsyncGenerator,
    cbgr_stress: CbgrStressGenerator,
}

impl CombinedGenerator {
    /// Create a new combined generator with default configs
    pub fn new() -> Self {
        Self {
            grammar: GrammarGenerator::new(GrammarConfig::default()),
            type_aware: TypeAwareGenerator::new(TypeAwareConfig::default()),
            refinement: RefinementGenerator::new(RefinementConfig::default()),
            async_gen: AsyncGenerator::new(AsyncConfig::default()),
            cbgr_stress: CbgrStressGenerator::new(CbgrStressConfig::default()),
        }
    }

    /// Generate using a randomly selected generator
    pub fn generate<R: Rng>(&self, rng: &mut R) -> (String, &'static str) {
        match rng.random_range(0..5) {
            0 => (self.grammar.generate(rng), self.grammar.name()),
            1 => (self.type_aware.generate(rng), self.type_aware.name()),
            2 => (self.refinement.generate(rng), self.refinement.name()),
            3 => (self.async_gen.generate(rng), self.async_gen.name()),
            _ => (self.cbgr_stress.generate(rng), self.cbgr_stress.name()),
        }
    }
}

impl Default for CombinedGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_combined_generator() {
        let generator = CombinedGenerator::new();
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let (program, name) = generator.generate(&mut rng);
            assert!(!program.is_empty());
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn test_all_generators_produce_output() {
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let grammar = GrammarGenerator::new(GrammarConfig::default());
        assert!(!grammar.generate(&mut rng).is_empty());

        let type_aware = TypeAwareGenerator::new(TypeAwareConfig::default());
        assert!(!type_aware.generate(&mut rng).is_empty());

        let refinement = RefinementGenerator::new(RefinementConfig::default());
        assert!(!refinement.generate(&mut rng).is_empty());

        let async_gen = AsyncGenerator::new(AsyncConfig::default());
        assert!(!async_gen.generate(&mut rng).is_empty());

        let cbgr = CbgrStressGenerator::new(CbgrStressConfig::default());
        assert!(!cbgr.generate(&mut rng).is_empty());
    }
}
