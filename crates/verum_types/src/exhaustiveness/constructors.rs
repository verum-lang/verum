//! Type Constructor Enumeration
//!
//! This module provides the ability to enumerate all constructors for a given type.
//! Constructors represent the different ways a value of a type can be built.
//!
//! # Examples
//!
//! - `Bool` has two constructors: `true` and `false`
//! - `Maybe<T>` has two constructors: `Some(T)` and `None`
//! - `Int` has infinitely many constructors (each integer value)
//! - `(A, B)` has one constructor: the tuple constructor with A and B fields

use crate::context::TypeEnv;
use crate::ty::{Type, TypeVar};
use verum_common::{List, Text};

/// A constructor for a type
#[derive(Debug, Clone, PartialEq)]
pub struct Constructor {
    /// Name of the constructor (e.g., "Some", "None", "true", "Cons")
    pub name: Text,

    /// Types of constructor arguments
    /// Empty for nullary constructors like `None` or `true`
    pub arg_types: List<Type>,

    /// Whether this is the "default" or "wildcard" constructor
    /// Used for infinite types to represent "all other values"
    pub is_default: bool,
}

impl Constructor {
    /// Create a nullary constructor (no arguments)
    pub fn nullary(name: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            arg_types: List::new(),
            is_default: false,
        }
    }

    /// Create a constructor with arguments
    pub fn with_args(name: impl Into<Text>, args: List<Type>) -> Self {
        Self {
            name: name.into(),
            arg_types: args,
            is_default: false,
        }
    }

    /// Create a default constructor (represents "everything else")
    pub fn default_ctor(name: impl Into<Text>) -> Self {
        Self {
            name: name.into(),
            arg_types: List::new(),
            is_default: true,
        }
    }
}

/// Collection of constructors for a type
#[derive(Debug, Clone)]
pub struct TypeConstructors {
    /// The constructors for this type
    constructors: List<Constructor>,

    /// Whether the type has infinite constructors (e.g., Int)
    pub is_infinite: bool,

    /// Whether the type is empty (uninhabited, like Never)
    pub is_empty: bool,
}

impl TypeConstructors {
    /// Create a new constructor collection
    pub fn new(constructors: List<Constructor>, is_infinite: bool) -> Self {
        Self {
            constructors,
            is_infinite,
            is_empty: false,
        }
    }

    /// Create an empty type (uninhabited)
    pub fn empty() -> Self {
        Self {
            constructors: List::new(),
            is_infinite: false,
            is_empty: true,
        }
    }

    /// Create a single constructor type
    pub fn single(ctor: Constructor) -> Self {
        Self {
            constructors: List::from_iter([ctor]),
            is_infinite: false,
            is_empty: false,
        }
    }

    /// Check if this type has no constructors (uninhabited)
    pub fn is_empty_type(&self) -> bool {
        self.is_empty
    }

    /// Iterate over constructors
    pub fn iter(&self) -> impl Iterator<Item = &Constructor> {
        self.constructors.iter()
    }

    /// Number of constructors (may not include all if infinite)
    pub fn len(&self) -> usize {
        self.constructors.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.constructors.is_empty()
    }
}

/// Get all constructors for a type
///
/// This is the main entry point for constructor enumeration.
pub fn get_type_constructors(ty: &Type, env: &TypeEnv) -> TypeConstructors {
    match ty {
        // Unit type - single nullary constructor
        Type::Unit => TypeConstructors::single(Constructor::nullary("()")),

        // Bool - two constructors
        Type::Bool => TypeConstructors::new(
            List::from_iter([Constructor::nullary("true"), Constructor::nullary("false")]),
            false,
        ),

        // Never/Bottom - empty type
        Type::Never => TypeConstructors::empty(),

        // Numeric types - infinite constructors
        Type::Int | Type::Float => {
            // For infinite numeric types, we use a "default" constructor
            // that represents all values not explicitly matched
            TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true)
        }

        // Char - large but finite (Unicode code points)
        Type::Char => TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true),

        // String/Text - infinite
        Type::Text => TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true),

        // Tuple - single constructor with element types
        Type::Tuple(elements) => {
            TypeConstructors::single(Constructor::with_args("()", elements.clone()))
        }

        // Array - single constructor with element pattern
        Type::Array { element, size } => {
            if let Some(n) = size {
                let args = (0..*n)
                    .map(|_| element.as_ref().clone())
                    .collect();
                TypeConstructors::single(Constructor::with_args("[]", args))
            } else {
                // Dynamic array - treat as infinite (like a slice)
                TypeConstructors::new(List::from_iter([Constructor::default_ctor("[]")]), true)
            }
        }

        // Slice - dynamically sized, infinite constructors
        Type::Slice { .. } => {
            TypeConstructors::new(List::from_iter([Constructor::default_ctor("[]")]), true)
        }

        // Reference - single constructor (deref)
        Type::Reference { inner, .. }
        | Type::CheckedReference { inner, .. }
        | Type::Pointer { inner, .. } => {
            TypeConstructors::single(Constructor::with_args("&", List::from_iter([inner.as_ref().clone()])))
        }

        // Variant type (sum type) - one constructor per variant
        Type::Variant(variants) => {
            let ctors = variants
                .iter()
                .map(|(name, variant_ty)| {
                    if *variant_ty == Type::Unit {
                        Constructor::nullary(name.clone())
                    } else {
                        Constructor::with_args(name.clone(), List::from_iter([variant_ty.clone()]))
                    }
                })
                .collect();
            TypeConstructors::new(ctors, false)
        }

        // Generic types - look up in environment
        Type::Generic { name, args } => {
            get_generic_constructors(name, args, env)
        }

        // Named types - look up definition
        Type::Named { path, args } => {
            let type_name = path
                .segments
                .last()
                .map(|s| match s {
                    verum_ast::ty::PathSegment::Name(id) => Text::from(id.name.as_str()),
                    _ => Text::from(""),
                })
                .unwrap_or_default();
            get_generic_constructors(&type_name, args, env)
        }

        // Record type - single constructor with field types
        Type::Record(fields) => {
            let field_types: List<Type> = fields.values().cloned().collect();
            TypeConstructors::single(Constructor::with_args("{}", field_types))
        }

        // Type variable - unknown, treat as infinite
        Type::Var(_) => {
            TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true)
        }

        // Unknown - runtime typed, infinite possibilities
        Type::Unknown => {
            TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true)
        }

        // Other types - default to single wildcard constructor
        _ => TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true),
    }
}

/// Get constructors for generic/named types from environment
fn get_generic_constructors(name: &Text, args: &List<Type>, env: &TypeEnv) -> TypeConstructors {
    // Look up the type definition from the environment bindings.
    // If the type's scheme instantiates to a Variant, extract constructors from it.
    if let Some(scheme) = env.lookup(name.as_str()) {
        let resolved = scheme.instantiate();
        if let Type::Variant(variants) = &resolved {
            let ctors: List<Constructor> = variants
                .iter()
                .map(|(ctor_name, ctor_ty)| {
                    if *ctor_ty == Type::Unit {
                        Constructor::nullary(ctor_name.clone())
                    } else {
                        Constructor::with_args(ctor_name.clone(), List::from_iter([ctor_ty.clone()]))
                    }
                })
                .collect();
            if !ctors.is_empty() {
                return TypeConstructors::new(ctors, false);
            }
        }
    }

    // Unknown type - treat as infinite for safety
    TypeConstructors::new(List::from_iter([Constructor::default_ctor("_")]), true)
}

/// Check if a constructor matches a specific name
pub fn constructor_matches_name(ctor: &Constructor, name: &str) -> bool {
    ctor.name.as_str() == name || ctor.is_default
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bool_constructors() {
        let env = TypeEnv::default();
        let ctors = get_type_constructors(&Type::Bool, &env);
        assert_eq!(ctors.len(), 2);
        assert!(!ctors.is_infinite);
    }

    #[test]
    fn test_int_constructors() {
        let env = TypeEnv::default();
        let ctors = get_type_constructors(&Type::Int, &env);
        assert!(ctors.is_infinite);
    }

    #[test]
    fn test_unit_constructors() {
        let env = TypeEnv::default();
        let ctors = get_type_constructors(&Type::Unit, &env);
        assert_eq!(ctors.len(), 1);
    }

    #[test]
    fn test_never_constructors() {
        let env = TypeEnv::default();
        let ctors = get_type_constructors(&Type::Never, &env);
        assert!(ctors.is_empty_type());
    }
}
