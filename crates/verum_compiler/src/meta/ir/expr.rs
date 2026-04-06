//! Meta expression IR
//!
//! This module defines the intermediate representation for meta expressions,
//! which are evaluated at compile time.

use verum_ast::{expr::Expr, MetaValue};
use verum_common::{Heap, List, Maybe, Text};

use super::pattern::MetaPattern;
use super::stmt::MetaStmt;

/// Meta expression kinds for compile-time execution
#[derive(Debug, Clone, PartialEq)]
pub enum MetaExpr {
    /// Literal value
    Literal(MetaValue),

    /// Variable reference
    Variable(Text),

    /// Function call
    Call(Text, List<MetaExpr>),

    /// If expression
    If {
        condition: Heap<MetaExpr>,
        then_branch: Heap<MetaExpr>,
        else_branch: Maybe<Heap<MetaExpr>>,
    },

    /// Match expression
    Match {
        scrutinee: Heap<MetaExpr>,
        arms: List<MetaArm>,
    },

    /// Let binding
    Let {
        name: Text,
        value: Heap<MetaExpr>,
        body: Heap<MetaExpr>,
    },

    /// Block expression
    Block(List<MetaStmt>),

    /// Quote expression (returns AST)
    Quote(Expr),

    /// Unquote expression (splices AST)
    Unquote(Heap<MetaExpr>),

    /// TypeOf expression (get type at compile time)
    TypeOf(Expr),

    /// Binary operation
    Binary {
        op: verum_ast::expr::BinOp,
        left: Heap<MetaExpr>,
        right: Heap<MetaExpr>,
    },

    /// Unary operation
    Unary {
        op: verum_ast::expr::UnOp,
        expr: Heap<MetaExpr>,
    },

    /// List comprehension
    ListComp {
        expr: Heap<MetaExpr>,
        var: Text,
        iter: Heap<MetaExpr>,
        filter: Maybe<Heap<MetaExpr>>,
    },

    /// Method call: receiver.method(args)
    MethodCall {
        receiver: Heap<MetaExpr>,
        method: Text,
        args: List<MetaExpr>,
    },

    /// Field access: expr.field
    FieldAccess {
        expr: Heap<MetaExpr>,
        field: Text,
    },

    /// Index: expr[index]
    Index {
        expr: Heap<MetaExpr>,
        index: Heap<MetaExpr>,
    },

    /// Closure: |params| body
    Closure {
        params: List<Text>,
        body: Heap<MetaExpr>,
        return_type: Maybe<verum_ast::ty::Type>,
    },

    /// For loop: for pattern in iter { body }
    For {
        pattern: MetaPattern,
        iter: Heap<MetaExpr>,
        body: List<MetaStmt>,
    },

    /// While loop: while condition { body }
    While {
        condition: Heap<MetaExpr>,
        body: List<MetaStmt>,
    },

    /// Tuple index: expr.0
    TupleIndex {
        expr: Heap<MetaExpr>,
        index: u32,
    },

    /// Record literal: Name { field: value, ... }
    Record {
        name: Text,
        fields: List<(Text, MetaExpr)>,
        base: Maybe<Heap<MetaExpr>>,
    },

    /// Return expression: return value
    Return(Maybe<Heap<MetaExpr>>),

    /// Break expression: break 'label value
    Break {
        label: Maybe<Text>,
        value: Maybe<Heap<MetaExpr>>,
    },

    /// Continue expression: continue 'label
    Continue {
        label: Maybe<Text>,
    },

    /// Cast expression: expr as T
    Cast {
        expr: Heap<MetaExpr>,
        ty: verum_ast::ty::Type,
    },

    /// Variable assignment: name = value
    Assign {
        target: Text,
        value: Heap<MetaExpr>,
    },

    /// Array element assignment: name[index] = value
    AssignIndex {
        target: Text,
        index: Heap<MetaExpr>,
        value: Heap<MetaExpr>,
    },
}

/// Match arm for meta expressions
#[derive(Debug, Clone, PartialEq)]
pub struct MetaArm {
    pub pattern: MetaPattern,
    pub guard: Maybe<MetaExpr>,
    pub body: MetaExpr,
}

impl MetaArm {
    /// Create a new match arm
    pub fn new(pattern: MetaPattern, body: MetaExpr) -> Self {
        Self {
            pattern,
            guard: Maybe::None,
            body,
        }
    }

    /// Create a match arm with a guard
    pub fn with_guard(pattern: MetaPattern, guard: MetaExpr, body: MetaExpr) -> Self {
        Self {
            pattern,
            guard: Maybe::Some(guard),
            body,
        }
    }
}
