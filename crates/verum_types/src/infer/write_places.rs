//! Write permission for explicit places and implicit method borrows.
//! A place uses the types observed while evaluating its actual operands.
//! Re-inferring a call here would consume its affine arguments a second time.
use super::TypeChecker;
use crate::{Result, TypeError, ty::{RecordReceiver, Type}};
use verum_ast::{Expr, ExprKind, Span, UnOp};
use verum_common::{List, Map, Text};

/// Evidence about one evaluated operand, not a later read of its source place.
#[derive(Clone)]
pub(super) struct EvaluatedOperand {
    ty: Type,
    origins: List<RecordReceiver>,
    known_owned_payload: bool,
    raw_address: Option<Box<[RecordReceiver]>>,
    indexed_owners: Option<List<RecordReceiver>>,
}

/// Selecting a deeper inline field uses its parent's type without copying the
/// parent or observing loans in unrelated sibling fields.
pub(super) enum PlaceOperand {
    TypeOnly(Option<Type>),
    Value(Option<EvaluatedOperand>),
}

fn unparen(mut expr: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expr.kind {
        expr = inner;
    }
    expr
}

/// Local place roots include the method receiver, whose path has its own AST segment.
pub(super) fn lexical_place_root(expr: &Expr) -> Option<&str> {
    let ExprKind::Path(path) = &unparen(expr).kind else { return None };
    match path.segments.as_slice() {
        [verum_ast::ty::PathSegment::Name(name)] => Some(name.as_str()),
        [verum_ast::ty::PathSegment::SelfValue] => Some("self"),
        _ => None,
    }
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

    pub(super) fn with_expression_places(
        &mut self, expr: &Expr, infer: impl FnOnce(&mut Self) -> Result<super::InferResult>,
    ) -> Result<super::InferResult> {
        let args = match &unparen(expr).kind {
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => args.as_slice(),
            _ => &[],
        };
        let evaluate = |checker: &mut Self| checker.with_call_place_types(expr, args, |checker| {
            let result = infer(checker)?;
            checker.observe_aggregate_result_origins(expr, &result.ty);
            Ok(result)
        });
        // These operations can skip, repeat or defer evaluation of a child.
        // One lexical merge boundary keeps their writes conservative even
        // when a child contains otherwise unconditional expression blocks.
        if matches!(unparen(expr).kind,
            ExprKind::Binary { op: verum_ast::BinOp::And | verum_ast::BinOp::Or | verum_ast::BinOp::Imply, .. }
            | ExprKind::NullCoalesce { .. } | ExprKind::OptionalChain { .. }
            | ExprKind::If { .. } | ExprKind::Match { .. }
            | ExprKind::Loop { .. } | ExprKind::While { .. } | ExprKind::For { .. } | ExprKind::ForAwait { .. }
            | ExprKind::Closure { .. } | ExprKind::Async(_) | ExprKind::Spawn { .. }
            | ExprKind::Select { .. } | ExprKind::Nursery { .. }
            | ExprKind::TryBlock(_) | ExprKind::TryRecover { .. } | ExprKind::TryFinally { .. } | ExprKind::TryRecoverFinally { .. }
            | ExprKind::Comprehension { .. } | ExprKind::StreamComprehension { .. }
            | ExprKind::MapComprehension { .. } | ExprKind::SetComprehension { .. } | ExprKind::GeneratorComprehension { .. }
            | ExprKind::Typeof(_) | ExprKind::Quote { .. })
        {
            self.with_origin_merge_scope(|checker| {
                let result = evaluate(checker)?;
                checker.check_indexed_temporary_result(expr, &result.ty)?;
                Ok(result)
            })
        } else {
            evaluate(self)
        }
    }

    pub(super) fn with_origin_merge_scope<T>(
        &mut self, evaluate: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let depth = self.ctx.env.scope_depth();
        self.ctx.env.push_scope();
        let result = evaluate(self);
        self.ctx.env.restore_scope_depth(depth);
        result
    }

    /// Operand observations need temporary storage. Keys are identities of
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
        let mut requested = Map::new();
        for arg in args {
            let arg = unparen(arg);
            requested.insert(std::ptr::from_ref(arg).addr(), PlaceOperand::Value(None));
        }
        match &unparen(place).kind {
            ExprKind::Call { func, .. } => {
                requested.insert(std::ptr::from_ref(unparen(func)).addr(), PlaceOperand::Value(None));
            }
            ExprKind::MethodCall { receiver, .. } => {
                requested.insert(std::ptr::from_ref(unparen(receiver)).addr(), PlaceOperand::Value(None));
            }
            ExprKind::Tuple(values) | ExprKind::Array(verum_ast::ArrayExpr::List(values)) => {
                for value in values {
                    requested.insert(std::ptr::from_ref(unparen(value)).addr(), PlaceOperand::Value(None));
                }
            }
            ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, .. }) => {
                requested.insert(std::ptr::from_ref(unparen(value)).addr(), PlaceOperand::Value(None));
            }
            ExprKind::Record { fields, base, .. } => {
                for value in fields.iter().filter_map(|field| field.value.as_ref()).chain(base.as_deref()) {
                    requested.insert(std::ptr::from_ref(unparen(value)).addr(), PlaceOperand::Value(None));
                }
            }
            _ if !args.is_empty() => {
                requested.insert(std::ptr::from_ref(unparen(place)).addr(), PlaceOperand::Value(None));
            }
            _ => {}
        }
        for place in std::iter::once(place).chain(args.iter()) {
            let mut step = unparen(place);
            while let Some((operand, _)) = place_operand(step) {
                let selecting_field = matches!(&step.kind, ExprKind::Field { .. });
                step = unparen(operand);
                if !matches!(&step.kind, ExprKind::Path(p) if p.segments.len() == 1 && self.ctx.env.lookup(p.last_segment_name()).is_some())
                {
                    let entry = requested.entry(std::ptr::from_ref(step).addr());
                    entry.or_insert_with(|| {
                        if selecting_field && matches!(&step.kind, ExprKind::Field { .. })
                            && self.call_input_place(step).is_some()
                        {
                            PlaceOperand::TypeOnly(None)
                        } else {
                            PlaceOperand::Value(None)
                        }
                    });
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
        let mut observe_value = false;
        for scope in &mut self.place_operands {
            match scope.get_mut(&key) {
                Some(PlaceOperand::TypeOnly(slot)) => *slot = Some(ty.clone()),
                Some(PlaceOperand::Value(_)) => observe_value = true,
                None => (),
            }
        }
        if !observe_value { return; }
        // Observe before another argument or the invocation can mutate a
        // source place. Empty candidates are not a no-write/cleanup proof.
        let previous_type = self.observed_operand(expr).map(|operand| self.unifier.apply(&operand.ty));
        let previous_origins = self.observed_operand_origins(expr);
        let mut origins = self.aggregate_origins(expr, ty);
        if self.reference_mode(ty, expr.span).is_ok_and(|mode| mode.is_some()) {
            if previous_type.as_ref().is_some_and(|previous|
                self.reference_mode(previous, expr.span).is_ok_and(|mode| mode.is_none()))
            {
                // Contextual checking has formed an implicit borrow of the
                // already evaluated place. Its loan is not its old payload.
                self.collect_borrowed_place_candidates(expr, &mut origins);
            } else if let Some(previous) = previous_origins {
                for origin in previous {
                    if !origins.contains(&origin) { origins.push(origin); }
                }
            }
        }
        let known_owned_payload = self.carries_stored_loan(ty) && self.complete_slot_value_origins(expr);
        let raw_address = match ty {
            Type::Pointer { mutable: true, inner } | Type::VolatilePointer { mutable: true, inner } =>
                self.raw_store_destinations(expr, inner).map(List::into_boxed_slice),
            _ => None,
        };
        let indexed_owners = self.observed_indexed_owners(expr).or_else(|| self.indexed_container_origins(expr));
        let observed = EvaluatedOperand { ty: ty.clone(), origins, known_owned_payload, raw_address, indexed_owners };
        for scope in &mut self.place_operands {
            if let Some(PlaceOperand::Value(slot)) = scope.get_mut(&key) {
                *slot = Some(observed.clone());
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
        self.check_place_write(place, method_receiver, write_site, None)
    }

    /// A replace and a mutable borrow share permission and alias checks. Only
    /// replacement retires an initialized value reached through the reference.
    pub(super) fn check_place_replacement(&mut self, place: &Expr, stored: &Type) -> Result<()> {
        self.check_place_write(place, None, place.span, Some(stored))
    }

    fn check_place_write(
        &mut self,
        place: &Expr,
        method_receiver: Option<&Type>,
        write_site: Span,
        replacement: Option<&Type>,
    ) -> Result<()> {
        if let Some(ty) = method_receiver {
            self.check_write_through_reference(ty, true, place.span)?;
            if let Some(adjustment) = self.deref_adjustments.get(&place.span).copied() {
                let mut receiver = self.unwrap_reference_type(ty).clone();
                for _ in 0..adjustment.hops {
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
        let mut projects_through_reference = false;
        let mut replaces_referent = false;
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
            // Only the last step reaches the lexical root. Writing through a
            // reference leaves the binding that holds that reference intact.
            if tracks_indices && matches!(unparen(operand).kind, ExprKind::Path(_)) {
                projects_through_reference = self.reference_mode(&ty, operand.span)?.is_some();
            }
            let crossed_reference = self.check_write_through_reference(&ty, auto_deref, operand.span)?;
            replaces_referent |= crossed_reference || !auto_deref;
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
        let borrowed = self.borrow_place(place);
        let written = self.flow_write_origins(place, borrowed.as_ref().map(|(name, _)| name), replaces_referent);
        if let Some((name, projection)) = borrowed {
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
                    .check_record_place_write(
                        &name,
                        conflict.is_none() && !projects_through_reference,
                    )
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
        if replaces_referent && let Some(ty) = replacement {
            self.check_referent_overwrite(ty, write_site)?;
        }
        self.refinement_evidence.invalidate_write(&written);
        Ok(())
    }

    pub(super) fn flow_write_origins(
        &mut self, place: &Expr, root: Option<&Text>, through_reference: bool,
    ) -> List<RecordReceiver> {
        let mut sources = List::new();
        if self.refinement_evidence.is_empty() { return sources; }
        if through_reference && root.is_none_or(|name| self.ctx.env.reference_candidates(name).is_empty()) {
            // An untracked pointer can alias a different lexical binding.
            return sources;
        }
        self.collect_borrowed_place_candidates(place, &mut sources);
        if let Some(binding) = root.and_then(|name| self.ctx.env.flow_binding(name)) {
            sources.push(RecordReceiver { binding, fields: List::new() });
        }
        sources
    }

    /// Resolve the storage a borrow or write may touch. Dynamic projections
    /// cover their enclosing place; computed receiver values have no lexical root.
    pub(super) fn borrow_place(&mut self, place: &Expr) -> Option<(Text, Option<String>)> {
        self.borrow_place_projection(place).map(|(root, path, _)| (root, path))
    }

    /// Permission of the root reference when the first projection addresses its
    /// referent. Borrowing the reference slot itself has no such projection.
    pub(super) fn projected_referent_mode(&self, place: &Expr) -> Option<bool> {
        let mut step = unparen(place);
        while let Some((operand, _)) = place_operand(step) {
            let operand = unparen(operand);
            if let ExprKind::Path(path) = &operand.kind {
                let ty = &self.ctx.env.lookup(&path.as_ident()?.name)?.ty;
                let head = self.place_reference_head(ty, operand.span).ok()?;
                let (inner, mode, _) = head.reference_parts()?;
                let referent = self.place_reference_head(inner, operand.span).ok()?;
                let through_reference = match &step.kind {
                    ExprKind::Field { field, .. } if referent.reference_parts().is_none() => {
                        self.lookup_field_type(&self.record_conversion_head(&referent), &field.name)
                            .is_some()
                    }
                    ExprKind::TupleIndex { index, .. } => {
                        matches!(referent, Type::Tuple(ref fields) if (*index as usize) < fields.len())
                    }
                    ExprKind::Index { .. } => matches!(referent, Type::Array { .. } | Type::Slice { .. }),
                    ExprKind::Unary { op: UnOp::Deref, .. } => true,
                    _ => false,
                };
                return through_reference.then_some(mode);
            }
            step = operand;
        }
        None
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
        let root = Text::from(lexical_place_root(step)?);
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
        let root = Text::from(lexical_place_root(step)?);
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
    fn check_write_through_reference(&self, ty: &Type, auto_deref: bool, span: Span) -> Result<bool> {
        let mut current = self.place_reference_head(ty, span)?;
        for depth in 0..64 {
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
                return Ok(depth != 0);
            };
            if !mutable {
                return Err(TypeError::SharedReferenceWrite { span });
            }
            if !auto_deref {
                return Ok(true);
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
        if let Some(name) = lexical_place_root(expr)
            && let Some(scheme) = self.ctx.env.lookup(name)
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
        self.place_operands.iter().rev().find_map(|scope| match scope.get(&key)? {
            PlaceOperand::TypeOnly(ty) => ty.as_ref(),
            PlaceOperand::Value(operand) => operand.as_ref().map(|operand| &operand.ty),
        }).map(|ty| self.unifier.apply(ty))
    }

    fn observed_operand(&self, expr: &Expr) -> Option<&EvaluatedOperand> {
        let key = std::ptr::from_ref(unparen(expr)).addr();
        self.place_operands.iter().rev().find_map(|scope| match scope.get(&key)? {
            PlaceOperand::Value(operand) => operand.as_ref(),
            PlaceOperand::TypeOnly(_) => None,
        })
    }

    pub(super) fn observed_operand_origins(&self, expr: &Expr) -> Option<List<RecordReceiver>> {
        Some(self.observed_operand(expr)?.origins.clone())
    }

    pub(super) fn observed_known_owned_payload(&self, expr: &Expr) -> bool {
        self.observed_operand(expr).is_some_and(|operand| operand.known_owned_payload)
    }

    pub(super) fn observed_raw_address(&self, expr: &Expr) -> Option<List<RecordReceiver>> {
        self.observed_operand(expr)?.raw_address.as_deref().map(List::from)
    }

    pub(super) fn observed_indexed_owners(&self, expr: &Expr) -> Option<List<RecordReceiver>> {
        self.observed_operand(expr)?.indexed_owners.clone()
    }
}
