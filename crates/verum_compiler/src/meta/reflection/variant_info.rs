//! Variant information for compile-time reflection
//!
//! Provides detailed enum variant metadata matching core/meta/reflection.vr VariantInfo.

use verum_ast::MetaValue;
use verum_common::{List, Maybe, Text};

use super::field_info::FieldInfo;
use super::type_kind::VariantKind;

/// Rich variant information for compile-time reflection
///
/// Used by `variants_of` to provide detailed enum variant metadata.
/// Matches: core/meta/reflection.vr VariantInfo
#[derive(Debug, Clone, PartialEq)]
pub struct VariantInfo {
    /// Variant name
    pub name: Text,
    /// Variant index (discriminant)
    pub index: i64,
    /// Variant kind
    pub kind: VariantKind,
    /// Fields (for struct/tuple variants)
    pub fields: List<FieldInfo>,
    /// Variant attributes
    pub attributes: List<Text>,
    /// Variant documentation
    pub doc: Maybe<Text>,
}

impl VariantInfo {
    /// Create a unit variant info
    pub fn unit(name: Text, index: i64) -> Self {
        Self {
            name,
            index,
            kind: VariantKind::Unit,
            fields: List::new(),
            attributes: List::new(),
            doc: Maybe::None,
        }
    }

    /// Create a tuple variant info
    pub fn tuple(name: Text, fields: List<FieldInfo>, index: i64) -> Self {
        Self {
            name,
            index,
            kind: VariantKind::Tuple,
            fields,
            attributes: List::new(),
            doc: Maybe::None,
        }
    }

    /// Create a struct variant info
    pub fn record(name: Text, fields: List<FieldInfo>, index: i64) -> Self {
        Self {
            name,
            index,
            kind: VariantKind::Struct,
            fields,
            attributes: List::new(),
            doc: Maybe::None,
        }
    }

    /// Add documentation
    #[inline]
    pub fn with_doc(mut self, doc: Text) -> Self {
        self.doc = Maybe::Some(doc);
        self
    }

    /// Check if variant has specific attribute
    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.iter().any(|a| a.as_str() == name)
    }

    /// Check if this is a unit variant (no data)
    #[inline]
    pub fn is_unit(&self) -> bool {
        self.kind == VariantKind::Unit
    }

    /// Check if this is a tuple variant
    #[inline]
    pub fn is_tuple(&self) -> bool {
        self.kind == VariantKind::Tuple
    }

    /// Check if this is a struct variant
    #[inline]
    pub fn is_struct(&self) -> bool {
        self.kind == VariantKind::Struct
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.index as i128),
                MetaValue::Int(self.kind as i128),
                MetaValue::Int(self.fields.len() as i128),
            ]
            .into_iter()
            .collect(),
        )
    }

    /// Alias for to_meta_value for backward compatibility
    #[inline]
    pub fn to_const_value(&self) -> MetaValue {
        self.to_meta_value()
    }
}
