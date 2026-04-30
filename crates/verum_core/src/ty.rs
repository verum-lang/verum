//! IR types.
//!
//! [`IrType`] is the typed-IR's type form. Covers the primitives the
//! SMT translator recognises directly (Int, Bool, Real, Text), the
//! common collection shapes (List, Tuple), refinement types (a base
//! type constrained by a predicate expression), and named references
//! to user-declared types (variants, records, aliases).
//!
//! Types with richer shape — protocols, HOT-dependent forms, cubical
//! paths — are preserved via [`IrType::Named`] with the qualified
//! name and generic arguments. Downstream consumers that need the
//! richer form look the name up in the module's type registry.

use serde::{Deserialize, Serialize};
use verum_common::{Heap, List, Text};
use verum_common::span::Span;

use crate::expr::IrExpr;

/// IR type form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IrType {
    /// Unit type.
    Unit,
    /// Boolean.
    Bool,
    /// Signed integer.
    Int,
    /// IEEE-754 real.
    Real,
    /// UTF-8 text.
    Text,
    /// Named reference to a user-declared type (variant, record, alias).
    Named {
        /// Qualified type name.
        name: Text,
        /// Generic type arguments.
        args: List<IrType>,
        /// Source span of the reference.
        span: Span,
    },
    /// List / sequence parameterised by element type.
    List(Heap<IrType>),
    /// Tuple with positional element types.
    Tuple(List<IrType>),
    /// Refinement type: `{ it: base | predicate(it) }`.
    Refined {
        /// Base type being refined.
        base: Heap<IrType>,
        /// Convention-driven binder name (`self` / `it`).
        binder: Text,
        /// Refinement predicate expression.
        predicate: Heap<IrExpr>,
    },
    /// Function type: `fn(params) -> return`.
    Function {
        /// Parameter types.
        params: List<IrType>,
        /// Return type.
        return_type: Heap<IrType>,
    },
    /// Unknown / inferred type. Downstream passes should either infer
    /// a concrete type or treat the expression opaquely.
    Inferred,
}

impl IrType {
    /// Construct a named-type reference.
    #[must_use]
    pub fn named(name: impl Into<Text>, args: List<IrType>, span: Span) -> Self {
        Self::Named {
            name: name.into(),
            args,
            span,
        }
    }

    /// Check whether the type is a primitive boolean.
    #[must_use]
    pub fn is_bool(&self) -> bool {
        matches!(self, IrType::Bool)
    }

    /// Check whether the type is a primitive integer.
    #[must_use]
    pub fn is_int(&self) -> bool {
        matches!(self, IrType::Int)
    }

    /// Return the base type of a refinement, or `self` for non-refined
    /// types. Useful for sort-level reasoning where the refinement
    /// predicate is irrelevant.
    #[must_use]
    pub fn underlying_sort(&self) -> &IrType {
        match self {
            IrType::Refined { base, .. } => base.underlying_sort(),
            other => other,
        }
    }

    /// Map the type to an SMT-LIB sort name, using `"Int"` as the
    /// conservative default. This matches the convention
    /// `verum_smt::expr_to_smtlib::type_to_sort` follows.
    #[must_use]
    pub fn to_smt_sort(&self) -> &'static str {
        match self.underlying_sort() {
            IrType::Bool => "Bool",
            IrType::Real => "Real",
            _ => "Int",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use verum_common::span::Span;

    #[test]
    fn primitive_classifications() {
        assert!(IrType::Bool.is_bool());
        assert!(!IrType::Int.is_bool());
        assert!(IrType::Int.is_int());
    }

    #[test]
    fn refinement_underlying_sort_strips_predicate() {
        use verum_common::Heap;
        use crate::expr::{IrExpr, IrExprKind};

        let refined = IrType::Refined {
            base: Heap::new(IrType::Int),
            binder: Text::from("self"),
            predicate: Heap::new(IrExpr::new(
                IrExprKind::BoolLit(true),
                None,
                Span::dummy(),
            )),
        };
        match refined.underlying_sort() {
            IrType::Int => {}
            other => panic!("expected Int base, got {other:?}"),
        }
    }

    #[test]
    fn smt_sort_mapping() {
        assert_eq!(IrType::Bool.to_smt_sort(), "Bool");
        assert_eq!(IrType::Real.to_smt_sort(), "Real");
        assert_eq!(IrType::Int.to_smt_sort(), "Int");
        assert_eq!(
            IrType::Named {
                name: Text::from("Color"),
                args: List::new(),
                span: Span::dummy(),
            }
            .to_smt_sort(),
            "Int"
        );
    }
}
