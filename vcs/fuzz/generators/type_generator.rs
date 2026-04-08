//! Type generator for fuzz testing
//!
//! This module provides random type generation with Arbitrary trait
//! implementations for property-based testing. It supports:
//!
//! - All Verum type kinds (primitives, generics, references, etc.)
//! - CBGR three-tier reference model
//! - Refinement types
//! - Generic types with constraints
//! - Shrinking for minimal counterexamples
//!
//! # Arbitrary Trait
//!
//! The Arbitrary trait implementation allows types to be used with
//! property-based testing frameworks.
//!
//! # Usage
//!
//! ```rust,no_run
//! use verum_fuzz::generators::type_generator::{TypeGenerator, ArbitraryType};
//! use rand::rng;
//!
//! let generator = TypeGenerator::new(Default::default());
//! let ty = generator.generate(&mut rng());
//! ```

use super::config::GeneratorConfig;
use rand::Rng;
use rand::distr::Distribution;
use rand::distr::weighted::WeightedIndex;
use rand::seq::IndexedRandom;
use std::fmt;

/// Generated type with source representation
#[derive(Clone, PartialEq, Eq)]
pub struct ArbitraryType {
    /// Source code representation
    pub source: String,
    /// Type kind for shrinking
    pub kind: TypeKind,
    /// Depth of this type
    pub depth: usize,
    /// Estimated complexity score
    pub complexity: usize,
}

impl fmt::Debug for ArbitraryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ArbitraryType")
            .field("source", &self.source)
            .field("kind", &self.kind)
            .field("depth", &self.depth)
            .finish()
    }
}

impl fmt::Display for ArbitraryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl ArbitraryType {
    /// Create a new type
    pub fn new(source: String, kind: TypeKind, depth: usize) -> Self {
        let complexity = Self::calculate_complexity(&source, depth);
        Self {
            source,
            kind,
            depth,
            complexity,
        }
    }

    /// Calculate complexity score for a type
    fn calculate_complexity(source: &str, depth: usize) -> usize {
        let mut score = depth * 5;
        score += source.len();
        score += source.matches('<').count() * 3;
        score += source.matches('&').count() * 2;
        score += source.matches("fn").count() * 5;
        score += source.matches("where").count() * 4;
        score
    }

    /// Generate shrunk versions of this type
    pub fn shrink(&self) -> Vec<ArbitraryType> {
        let mut shrunk = Vec::new();

        match &self.kind {
            TypeKind::List(inner) | TypeKind::Maybe(inner) | TypeKind::Set(inner) => {
                // Try just the inner type
                shrunk.push(inner.as_ref().clone());
            }
            TypeKind::Map(key, value) => {
                // Try just key or value type
                shrunk.push(key.as_ref().clone());
                shrunk.push(value.as_ref().clone());
            }
            TypeKind::Tuple(types) => {
                // Try individual element types
                shrunk.extend(types.iter().cloned());
                // Try smaller tuples
                if types.len() > 2 {
                    shrunk.push(ArbitraryType::new(
                        format!("({}, {})", types[0].source, types[1].source),
                        TypeKind::Tuple(types[..2].to_vec()),
                        1,
                    ));
                }
            }
            TypeKind::Function { params, ret } => {
                // Try just the return type
                shrunk.push(ret.as_ref().clone());
                // Try with fewer params
                for param in params {
                    shrunk.push(param.clone());
                }
            }
            TypeKind::Reference { inner, .. } => {
                // Try without the reference
                shrunk.push(inner.as_ref().clone());
            }
            TypeKind::Refined { base, .. } => {
                // Try without the refinement
                shrunk.push(base.as_ref().clone());
            }
            TypeKind::Generic { base, args } => {
                // Try without type arguments
                shrunk.push(base.as_ref().clone());
                // Try type arguments individually
                for arg in args {
                    shrunk.push(arg.clone());
                }
            }
            _ => {
                // For primitive types, try simpler primitives
                if !matches!(self.kind, TypeKind::Int | TypeKind::Unit) {
                    shrunk.push(ArbitraryType::new("Int".to_string(), TypeKind::Int, 0));
                }
            }
        }

        // Filter out types that are not simpler
        shrunk.retain(|s| s.complexity < self.complexity);
        shrunk
    }

    /// Check if this type is numeric
    pub fn is_numeric(&self) -> bool {
        matches!(self.kind, TypeKind::Int | TypeKind::Float)
    }

    /// Check if this type is equatable
    pub fn is_equatable(&self) -> bool {
        match &self.kind {
            TypeKind::Int
            | TypeKind::Float
            | TypeKind::Bool
            | TypeKind::Text
            | TypeKind::Char
            | TypeKind::Unit => true,
            TypeKind::List(inner) | TypeKind::Maybe(inner) | TypeKind::Set(inner) => {
                inner.is_equatable()
            }
            TypeKind::Tuple(types) => types.iter().all(|t| t.is_equatable()),
            _ => false,
        }
    }

    /// Check if this type is orderable
    pub fn is_orderable(&self) -> bool {
        matches!(
            self.kind,
            TypeKind::Int | TypeKind::Float | TypeKind::Text | TypeKind::Char
        )
    }
}

/// Type kind for structured representation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeKind {
    // Primitive types
    Int,
    Float,
    Bool,
    Text,
    Char,
    Unit,

    // Compound types
    List(Box<ArbitraryType>),
    Map(Box<ArbitraryType>, Box<ArbitraryType>),
    Maybe(Box<ArbitraryType>),
    Set(Box<ArbitraryType>),
    Tuple(Vec<ArbitraryType>),

    // Function type
    Function {
        params: Vec<ArbitraryType>,
        ret: Box<ArbitraryType>,
    },

    // Reference types (CBGR)
    Reference {
        mutable: bool,
        tier: RefTier,
        inner: Box<ArbitraryType>,
    },

    // Generic type
    Generic {
        base: Box<ArbitraryType>,
        args: Vec<ArbitraryType>,
    },

    // Named type (struct/enum)
    Named(String),

    // Type variable
    TypeVar(String),

    // Refined type
    Refined {
        base: Box<ArbitraryType>,
        predicate: String,
    },

    // Array type
    Array {
        element: Box<ArbitraryType>,
        size: Option<usize>,
    },

    // Slice type
    Slice(Box<ArbitraryType>),

    // Tensor type
    Tensor {
        element: Box<ArbitraryType>,
        shape: Vec<usize>,
    },
}

/// CBGR reference tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefTier {
    /// Tier 0: Full CBGR protection (~15ns overhead)
    Managed,
    /// Tier 1: Compiler-proven safe (0ns overhead)
    Checked,
    /// Tier 2: Manual safety proof (0ns overhead)
    Unsafe,
}

impl RefTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            RefTier::Managed => "&",
            RefTier::Checked => "&checked ",
            RefTier::Unsafe => "&unsafe ",
        }
    }

    pub fn all() -> &'static [RefTier] {
        &[RefTier::Managed, RefTier::Checked, RefTier::Unsafe]
    }
}

/// Type generator
pub struct TypeGenerator {
    config: GeneratorConfig,
    type_dist: WeightedIndex<u32>,
}

impl TypeGenerator {
    /// Create a new type generator with the given configuration
    pub fn new(config: GeneratorConfig) -> Self {
        let weights = config.weights.types.as_vec();
        let type_dist = WeightedIndex::new(&weights).unwrap();
        Self { config, type_dist }
    }

    /// Generate a random type
    pub fn generate<R: Rng>(&self, rng: &mut R) -> ArbitraryType {
        self.generate_type(rng, 0)
    }

    /// Generate a primitive type
    pub fn generate_primitive<R: Rng>(&self, rng: &mut R) -> ArbitraryType {
        self.generate_primitive_type(rng)
    }

    /// Generate a type at a given depth
    fn generate_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        // At max depth, only generate simple types
        if depth >= 3 {
            return self.generate_primitive_type(rng);
        }

        match self.type_dist.sample(rng) {
            0 => self.generate_primitive_type(rng),
            1 => self.generate_list_type(rng, depth),
            2 => self.generate_map_type(rng, depth),
            3 => self.generate_maybe_type(rng, depth),
            4 => self.generate_set_type(rng, depth),
            5 => self.generate_tuple_type(rng, depth),
            6 => self.generate_function_type(rng, depth),
            7 if self.config.features.cbgr => self.generate_reference_type(rng, depth),
            8 => self.generate_named_type(rng),
            _ => self.generate_primitive_type(rng),
        }
    }

    /// Generate a primitive type
    fn generate_primitive_type<R: Rng>(&self, rng: &mut R) -> ArbitraryType {
        let (source, kind) = match rng.random_range(0..6) {
            0 => ("Int".to_string(), TypeKind::Int),
            1 => ("Float".to_string(), TypeKind::Float),
            2 => ("Bool".to_string(), TypeKind::Bool),
            3 => ("Text".to_string(), TypeKind::Text),
            4 => ("Char".to_string(), TypeKind::Char),
            _ => ("Unit".to_string(), TypeKind::Unit),
        };

        ArbitraryType::new(source, kind, 0)
    }

    /// Generate a List type
    fn generate_list_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let inner = self.generate_type(rng, depth + 1);
        let source = format!("List<{}>", inner.source);

        ArbitraryType::new(source, TypeKind::List(Box::new(inner)), depth + 1)
    }

    /// Generate a Map type
    fn generate_map_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let key = self.generate_type(rng, depth + 1);
        let value = self.generate_type(rng, depth + 1);
        let source = format!("Map<{}, {}>", key.source, value.source);

        ArbitraryType::new(
            source,
            TypeKind::Map(Box::new(key), Box::new(value)),
            depth + 1,
        )
    }

    /// Generate a Maybe type
    fn generate_maybe_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let inner = self.generate_type(rng, depth + 1);
        let source = format!("Maybe<{}>", inner.source);

        ArbitraryType::new(source, TypeKind::Maybe(Box::new(inner)), depth + 1)
    }

    /// Generate a Set type
    fn generate_set_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let inner = self.generate_type(rng, depth + 1);
        let source = format!("Set<{}>", inner.source);

        ArbitraryType::new(source, TypeKind::Set(Box::new(inner)), depth + 1)
    }

    /// Generate a Tuple type
    fn generate_tuple_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let num_elements = rng.random_range(2..=4);
        let elements: Vec<ArbitraryType> = (0..num_elements)
            .map(|_| self.generate_type(rng, depth + 1))
            .collect();

        let source = format!(
            "({})",
            elements
                .iter()
                .map(|t| t.source.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );

        ArbitraryType::new(source, TypeKind::Tuple(elements), depth + 1)
    }

    /// Generate a Function type
    fn generate_function_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let num_params = rng.random_range(0..=3);
        let params: Vec<ArbitraryType> = (0..num_params)
            .map(|_| self.generate_type(rng, depth + 1))
            .collect();
        let ret = self.generate_type(rng, depth + 1);

        let params_str = params
            .iter()
            .map(|t| t.source.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("fn({}) -> {}", params_str, ret.source);

        ArbitraryType::new(
            source,
            TypeKind::Function {
                params,
                ret: Box::new(ret),
            },
            depth + 1,
        )
    }

    /// Generate a Reference type (CBGR)
    fn generate_reference_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let inner = self.generate_type(rng, depth + 1);
        let mutable = rng.random_bool(0.3);
        let tier = *RefTier::all().choose(rng).unwrap();

        let mut_str = if mutable { "mut " } else { "" };
        let source = format!("{}{}{}", tier.as_str(), mut_str, inner.source);

        ArbitraryType::new(
            source,
            TypeKind::Reference {
                mutable,
                tier,
                inner: Box::new(inner),
            },
            depth + 1,
        )
    }

    /// Generate a named type
    fn generate_named_type<R: Rng>(&self, rng: &mut R) -> ArbitraryType {
        let names = [
            "MyStruct", "Data", "Record", "Point", "Result", "Error", "Config", "State",
        ];
        let name = (*names.choose(rng).unwrap()).to_string();

        ArbitraryType::new(name.clone(), TypeKind::Named(name), 0)
    }

    /// Generate a refined type
    pub fn generate_refined_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let base = self.generate_type(rng, depth + 1);
        let predicate = self.generate_predicate(rng);
        let source = format!("{}{{{}}}", base.source, predicate);

        ArbitraryType::new(
            source,
            TypeKind::Refined {
                base: Box::new(base),
                predicate,
            },
            depth + 1,
        )
    }

    /// Generate a predicate for refinement types
    fn generate_predicate<R: Rng>(&self, rng: &mut R) -> String {
        let predicates = [
            "> 0",
            ">= 0",
            "< 100",
            "<= 100",
            "!= 0",
            "> -1000",
            "< 1000",
            "> 0 && < 100",
            ">= 0 && <= 255",
            "% 2 == 0",
        ];
        (*predicates.choose(rng).unwrap()).to_string()
    }

    /// Generate an array type
    pub fn generate_array_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let element = self.generate_type(rng, depth + 1);
        let size = if rng.random_bool(0.7) {
            Some(rng.random_range(1..100))
        } else {
            None
        };

        let source = match size {
            Some(n) => format!("[{}; {}]", element.source, n),
            None => format!("[{}]", element.source),
        };

        ArbitraryType::new(
            source,
            TypeKind::Array {
                element: Box::new(element),
                size,
            },
            depth + 1,
        )
    }

    /// Generate a tensor type
    pub fn generate_tensor_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let element = self.generate_primitive_type(rng);
        let num_dims = rng.random_range(1..=4);
        let shape: Vec<usize> = (0..num_dims).map(|_| rng.random_range(1..=16)).collect();

        let shape_str = shape
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("Tensor<{}, [{}]>", element.source, shape_str);

        ArbitraryType::new(
            source,
            TypeKind::Tensor {
                element: Box::new(element),
                shape,
            },
            depth + 1,
        )
    }

    /// Generate a type variable
    pub fn generate_type_var<R: Rng>(&self, rng: &mut R) -> ArbitraryType {
        let var_names = ["T", "U", "V", "W", "A", "B", "K", "V", "Item", "Output"];
        let name = (*var_names.choose(rng).unwrap()).to_string();

        ArbitraryType::new(name.clone(), TypeKind::TypeVar(name), 0)
    }

    /// Generate a generic type
    pub fn generate_generic_type<R: Rng>(&self, rng: &mut R, depth: usize) -> ArbitraryType {
        let base = self.generate_named_type(rng);
        let num_args = rng.random_range(1..=3);
        let args: Vec<ArbitraryType> = (0..num_args)
            .map(|_| self.generate_type(rng, depth + 1))
            .collect();

        let args_str = args
            .iter()
            .map(|t| t.source.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("{}<{}>", base.source, args_str);

        ArbitraryType::new(
            source,
            TypeKind::Generic {
                base: Box::new(base),
                args,
            },
            depth + 1,
        )
    }
}

/// Primitive types for convenience
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    Int,
    Float,
    Bool,
    Text,
    Char,
    Unit,
}

impl PrimitiveType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrimitiveType::Int => "Int",
            PrimitiveType::Float => "Float",
            PrimitiveType::Bool => "Bool",
            PrimitiveType::Text => "Text",
            PrimitiveType::Char => "Char",
            PrimitiveType::Unit => "Unit",
        }
    }

    pub fn all() -> &'static [PrimitiveType] {
        &[
            PrimitiveType::Int,
            PrimitiveType::Float,
            PrimitiveType::Bool,
            PrimitiveType::Text,
            PrimitiveType::Char,
            PrimitiveType::Unit,
        ]
    }

    pub fn to_arbitrary_type(&self) -> ArbitraryType {
        let kind = match self {
            PrimitiveType::Int => TypeKind::Int,
            PrimitiveType::Float => TypeKind::Float,
            PrimitiveType::Bool => TypeKind::Bool,
            PrimitiveType::Text => TypeKind::Text,
            PrimitiveType::Char => TypeKind::Char,
            PrimitiveType::Unit => TypeKind::Unit,
        };
        ArbitraryType::new(self.as_str().to_string(), kind, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_generate_type() {
        let config = GeneratorConfig::default();
        let generator = TypeGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..20 {
            let ty = generator.generate(&mut rng);
            assert!(!ty.source.is_empty());
        }
    }

    #[test]
    fn test_generate_primitive() {
        let config = GeneratorConfig::default();
        let generator = TypeGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        for _ in 0..10 {
            let ty = generator.generate_primitive(&mut rng);
            assert!(matches!(
                ty.kind,
                TypeKind::Int
                    | TypeKind::Float
                    | TypeKind::Bool
                    | TypeKind::Text
                    | TypeKind::Char
                    | TypeKind::Unit
            ));
        }
    }

    #[test]
    fn test_shrinking() {
        let inner = ArbitraryType::new("Int".to_string(), TypeKind::Int, 0);
        let list = ArbitraryType::new("List<Int>".to_string(), TypeKind::List(Box::new(inner)), 1);

        let shrunk = list.shrink();
        assert!(!shrunk.is_empty());
        assert!(shrunk.iter().all(|s| s.complexity < list.complexity));
    }

    #[test]
    fn test_type_properties() {
        let int_ty = ArbitraryType::new("Int".to_string(), TypeKind::Int, 0);
        assert!(int_ty.is_numeric());
        assert!(int_ty.is_equatable());
        assert!(int_ty.is_orderable());

        let bool_ty = ArbitraryType::new("Bool".to_string(), TypeKind::Bool, 0);
        assert!(!bool_ty.is_numeric());
        assert!(bool_ty.is_equatable());
        assert!(!bool_ty.is_orderable());
    }

    #[test]
    fn test_ref_tier() {
        assert_eq!(RefTier::Managed.as_str(), "&");
        assert_eq!(RefTier::Checked.as_str(), "&checked ");
        assert_eq!(RefTier::Unsafe.as_str(), "&unsafe ");
    }

    #[test]
    fn test_primitive_type() {
        for prim in PrimitiveType::all() {
            let ty = prim.to_arbitrary_type();
            assert_eq!(ty.source, prim.as_str());
        }
    }

    #[test]
    fn test_deterministic_with_seed() {
        let config = GeneratorConfig::default();
        let generator = TypeGenerator::new(config);

        let mut rng1 = ChaCha8Rng::seed_from_u64(12345);
        let mut rng2 = ChaCha8Rng::seed_from_u64(12345);

        let ty1 = generator.generate(&mut rng1);
        let ty2 = generator.generate(&mut rng2);

        assert_eq!(ty1.source, ty2.source);
    }

    #[test]
    fn test_generate_refined_type() {
        let config = GeneratorConfig::default();
        let generator = TypeGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let ty = generator.generate_refined_type(&mut rng, 0);
        assert!(ty.source.contains("{"));
        assert!(ty.source.contains("}"));
    }

    #[test]
    fn test_generate_tensor_type() {
        let config = GeneratorConfig::default();
        let generator = TypeGenerator::new(config);
        let mut rng = ChaCha8Rng::seed_from_u64(42);

        let ty = generator.generate_tensor_type(&mut rng, 0);
        assert!(ty.source.contains("Tensor"));
    }
}
