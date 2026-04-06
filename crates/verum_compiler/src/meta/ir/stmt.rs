//! Meta statement IR
//!
//! This module defines the intermediate representation for meta statements.

use verum_common::{Maybe, Text};

use super::expr::MetaExpr;

/// Meta statement kinds
#[derive(Debug, Clone, PartialEq)]
pub enum MetaStmt {
    /// Expression statement
    Expr(MetaExpr),

    /// Let binding
    Let { name: Text, value: MetaExpr },

    /// Tuple destructuring let binding
    /// Names are Option<Text> where None represents wildcard `_`
    LetTuple {
        names: Vec<Option<Text>>,
        value: MetaExpr,
    },

    /// Return statement
    Return(Maybe<MetaExpr>),
}

impl MetaStmt {
    /// Create an expression statement
    #[inline]
    pub fn expr(e: MetaExpr) -> Self {
        MetaStmt::Expr(e)
    }

    /// Create a let binding
    #[inline]
    pub fn let_binding(name: Text, value: MetaExpr) -> Self {
        MetaStmt::Let { name, value }
    }

    /// Create a return statement
    #[inline]
    pub fn return_stmt(value: Option<MetaExpr>) -> Self {
        MetaStmt::Return(value.map_or(Maybe::None, Maybe::Some))
    }
}
