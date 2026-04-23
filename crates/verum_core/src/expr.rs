//! IR expressions.
//!
//! [`IrExpr`] is a typed expression form that lives between the raw
//! surface AST and the kernel's `CoreTerm`. It canonicalises the shapes
//! downstream passes (SMT translator, proof engine, kernel replay)
//! depend on: variable identity, constructor references, quantifier
//! bindings, and the arithmetic / propositional / equality operators
//! the verifier actually reasons about.
//!
//! The shape is intentionally close to the surface AST so lowering is
//! a direct walk — the IR's value is *stability* and *shared vocabulary*,
//! not a radical transformation.

use serde::{Deserialize, Serialize};
use verum_common::{Heap, List, Text};
use verum_common::span::Span;

use crate::ty::IrType;

/// Typed IR expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrExpr {
    /// Kind of expression.
    pub kind: IrExprKind,
    /// Type assigned by the lowering pass. May be `None` when the
    /// lowerer couldn't assign a concrete type and the consumer must
    /// use a default.
    pub ty: Option<IrType>,
    /// Source span.
    pub span: Span,
}

/// Kinds of IR expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IrExprKind {
    /// Integer literal.
    IntLit(i128),
    /// Boolean literal.
    BoolLit(bool),
    /// String literal.
    TextLit(Text),
    /// Reference to a bound variable (function parameter, let-bound
    /// local, or quantifier-bound).
    Var(Text),
    /// Reference to a variant constructor: `Color.Red`.
    Ctor {
        /// The type name (`Color`).
        type_name: Text,
        /// The constructor name (`Red`).
        ctor_name: Text,
    },
    /// Function call: `f(a, b)`.
    Call {
        /// The callee identifier.
        callee: Text,
        /// The argument list.
        args: List<IrExpr>,
    },
    /// Binary operation.
    Binary {
        /// The operator.
        op: IrBinOp,
        /// Left operand.
        left: Heap<IrExpr>,
        /// Right operand.
        right: Heap<IrExpr>,
    },
    /// Unary operation.
    Unary {
        /// The operator.
        op: IrUnOp,
        /// Operand.
        expr: Heap<IrExpr>,
    },
    /// Conditional expression: `if c { t } else { e }`.
    If {
        /// Condition.
        cond: Heap<IrExpr>,
        /// Then branch.
        then_branch: Heap<IrExpr>,
        /// Else branch.
        else_branch: Heap<IrExpr>,
    },
    /// Universal quantifier.
    Forall {
        /// Bound-variable name.
        var: Text,
        /// Type annotation.
        ty: IrType,
        /// Body.
        body: Heap<IrExpr>,
    },
    /// Existential quantifier.
    Exists {
        /// Bound-variable name.
        var: Text,
        /// Type annotation.
        ty: IrType,
        /// Body.
        body: Heap<IrExpr>,
    },
    /// Field access: `p.x`.
    Field {
        /// Record-valued expression.
        expr: Heap<IrExpr>,
        /// Field identifier.
        field: Text,
    },
    /// Tuple element access: `p.0`.
    TupleIndex {
        /// Tuple-valued expression.
        expr: Heap<IrExpr>,
        /// Element index.
        index: u32,
    },
    /// Opaque reference to an un-modelled AST shape. Carries the
    /// pretty-printed surface so downstream passes can at least
    /// canonicalise identical occurrences to the same Z3 symbol.
    Opaque {
        /// The pretty-printed surface form of the original AST.
        pretty: Text,
    },
}

/// IR binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IrBinOp {
    /// Integer / real addition.
    Add,
    /// Integer / real subtraction.
    Sub,
    /// Integer / real multiplication.
    Mul,
    /// Integer division.
    Div,
    /// Integer modulo.
    Rem,
    /// Equality.
    Eq,
    /// Inequality.
    Ne,
    /// Less-than.
    Lt,
    /// Less-than-or-equal.
    Le,
    /// Greater-than.
    Gt,
    /// Greater-than-or-equal.
    Ge,
    /// Logical AND.
    And,
    /// Logical OR.
    Or,
    /// Logical implication.
    Imply,
    /// Logical bi-implication.
    Iff,
    /// List / sequence concatenation.
    Concat,
}

/// IR unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IrUnOp {
    /// Arithmetic negation.
    Neg,
    /// Logical negation.
    Not,
}

impl IrExpr {
    /// Construct a new typed IR expression.
    #[must_use]
    pub fn new(kind: IrExprKind, ty: Option<IrType>, span: Span) -> Self {
        Self { kind, ty, span }
    }

    /// Classify the expression as an integer literal if it is one.
    #[must_use]
    pub fn as_int_lit(&self) -> Option<i128> {
        match &self.kind {
            IrExprKind::IntLit(n) => Some(*n),
            _ => None,
        }
    }

    /// Classify the expression as a boolean literal if it is one.
    #[must_use]
    pub fn as_bool_lit(&self) -> Option<bool> {
        match &self.kind {
            IrExprKind::BoolLit(b) => Some(*b),
            _ => None,
        }
    }

    /// Classify the expression as a variable reference if it is one.
    #[must_use]
    pub fn as_var(&self) -> Option<&Text> {
        match &self.kind {
            IrExprKind::Var(n) => Some(n),
            _ => None,
        }
    }

    /// Classify the expression as a variant constructor reference if
    /// it is one.
    #[must_use]
    pub fn as_ctor(&self) -> Option<(&Text, &Text)> {
        match &self.kind {
            IrExprKind::Ctor { type_name, ctor_name } => Some((type_name, ctor_name)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp() -> Span {
        Span::dummy()
    }

    #[test]
    fn int_lit_roundtrips() {
        let e = IrExpr::new(IrExprKind::IntLit(42), None, sp());
        assert_eq!(e.as_int_lit(), Some(42));
        assert_eq!(e.as_bool_lit(), None);
    }

    #[test]
    fn ctor_accessor_returns_both_parts() {
        let e = IrExpr::new(
            IrExprKind::Ctor {
                type_name: Text::from("Color"),
                ctor_name: Text::from("Red"),
            },
            None,
            sp(),
        );
        let (t, c) = e.as_ctor().unwrap();
        assert_eq!(t.as_str(), "Color");
        assert_eq!(c.as_str(), "Red");
    }

    #[test]
    fn var_accessor_returns_name() {
        let e = IrExpr::new(
            IrExprKind::Var(Text::from("x")),
            None,
            sp(),
        );
        assert_eq!(e.as_var().unwrap().as_str(), "x");
    }
}
