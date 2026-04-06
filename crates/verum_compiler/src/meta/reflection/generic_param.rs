//! Generic parameter information for compile-time reflection
//!
//! Provides generic type parameter metadata matching core/meta/reflection.vr GenericParam.

use verum_ast::MetaValue;
use verum_common::{List, Maybe, Text};

/// Generic parameter kind matching stdlib GenericParamKind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GenericParamKind {
    /// Type parameter: `T` in `fn foo<T>()`
    Type = 0,
    /// Lifetime parameter: `'a` in `fn foo<'a>()`
    Lifetime = 1,
    /// Const parameter: `N` in `fn foo<const N: Int>()`
    Const = 2,
}

/// Information about a generic type parameter
///
/// Matches: core/meta/reflection.vr GenericParam
#[derive(Debug, Clone, PartialEq)]
pub struct GenericParam {
    /// Parameter name
    pub name: Text,
    /// Parameter index
    pub index: i64,
    /// Parameter kind
    pub kind: GenericParamKind,
    /// Bounds on this parameter
    pub bounds: List<Text>,
    /// Default value (if any)
    pub default: Maybe<Text>,
}

impl GenericParam {
    /// Create a new type parameter
    pub fn type_param(name: Text, index: i64) -> Self {
        Self {
            name,
            index,
            kind: GenericParamKind::Type,
            bounds: List::new(),
            default: Maybe::None,
        }
    }

    /// Create a new lifetime parameter
    pub fn lifetime_param(name: Text, index: i64) -> Self {
        Self {
            name,
            index,
            kind: GenericParamKind::Lifetime,
            bounds: List::new(),
            default: Maybe::None,
        }
    }

    /// Create a new const parameter
    pub fn const_param(name: Text, index: i64) -> Self {
        Self {
            name,
            index,
            kind: GenericParamKind::Const,
            bounds: List::new(),
            default: Maybe::None,
        }
    }

    /// Add a bound
    #[inline]
    pub fn with_bound(mut self, bound: Text) -> Self {
        self.bounds.push(bound);
        self
    }

    /// Set default value
    #[inline]
    pub fn with_default(mut self, default: Text) -> Self {
        self.default = Maybe::Some(default);
        self
    }

    /// Check if parameter has bound
    pub fn has_bound(&self, bound: &str) -> bool {
        self.bounds.iter().any(|b| b.as_str() == bound)
    }

    /// Check if parameter is constrained
    #[inline]
    pub fn is_constrained(&self) -> bool {
        !self.bounds.is_empty()
    }

    /// Check if parameter has default
    #[inline]
    pub fn has_default(&self) -> bool {
        matches!(self.default, Maybe::Some(_))
    }

    /// Get declaration syntax
    pub fn declaration(&self) -> Text {
        let mut decl = self.name.clone();

        if !self.bounds.is_empty() {
            let bounds_str: Vec<&str> = self.bounds.iter().map(|b| b.as_str()).collect();
            decl = Text::from(format!("{}: {}", decl.as_str(), bounds_str.join(" + ")));
        }

        if let Maybe::Some(ref def) = self.default {
            decl = Text::from(format!("{} = {}", decl.as_str(), def.as_str()));
        }

        decl
    }

    /// Convert to MetaValue
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.index as i128),
                MetaValue::Int(self.kind as i128),
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

/// Information about a lifetime parameter
///
/// Matches: core/meta/reflection.vr LifetimeParam
#[derive(Debug, Clone, PartialEq)]
pub struct LifetimeParam {
    /// Lifetime name (e.g., "a" for 'a, "static" for 'static)
    pub name: Text,
    /// Parameter index
    pub index: i64,
    /// Bounds on this lifetime (other lifetimes it must outlive)
    pub bounds: List<Text>,
    /// Whether this is the 'static lifetime
    pub is_static: bool,
    /// Whether this is an anonymous lifetime ('_)
    pub is_anonymous: bool,
}

impl LifetimeParam {
    /// Create a new lifetime parameter
    pub fn new(name: Text, index: i64) -> Self {
        let is_static = name.as_str() == "static";
        let is_anonymous = name.as_str() == "_";
        Self {
            name,
            index,
            bounds: List::new(),
            is_static,
            is_anonymous,
        }
    }

    /// Get lifetime syntax ('a, 'static, etc.)
    pub fn syntax(&self) -> Text {
        if self.is_static {
            Text::from("'static")
        } else if self.is_anonymous {
            Text::from("'_")
        } else {
            Text::from(format!("'{}", self.name.as_str()))
        }
    }

    /// Get declaration with bounds
    pub fn declaration(&self) -> Text {
        let base = self.syntax();
        if self.bounds.is_empty() {
            base
        } else {
            let bounds: Vec<String> = self
                .bounds
                .iter()
                .map(|b| format!("'{}", b.as_str()))
                .collect();
            Text::from(format!("{}: {}", base.as_str(), bounds.join(" + ")))
        }
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.name.clone()),
                MetaValue::Int(self.index as i128),
                MetaValue::Bool(self.is_static),
                MetaValue::Bool(self.is_anonymous),
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
