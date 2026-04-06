//! Complete type information for compile-time reflection
//!
//! Provides comprehensive type metadata combining all reflection components.

use verum_common::{List, Maybe, Text};

use super::field_info::FieldInfo;
use super::function_info::FunctionInfo;
use super::generic_param::GenericParam;
use super::type_kind::TypeKind;
use super::variant_info::VariantInfo;

/// Complete type information for compile-time reflection
///
/// Provides comprehensive type metadata including fields, variants, and protocol implementations.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// Type name
    pub name: Text,
    /// Type kind
    pub kind: TypeKind,
    /// Generic parameters
    pub generics: List<GenericParam>,
    /// Documentation comment
    pub doc: Maybe<Text>,
    /// Type attributes
    pub attributes: List<Text>,
    /// Protocols this type implements
    pub implements: List<Text>,
    /// Fields (for struct types)
    pub fields: List<FieldInfo>,
    /// Variants (for enum types)
    pub variants: List<VariantInfo>,
    /// Methods (for protocol types)
    pub methods: List<FunctionInfo>,
}

impl TypeInfo {
    /// Create a new type info
    pub fn new(name: Text, kind: TypeKind) -> Self {
        Self {
            name,
            kind,
            generics: List::new(),
            doc: Maybe::None,
            attributes: List::new(),
            implements: List::new(),
            fields: List::new(),
            variants: List::new(),
            methods: List::new(),
        }
    }

    /// Create a struct type info
    pub fn struct_type(name: Text, fields: List<FieldInfo>) -> Self {
        Self {
            name,
            kind: TypeKind::Struct,
            generics: List::new(),
            doc: Maybe::None,
            attributes: List::new(),
            implements: List::new(),
            fields,
            variants: List::new(),
            methods: List::new(),
        }
    }

    /// Create an enum type info
    pub fn enum_type(name: Text, variants: List<VariantInfo>) -> Self {
        Self {
            name,
            kind: TypeKind::Enum,
            generics: List::new(),
            doc: Maybe::None,
            attributes: List::new(),
            implements: List::new(),
            fields: List::new(),
            variants,
            methods: List::new(),
        }
    }

    /// Check if type is a struct
    #[inline]
    pub fn is_struct(&self) -> bool {
        self.kind == TypeKind::Struct
    }

    /// Check if type is an enum
    #[inline]
    pub fn is_enum(&self) -> bool {
        self.kind == TypeKind::Enum
    }

    /// Check if type implements a protocol
    pub fn implements_protocol(&self, protocol: &str) -> bool {
        self.implements.iter().any(|p| p.as_str() == protocol)
    }

    /// Get field by name
    pub fn get_field(&self, name: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| f.name.as_str() == name)
    }

    /// Get variant by name
    pub fn get_variant(&self, name: &str) -> Option<&VariantInfo> {
        self.variants.iter().find(|v| v.name.as_str() == name)
    }
}
