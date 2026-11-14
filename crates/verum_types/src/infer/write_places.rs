//! Write permission for explicit places and implicit method borrows.
//! A place uses the types observed while evaluating its actual operands.
//! Re-inferring a call here would consume its affine arguments a second time.
use super::TypeChecker;
use crate::{Result, TypeError, ty::Type};
use std::collections::HashMap;
use verum_ast::{Expr, ExprKind, Span, UnOp};
use verum_common::Text;

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
    pub(super) fn check_resource_copy(&self, ty: &Type, span: Span) -> Result<()> {
        let ty = self.unifier.try_apply(ty, span)?;
        if self.type_contains_affine(&ty) {
            Err(TypeError::OtherWithCodeSpanned {
                code: "E310".into(),
                msg: format!("cannot move resource value `{ty}` out of borrowed storage").into(),
                span,
            })
        } else {
            Ok(())
        }
    }

    pub(super) fn check_owned_place_read(
        &mut self, place: &Expr, value_type: &Type, source_type: Option<&Type>,
    ) -> Result<()> {
        if source_type.is_none() && place_operand(unparen(place)).is_none() {
            return Ok(());
        }
        let value_type = self.unifier.try_apply(value_type, place.span)?;
        if !self.type_contains_affine(&value_type) {
            return Ok(());
        }
        if let Some(source) = source_type
            && self.place_reference_head(source, place.span)?.reference_parts().is_some()
        {
            return self.check_resource_copy(&value_type, place.span);
        }
        let mut step = unparen(place);
        while let Some((operand, _)) = place_operand(step) {
            if matches!(step.kind, ExprKind::Field { .. })
                && self.declaration_property_operand(operand).is_some()
            {
                break;
            }
            if self.place_reference_head(&self.place_operand_type(operand)?, operand.span)?
                .reference_parts().is_some()
            {
                return self.check_resource_copy(&value_type, place.span);
            }
            step = unparen(operand);
        }
        Ok(())
    }

    pub(super) fn with_expression_places<T>(
        &mut self, expr: &Expr, infer: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let args = match &unparen(expr).kind {
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => args.as_slice(),
            _ => &[],
        };
        self.with_call_place_types(expr, args, infer)
    }

    /// Only computed operands need temporary storage. Keys are identities of
    /// live, borrowed AST nodes and are discarded when this operation returns.
    pub(super) fn with_place_types<T>(
        &mut self,
        place: &Expr,
        infer: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.with_call_place_types(place, &[], infer)
    }

    pub(super) fn with_call_place_types<T>(
        &mut self,
        place: &Expr,
        args: &[Expr],
        infer: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let mut requested = HashMap::new();
        for arg in args {
            let arg = unparen(arg);
            if !matches!(
                arg.kind,
                ExprKind::Literal(_) | ExprKind::Path(_) | ExprKind::Unary { .. }
            ) {
                requested.insert(std::ptr::from_ref(arg).addr(), None);
            }
        }
        for place in std::iter::once(place).chain(args.iter()) {
            let mut step = unparen(place);
            while let Some((operand, _)) = place_operand(step) {
                step = unparen(operand);
                if !matches!(&step.kind, ExprKind::Path(p) if p.segments.len() == 1 && self.ctx.env.lookup(p.last_segment_name()).is_some())
                {
                    requested.insert(std::ptr::from_ref(step).addr(), None);
                }
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

    pub(super) fn observe_expression_type(&mut self, expr: &Expr, ty: &Type) {
        if !expr.span.is_empty() {
            self.type_registry.register_expr(expr.span, ty.clone());
        }
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
            checker.check_write_access(place, None, place.span)?;
            Ok(result)
        })
    }

    pub(super) fn check_write_access(
        &mut self,
        place: &Expr,
        method_receiver: Option<&Type>,
        write_site: Span,
    ) -> Result<()> {
        if let Some(ty) = method_receiver {
            self.check_write_through_reference(ty, true, place.span)?;
            if let Some(&hops) = self.deref_adjustments.get(&place.span) {
                let mut receiver = self.unwrap_reference_type(ty).clone();
                for _ in 0..hops {
                    let Some(target) = self.find_deref_target_type(&receiver, place.span)? else { break };
                    self.require_mutable_deref(&receiver, place.span)?;
                    receiver = self.unwrap_reference_type(&target).clone();
                }
            }
        }
        let tracks_indices = self
            .ctx
            .lookup_type("__has_dependent_record_layouts")
            .is_some();
        let mut step = unparen(place);
        let mut target = true;
        while let Some((operand, auto_deref)) = place_operand(step) {
            // A selected member of a declared type is a produced value, not
            // storage reached through that type name. Receiver borrows may
            // use the temporary, but assignment still requires a real place.
            // Use the same declaration authority as field resolution; lexical
            // values shadow types and retain every reference permission check.
            if method_receiver.is_some()
                && matches!(step.kind, ExprKind::Field { .. })
                && self.declaration_property_operand(operand).is_some()
            {
                break;
            }
            let ty = self.place_operand_type(operand)?;
            self.check_write_through_reference(&ty, auto_deref, operand.span)?;
            if auto_deref && self.projection_carrier(&ty, operand.span)?.refined {
                return Err(TypeError::OtherWithCodeSpanned {
                    code: "E405".into(),
                    msg: "projected write requires preserving the enclosing refinement; replace the whole refined value".into(),
                    span: write_site,
                });
            }
            if !auto_deref {
                self.check_explicit_mutable_deref(&ty, operand.span)?;
            }
            if tracks_indices
                && target
                && let ExprKind::Field { field, .. } = &step.kind
            {
                self.dependent_field_type(operand, &ty, field.name.as_str(), true, place.span)?;
            }
            step = unparen(operand);
            target = false;
        }
        if let Some((name, projection)) = self.borrow_place(place) {
            let conflict = self.borrow_tracker.check_place_borrow(
                &name,
                projection.as_deref(),
                true,
                write_site,
            );
            if tracks_indices
                && let Err(origin) = self
                    .ctx
                    .env
                    .check_record_place_write(&name, conflict.is_none())
            {
                return Err(TypeError::DependentReceiverMutation {
                    receiver: name,
                    projection: origin,
                    mutation: write_site,
                });
            }
            if let Some(conflict) = conflict {
                return Err(conflict);
            }
        }
        Ok(())
    }

    /// Resolve the storage a borrow or write may touch. Dynamic projections
    /// cover their enclosing place; computed receiver values have no lexical root.
    pub(super) fn borrow_place(&mut self, place: &Expr) -> Option<(Text, Option<String>)> {
        self.borrow_place_projection(place).map(|(root, path, _)| (root, path))
    }

    /// A coarsened place still protects its enclosing storage, but must not be
    /// refined with a payload path belonging to the original computed place.
    pub(super) fn borrow_place_projection(
        &mut self, place: &Expr,
    ) -> Option<(Text, Option<String>, bool)> {
        let mut step = unparen(place);
        let mut projection = Vec::new();
        let mut exact = true;
        while let Some((operand, _)) = place_operand(step) {
            match &step.kind {
                ExprKind::Field { field, .. } => projection.push(field.name.to_string()),
                ExprKind::Index { index, .. } => {
                    if let Some(index) = self.try_extract_const_index(index) {
                        projection.push(format!("[{index}]"));
                    } else {
                        // A dynamic subscript can select any element.
                        projection.clear();
                        exact = false;
                    }
                }
                // The borrow tracker has no finer location for a pointer
                // dereference or tuple projection.
                _ => { projection.clear(); exact = false; },
            }
            step = unparen(operand);
        }
        let ExprKind::Path(path) = &step.kind else {
            return None;
        };
        let root = match path.segments.as_slice() {
            [verum_ast::ty::PathSegment::Name(name)] => name.name.clone(),
            [verum_ast::ty::PathSegment::SelfValue] => "self".into(),
            _ => return None,
        };
        projection.reverse();
        Some((root, (!projection.is_empty()).then(|| projection.join(".")), exact))
    }

    /// A statically named field path can be tracked independently from its siblings.
    pub(super) fn named_field_place(place: &Expr) -> Option<(Text, String)> {
        let mut step = unparen(place);
        let mut fields = Vec::new();
        while let ExprKind::Field { expr, field } = &step.kind {
            fields.push(field.name.as_str());
            step = unparen(expr);
        }
        if fields.is_empty() {
            return None;
        }
        let ExprKind::Path(path) = &step.kind else { return None };
        let root = match path.segments.as_slice() {
            [verum_ast::ty::PathSegment::Name(name)] => name.name.clone(),
            [verum_ast::ty::PathSegment::SelfValue] => "self".into(),
            _ => return None,
        };
        fields.reverse();
        Some((root, fields.join(".")))
    }

    /// Expand transparent type names without erasing the permission on a reference.
    pub(super) fn place_reference_head(&self, ty: &Type, span: Span) -> Result<Type> {
        let mut current = self.unifier.try_apply(ty, span)?;
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

    fn check_explicit_mutable_deref(&mut self, receiver: &Type, span: Span) -> Result<()> {
        let receiver = self.place_reference_head(receiver, span)?;
        if matches!(receiver, Type::Named { .. } | Type::Generic { .. })
            && self.find_deref_target_type(&receiver, span)?.is_some()
        {
            self.require_mutable_deref(&receiver, span)?;
        }
        Ok(())
    }

    pub(super) fn require_mutable_deref(&mut self, receiver: &Type, span: Span) -> Result<()> {
        use verum_common::well_known_types::WellKnownProtocol;
        if self.protocol_checker.read().implements_mutable_deref(receiver) {
            return Ok(());
        }
        if let Some(name) = self.source_operator_name(WellKnownProtocol::DerefMut) {
            let path = Self::text_to_path(&name);
            if self.protocol_checker.read().find_impl_with_substitution(receiver, &path).is_some() {
                return Ok(());
            }
            self.load_source_operator_implementation(receiver, WellKnownProtocol::DerefMut, span)?;
            if self.protocol_checker.read().find_impl_with_substitution(receiver, &path).is_some() {
                return Ok(());
            }
        }
        Err(TypeError::OtherWithCodeSpanned {
            code: "E409".into(),
            msg: format!("writing through a dereference of {receiver} requires DerefMut").into(),
            span,
        })
    }

    /// Only the outer reference is crossed by an explicit dereference.
    /// An implicit field, index or method borrow crosses the full chain.
    fn check_write_through_reference(&self, ty: &Type, auto_deref: bool, span: Span) -> Result<()> {
        let mut current = self.place_reference_head(ty, span)?;
        for _ in 0..64 {
            if !auto_deref && matches!(
                current,
                Type::Pointer { mutable: false, .. } | Type::VolatilePointer { mutable: false, .. }
            ) {
                return Err(TypeError::SharedReferenceWrite { span });
            }
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
        self.observed_operand_type(expr)
            .ok_or_else(|| TypeError::UniverseBinding {
                message: "write permission requires the inferred type of this operand".into(),
                span: expr.span,
            })
    }

    pub(super) fn observed_operand_type(&self, expr: &Expr) -> Option<Type> {
        let key = std::ptr::from_ref(unparen(expr)).addr();
        self.place_operands
            .iter()
            .rev()
            .find_map(|scope| scope.get(&key)?.as_ref())
            .map(|ty| self.unifier.apply(ty))
    }
}
