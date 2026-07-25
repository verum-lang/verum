//! Code search types for meta functions
//!
//! Provides type information and usage tracking for code search functionality.

use verum_ast::Span;
use verum_common::{List, Text};

/// Type information for code search
#[derive(Debug, Clone)]
pub enum CodeSearchTypeInfo {
    /// Function definition
    Function {
        /// Function return type as string
        return_type: Text,
        /// Function attributes
        attributes: List<Text>,
        /// Source span
        span: Span,
    },
    /// Type definition
    Type {
        /// Protocols this type implements
        protocols: List<Text>,
        /// Type attributes
        attributes: List<Text>,
        /// Source span
        span: Span,
    },
}

/// Usage information for code search
#[derive(Debug, Clone)]
pub struct UsageInfo {
    /// Location of the usage
    pub span: Span,
    /// Context description (e.g., "call", "type annotation")
    pub context: Text,
}

impl UsageInfo {
    /// Create a new usage info
    pub fn new(span: Span, context: Text) -> Self {
        Self { span, context }
    }
}

/// Module information for code search
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    /// Public items exported by this module
    pub public_items: List<ItemInfo>,
    /// Dependencies (other modules this module imports)
    pub dependencies: List<Text>,
}

impl ModuleInfo {
    /// Create a new module info
    pub fn new() -> Self {
        Self {
            public_items: List::new(),
            dependencies: List::new(),
        }
    }
}

impl Default for ModuleInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Item information for module exports
#[derive(Debug, Clone)]
pub struct ItemInfo {
    /// Item name
    pub name: Text,
    /// Item kind
    pub kind: ItemKind,
}

impl ItemInfo {
    /// Create a new item info
    pub fn new(name: Text, kind: ItemKind) -> Self {
        Self { name, kind }
    }
}

/// Item kind for module exports
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Function,
    Type,
    Const,
    Module,
}

impl ItemKind {
    /// Get string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            ItemKind::Function => "function",
            ItemKind::Type => "type",
            ItemKind::Const => "const",
            ItemKind::Module => "module",
        }
    }
}
