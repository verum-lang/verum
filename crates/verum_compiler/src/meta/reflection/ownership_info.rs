//! Ownership information for compile-time reflection
//!
//! Provides ownership and borrowing metadata matching core/meta/reflection.vr OwnershipInfo.

use verum_ast::MetaValue;
use verum_common::{List, Text};

/// Information about ownership and borrowing for a type
///
/// Matches: core/meta/reflection.vr OwnershipInfo
#[derive(Debug, Clone, PartialEq)]
pub struct OwnershipInfo {
    /// Whether type implements Copy
    pub is_copy: bool,
    /// Whether type implements Clone
    pub is_clone: bool,
    /// Whether type is Send (safe to transfer between threads)
    pub is_send: bool,
    /// Whether type is Sync (safe to share between threads)
    pub is_sync: bool,
    /// Whether type has Drop implementation
    pub has_drop: bool,
    /// Whether type needs drop (has non-trivial destructor)
    pub needs_drop: bool,
    /// Whether type is Unpin (safe to move while borrowed)
    pub is_unpin: bool,
    /// Whether type has interior mutability
    pub has_interior_mutability: bool,
    /// Fields that prevent Copy/Send/Sync
    pub blocking_fields: List<Text>,
}

impl Default for OwnershipInfo {
    fn default() -> Self {
        Self {
            is_copy: false,
            is_clone: false,
            is_send: true,
            is_sync: true,
            has_drop: false,
            needs_drop: false,
            is_unpin: true,
            has_interior_mutability: false,
            blocking_fields: List::new(),
        }
    }
}

impl OwnershipInfo {
    /// Check if type can be trivially copied
    #[inline]
    pub fn is_trivially_copyable(&self) -> bool {
        self.is_copy && !self.needs_drop
    }

    /// Check if type is thread-safe
    #[inline]
    pub fn is_thread_safe(&self) -> bool {
        self.is_send && self.is_sync
    }

    /// Check if type requires explicit cleanup
    #[inline]
    pub fn requires_cleanup(&self) -> bool {
        self.has_drop || self.needs_drop
    }

    /// Get reason why type is not Copy
    pub fn non_copy_reason(&self) -> Option<Text> {
        if self.is_copy {
            None
        } else if !self.blocking_fields.is_empty() {
            let fields: Vec<&str> = self.blocking_fields.iter().map(|f| f.as_str()).collect();
            Some(Text::from(format!(
                "contains non-Copy field(s): {}",
                fields.join(", ")
            )))
        } else if self.has_drop {
            Some(Text::from("has custom Drop implementation"))
        } else {
            Some(Text::from("unknown reason"))
        }
    }

    /// Convert to MetaValue for meta-programming use
    pub fn to_meta_value(&self) -> MetaValue {
        MetaValue::Tuple(
            vec![
                MetaValue::Bool(self.is_copy),
                MetaValue::Bool(self.is_clone),
                MetaValue::Bool(self.is_send),
                MetaValue::Bool(self.is_sync),
                MetaValue::Bool(self.has_drop),
                MetaValue::Bool(self.needs_drop),
                MetaValue::Bool(self.is_unpin),
                MetaValue::Bool(self.has_interior_mutability),
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
