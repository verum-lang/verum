//! Trait bound information for compile-time reflection
//!
//! Provides trait/protocol bound metadata matching core/meta/reflection.vr TraitBound.

use verum_ast::MetaValue;
use verum_common::{List, Text};

/// Information about a trait/protocol bound
///
/// Matches: core/meta/reflection.vr TraitBound
#[derive(Debug, Clone, PartialEq)]
pub struct TraitBound {
    /// Protocol/trait name
    pub protocol_name: Text,
    /// Full path to protocol
    pub protocol_path: Text,
    /// Generic arguments to the protocol
    pub type_args: List<Text>,
    /// Associated type bindings (e.g., `Item = Int`)
    pub associated_types: List<(Text, Text)>,
    /// Whether this is a negative bound (`!Send`)
    pub is_negative: bool,
    /// Whether this is a maybe bound (`?Sized`)
    pub is_maybe: bool,
}

impl TraitBound {
    /// Create a new trait bound
    pub fn new(name: Text) -> Self {
        Self {
            protocol_name: name.clone(),
            protocol_path: name,
            type_args: List::new(),
            associated_types: List::new(),
            is_negative: false,
            is_maybe: false,
        }
    }

    /// Get simplified bound syntax
    pub fn syntax(&self) -> Text {
        let mut result = self.protocol_name.clone();

        if !self.type_args.is_empty() || !self.associated_types.is_empty() {
            let mut args: Vec<String> = self.type_args.iter().map(|t| t.to_string()).collect();
            for (name, ty) in &self.associated_types {
                args.push(format!("{} = {}", name.as_str(), ty.as_str()));
            }
            result = Text::from(format!("{}<{}>", result.as_str(), args.join(", ")));
        }

        if self.is_negative {
            result = Text::from(format!("!{}", result.as_str()));
        } else if self.is_maybe {
            result = Text::from(format!("?{}", result.as_str()));
        }

        result
    }

    /// Check if this is a marker trait bound
    pub fn is_marker(&self) -> bool {
        matches!(
            self.protocol_name.as_str(),
            "Copy" | "Clone" | "Send" | "Sync" | "Sized" | "Unpin"
        )
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Text(self.protocol_name.clone()),
                MetaValue::Text(self.protocol_path.clone()),
                MetaValue::Bool(self.is_negative),
                MetaValue::Bool(self.is_maybe),
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
