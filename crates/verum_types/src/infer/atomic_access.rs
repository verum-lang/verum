//! Generic atomic loads and stores use the pointee type and execute synchronously.
use super::TypeChecker;
use crate::{
    Result, Type, TypeError,
    computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet},
};
use verum_ast::{Expr, Span};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AtomicAccess {
    Load,
    Store,
}

impl AtomicAccess {
    pub(super) fn from_intrinsic(name: &str) -> Option<Self> {
        match name {
            "atomic_load" => Some(Self::Load),
            "atomic_store" => Some(Self::Store),
            _ => None,
        }
    }

    pub(super) fn properties(self) -> PropertyRequirement {
        let operation = match self {
            Self::Load => ComputationalProperty::ReadsExternal,
            Self::Store => ComputationalProperty::Mutates,
        };
        PropertyRequirement::AtMost(PropertySet::single(operation))
    }
}

impl TypeChecker {
    pub(super) fn atomic_access_type(
        &mut self,
        operation: AtomicAccess,
        args: &[Expr],
        span: Span,
    ) -> Result<Type> {
        let (address, value, order) = match (operation, args) {
            (AtomicAccess::Load, [_, address, order]) => (address, None, order),
            (AtomicAccess::Store, [_, address, value, order]) => (address, Some(value), order),
            _ => {
                return Err(TypeError::OtherWithCodeSpanned {
                    code: "E0444".into(),
                    msg: "invalid atomic intrinsic operand count".into(),
                    span,
                });
            }
        };
        let address_ty = self.synth_expr(address)?.ty;
        let (mutable, target) = match self.unifier.apply(&address_ty) {
            Type::Reference { mutable, inner }
            | Type::CheckedReference { mutable, inner }
            | Type::UnsafeReference { mutable, inner }
            | Type::Pointer { mutable, inner }
            | Type::VolatilePointer { mutable, inner } => (mutable, *inner),
            actual => {
                return Err(TypeError::OtherWithCodeSpanned {
                    code: "E0444".into(),
                    msg: format!("atomic access needs an address, found {actual}").into(),
                    span: address.span,
                });
            }
        };
        if let Some(value) = value {
            if !mutable {
                return Err(TypeError::SharedReferenceWrite { span: address.span });
            }
            self.check_expr(value, &target)?;
        }
        self.synth_expr(order)?;
        Ok(if operation == AtomicAccess::Load {
            target
        } else {
            Type::Unit
        })
    }
}
