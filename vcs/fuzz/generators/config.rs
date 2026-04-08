//! Generator configuration for fuzz testing
//!
//! This module provides configurable parameters for controlling the complexity
//! and characteristics of generated Verum programs. It supports:
//!
//! - Depth/complexity limits for AST generation
//! - Feature toggles (async, CBGR, refinements, etc.)
//! - Probability weights for different constructs
//! - Shrinking configuration for minimal counterexamples
//!
//! # Usage
//!
//! ```rust
//! use verum_fuzz::generators::config::{GeneratorConfig, ComplexityLimits};
//!
//! let config = GeneratorConfig::builder()
//!     .max_depth(5)
//!     .max_statements(20)
//!     .enable_async(true)
//!     .enable_cbgr(true)
//!     .build();
//! ```

use serde::{Deserialize, Serialize};
use std::ops::RangeInclusive;

/// Main configuration for program generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorConfig {
    /// Complexity limits for generated programs
    pub complexity: ComplexityLimits,
    /// Feature toggles
    pub features: FeatureToggles,
    /// Probability weights for different constructs
    pub weights: GenerationWeights,
    /// Shrinking configuration
    pub shrinking: ShrinkingConfig,
    /// Random seed for reproducibility (None = random)
    pub seed: Option<u64>,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            complexity: ComplexityLimits::default(),
            features: FeatureToggles::default(),
            weights: GenerationWeights::default(),
            shrinking: ShrinkingConfig::default(),
            seed: None,
        }
    }
}

impl GeneratorConfig {
    /// Create a new builder for GeneratorConfig
    pub fn builder() -> GeneratorConfigBuilder {
        GeneratorConfigBuilder::default()
    }

    /// Create a minimal configuration for quick testing
    pub fn minimal() -> Self {
        Self {
            complexity: ComplexityLimits::minimal(),
            features: FeatureToggles::minimal(),
            weights: GenerationWeights::default(),
            shrinking: ShrinkingConfig::default(),
            seed: None,
        }
    }

    /// Create a stress-test configuration with maximum complexity
    pub fn stress() -> Self {
        Self {
            complexity: ComplexityLimits::stress(),
            features: FeatureToggles::all(),
            weights: GenerationWeights::default(),
            shrinking: ShrinkingConfig::default(),
            seed: None,
        }
    }

    /// Create a configuration focused on edge cases
    pub fn edge_cases() -> Self {
        Self {
            complexity: ComplexityLimits::edge_cases(),
            features: FeatureToggles::all(),
            weights: GenerationWeights::edge_case_focused(),
            shrinking: ShrinkingConfig::aggressive(),
            seed: None,
        }
    }
}

/// Builder for GeneratorConfig
#[derive(Debug, Default)]
pub struct GeneratorConfigBuilder {
    config: GeneratorConfig,
}

impl GeneratorConfigBuilder {
    /// Set maximum AST depth
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.config.complexity.max_depth = depth;
        self
    }

    /// Set maximum statements per block
    pub fn max_statements(mut self, count: usize) -> Self {
        self.config.complexity.max_statements = count;
        self
    }

    /// Set maximum function count
    pub fn max_functions(mut self, count: usize) -> Self {
        self.config.complexity.max_functions = count;
        self
    }

    /// Set maximum type definition count
    pub fn max_types(mut self, count: usize) -> Self {
        self.config.complexity.max_types = count;
        self
    }

    /// Set maximum parameters per function
    pub fn max_params(mut self, count: usize) -> Self {
        self.config.complexity.max_params = count;
        self
    }

    /// Enable/disable async generation
    pub fn enable_async(mut self, enable: bool) -> Self {
        self.config.features.async_await = enable;
        self
    }

    /// Enable/disable CBGR generation
    pub fn enable_cbgr(mut self, enable: bool) -> Self {
        self.config.features.cbgr = enable;
        self
    }

    /// Enable/disable refinement types
    pub fn enable_refinements(mut self, enable: bool) -> Self {
        self.config.features.refinement_types = enable;
        self
    }

    /// Enable/disable unsafe blocks
    pub fn enable_unsafe(mut self, enable: bool) -> Self {
        self.config.features.unsafe_blocks = enable;
        self
    }

    /// Enable/disable meta programming
    pub fn enable_meta(mut self, enable: bool) -> Self {
        self.config.features.meta_programming = enable;
        self
    }

    /// Set random seed for reproducibility
    pub fn seed(mut self, seed: u64) -> Self {
        self.config.seed = Some(seed);
        self
    }

    /// Build the configuration
    pub fn build(self) -> GeneratorConfig {
        self.config
    }
}

/// Limits on program complexity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityLimits {
    /// Maximum AST depth (nesting level)
    pub max_depth: usize,
    /// Maximum statements per block/function
    pub max_statements: usize,
    /// Maximum number of functions per program
    pub max_functions: usize,
    /// Maximum number of type definitions
    pub max_types: usize,
    /// Maximum parameters per function
    pub max_params: usize,
    /// Maximum generic type parameters
    pub max_type_params: usize,
    /// Maximum list/array literal size
    pub max_list_size: usize,
    /// Maximum string literal length
    pub max_string_length: usize,
    /// Maximum integer literal absolute value
    pub max_int_value: i64,
    /// Maximum floating point literal value
    pub max_float_value: f64,
    /// Maximum number of match arms
    pub max_match_arms: usize,
    /// Maximum number of enum variants
    pub max_enum_variants: usize,
    /// Maximum number of struct fields
    pub max_struct_fields: usize,
    /// Maximum closure nesting depth
    pub max_closure_depth: usize,
}

impl Default for ComplexityLimits {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_statements: 15,
            max_functions: 5,
            max_types: 3,
            max_params: 4,
            max_type_params: 2,
            max_list_size: 10,
            max_string_length: 50,
            max_int_value: 1_000_000,
            max_float_value: 1e10,
            max_match_arms: 5,
            max_enum_variants: 5,
            max_struct_fields: 6,
            max_closure_depth: 3,
        }
    }
}

impl ComplexityLimits {
    /// Create minimal limits for quick testing
    pub fn minimal() -> Self {
        Self {
            max_depth: 2,
            max_statements: 3,
            max_functions: 1,
            max_types: 0,
            max_params: 2,
            max_type_params: 0,
            max_list_size: 3,
            max_string_length: 10,
            max_int_value: 100,
            max_float_value: 100.0,
            max_match_arms: 2,
            max_enum_variants: 2,
            max_struct_fields: 2,
            max_closure_depth: 1,
        }
    }

    /// Create stress-test limits
    pub fn stress() -> Self {
        Self {
            max_depth: 15,
            max_statements: 100,
            max_functions: 20,
            max_types: 10,
            max_params: 10,
            max_type_params: 5,
            max_list_size: 100,
            max_string_length: 1000,
            max_int_value: i64::MAX / 2,
            max_float_value: f64::MAX / 2.0,
            max_match_arms: 20,
            max_enum_variants: 20,
            max_struct_fields: 20,
            max_closure_depth: 10,
        }
    }

    /// Create limits focused on edge cases
    pub fn edge_cases() -> Self {
        Self {
            max_depth: 50,
            max_statements: 200,
            max_functions: 30,
            max_types: 15,
            max_params: 50,
            max_type_params: 10,
            max_list_size: 500,
            max_string_length: 10000,
            max_int_value: i64::MAX,
            max_float_value: f64::MAX,
            max_match_arms: 50,
            max_enum_variants: 50,
            max_struct_fields: 50,
            max_closure_depth: 20,
        }
    }
}

/// Feature toggles for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureToggles {
    /// Generate async/await constructs
    pub async_await: bool,
    /// Generate CBGR references (&, &checked, &unsafe)
    pub cbgr: bool,
    /// Generate refinement types (Int{> 0})
    pub refinement_types: bool,
    /// Generate unsafe blocks
    pub unsafe_blocks: bool,
    /// Generate meta programming constructs
    pub meta_programming: bool,
    /// Generate context requirements (using [...])
    pub contexts: bool,
    /// Generate protocols (traits)
    pub protocols: bool,
    /// Generate generics
    pub generics: bool,
    /// Generate closures/lambdas
    pub closures: bool,
    /// Generate pattern matching
    pub pattern_matching: bool,
    /// Generate loop constructs
    pub loops: bool,
    /// Generate match expressions
    pub match_expressions: bool,
    /// Generate try/recover/finally
    pub error_handling: bool,
    /// Generate comprehensions
    pub comprehensions: bool,
    /// Generate tensor types
    pub tensors: bool,
    /// Generate FFI constructs
    pub ffi: bool,
    /// Generate proof constructs (forall, exists)
    pub proofs: bool,
}

impl Default for FeatureToggles {
    fn default() -> Self {
        Self {
            async_await: true,
            cbgr: true,
            refinement_types: false,
            unsafe_blocks: false,
            meta_programming: false,
            contexts: true,
            protocols: true,
            generics: true,
            closures: true,
            pattern_matching: true,
            loops: true,
            match_expressions: true,
            error_handling: true,
            comprehensions: true,
            tensors: false,
            ffi: false,
            proofs: false,
        }
    }
}

impl FeatureToggles {
    /// Enable minimal features only
    pub fn minimal() -> Self {
        Self {
            async_await: false,
            cbgr: false,
            refinement_types: false,
            unsafe_blocks: false,
            meta_programming: false,
            contexts: false,
            protocols: false,
            generics: false,
            closures: false,
            pattern_matching: true,
            loops: true,
            match_expressions: true,
            error_handling: false,
            comprehensions: false,
            tensors: false,
            ffi: false,
            proofs: false,
        }
    }

    /// Enable all features
    pub fn all() -> Self {
        Self {
            async_await: true,
            cbgr: true,
            refinement_types: true,
            unsafe_blocks: true,
            meta_programming: true,
            contexts: true,
            protocols: true,
            generics: true,
            closures: true,
            pattern_matching: true,
            loops: true,
            match_expressions: true,
            error_handling: true,
            comprehensions: true,
            tensors: true,
            ffi: true,
            proofs: true,
        }
    }

    /// Features for core language testing (no advanced features)
    pub fn core_only() -> Self {
        Self {
            async_await: false,
            cbgr: true,
            refinement_types: false,
            unsafe_blocks: false,
            meta_programming: false,
            contexts: false,
            protocols: false,
            generics: true,
            closures: true,
            pattern_matching: true,
            loops: true,
            match_expressions: true,
            error_handling: false,
            comprehensions: false,
            tensors: false,
            ffi: false,
            proofs: false,
        }
    }
}

/// Probability weights for different language constructs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationWeights {
    /// Expression type weights
    pub expressions: ExpressionWeights,
    /// Statement type weights
    pub statements: StatementWeights,
    /// Type weights
    pub types: TypeWeights,
    /// Pattern weights
    pub patterns: PatternWeights,
}

impl Default for GenerationWeights {
    fn default() -> Self {
        Self {
            expressions: ExpressionWeights::default(),
            statements: StatementWeights::default(),
            types: TypeWeights::default(),
            patterns: PatternWeights::default(),
        }
    }
}

impl GenerationWeights {
    /// Weights focused on generating edge cases
    pub fn edge_case_focused() -> Self {
        Self {
            expressions: ExpressionWeights::edge_case_focused(),
            statements: StatementWeights::default(),
            types: TypeWeights::edge_case_focused(),
            patterns: PatternWeights::default(),
        }
    }
}

/// Weights for different expression types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpressionWeights {
    pub literal: u32,
    pub identifier: u32,
    pub binary: u32,
    pub unary: u32,
    pub call: u32,
    pub method_call: u32,
    pub field_access: u32,
    pub index: u32,
    pub if_expr: u32,
    pub match_expr: u32,
    pub block: u32,
    pub lambda: u32,
    pub tuple: u32,
    pub list: u32,
    pub record: u32,
    pub range: u32,
    pub try_expr: u32,
    pub async_expr: u32,
    pub spawn: u32,
}

impl Default for ExpressionWeights {
    fn default() -> Self {
        Self {
            literal: 25,
            identifier: 20,
            binary: 15,
            unary: 5,
            call: 10,
            method_call: 5,
            field_access: 3,
            index: 3,
            if_expr: 8,
            match_expr: 4,
            block: 3,
            lambda: 3,
            tuple: 2,
            list: 3,
            record: 2,
            range: 2,
            try_expr: 2,
            async_expr: 1,
            spawn: 1,
        }
    }
}

impl ExpressionWeights {
    /// Weights focused on edge case generation
    pub fn edge_case_focused() -> Self {
        Self {
            literal: 15,
            identifier: 10,
            binary: 20,
            unary: 10,
            call: 8,
            method_call: 5,
            field_access: 5,
            index: 8,
            if_expr: 10,
            match_expr: 8,
            block: 5,
            lambda: 5,
            tuple: 5,
            list: 8,
            record: 5,
            range: 5,
            try_expr: 5,
            async_expr: 3,
            spawn: 3,
        }
    }

    /// Get weights as a vector for weighted sampling
    pub fn as_vec(&self) -> Vec<u32> {
        vec![
            self.literal,
            self.identifier,
            self.binary,
            self.unary,
            self.call,
            self.method_call,
            self.field_access,
            self.index,
            self.if_expr,
            self.match_expr,
            self.block,
            self.lambda,
            self.tuple,
            self.list,
            self.record,
            self.range,
            self.try_expr,
            self.async_expr,
            self.spawn,
        ]
    }
}

/// Weights for different statement types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatementWeights {
    pub let_binding: u32,
    pub assignment: u32,
    pub expression: u32,
    pub if_stmt: u32,
    pub match_stmt: u32,
    pub for_loop: u32,
    pub while_loop: u32,
    pub loop_stmt: u32,
    pub return_stmt: u32,
    pub break_stmt: u32,
    pub continue_stmt: u32,
}

impl Default for StatementWeights {
    fn default() -> Self {
        Self {
            let_binding: 30,
            assignment: 15,
            expression: 20,
            if_stmt: 10,
            match_stmt: 5,
            for_loop: 8,
            while_loop: 7,
            loop_stmt: 2,
            return_stmt: 5,
            break_stmt: 2,
            continue_stmt: 2,
        }
    }
}

impl StatementWeights {
    /// Get weights as a vector for weighted sampling
    pub fn as_vec(&self) -> Vec<u32> {
        vec![
            self.let_binding,
            self.assignment,
            self.expression,
            self.if_stmt,
            self.match_stmt,
            self.for_loop,
            self.while_loop,
            self.loop_stmt,
            self.return_stmt,
            self.break_stmt,
            self.continue_stmt,
        ]
    }
}

/// Weights for different type constructs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeWeights {
    pub primitive: u32,
    pub list: u32,
    pub map: u32,
    pub maybe: u32,
    pub set: u32,
    pub tuple: u32,
    pub function: u32,
    pub reference: u32,
    pub named: u32,
}

impl Default for TypeWeights {
    fn default() -> Self {
        Self {
            primitive: 50,
            list: 15,
            map: 8,
            maybe: 10,
            set: 5,
            tuple: 5,
            function: 3,
            reference: 5,
            named: 5,
        }
    }
}

impl TypeWeights {
    /// Weights focused on edge case types
    pub fn edge_case_focused() -> Self {
        Self {
            primitive: 30,
            list: 15,
            map: 12,
            maybe: 12,
            set: 10,
            tuple: 10,
            function: 8,
            reference: 10,
            named: 8,
        }
    }

    /// Get weights as a vector for weighted sampling
    pub fn as_vec(&self) -> Vec<u32> {
        vec![
            self.primitive,
            self.list,
            self.map,
            self.maybe,
            self.set,
            self.tuple,
            self.function,
            self.reference,
            self.named,
        ]
    }
}

/// Weights for different pattern types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternWeights {
    pub wildcard: u32,
    pub identifier: u32,
    pub literal: u32,
    pub tuple: u32,
    pub list: u32,
    pub constructor: u32,
    pub or_pattern: u32,
    pub guard: u32,
}

impl Default for PatternWeights {
    fn default() -> Self {
        Self {
            wildcard: 15,
            identifier: 30,
            literal: 20,
            tuple: 10,
            list: 5,
            constructor: 10,
            or_pattern: 5,
            guard: 5,
        }
    }
}

impl PatternWeights {
    /// Get weights as a vector for weighted sampling
    pub fn as_vec(&self) -> Vec<u32> {
        vec![
            self.wildcard,
            self.identifier,
            self.literal,
            self.tuple,
            self.list,
            self.constructor,
            self.or_pattern,
            self.guard,
        ]
    }
}

/// Configuration for shrinking (minimizing counterexamples)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShrinkingConfig {
    /// Maximum shrinking iterations
    pub max_iterations: usize,
    /// Enable statement removal
    pub remove_statements: bool,
    /// Enable expression simplification
    pub simplify_expressions: bool,
    /// Enable type simplification
    pub simplify_types: bool,
    /// Enable literal shrinking
    pub shrink_literals: bool,
    /// Enable function inlining during shrinking
    pub inline_functions: bool,
    /// Enable dead code elimination
    pub eliminate_dead_code: bool,
    /// Preserve minimum number of statements
    pub min_statements: usize,
}

impl Default for ShrinkingConfig {
    fn default() -> Self {
        Self {
            max_iterations: 1000,
            remove_statements: true,
            simplify_expressions: true,
            simplify_types: true,
            shrink_literals: true,
            inline_functions: true,
            eliminate_dead_code: true,
            min_statements: 1,
        }
    }
}

impl ShrinkingConfig {
    /// Create aggressive shrinking configuration
    pub fn aggressive() -> Self {
        Self {
            max_iterations: 5000,
            remove_statements: true,
            simplify_expressions: true,
            simplify_types: true,
            shrink_literals: true,
            inline_functions: true,
            eliminate_dead_code: true,
            min_statements: 0,
        }
    }

    /// Create minimal shrinking configuration
    pub fn minimal() -> Self {
        Self {
            max_iterations: 100,
            remove_statements: true,
            simplify_expressions: false,
            simplify_types: false,
            shrink_literals: true,
            inline_functions: false,
            eliminate_dead_code: false,
            min_statements: 1,
        }
    }
}

/// Integer range specification for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntRange {
    pub min: i64,
    pub max: i64,
}

impl IntRange {
    pub fn new(min: i64, max: i64) -> Self {
        Self { min, max }
    }

    pub fn as_range(&self) -> RangeInclusive<i64> {
        self.min..=self.max
    }

    /// Check if a value is within this range
    pub fn contains(&self, value: i64) -> bool {
        value >= self.min && value <= self.max
    }
}

impl Default for IntRange {
    fn default() -> Self {
        Self {
            min: i64::MIN,
            max: i64::MAX,
        }
    }
}

/// Float range specification for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatRange {
    pub min: f64,
    pub max: f64,
}

impl FloatRange {
    pub fn new(min: f64, max: f64) -> Self {
        Self { min, max }
    }

    pub fn as_range(&self) -> RangeInclusive<f64> {
        self.min..=self.max
    }

    /// Check if a value is within this range
    pub fn contains(&self, value: f64) -> bool {
        value >= self.min && value <= self.max
    }
}

impl Default for FloatRange {
    fn default() -> Self {
        Self {
            min: f64::MIN,
            max: f64::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = GeneratorConfig::default();
        assert_eq!(config.complexity.max_depth, 5);
        assert!(config.features.async_await);
        assert!(config.features.cbgr);
    }

    #[test]
    fn test_builder() {
        let config = GeneratorConfig::builder()
            .max_depth(10)
            .max_statements(50)
            .enable_async(false)
            .enable_cbgr(true)
            .seed(12345)
            .build();

        assert_eq!(config.complexity.max_depth, 10);
        assert_eq!(config.complexity.max_statements, 50);
        assert!(!config.features.async_await);
        assert!(config.features.cbgr);
        assert_eq!(config.seed, Some(12345));
    }

    #[test]
    fn test_preset_configs() {
        let minimal = GeneratorConfig::minimal();
        assert_eq!(minimal.complexity.max_depth, 2);
        assert!(!minimal.features.async_await);

        let stress = GeneratorConfig::stress();
        assert_eq!(stress.complexity.max_depth, 15);
        assert!(stress.features.meta_programming);

        let edge = GeneratorConfig::edge_cases();
        assert_eq!(edge.complexity.max_depth, 50);
    }

    #[test]
    fn test_weight_vectors() {
        let expr_weights = ExpressionWeights::default();
        let vec = expr_weights.as_vec();
        assert!(!vec.is_empty());
        assert!(vec.iter().all(|&w| w > 0));

        let stmt_weights = StatementWeights::default();
        let vec = stmt_weights.as_vec();
        assert!(!vec.is_empty());
    }

    #[test]
    fn test_feature_presets() {
        let minimal = FeatureToggles::minimal();
        assert!(!minimal.async_await);
        assert!(!minimal.cbgr);

        let all = FeatureToggles::all();
        assert!(all.async_await);
        assert!(all.cbgr);
        assert!(all.ffi);
        assert!(all.proofs);
    }

    #[test]
    fn test_int_range() {
        let range = IntRange::new(-100, 100);
        assert!(range.contains(0));
        assert!(range.contains(-100));
        assert!(range.contains(100));
        assert!(!range.contains(-101));
        assert!(!range.contains(101));
    }
}
