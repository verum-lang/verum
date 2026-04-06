//! Function parameter information for compile-time reflection
//!
//! Provides parameter metadata matching core/meta/reflection.vr ParamInfo.

use verum_ast::MetaValue;
use verum_common::{List, Maybe, Text};

use super::type_kind::TypeKind;

/// Self parameter kind matching stdlib SelfKind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SelfKind {
    /// `self` - takes ownership
    Value = 0,
    /// `&self` - shared reference
    Ref = 1,
    /// `&mut self` - mutable reference
    RefMut = 2,
}

/// Function parameter information
///
/// Matches: core/meta/reflection.vr ParamInfo
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    /// Parameter name
    pub name: Text,
    /// Parameter index
    pub index: i64,
    /// Parameter type name
    pub type_name: Text,
    /// Parameter type kind
    pub type_kind: TypeKind,
    /// Whether parameter is mutable
    pub is_mut: bool,
    /// Whether this is self parameter
    pub is_self_param: bool,
    /// Self reference kind (if self parameter)
    pub self_kind: Maybe<SelfKind>,
    /// Default value (if any)
    pub default: Maybe<Text>,
    /// Parameter attributes
    pub attributes: List<Text>,
}

impl ParamInfo {
    /// Create a new parameter
    pub fn new(name: Text, index: i64, type_name: Text) -> Self {
        Self {
            name,
            index,
            type_name,
            type_kind: TypeKind::Unknown,
            is_mut: false,
            is_self_param: false,
            self_kind: Maybe::None,
            default: Maybe::None,
            attributes: List::new(),
        }
    }

    /// Create a self parameter
    pub fn self_param(kind: SelfKind) -> Self {
        Self {
            name: Text::from("self"),
            index: 0,
            type_name: Text::from("Self"),
            type_kind: TypeKind::Unknown,
            is_mut: matches!(kind, SelfKind::RefMut),
            is_self_param: true,
            self_kind: Maybe::Some(kind),
            default: Maybe::None,
            attributes: List::new(),
        }
    }

    /// Check if this is a self parameter
    #[inline]
    pub fn is_self(&self) -> bool {
        self.is_self_param
    }

    /// Get declaration syntax
    pub fn declaration(&self) -> Text {
        if self.is_self_param {
            match self.self_kind {
                Maybe::Some(SelfKind::Value) => Text::from("self"),
                Maybe::Some(SelfKind::Ref) => Text::from("&self"),
                Maybe::Some(SelfKind::RefMut) => Text::from("&mut self"),
                Maybe::None => Text::from("self"),
            }
        } else {
            let mut decl = self.name.clone();
            if self.is_mut {
                decl = Text::from(format!("mut {}", decl.as_str()));
            }
            decl = Text::from(format!("{}: {}", decl.as_str(), self.type_name.as_str()));
            if let Maybe::Some(ref def) = self.default {
                decl = Text::from(format!("{} = {}", decl.as_str(), def.as_str()));
            }
            decl
        }
    }

    /// Convert to MetaValue
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.index as i128),
                MetaValue::Text(self.type_name.clone()),
                MetaValue::Bool(self.is_self_param),
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
