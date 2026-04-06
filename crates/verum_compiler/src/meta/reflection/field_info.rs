//! Field information for compile-time reflection
//!
//! Provides detailed field metadata for struct types, matching core/meta/reflection.vr FieldInfo.

use verum_ast::{ty::Type, MetaValue};
use verum_common::{List, Maybe, Text};

use super::type_kind::{TypeKind, Visibility};

/// Rich field information for compile-time reflection
///
/// Used by `fields_of` to provide detailed field metadata.
/// Matches: core/meta/reflection.vr FieldInfo
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInfo {
    /// Field name (empty for tuple struct fields)
    pub name: Text,
    /// Field index (position in struct)
    pub index: i64,
    /// Field type name
    pub type_name: Text,
    /// Field type kind
    pub type_kind: TypeKind,
    /// Field type as AST Type (for internal use)
    pub field_type: Type,
    /// Field visibility
    pub visibility: Visibility,
    /// Whether field is mutable
    pub is_mutable: bool,
    /// Field attributes
    pub attributes: List<Text>,
    /// Optional documentation comment
    pub doc: Maybe<Text>,
}

impl FieldInfo {
    /// Create a new field info with minimal data
    pub fn new(name: Text, field_type: Type, index: i64) -> Self {
        let type_name = Text::from(format!("{:?}", field_type));
        Self {
            name,
            index,
            type_name,
            type_kind: TypeKind::Unknown,
            field_type,
            visibility: Visibility::Public,
            is_mutable: false,
            attributes: List::new(),
            doc: Maybe::None,
        }
    }

    /// Add documentation to field
    #[inline]
    pub fn with_doc(mut self, doc: Text) -> Self {
        self.doc = Maybe::Some(doc);
        self
    }

    /// Set visibility
    #[inline]
    pub fn with_visibility(mut self, visibility: Visibility) -> Self {
        self.visibility = visibility;
        self
    }

    /// Set type kind
    #[inline]
    pub fn with_type_kind(mut self, kind: TypeKind) -> Self {
        self.type_kind = kind;
        self
    }

    /// Add an attribute
    #[inline]
    pub fn with_attribute(mut self, attr: Text) -> Self {
        self.attributes.push(attr);
        self
    }

    /// Check if field has specific attribute
    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes.iter().any(|a| a.as_str() == name)
    }

    /// Check if field is public
    #[inline]
    pub fn is_public(&self) -> bool {
        self.visibility.is_public()
    }

    /// Check if this is a tuple field (has numeric index, no name)
    pub fn is_tuple_field(&self) -> bool {
        self.name.is_empty() || self.name.chars().all(|c| c.is_ascii_digit())
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.index as i128),
                MetaValue::Text(self.type_name.clone()),
                self.type_kind.to_meta_value(),
                MetaValue::Bool(self.visibility.is_public()),
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

/// Information about a field's memory layout
///
/// Matches: core/meta/reflection.vr FieldOffset
#[derive(Debug, Clone, PartialEq)]
pub struct FieldOffset {
    /// Field name
    pub name: Text,
    /// Byte offset from start of struct
    pub offset: i64,
    /// Field size in bytes
    pub size: i64,
    /// Field alignment in bytes
    pub align: i64,
    /// Padding bytes before this field
    pub padding_before: i64,
}

impl FieldOffset {
    /// Create a new field offset
    pub fn new(name: Text, offset: i64, size: i64, align: i64) -> Self {
        Self {
            name,
            offset,
            size,
            align,
            padding_before: 0,
        }
    }

    /// Get end position (offset + size)
    #[inline]
    pub fn end(&self) -> i64 {
        self.offset + self.size
    }

    /// Check if field has padding before it
    #[inline]
    pub fn has_padding(&self) -> bool {
        self.padding_before > 0
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.offset as i128),
                MetaValue::Int(self.size as i128),
                MetaValue::Int(self.align as i128),
                MetaValue::Int(self.padding_before as i128),
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
