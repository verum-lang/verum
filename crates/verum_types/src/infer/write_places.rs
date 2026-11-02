//! Write permission for explicit places and implicit method borrows.
//! A place uses the types observed while evaluating its actual operands.
//! Re-inferring a call here would consume its affine arguments a second time.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use std::collections::HashMap;
use verum_ast::{Expr, ExprKind, Span, UnOp};

fn unparen(mut expr: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expr.kind {
        expr = inner;
    }
    expr
}

fn place_operand(expr: &Expr) -> Option<(&Expr, bool)> {
    match &expr.kind {
        ExprKind::Field { expr, .. }
        | ExprKind::Index { expr, .. }
        | ExprKind::TupleIndex { expr, .. } => Some((expr, true)),
        ExprKind::Unary {
            op: UnOp::Deref,
            expr,
        } => Some((expr, false)),
        _ => None,
    }
}

impl TypeChecker {
    /// Only computed operands need temporary storage. Keys are identities of
    /// live, borrowed AST nodes and are discarded when this operation returns.
    pub(super) fn with_place_types<T>(
        &mut self,
        place: &Expr,
        infer: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut requested = HashMap::new();
        let mut step = unparen(place);
        while let Some((operand, _)) = place_operand(step) {
            step = unparen(operand);
            if !matches!(&step.kind, ExprKind::Path(p) if p.segments.len() == 1 && self.ctx.env.lookup(p.last_segment_name()).is_some())
            {
                requested.insert(std::ptr::from_ref(step).addr(), None);
            }
        }
        if requested.is_empty() {
            return infer(self);
        }
        self.place_operands.push(requested);
        let result = infer(self);
        self.place_operands.pop();
        result
    }

    pub(super) fn observe_place_operand(&mut self, expr: &Expr, ty: &Type) {
        if self.place_operands.is_empty() {
            return;
        }
        let key = std::ptr::from_ref(unparen(expr)).addr();
        for scope in &mut self.place_operands {
            if let Some(slot) = scope.get_mut(&key) {
                *slot = Some(ty.clone());
            }
        }
    }

    pub(super) fn with_checked_write<T>(
        &mut self,
        place: &Expr,
        infer: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_place_types(place, |checker| {
            let result = infer(checker)?;
            checker.check_dependent_write_access(place, None)?;
            Ok(result)
        })
    }

    pub(super) fn check_dependent_write_access(
        &mut self,
        place: &Expr,
        method_receiver: Option<&Type>,
    ) -> Result<()> {
        if let Some(ty) = method_receiver {
            self.check_write_through_reference(ty, true, place.span)?;
        }
        let tracks_indices = self
            .ctx
            .lookup_type("__has_dependent_record_layouts")
            .is_some();
        let mut step = unparen(place);
        let mut target = true;
        while let Some((operand, auto_deref)) = place_operand(step) {
            let ty = self.place_operand_type(operand)?;
            self.check_write_through_reference(&ty, auto_deref, operand.span)?;
            if tracks_indices
                && target
                && let ExprKind::Field { field, .. } = &step.kind
            {
                self.dependent_field_type(operand, &ty, field.name.as_str(), true, place.span)?;
            }
            step = unparen(operand);
            target = false;
        }
        if tracks_indices
            && let ExprKind::Path(path) = &step.kind
            && let [verum_ast::ty::PathSegment::Name(name)] = path.segments.as_slice()
            && let Err(projection) = self.ctx.env.note_record_place_write(&name.name)
        {
            return Err(TypeError::DependentReceiverMutation {
                receiver: name.name.clone(),
                projection,
                mutation: place.span,
            });
        }
        Ok(())
    }

    /// Expand transparent type names without erasing the permission on a reference.
    pub(super) fn place_reference_head(&self, ty: &Type, span: Span) -> Result<Type> {
        let mut current = self.unifier.apply(ty);
        for _ in 0..64 {
            match &current {
                Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => {
                    current = (**base).clone();
                }
                _ => {
                    let Some((name, args)) = self.conversion_nominal_head(&current) else {
                        return Ok(current);
                    };
                    let Some(target) = self.ctx.type_aliases.get(&name) else {
                        return Ok(current);
                    };
                    let next = self.substitute_with_params(&name, target, args);
                    if next == current {
                        return Ok(current);
                    }
                    current = next;
                }
            }
        }
        Err(TypeError::UniverseBinding {
            message: "reference permission exceeds the alias resolution limit".into(),
            span,
        })
    }

    /// Only the outer reference is crossed by an explicit dereference.
    /// An implicit field, index or method borrow crosses the full chain.
    fn check_write_through_reference(&self, ty: &Type, auto_deref: bool, span: Span) -> Result<()> {
        let mut current = self.place_reference_head(ty, span)?;
        for _ in 0..64 {
            let (Type::Reference { mutable, inner }
            | Type::CheckedReference { mutable, inner }
            | Type::UnsafeReference { mutable, inner }) = current
            else {
                return Ok(());
            };
            if !mutable {
                return Err(TypeError::SharedReferenceWrite { span });
            }
            if !auto_deref {
                return Ok(());
            }
            current = self.place_reference_head(&inner, span)?;
        }
        Err(TypeError::UniverseBinding {
            message: "reference chain exceeds the resolution limit".into(),
            span,
        })
    }

    fn place_operand_type(&self, expr: &Expr) -> Result<Type> {
        let expr = unparen(expr);
        if let ExprKind::Path(path) = &expr.kind
            && path.segments.len() == 1
            && let Some(scheme) = self.ctx.env.lookup(path.last_segment_name())
        {
            return Ok(self.unifier.apply(&scheme.ty));
        }
        let key = std::ptr::from_ref(expr).addr();
        self.place_operands
            .iter()
            .rev()
            .find_map(|scope| scope.get(&key)?.as_ref())
            .map(|ty| self.unifier.apply(ty))
            .ok_or_else(|| TypeError::UniverseBinding {
                message: "write permission requires the inferred type of this operand".into(),
                span: expr.span,
            })
    }
}
