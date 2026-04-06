//! Protocol information for compile-time reflection
//!
//! Provides protocol/trait metadata matching core/meta/reflection.vr ProtocolInfo.

use verum_common::{List, Maybe, Text};

use super::function_info::FunctionInfo;
use super::generic_param::GenericParam;

/// Associated type information
///
/// Matches: core/meta/reflection.vr AssociatedTypeInfo
#[derive(Debug, Clone, PartialEq)]
pub struct AssociatedTypeInfo {
    /// Associated type name
    pub name: Text,
    /// Bounds on the associated type
    pub bounds: List<Text>,
    /// Default type (if any)
    pub default: Maybe<Text>,
    /// Documentation
    pub doc: Maybe<Text>,
}

impl AssociatedTypeInfo {
    /// Create a new associated type
    pub fn new(name: Text) -> Self {
        Self {
            name,
            bounds: List::new(),
            default: Maybe::None,
            doc: Maybe::None,
        }
    }

    /// Add a bound
    #[inline]
    pub fn with_bound(mut self, bound: Text) -> Self {
        self.bounds.push(bound);
        self
    }

    /// Set default type
    #[inline]
    pub fn with_default(mut self, default: Text) -> Self {
        self.default = Maybe::Some(default);
        self
    }
}

/// Information about a protocol (trait)
///
/// Matches: core/meta/reflection.vr ProtocolInfo
#[derive(Debug, Clone, PartialEq)]
pub struct ProtocolInfo {
    /// Protocol name
    pub name: Text,
    /// Full path
    pub path: Text,
    /// Generic parameters
    pub generics: List<GenericParam>,
    /// Super protocols (bounds)
    pub super_protocols: List<Text>,
    /// Associated types
    pub associated_types: List<AssociatedTypeInfo>,
    /// Required methods
    pub required_methods: List<FunctionInfo>,
    /// Provided methods (with default impl)
    pub provided_methods: List<FunctionInfo>,
    /// Protocol attributes
    pub attributes: List<Text>,
    /// Protocol documentation
    pub doc: Maybe<Text>,
}

impl ProtocolInfo {
    /// Create a new protocol info
    pub fn new(name: Text) -> Self {
        Self {
            name: name.clone(),
            path: name,
            generics: List::new(),
            super_protocols: List::new(),
            associated_types: List::new(),
            required_methods: List::new(),
            provided_methods: List::new(),
            attributes: List::new(),
            doc: Maybe::None,
        }
    }

    /// Check if protocol is marker (no methods)
    #[inline]
    pub fn is_marker(&self) -> bool {
        self.required_methods.is_empty() && self.provided_methods.is_empty()
    }

    /// Get all methods (required + provided)
    pub fn all_methods(&self) -> impl Iterator<Item = &FunctionInfo> {
        self.required_methods.iter().chain(self.provided_methods.iter())
    }

    /// Check if protocol has associated type
    pub fn has_associated_type(&self, name: &str) -> bool {
        self.associated_types.iter().any(|at| at.name.as_str() == name)
    }

    /// Get associated type by name
    pub fn get_associated_type(&self, name: &str) -> Option<&AssociatedTypeInfo> {
        self.associated_types
            .iter()
            .find(|at| at.name.as_str() == name)
    }
}
