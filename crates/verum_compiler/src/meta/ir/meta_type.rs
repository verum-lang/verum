//! Meta type system IR
//!
//! This module defines the type system for meta expressions,
//! used for parameter typing in meta functions.

use verum_common::{Heap, List};

/// Meta type kinds for parameter typing
#[derive(Debug, Clone, PartialEq)]
pub enum MetaType {
    /// Type parameter (takes a Type as input)
    Type,

    /// Expression parameter
    Expr,

    /// Statement parameter
    Stmt,

    /// Pattern parameter
    Pattern,

    /// Compile-time integer
    Integer,

    /// Compile-time text
    Text,

    /// Compile-time boolean
    Bool,

    /// List of meta values
    List(Heap<MetaType>),

    /// Tuple of meta values
    Tuple(List<MetaType>),

    /// Any meta value
    Any,
}

impl MetaType {
    /// Check if this type is a primitive meta type
    #[inline]
    pub fn is_primitive(&self) -> bool {
        matches!(
            self,
            MetaType::Integer | MetaType::Text | MetaType::Bool
        )
    }

    /// Check if this type accepts any value
    #[inline]
    pub fn is_any(&self) -> bool {
        matches!(self, MetaType::Any)
    }

    /// Check if this type represents AST nodes
    #[inline]
    pub fn is_ast_type(&self) -> bool {
        matches!(
            self,
            MetaType::Type | MetaType::Expr | MetaType::Stmt | MetaType::Pattern
        )
    }

    /// Check if this type is a collection type
    #[inline]
    pub fn is_collection(&self) -> bool {
        matches!(self, MetaType::List(_) | MetaType::Tuple(_))
    }

    /// Get human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            MetaType::Type => "Type",
            MetaType::Expr => "Expr",
            MetaType::Stmt => "Stmt",
            MetaType::Pattern => "Pattern",
            MetaType::Integer => "Int",
            MetaType::Text => "Text",
            MetaType::Bool => "Bool",
            MetaType::List(_) => "List",
            MetaType::Tuple(_) => "Tuple",
            MetaType::Any => "Any",
        }
    }
}
