//! A view of a projection's carrier never rewrites the stored value's type.
use super::{MAX_INFERENCE_DEPTH, TypeChecker};
use crate::{Result, Type, TypeError};
use verum_ast::Span;

pub(super) struct ProjectionCarrier {
    pub ty: Type,
    pub borrowed: bool,
    pub refined: bool,
}

impl TypeChecker {
    pub(super) fn projection_carrier(&self, ty: &Type, span: Span) -> Result<ProjectionCarrier> {
        let mut carrier = ProjectionCarrier {
            ty: self.unifier.apply(ty),
            borrowed: false,
            refined: false,
        };
        for _ in 0..MAX_INFERENCE_DEPTH {
            match carrier.ty {
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } => {
                    carrier.borrowed = true;
                    carrier.ty = *inner;
                }
                Type::Refined { base, .. } => {
                    carrier.refined = true;
                    carrier.ty = *base;
                }
                Type::CapabilityRestricted { base, .. } => carrier.ty = *base,
                Type::Named { .. } | Type::Generic { .. } | Type::TypeApp { .. } => {
                    let normalized = self.normalize_type(&carrier.ty);
                    if normalized == carrier.ty {
                        return Ok(carrier);
                    }
                    carrier.ty = normalized;
                }
                _ => return Ok(carrier),
            }
        }
        Err(TypeError::OtherWithCodeSpanned {
            code: "E321".into(),
            msg: "projection carrier exceeds the type resolution limit".into(),
            span,
        })
    }
}
