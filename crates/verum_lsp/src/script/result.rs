//! Parse result types for script parsing
//!
//! Moved from verum_parser::script

use verum_ast::{Expr, Item, Module, Stmt};
use verum_common::Text;

/// Result of parsing a script line
#[derive(Debug, Clone)]
pub enum ScriptParseResult {
    /// Successfully parsed an expression
    Expression(Expr),
    /// Successfully parsed a statement
    Statement(Stmt),
    /// Successfully parsed an item (function, type, etc.)
    Item(Item),
    /// Successfully parsed multiple items
    Module(Module),
    /// Input is incomplete, more lines needed
    Incomplete(Text),
    /// Empty input (whitespace only)
    Empty,
}

/// Parsing mode for script contexts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    /// Try expression first, then statement, then item
    Auto,
    /// Parse as expression only
    Expression,
    /// Parse as statement only
    Statement,
    /// Parse as item only
    Item,
    /// Parse as full module
    Module,
}
