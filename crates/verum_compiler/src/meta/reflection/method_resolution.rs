//! Method resolution for compile-time reflection
//!
//! Provides method resolution metadata matching core/meta/reflection.vr MethodResolution.

use verum_ast::MetaValue;
use verum_common::{Maybe, Text};

use super::function_info::FunctionInfo;

/// Source of a resolved method
///
/// Matches: core/meta/reflection.vr MethodSource
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MethodSource {
    /// Inherent method (defined directly on type)
    Inherent = 0,
    /// Protocol method (from trait implementation)
    Protocol = 1,
    /// Auto-derived method
    Derived = 2,
    /// Compiler-generated method
    Generated = 3,
}

/// Result of method resolution for a type
///
/// Matches: core/meta/reflection.vr MethodResolution
#[derive(Debug, Clone, PartialEq)]
pub struct MethodResolution {
    /// The resolved function info
    pub function: FunctionInfo,
    /// Where the method comes from
    pub source: MethodSource,
    /// Protocol that provides the method (if from protocol)
    pub providing_protocol: Maybe<Text>,
    /// Whether this is a default implementation
    pub is_default_impl: bool,
}

impl MethodResolution {
    /// Create a new method resolution
    pub fn inherent(function: FunctionInfo) -> Self {
        Self {
            function,
            source: MethodSource::Inherent,
            providing_protocol: Maybe::None,
            is_default_impl: false,
        }
    }

    /// Create a protocol method resolution
    pub fn protocol(function: FunctionInfo, protocol: Text, is_default: bool) -> Self {
        Self {
            function,
            source: MethodSource::Protocol,
            providing_protocol: Maybe::Some(protocol),
            is_default_impl: is_default,
        }
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                self.function.to_meta_value(),
                MetaValue::Int(self.source as i128),
                match &self.providing_protocol {
                    Maybe::Some(p) => MetaValue::Text(p.clone()),
                    Maybe::None => MetaValue::Unit,
                },
                MetaValue::Bool(self.is_default_impl),
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
