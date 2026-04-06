//! Core type definitions for meta system
//!
//! This module provides the fundamental type definition structures for
//! struct/enum field and variant lookup during compile-time execution.
//!
//! ## Industrial-Grade Features
//!
//! This module supports comprehensive type introspection:
//! - Field and variant lookup
//! - Attribute retrieval with values
//! - Documentation strings
//! - Associated types
//! - Super types (protocol inheritance)
//! - Method signatures
//!
//! Verum unified meta-system: all compile-time computation uses `meta` (meta fn,
//! @tagged_literal, @derive, @interpolation_handler). Multi-pass architecture:
//! Pass 1 parses and registers meta handlers, Pass 2 expands using complete
//! registry, Pass 3+ performs semantic analysis. Sandboxed execution (no I/O).

use verum_ast::ty::Type;
use verum_ast::MetaValue;
use verum_common::{List, Maybe, Text};

/// Attribute definition with optional value
///
/// Represents a compile-time attribute like `@derive(Debug)` or `@deprecated("use X instead")`.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeAttribute {
    /// Attribute name (e.g., "derive", "deprecated", "repr")
    pub name: Text,
    /// Attribute arguments/value (e.g., [Debug, Clone] or "use X instead")
    pub value: Maybe<MetaValue>,
    /// Span where the attribute was defined
    pub span: Maybe<verum_ast::Span>,
}

impl TypeAttribute {
    /// Create a simple attribute without a value
    pub fn simple(name: impl Into<Text>) -> Self {
        TypeAttribute {
            name: name.into(),
            value: Maybe::None,
            span: Maybe::None,
        }
    }

    /// Create an attribute with a value
    pub fn with_value(name: impl Into<Text>, value: MetaValue) -> Self {
        TypeAttribute {
            name: name.into(),
            value: Maybe::Some(value),
            span: Maybe::None,
        }
    }
}

/// Method signature for protocol/type methods
#[derive(Debug, Clone, PartialEq)]
pub struct MethodSignature {
    /// Method name
    pub name: Text,
    /// Parameter types
    pub params: List<Type>,
    /// Return type
    pub return_type: Type,
    /// Whether the method takes self by reference
    pub takes_self: bool,
    /// Whether self is mutable
    pub self_mutable: bool,
    /// Documentation for the method
    pub doc: Maybe<Text>,
}

/// Type definition for struct/enum field and variant lookup
///
/// This enables `fields_of` and `variants_of` to work with named types
/// by storing their definitions at compile time.
///
/// ## Industrial-Grade Features
///
/// Each type definition now includes:
/// - Attributes (`@derive`, `@repr`, etc.)
/// - Documentation strings
/// - Associated types
/// - Super types (protocols/traits implemented)
/// - Method signatures
#[derive(Debug, Clone)]
pub enum TypeDefinition {
    /// Struct type with named fields
    Struct {
        /// Struct name
        name: Text,
        /// List of (field_name, field_type) pairs
        fields: List<(Text, Type)>,
        /// Type attributes (e.g., @derive(Debug), @repr(C))
        attributes: List<TypeAttribute>,
        /// Documentation string
        doc: Maybe<Text>,
        /// Associated types
        associated_types: List<(Text, Type)>,
        /// Super types (protocols this type implements)
        super_types: List<Text>,
        /// Methods defined on this type
        methods: List<MethodSignature>,
        /// Generic parameters (e.g., T, U in Point<T, U>)
        generic_params: List<Text>,
    },
    /// Enum type with variants
    Enum {
        /// Enum name
        name: Text,
        /// List of (variant_name, variant_type) pairs
        variants: List<(Text, Type)>,
        /// Type attributes
        attributes: List<TypeAttribute>,
        /// Documentation string
        doc: Maybe<Text>,
        /// Associated types
        associated_types: List<(Text, Type)>,
        /// Super types (protocols this type implements)
        super_types: List<Text>,
        /// Methods defined on this enum
        methods: List<MethodSignature>,
        /// Generic parameters
        generic_params: List<Text>,
    },
    /// Protocol/trait definition
    Protocol {
        /// Protocol name
        name: Text,
        /// Required methods with signatures
        methods: List<MethodSignature>,
        /// Type attributes
        attributes: List<TypeAttribute>,
        /// Documentation string
        doc: Maybe<Text>,
        /// Associated types that implementors must define
        associated_types: List<(Text, Maybe<Type>)>,
        /// Super protocols (this protocol extends)
        super_protocols: List<Text>,
        /// Generic parameters
        generic_params: List<Text>,
    },
    /// Type alias
    Alias {
        /// Alias name
        name: Text,
        /// The type this aliases to
        target: Type,
        /// Type attributes
        attributes: List<TypeAttribute>,
        /// Documentation string
        doc: Maybe<Text>,
        /// Generic parameters
        generic_params: List<Text>,
    },
    /// Newtype wrapper
    Newtype {
        /// Newtype name
        name: Text,
        /// The wrapped type
        inner: Type,
        /// Type attributes
        attributes: List<TypeAttribute>,
        /// Documentation string
        doc: Maybe<Text>,
        /// Methods defined on this newtype
        methods: List<MethodSignature>,
    },
}

impl TypeDefinition {
    /// Get the name of this type definition
    pub fn name(&self) -> &Text {
        match self {
            TypeDefinition::Struct { name, .. } => name,
            TypeDefinition::Enum { name, .. } => name,
            TypeDefinition::Protocol { name, .. } => name,
            TypeDefinition::Alias { name, .. } => name,
            TypeDefinition::Newtype { name, .. } => name,
        }
    }

    /// Get attributes for this type
    pub fn attributes(&self) -> &[TypeAttribute] {
        match self {
            TypeDefinition::Struct { attributes, .. } => attributes,
            TypeDefinition::Enum { attributes, .. } => attributes,
            TypeDefinition::Protocol { attributes, .. } => attributes,
            TypeDefinition::Alias { attributes, .. } => attributes,
            TypeDefinition::Newtype { attributes, .. } => attributes,
        }
    }

    /// Get documentation for this type
    pub fn doc(&self) -> Maybe<&Text> {
        match self {
            TypeDefinition::Struct { doc, .. } => doc.as_ref(),
            TypeDefinition::Enum { doc, .. } => doc.as_ref(),
            TypeDefinition::Protocol { doc, .. } => doc.as_ref(),
            TypeDefinition::Alias { doc, .. } => doc.as_ref(),
            TypeDefinition::Newtype { doc, .. } => doc.as_ref(),
        }
    }

    /// Get associated types
    pub fn associated_types(&self) -> List<(Text, Type)> {
        match self {
            TypeDefinition::Struct { associated_types, .. } => associated_types.clone(),
            TypeDefinition::Enum { associated_types, .. } => associated_types.clone(),
            TypeDefinition::Protocol { associated_types, .. } => {
                associated_types
                    .iter()
                    .filter_map(|(name, ty)| {
                        ty.as_ref().map(|t| (name.clone(), t.clone()))
                    })
                    .collect()
            }
            TypeDefinition::Alias { .. } | TypeDefinition::Newtype { .. } => List::new(),
        }
    }

    /// Get super types/protocols
    pub fn super_types(&self) -> List<Text> {
        match self {
            TypeDefinition::Struct { super_types, .. } => super_types.clone(),
            TypeDefinition::Enum { super_types, .. } => super_types.clone(),
            TypeDefinition::Protocol { super_protocols, .. } => super_protocols.clone(),
            TypeDefinition::Alias { .. } | TypeDefinition::Newtype { .. } => List::new(),
        }
    }

    /// Get methods defined on this type
    pub fn get_methods(&self) -> &[MethodSignature] {
        match self {
            TypeDefinition::Struct { methods, .. } => methods,
            TypeDefinition::Enum { methods, .. } => methods,
            TypeDefinition::Protocol { methods, .. } => methods,
            TypeDefinition::Newtype { methods, .. } => methods,
            TypeDefinition::Alias { .. } => &[],
        }
    }

    /// Check if type has a specific attribute
    pub fn has_attribute(&self, attr_name: &str) -> bool {
        self.attributes().iter().any(|a| a.name.as_str() == attr_name)
    }

    /// Get attribute value by name
    pub fn get_attribute(&self, attr_name: &str) -> Option<&TypeAttribute> {
        self.attributes().iter().find(|a| a.name.as_str() == attr_name)
    }

    /// Create a simple struct definition (for backward compatibility)
    pub fn simple_struct(name: Text, fields: List<(Text, Type)>) -> Self {
        TypeDefinition::Struct {
            name,
            fields,
            attributes: List::new(),
            doc: Maybe::None,
            associated_types: List::new(),
            super_types: List::new(),
            methods: List::new(),
            generic_params: List::new(),
        }
    }

    /// Create a simple enum definition (for backward compatibility)
    pub fn simple_enum(name: Text, variants: List<(Text, Type)>) -> Self {
        TypeDefinition::Enum {
            name,
            variants,
            attributes: List::new(),
            doc: Maybe::None,
            associated_types: List::new(),
            super_types: List::new(),
            methods: List::new(),
            generic_params: List::new(),
        }
    }

    /// Create a simple protocol definition (for backward compatibility)
    pub fn simple_protocol(name: Text, method_names: List<Text>) -> Self {
        let methods = method_names
            .iter()
            .map(|n| MethodSignature {
                name: n.clone(),
                params: List::new(),
                return_type: Type::unit(verum_ast::Span::dummy()),
                takes_self: true,
                self_mutable: false,
                doc: Maybe::None,
            })
            .collect();
        TypeDefinition::Protocol {
            name,
            methods,
            attributes: List::new(),
            doc: Maybe::None,
            associated_types: List::new(),
            super_protocols: List::new(),
            generic_params: List::new(),
        }
    }
}

/// Protocol implementation record
///
/// Tracks which types implement which protocols and with what methods.
#[derive(Debug, Clone)]
pub struct ProtocolImplementation {
    /// The type that implements the protocol
    pub implementing_type: Text,
    /// The protocol being implemented
    pub protocol_name: Text,
    /// Methods provided by this implementation
    pub implemented_methods: List<Text>,
}
