//! A reference result can identify a carrier only with a derivation for its
//! referent. Returning the same reference type does not prove pointer identity.
use super::TypeChecker;
use super::write_places::lexical_place_root;
use crate::context::{RecordFieldOrigins, ReferenceOrigin};
use crate::ty::{RecordReceiver, Type};
use verum_common::List;
use verum_ast::{
    Expr, ExprKind, StmtKind, UnOp,
};

const INDEXED_TEMPORARY: &str = "<temporary indexed container>";

fn unparen(mut expr: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expr.kind {
        expr = inner;
    }
    expr
}

fn field_root(expr: &Expr) -> (&Expr, List<&verum_ast::Ident>) {
    let mut fields = List::new();
    let mut root = unparen(expr);
    while let ExprKind::Field { expr, field } = &root.kind {
        fields.push(field);
        root = unparen(expr);
    }
    (root, fields)
}

#[derive(Clone, Copy, PartialEq)]
enum AutoBorrowRoot {
    SharedOnly,
    IncludeExclusive,
}

enum ProjectionOrigins {
    Known(List<RecordReceiver>),
    Unknown,
    /// The field read already failed. Reading its enclosing record again
    /// cannot supply evidence for it.
    Invalidated,
}

/// Observed candidates and whether they describe the complete payload.
#[derive(Debug)]
pub(super) struct EvaluatedValueOrigins {
    origins: Option<List<RecordReceiver>>,
    complete: bool,
    fields: Option<RecordFieldOrigins>,
}

/// Loans carried on the early exit from the innermost lexical try expression.
pub(super) struct ResidualLoanTarget {
    region: usize,
    origins: List<RecordReceiver>,
}

pub(super) struct CopiedCapture {
    name: verum_common::Text,
    scheme: crate::context::TypeScheme,
    origins: List<RecordReceiver>,
}

fn reference_mutability(ty: &Type) -> Option<bool> {
    match ty {
        Type::Reference { mutable, .. }
        | Type::CheckedReference { mutable, .. }
        | Type::UnsafeReference { mutable, .. } => Some(*mutable),
        _ => None,
    }
}


impl TypeChecker {
    /// A projection through a reference writes the referent's storage. The
    /// lexical binding holding the reference can leave scope before that value.
    fn projected_store_destinations(&mut self, destination: &Expr) -> Option<List<RecordReceiver>> {
        let (owner, field) = match &unparen(destination).kind {
            ExprKind::Field { expr, field } => (Some(expr.as_ref()), Some(&field.name)),
            ExprKind::Unary { op: UnOp::Deref, expr } => (Some(expr.as_ref()), None),
            _ => (None, None),
        };
        if let Some(owner) = owner {
            if let Some(field) = field
                && let Some((mut place, ty)) = self.call_input_place(owner)
                && self.lookup_field_type(&self.record_conversion_head(&ty), field).is_some()
            {
                place.fields.push(field.clone());
                return Some(List::from_iter([place]));
            }
            let ty = self.observed_operand_type(owner)
                .or_else(|| self.type_registry.lookup_expr(owner.span).cloned());
            if let Some(ty) = ty && self.reference_mode(&ty, owner.span).ok()?.is_some() {
                let mut targets = List::new();
                self.collect_reference_candidates(owner, &mut targets);
                if let Some(field) = field {
                    for target in &mut targets { target.fields.push(field.clone()); }
                }
                if !targets.is_empty() { return Some(targets); }
                // A reference field of an incoming referent may point to
                // separate caller storage. Keep its external lifetime bound.
                let (holder, _) = self.borrow_place(owner)?;
                let incoming = self.ctx.env.reference_candidates(&holder);
                return (!incoming.is_empty() && incoming.iter()
                    .all(|target| self.ctx.env.is_incoming_referent(target.binding)))
                    .then_some(incoming);
            }
        }
        let (holder, _) = self.borrow_place(destination)?;
        let ty = self.place_reference_head(&self.ctx.env.lookup(&holder)?.ty, destination.span).ok()?;
        if matches!(ty, Type::Pointer { .. } | Type::VolatilePointer { .. }) {
            let mut targets = self.ctx.env.raw_address_origins(&holder)?;
            let place = unparen(destination);
            let (deref, field) = match &place.kind {
                ExprKind::Field { expr, field } => (unparen(expr), Some(&field.name)),
                _ => (place, None),
            };
            let ExprKind::Unary { op: UnOp::Deref, expr } = &deref.kind else { return None };
            if !matches!(&unparen(expr).kind, ExprKind::Path(path) if path.as_ident()?.name == holder) {
                return None;
            }
            if let Some(field) = field {
                for target in &mut targets { target.fields.push(field.clone()); }
            }
            return Some(targets);
        }
        if self.reference_mode(&ty, destination.span).ok()?.is_some() {
            let targets = self.ctx.env.reference_candidates(&holder);
            return (!targets.is_empty()).then_some(targets);
        }
        Some(List::from_iter([self.ctx.env.record_source(&holder, false)?]))
    }

    fn retain_projected_store(
        &mut self,
        destination: &Expr,
        incoming: &[RecordReceiver],
        complete: bool,
    ) -> crate::Result<()> {
        let targets = self.projected_store_destinations(destination);
        let inline = if !complete {
            self.call_input_place(destination).map(|(place, _)| place)
        } else { None };
        self.retain_addressed_store(targets, inline.as_ref(), incoming, complete, destination.span)
    }

    fn retain_addressed_store(
        &mut self, targets: Option<List<RecordReceiver>>, inline: Option<&RecordReceiver>,
        incoming: &[RecordReceiver], complete: bool, span: verum_ast::Span,
    ) -> crate::Result<()> {
        let Some(targets) = targets else {
            self.refinement_evidence.invalidate_write(&[]);
            if complete && incoming.is_empty() { return Ok(()); }
            return Err(crate::TypeError::UnknownRecordOrigin { span });
        };
        self.refinement_evidence.invalidate_write(&targets);
        for target in targets {
            self.check_input_storage_retention(incoming, &target, span)?;
            let region = self.ctx.env.storage_loan_region(target.binding)
                .ok_or(crate::TypeError::UnknownRecordOrigin { span })?;
            self.retain_origins_in_region(incoming, region, span)?;
            self.ctx.env.insert_owned_slot_origins(&target, incoming);
            if !complete {
                let fields = if inline == Some(&target) { target.fields.as_slice() } else { &[] };
                self.ctx.env.invalidate_payload_origins(&target, fields, span);
            }
        }
        Ok(())
    }

    pub(super) fn writable_raw_element(&self, ty: &Type, span: verum_ast::Span) -> Option<Type> {
        match self.place_reference_head(ty, span).ok()? {
            Type::Pointer { mutable: true, inner } | Type::VolatilePointer { mutable: true, inner } => Some(*inner),
            _ => None,
        }
    }

    /// Only a type-preserving cast from an exclusive reference or an immutable
    /// pointer already derived from it identifies the original managed slot.
    pub(super) fn raw_store_destinations(&mut self, address: &Expr, element: &Type) -> Option<List<RecordReceiver>> {
        if element.contains_type(|ty| matches!(ty, Type::Unknown | Type::Placeholder { .. })) { return None; }
        let mut address = unparen(address);
        while let ExprKind::Cast { expr, .. } = &address.kind {
            let source = unparen(expr);
            let source_ty = self.type_registry.lookup_expr(expr.span)
                .or_else(|| self.type_registry.lookup_expr(source.span))?;
            let head = self.place_reference_head(source_ty, source.span).ok()?;
            match head {
                Type::Pointer { mutable: true, inner } | Type::VolatilePointer { mutable: true, inner }
                    if self.types_equivalent(&inner, element) => { address = source; }
                Type::Reference { mutable: true, inner } | Type::CheckedReference { mutable: true, inner }
                    if self.types_equivalent(&inner, element) => {
                    let target = if let ExprKind::Unary { op: UnOp::RefMut | UnOp::RefCheckedMut, expr } = &source.kind {
                        match &unparen(expr).kind {
                            ExprKind::Path(path) => self.ctx.env.record_source(path.as_ident()?.as_str(), false)?,
                            ExprKind::Field { expr: owner, field } => {
                                let (mut place, _) = self.call_input_place(owner)?;
                                place.fields.push(field.name.clone());
                                place
                            }
                            _ => return None,
                        }
                    } else { self.reference_result_origin(source)? };
                    return Some(List::from_iter([target]));
                }
                _ => return None,
            }
        }
        let ExprKind::Path(path) = &address.kind else { return None };
        self.ctx.env.raw_address_origins(path.as_ident()?.as_str())
    }

    fn retain_raw_store(&mut self, address: &Expr, value: &Expr, element: &Type, span: verum_ast::Span) -> crate::Result<()> {
        let targets = self.observed_raw_address(address);
        if !self.carries_stored_loan(element) {
            if !self.refinement_evidence.is_empty() {
                self.refinement_evidence.invalidate_write(targets.as_deref().unwrap_or(&[]));
            }
            return Ok(());
        }
        let incoming = self.observed_operand_origins(value);
        let complete = incoming.is_some() && self.observed_known_owned_payload(value);
        self.retain_addressed_store(targets, None, incoming.as_deref().unwrap_or(&[]), complete, span)
    }

    pub(super) fn check_raw_store_intrinsic(
        &mut self, operation: &'static verum_vbc::intrinsics::registry::Intrinsic,
        args: &[Expr], expression: &Expr,
    ) -> crate::Result<Type> {
        let span = expression.span;
        let [_, address, value] = args else {
            return Err(crate::TypeError::wrong_arg_count(operation.name.into(), 2, args.len().saturating_sub(1), span));
        };
        self.require_slot_access(operation.name, span)?;
        self.with_call_place_types(expression, &args[1..], |checker| {
            let pointer = checker.synth_expr(address)?.ty;
            let element = checker.writable_raw_element(&pointer, address.span)
                .ok_or_else(|| crate::TypeError::OtherWithCodeSpanned {
                    code: "E0444".into(), msg: format!("{} requires a writable pointer", operation.name).into(), span: address.span,
                })?;
            checker.check_expr(value, &element)?;
            let parameters = [pointer, element];
            checker.check_call_loans(&args[1..], Some(&parameters), None)?;
            checker.consume_affine_call_args(&args[1..], &parameters)?;
            checker.retain_raw_store(address, value, &parameters[1], span)?;
            Ok(Type::Unit)
        })
    }

    pub(super) fn check_declared_raw_store(&mut self, span: verum_ast::Span, args: &[Expr], params: &[Type], result: &Type) -> crate::Result<bool> {
        if *result != Type::Unit || params.len() != 2 || args.len() != 2 { return Ok(false); }
        let Some((op, order)) = self.direct_slot_operation(span, 2) else { return Ok(false) };
        let [pointer, payload] = order.as_slice() else { return Ok(false) };
        if !op.is_raw_pointer_store() || args.iter().any(|arg| self.deref_adjustments.contains_key(&arg.span))
        { return Ok(false); }
        let Some(element) = self.writable_raw_element(&params[*pointer], span) else { return Ok(false) };
        if !self.types_equivalent(&element, &params[*payload]) { return Ok(false); }
        self.retain_raw_store(&args[*pointer], &args[*payload], &element, span)?;
        Ok(true)
    }

    pub(super) fn retain_iteration_origin(&mut self, origin: &RecordReceiver, span: verum_ast::Span) -> crate::Result<()> {
        let root = self.ctx.env.record_storage_name(origin.binding)
            .ok_or(crate::TypeError::UnknownRecordOrigin { span })?;
        let field = (!origin.fields.is_empty()).then(|| origin.fields.join("."));
        if !self.borrow_tracker.retain_place_loans(&root, field.as_deref(), self.borrow_tracker.scope_depth()) {
            self.retain_origin(origin, None, span)?;
        }
        Ok(())
    }

    pub(super) fn retain_assignment_origins(
        &mut self,
        destination: &Expr,
        origins: &[RecordReceiver],
        carries: bool,
        complete: bool,
    ) -> crate::Result<()> {
        let ExprKind::Path(path) = &unparen(destination).kind else {
            return if carries { self.retain_projected_store(destination, origins, complete) } else { Ok(()) };
        };
        let Some(holder) = path.as_ident() else {
            return if origins.is_empty() { Ok(()) } else {
                Err(crate::TypeError::UnknownRecordOrigin { span: destination.span })
            };
        };
        if !origins.is_empty() {
            let region = self.ctx.env.binding_loan_region(&holder.name)
                .ok_or(crate::TypeError::UnknownRecordOrigin { span: destination.span })?;
            self.retain_origins_in_region(origins, region, destination.span)?;
        }
        if !self.ctx.env.replace_current_binding_origins(&holder.name, origins, complete) && !origins.is_empty() {
            self.ctx.env.assign_reference_origins(&holder.name, origins);
        }
        Ok(())
    }

    fn check_input_storage_retention(
        &self, origins: &[RecordReceiver], destination: &RecordReceiver, span: verum_ast::Span,
    ) -> crate::Result<()> {
        if self.ctx.env.is_incoming_referent(destination.binding) {
            for origin in origins {
                if let Some(var) = self.ctx.env.dropping_loan_storage(
                    origin.binding, true, self.closure_return_depth,
                ) {
                    return Err(crate::TypeError::DanglingReference {
                        var, ref_span: span, drop_span: None,
                    });
                }
            }
        }
        Ok(())
    }

    fn retain_origins_in_region(
        &mut self,
        origins: &[RecordReceiver],
        region: usize,
        span: verum_ast::Span,
    ) -> crate::Result<()> {
        for origin in origins {
            let root = self.ctx.env.record_storage_name(origin.binding)
                .ok_or(crate::TypeError::UnknownRecordOrigin { span })?;
            if self.ctx.env.storage_loan_region(origin.binding).is_some_and(|source| source > region) {
                return Err(crate::TypeError::DanglingReference {
                    var: root, ref_span: span, drop_span: None,
                });
            }
            let field = (!origin.fields.is_empty()).then(|| origin.fields.join("."));
            if !self.borrow_tracker.retain_place_loans(&root, field.as_deref(), region) {
                self.retain_origin(origin, None, span)?;
                self.borrow_tracker.retain_place_loans(&root, field.as_deref(), region);
            }
        }
        Ok(())
    }

    pub(super) fn with_result_borrow_scope(
        &mut self,
        infer: impl FnOnce(&mut Self) -> crate::Result<super::InferResult>,
    ) -> crate::Result<super::InferResult> {
        self.borrow_tracker.enter_scope();
        let result = infer(self);
        if result
            .as_ref()
            .is_ok_and(|result| self.carries_stored_loan(&self.unifier.apply(&result.ty)))
        {
            self.borrow_tracker.exit_scope_retaining_loans();
        } else {
            self.borrow_tracker.exit_scope();
        }
        result
    }

    pub(super) fn reference_mode(
        &self,
        ty: &Type,
        span: verum_ast::Span,
    ) -> crate::Result<Option<bool>> {
        match ty {
            Type::Named { .. }
            | Type::Generic { .. }
            | Type::Var(_)
            | Type::Refined { .. }
            | Type::CapabilityRestricted { .. } => {
                Ok(reference_mutability(&self.place_reference_head(ty, span)?))
            }
            _ => Ok(reference_mutability(ty)),
        }
    }

    pub(super) fn is_shared_reference_head(
        &self,
        ty: &Type,
        span: verum_ast::Span,
    ) -> crate::Result<bool> {
        match ty {
            Type::Reference { mutable, .. } => Ok(!mutable),
            Type::Named { .. }
            | Type::Generic { .. }
            | Type::Var(_)
            | Type::Refined { .. }
            | Type::CapabilityRestricted { .. } => Ok(matches!(
                self.place_reference_head(ty, span)?,
                Type::Reference { mutable: false, .. }
            )),
            _ => Ok(false),
        }
    }

    pub(super) fn aggregate_origins(&mut self, value: &Expr, ty: &Type) -> List<RecordReceiver> {
        let mut found = List::new();
        if !self.carries_stored_loan(ty) {
            return found;
        }
        let value = unparen(value);
        if matches!(value.kind, ExprKind::Tuple(_) | ExprKind::Array(_) | ExprKind::Record { .. })
            && let Some(origins) = self.evaluated_value_origins.get(&value.span).and_then(|observed| observed.origins.as_ref())
        {
            return origins.clone();
        }
        match (&value.kind, ty) {
            (ExprKind::Tuple(values), Type::Tuple(types)) if values.len() == types.len() => {
                for (value, ty) in values.iter().zip(types) {
                    for origin in self.aggregate_origins(value, ty) {
                        if !found.contains(&origin) {
                            found.push(origin);
                        }
                    }
                }
            }
            (ExprKind::Record { fields, base, .. }, _)
                if let Ok(types) = self.extract_record_fields(ty) =>
            {
                for field in fields {
                    let nested = if let (Some(field_type), Some(value)) =
                        (types.get(&field.name.name), &field.value)
                    {
                        self.aggregate_origins(value, field_type)
                    } else {
                        self.ctx.env.reference_candidates(&field.name.name)
                    };
                    for origin in nested {
                        if !found.contains(&origin) {
                            found.push(origin);
                        }
                    }
                }
                if let Some(base) = base {
                    self.collect_reference_candidates(base, &mut found);
                }
            }
            _ => self.collect_reference_candidates(value, &mut found),
        }
        found
    }

    pub(super) fn check_borrowed_move(
        &self,
        name: &verum_common::Text,
        ty: &Type,
        span: verum_ast::Span,
    ) -> crate::Result<()> {
        if let Some(borrow) = self.borrow_tracker.overlapping_borrow(name, None, true)
            && !self.protocol_checker.read().implements_protocol(
                ty,
                verum_common::well_known_types::WellKnownProtocol::Copy.as_str(),
            )
        {
            return Err(crate::TypeError::MoveWhileBorrowed {
                var: name.clone(),
                move_span: span,
                borrow_span: borrow.span,
            });
        }
        Ok(())
    }

    pub(super) fn bind_aggregate_result(
        &mut self,
        value: &Expr,
        ty: &Type,
    ) -> crate::Result<Option<ReferenceOrigin>> {
        if self.reference_mode(ty, value.span)?.is_some() || !self.carries_stored_loan(ty) {
            return Ok(None);
        }
        let sources = self.aggregate_origins(value, ty);
        self.check_temporary_binding(&sources, value.span)?;
        for source in &sources {
            self.retain_origin(source, None, value.span)?;
        }
        if let Some(fields) = self.evaluated_value_origins.get(&unparen(value).span)
            .filter(|observed| observed.complete).and_then(|observed| observed.fields.as_ref())
        {
            Ok(Some(ReferenceOrigin::OwnedRecord {
                origins: sources,
                fields: fields.clone(),
            }))
        } else if self.complete_slot_value_origins(value) {
            Ok(Some(ReferenceOrigin::OwnedPayload(sources.into_boxed_slice())))
        } else {
            Ok((!sources.is_empty()).then(|| ReferenceOrigin::Candidates(sources.into_boxed_slice())))
        }
    }

    pub(super) fn check_aggregate_escape(
        &mut self,
        expr: &Expr,
        ty: &Type,
        returning: bool,
    ) -> crate::Result<()> {
        for source in self.aggregate_origins(expr, ty) {
            if let Some(var) = self
                .ctx
                .env
                .dropping_loan_storage(source.binding, returning, self.closure_return_depth)
            {
                return Err(crate::TypeError::DanglingReference {
                    var,
                    ref_span: self.escaping_operand_span(expr, &source),
                    drop_span: None,
                });
            }
        }
        Ok(())
    }

    /// Narrow the error to an operand observed to carry this exact loan.
    /// Re-reading a place here could attribute an earlier field's loan to a
    /// later assignment. Missing observations leave the enclosing span intact.
    fn escaping_operand_span(&self, expression: &Expr, source: &RecordReceiver) -> verum_ast::Span {
        let carries = |span| self.evaluated_value_origins.get(&span)
            .and_then(|observed| observed.origins.as_ref()).is_some_and(|origins| origins.contains(source));
        let mut value = expression;
        loop {
            let next = match &unparen(value).kind {
                ExprKind::Record { fields, base, .. } => {
                    let mut child = None;
                    for field in fields {
                        match &field.value {
                            Some(value) if carries(value.span) => { child = Some(value); break; }
                            None if carries(field.span) => return field.name.span,
                            _ => {}
                        }
                    }
                    child.or_else(|| base.as_deref().filter(|base| carries(base.span)))
                }
                ExprKind::Tuple(values) | ExprKind::Array(verum_ast::ArrayExpr::List(values)) =>
                    values.iter().find(|child| carries(child.span)),
                ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, .. }) =>
                    Some(value.as_ref()).filter(|child| carries(child.span)),
                _ => None,
            };
            let Some(child) = next else { return value.span };
            value = child;
        }
    }

    pub(super) fn check_return_value_loans(
        &mut self,
        expr: &Expr,
        ty: &Type,
        span: verum_ast::Span,
    ) -> crate::Result<()> {
        self.check_aggregate_escape(expr, ty, true)?;
        if self.reference_mode(ty, span)?.is_some() {
            self.check_return_lifetime(expr, span)?;
        }
        Ok(())
    }

    /// Unlike a witness identity, a lifetime restriction may conservatively
    /// retain several input loans when a callee can choose its return value.
    pub(super) fn bind_reference_result(
        &mut self,
        value: &Expr,
        holder: &str,
        shared: bool,
    ) -> crate::Result<Option<ReferenceOrigin>> {
        if let Some(origin) = self.reference_result_origin(value) {
            self.check_temporary_binding(std::slice::from_ref(&origin), value.span)?;
            if shared
                && matches!(
                    &unparen(value).kind,
                    ExprKind::Call { .. } | ExprKind::Path(_)
                )
            {
                self.retain_origin(&origin, Some(holder), value.span)?;
            }
            return Ok(Some(ReferenceOrigin::Exact(origin)));
        }
        let mut candidates = List::new();
        self.collect_reference_candidates(value, &mut candidates);
        self.check_temporary_binding(&candidates, value.span)?;
        if shared {
            for source in &candidates {
                self.retain_origin(source, Some(holder), value.span)?;
            }
        }
        Ok((!candidates.is_empty())
            .then(|| ReferenceOrigin::Candidates(candidates.into_boxed_slice())))
    }

    pub(super) fn observe_match_arm_origins(&mut self, body: &Expr, needed: bool) {
        if body.span.is_empty() {
            return;
        }
        if !needed {
            // A shared expansion site cannot authenticate a different binding.
            if let Some(previous) = self.evaluated_value_origins.get_mut(&body.span) {
                previous.origins = None;
                previous.complete = false;
            }
            return;
        }
        let origins = self.match_value_origins(body);
        self.record_evaluated_origins(body.span, origins);
    }

    pub(super) fn record_evaluated_origins(
        &mut self,
        span: verum_ast::Span,
        origins: Option<List<RecordReceiver>>,
    ) {
        self.record_owned_value_origins(span, origins, false);
    }

    /// Reused expansion sites can union possible loans but keep completeness
    /// only when every evaluation supplied an exhaustive set.
    pub(super) fn record_owned_value_origins(
        &mut self,
        span: verum_ast::Span,
        origins: Option<List<RecordReceiver>>,
        complete: bool,
    ) {
        self.record_payload_fields(span, origins, complete, None);
    }

    fn record_payload_fields(
        &mut self,
        span: verum_ast::Span,
        origins: Option<List<RecordReceiver>>,
        complete: bool,
        fields: Option<RecordFieldOrigins>,
    ) {
        if span.is_empty() {
            return;
        }
        self.evaluated_value_origins
            .entry(span)
            .and_modify(|previous| {
                previous.complete &= complete;
                match (previous.fields.as_mut(), fields.as_ref()) {
                    (Some(saved), Some(observed)) if saved.len() == observed.len()
                        && saved.keys().all(|field| observed.contains_key(field)) =>
                    {
                        for (field, retained) in saved {
                            for origin in &observed[field] {
                                if !retained.contains(origin) { retained.push(origin.clone()); }
                            }
                        }
                    }
                    _ => previous.fields = None,
                }
                match (previous.origins.as_mut(), origins.as_ref()) {
                    (Some(known), Some(observed)) => {
                        for origin in observed {
                            if !known.contains(origin) {
                                known.push(origin.clone());
                            }
                        }
                    }
                    _ => previous.origins = None,
                }
            })
            .or_insert(EvaluatedValueOrigins { origins, complete, fields });
    }

    pub(super) fn observe_closure_capture_origins(
        &mut self,
        span: verum_ast::Span,
        captures: &[(verum_common::Text, Option<verum_common::Text>, crate::aliasing::CaptureMode, verum_ast::Span)],
    ) -> List<CopiedCapture> {
        let mut origins = List::new();
        let mut copied = List::new();
        for (name, _, mode, capture_span) in captures {
            let Some(scheme) = self.ctx.env.lookup(name).cloned() else { continue; };
            let carries_loans = self.carries_stored_loan(&scheme.ty);
            let mutable = *mode == crate::aliasing::CaptureMode::MutBorrow
                || self.ctx.env.binding_place_is_mutable(name);
            // Mutable capture cells do not have an owned lifecycle receipt.
            // Keep their source storage until that boundary is established.
            if mutable
                && let Some(origin) = self.ctx.env.record_source(name, false)
            {
                if !origins.contains(&origin) { origins.push(origin); }
            }
            // Immutable captures copy their values, including any references
            // in those values. A scalar capture does not borrow its old slot.
            let payloads = if carries_loans {
                if let Some(place) = self.ctx.env.record_source(name, false) {
                    self.check_payload_origin_observation(&place, *capture_span);
                }
                self.ctx.env.reference_candidates(name)
            } else { List::new() };
            for origin in &payloads {
                if !origins.contains(origin) { origins.push(origin.clone()); }
            }
            if !mutable && self.ctx.env.binding_loan_region(name).is_some()
                && !copied.iter().any(|capture: &CopiedCapture| &capture.name == name)
            {
                copied.push(CopiedCapture { name: name.clone(), scheme, origins: payloads });
            }
        }
        self.record_evaluated_origins(span, Some(origins));
        copied
    }

    pub(super) fn bind_copied_capture_values(&mut self, captures: List<CopiedCapture>) {
        for capture in captures {
            // Formation copies an immutable capture into distinct owned
            // storage. Its payload keeps the old loans; taking its address
            // borrows this new slot, not the captured reference's referent.
            self.ctx.env.insert(capture.name.clone(), capture.scheme);
            self.ctx.env.set_binding_loan_region(&capture.name, self.borrow_tracker.scope_depth());
            let origin = if capture.origins.len() == 1 {
                ReferenceOrigin::Exact(capture.origins.into_iter().next().expect("one origin"))
            } else {
                ReferenceOrigin::Candidates(capture.origins.into_boxed_slice())
            };
            self.ctx.env.set_reference_origin(&capture.name, origin);
        }
    }

    pub(super) fn observe_record_shorthand_origins(
        &mut self, field: &verum_ast::expr::FieldInit, ty: &Type,
    ) {
        let mut origins = List::new();
        let complete = if self.carries_stored_loan(ty) {
            let name = &field.name.name;
            let known = self.ctx.env.binding_has_payload_origins(name)
                || (self.reference_mode(ty, field.span).is_ok_and(|mode| mode.is_some())
                    && self.ctx.env.record_source(name, true).is_some());
            let observed = self.ctx.env.record_source(name, false)
                .is_some_and(|place| self.check_payload_origin_observation(&place, field.name.span));
            origins = self.ctx.env.reference_candidates(name);
            known && observed
        } else {
            true
        };
        self.record_owned_value_origins(field.span, Some(origins), complete);
    }

    fn operand_has_complete_payload(&mut self, value: &Expr) -> bool {
        self.observed_operand_type(value).is_some_and(|ty|
            !self.carries_stored_loan(&ty) || self.observed_known_owned_payload(value))
    }

    /// Aggregate fields are evaluated individually, before later fields can
    /// replace the places from which earlier field values were copied.
    pub(super) fn observe_aggregate_result_origins(&mut self, value: &Expr, ty: &Type) {
        let mut origins = List::new();
        let mut complete = false;
        let mut field_origins = None;
        match &unparen(value).kind {
            ExprKind::Tuple(values) | ExprKind::Array(verum_ast::ArrayExpr::List(values)) => {
                complete = true;
                for value in values {
                    complete &= self.operand_has_complete_payload(value);
                    self.collect_evaluated_operand_origins(value, &mut origins);
                }
            }
            ExprKind::Array(verum_ast::ArrayExpr::Repeat { value, .. }) => {
                complete = self.operand_has_complete_payload(value);
                self.collect_evaluated_operand_origins(value, &mut origins);
            }
            ExprKind::Record { fields, base, .. } => {
                complete = true;
                if fields.len() > 1 && base.is_none() && self.carries_stored_loan(ty) {
                    field_origins = Some(RecordFieldOrigins::new());
                }
                for field in fields {
                    let retained = if let Some(value) = &field.value {
                        complete &= self.operand_has_complete_payload(value);
                        let mut retained = List::new();
                        self.collect_evaluated_operand_origins(value, &mut retained);
                        retained
                    } else if let Some(observed) = self.evaluated_value_origins.get(&field.span)
                        && let Some(known) = &observed.origins
                    {
                        complete &= observed.complete;
                        known.clone()
                    } else {
                        self.record_evaluated_origins(value.span, None);
                        return;
                    };
                    for origin in &retained {
                        if !origins.contains(origin) { origins.push(origin.clone()); }
                    }
                    if let Some(known) = &mut field_origins {
                        known.insert(field.name.name.clone(), retained);
                    }
                }
                if let Some(base) = base {
                    complete &= self.operand_has_complete_payload(base);
                    self.collect_evaluated_operand_origins(base, &mut origins);
                }
            }
            _ => return,
        }
        if !self.carries_stored_loan(ty) { origins.clear(); }
        let fields = field_origins.filter(|_| complete && !origins.is_empty());
        self.record_payload_fields(value.span, Some(origins), complete, fields);
    }

    /// A call result may carry any evaluated input or callable capture unless
    /// the selected body establishes a narrower result. This observation is
    /// deliberately independent of permission to mutate those inputs.
    pub(super) fn observe_call_result_origins(
        &mut self,
        span: verum_ast::Span,
        result: &Type,
        args: &[Expr],
        params: Option<&[Type]>,
        callable: Option<&Expr>,
        receiver: Option<(&Expr, bool)>,
        precise: Option<List<RecordReceiver>>,
    ) -> crate::Result<()> {
        if !self.carries_stored_loan(result) {
            self.record_evaluated_origins(span, Some(List::new()));
            return Ok(());
        }
        if let Some(origins) = precise {
            // A checked whole-value forward transfers exactly the payload
            // observed at its argument, including any uncertainty.
            let complete = self.type_registry.returned_value(span)
                .and_then(|body| body.forwarded_parameter())
                .and_then(|index| args.get(index))
                .is_some_and(|argument| self.operand_has_complete_payload(argument));
            self.record_owned_value_origins(span, Some(origins), complete);
            return Ok(());
        }
        if matches!(self.type_registry.lookup_call_target(span),
            Some(Type::Function { contexts: Some(contexts), .. }) if !contexts.is_empty())
        {
            // These operands do not describe a supplied context's payload.
            self.record_evaluated_origins(span, None);
            return Ok(());
        }
        let mut found = List::new();
        for (index, argument) in args.iter().enumerate() {
            let actual = self.observed_operand_type(argument)
                .or_else(|| self.type_registry.lookup_expr(argument.span).cloned());
            let borrowed = params.and_then(|params| params.get(index))
                .map(|ty| self.reference_mode(ty, argument.span))
                .transpose()?.flatten().is_some();
            if borrowed && actual.as_ref().is_some_and(|ty|
                self.reference_mode(ty, argument.span).is_ok_and(|mode| mode.is_none()))
            {
                if let Some(origins) = self.borrowed_call_input_origins(
                    argument, AutoBorrowRoot::IncludeExclusive,
                ) {
                    for origin in origins {
                        if !found.contains(&origin) { found.push(origin); }
                    }
                } else {
                    self.record_evaluated_origins(span, None);
                    return Ok(());
                }
            } else {
                self.collect_evaluated_operand_origins(argument, &mut found);
            }
        }
        if let Some(callable) = callable {
            self.collect_evaluated_operand_origins(callable, &mut found);
        }
        if let Some((receiver, borrowed)) = receiver {
            if borrowed {
                if let Some(origins) = self.borrowed_call_input_origins(
                    receiver, AutoBorrowRoot::IncludeExclusive,
                ) {
                    for origin in origins {
                        if !found.contains(&origin) { found.push(origin); }
                    }
                }
            }
            self.collect_evaluated_operand_origins(receiver, &mut found);
        }
        // A data constructor can only store its explicit fields. Read their
        // evaluation-time evidence before the enclosing call retires its operands.
        let complete = self.type_registry.constructed_value(span).is_some()
            && args.iter().all(|argument| self.operand_has_complete_payload(argument));
        self.record_owned_value_origins(span, Some(found), complete);
        Ok(())
    }

    pub(super) fn collect_evaluated_operand_origins(
        &mut self,
        value: &Expr,
        found: &mut List<RecordReceiver>,
    ) {
        if let Some(origins) = self.observed_operand_origins(value) {
            for origin in origins {
                if !found.contains(&origin) { found.push(origin); }
            }
        } else {
            self.collect_reference_candidates(value, found);
        }
    }

    /// Prove which lexical origins a value can carry while its pattern bindings
    /// are live. A missing observation preserves the scrutinee as a candidate.
    fn match_value_origins(&mut self, body: &Expr) -> Option<List<RecordReceiver>> {
        let mut pending = List::from_iter([body]);
        let mut origins = List::new();
        while let Some(value) = pending.pop() {
            if !value.span.is_empty()
                && self.type_registry.lookup_expr(value.span)
                    .is_some_and(|ty| !self.carries_stored_loan(ty))
            {
                continue;
            }
            match &unparen(value).kind {
                ExprKind::Path(path) => {
                    let binding = &path.as_ident()?.name;
                    let sources = self.ctx.env.reference_candidates(binding);
                    if sources.is_empty() {
                        return None;
                    }
                    for source in sources {
                        if !origins.contains(&source) {
                            origins.push(source);
                        }
                    }
                }
                ExprKind::Tuple(parts) | ExprKind::Array(verum_ast::expr::ArrayExpr::List(parts)) => {
                    pending.extend(parts.iter());
                }
                ExprKind::Record { fields, base, .. } => {
                    for field in fields {
                        pending.push(field.value.as_ref()?);
                    }
                    pending.extend(base.as_deref());
                }
                ExprKind::Field { expr, .. } | ExprKind::TupleIndex { expr, .. } => {
                    match self.projected_value_origins(value) {
                        ProjectionOrigins::Known(sources) => for source in sources {
                            if !origins.contains(&source) {
                                origins.push(source);
                            }
                        },
                        ProjectionOrigins::Unknown => pending.push(expr),
                        ProjectionOrigins::Invalidated => return None,
                    }
                }
                ExprKind::MethodCall { args, .. } => {
                    // The selected static declaration has no receiver, so a
                    // caller-local loan must enter through an argument or a
                    // provided context. Use the selected *parameter* types:
                    // an implicit borrow need not appear in the argument AST.
                    let Type::Function { params, contexts, .. } =
                        self.type_registry.explicit_input_target(value.span)? else { return None };
                    if params.len() != args.len()
                        || contexts.as_ref().is_some_and(|contexts| !contexts.is_empty())
                        || params.iter().any(|ty| self.may_supply_call_borrow(ty))
                    {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        Some(origins)
    }

    pub(super) fn with_residual_loan_target<T>(
        &mut self, check: impl FnOnce(&mut Self) -> crate::Result<T>,
    ) -> crate::Result<(T, List<RecordReceiver>)> {
        let outer = self.residual_loan_target.replace(ResidualLoanTarget {
            region: self.borrow_tracker.scope_depth(),
            origins: List::new(),
        });
        let result = check(self);
        let target = std::mem::replace(&mut self.residual_loan_target, outer)
            .expect("scoped residual destination");
        result.map(|value| (value, target.origins))
    }

    pub(super) fn check_residual_loans(
        &mut self, operand: &Expr, carrier: &Type, residual: &Type, target_residual: &Type,
    ) -> crate::Result<()> {
        if !self.carries_stored_loan(residual) || !self.carries_stored_loan(target_residual) {
            return Ok(());
        }
        let Some(region) = self.residual_loan_target.as_ref().map(|target| target.region) else {
            return self.check_aggregate_escape(operand, carrier, true);
        };
        let origins = self.aggregate_origins(operand, carrier);
        self.retain_origins_in_region(&origins, region, operand.span)?;
        let target = self.residual_loan_target.as_mut().expect("active residual destination");
        for origin in origins {
            if !target.origins.contains(&origin) { target.origins.push(origin); }
        }
        Ok(())
    }

    pub(super) fn observe_control_result(
        &mut self, span: verum_ast::Span, value: &Expr, result: &Type,
        mut origins: List<RecordReceiver>,
    ) {
        for origin in self.aggregate_origins(value, result) {
            if !origins.contains(&origin) { origins.push(origin); }
        }
        self.record_evaluated_origins(span, Some(origins));
    }

    /// The success payload of a core sum is moved out of the evaluated
    /// operand. A user-defined Try implementation can compute another value,
    /// so its input loans are candidates, never an exhaustive receipt.
    pub(super) fn observe_try_output(
        &mut self, operand: &Expr, carrier: &Type, output: &Type, span: verum_ast::Span,
    ) {
        if !self.carries_stored_loan(output) { return; }
        let projection = crate::protocol::TryProtocolResolution::core_sum_arguments(carrier, false)
            .is_some_and(|arguments| self.types_equivalent(&arguments[0], output));
        let origins = self.aggregate_origins(operand, carrier);
        let complete = projection && self.complete_slot_value_origins(operand);
        self.record_owned_value_origins(span, Some(origins), complete);
    }

    fn evaluated_result_origins(&self, value: &Expr) -> Option<&List<RecordReceiver>> {
        let value = unparen(value);
        match &value.kind {
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } | ExprKind::MetaFunction { .. } | ExprKind::Closure { .. }
            | ExprKind::Index { .. } | ExprKind::Tuple(_) | ExprKind::Array(_) | ExprKind::Record { .. }
            | ExprKind::Try(_) | ExprKind::TryBlock(_)
            | ExprKind::TryRecover { .. } | ExprKind::TryRecoverFinally { .. } =>
                self.evaluated_value_origins.get(&value.span)?.origins.as_ref(),
            _ => None,
        }
    }

    /// A visible loan is only one candidate when a producer can also read a
    /// context or callback environment. A checked insertion needs the complete
    /// set, including when an opaque result was first bound to a local.
    pub(super) fn complete_slot_value_origins(&mut self, value: &Expr) -> bool {
        let mut value = unparen(value);
        while let ExprKind::Block(block) | ExprKind::Unsafe(block) = &value.kind {
            let Some(tail) = block.expr.as_deref() else {
                return false;
            };
            if !block.stmts.is_empty() {
                return false;
            }
            value = unparen(tail);
        }
        if self
            .evaluated_value_origins
            .get(&value.span)
            .is_some_and(|observed| observed.complete && observed.origins.is_some())
        {
            return true;
        }
        match &value.kind {
            ExprKind::Path(_) => {
                self.reference_result_origin(value).is_some()
                    || lexical_place_root(value)
                        .is_some_and(|name| self.ctx.env.binding_has_payload_origins(name))
            }
            ExprKind::Unary {
                op: UnOp::Ref | UnOp::RefMut | UnOp::RefChecked | UnOp::RefCheckedMut,
                ..
            } => self.reference_result_origin(value).is_some(),
            ExprKind::Record {
                fields, base: None, ..
            } => fields.iter().all(|field| {
                let ty = match field.value.as_ref() {
                    Some(value) => self.type_registry.lookup_expr(value.span),
                    None => self
                        .ctx
                        .env
                        .lookup(&field.name.name)
                        .map(|scheme| &scheme.ty),
                };
                ty.is_some_and(|ty| !self.carries_stored_loan(ty))
            }),
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                self.evaluated_result_origins(value).is_some()
                    && self.type_registry.declared_receiver(value.span).is_none()
                    && matches!(self.type_registry.explicit_input_target(value.span), Some(Type::Function { params, contexts, .. })
                        if contexts.as_ref().is_none_or(|contexts| contexts.is_empty())
                            && params.iter().all(|ty| !self.may_return_input_loan(ty)))
            }
            _ => false,
        }
    }

    pub(super) fn collect_reference_candidates(&mut self, value: &Expr, found: &mut List<RecordReceiver>) {
        fn add(found: &mut List<RecordReceiver>, source: RecordReceiver) {
            if !found.contains(&source) {
                found.push(source);
            }
        }
        // Use the result established by ordinary inference. Re-inferring here
        // would consume affine operands twice; descending into a loan-free
        // result confuses its evaluation inputs with its stored value.
        if !value.span.is_empty()
            && self.type_registry.lookup_expr(value.span)
                .is_some_and(|ty| !self.carries_stored_loan(ty))
        {
            return;
        }
        if let Some(origins) = self.evaluated_result_origins(value) {
            for origin in origins { add(found, origin.clone()); }
            return;
        }
        if let Some(source) = self.reference_result_origin(value) {
            add(found, source);
            return;
        }
        match &unparen(value).kind {
            ExprKind::Path(path) => {
                if let Some(name) = lexical_place_root(value)
                    && (path.as_ident().is_some() || self.ctx.env.binding_has_payload_origins(name))
                {
                    if let Some(place) = self.ctx.env.record_source(name, false) {
                        self.check_payload_origin_observation(&place, value.span);
                    }
                    for origin in self.ctx.env.reference_candidates(name) {
                        add(found, origin);
                    }
                }
            }
            ExprKind::Tuple(values) | ExprKind::Array(verum_ast::expr::ArrayExpr::List(values)) => {
                for value in values {
                    self.collect_reference_candidates(value, found);
                }
            }
            ExprKind::Array(verum_ast::expr::ArrayExpr::Repeat { value, .. }) => {
                self.collect_reference_candidates(value, found)
            }
            ExprKind::Record { fields, base, .. } => {
                for field in fields {
                    if let Some(value) = &field.value {
                        self.collect_reference_candidates(value, found);
                    } else {
                        for origin in self.ctx.env.reference_candidates(&field.name.name) {
                            add(found, origin);
                        }
                    }
                }
                if let Some(base) = base {
                    self.collect_reference_candidates(base, found);
                }
            }
            ExprKind::Field { expr, .. } | ExprKind::TupleIndex { expr, .. } => {
                match self.projected_value_origins(value) {
                    ProjectionOrigins::Known(origins) => for origin in origins { add(found, origin); },
                    ProjectionOrigins::Unknown => self.collect_reference_candidates(expr, found),
                    ProjectionOrigins::Invalidated => {}
                }
            }
            ExprKind::Unary { op: UnOp::Deref, expr } => {
                let ty = self.observed_operand_type(expr)
                    .or_else(|| self.type_registry.lookup_expr(expr.span).cloned());
                let mut origins = List::new();
                self.collect_reference_candidates(expr, &mut origins);
                if let Some(ty) = ty
                    && let Some((_, payloads)) = self.read_value_origins(ty, origins.clone(), value.span, true)
                {
                    origins = payloads;
                }
                for origin in origins { add(found, origin); }
            }
            ExprKind::Call { func, args, .. } => {
                if let Some(origins) = self.returned_value_origins(value) {
                    for origin in origins { add(found, origin); }
                    return;
                }
                for arg in args {
                    self.collect_reference_candidates(arg, found);
                }
                self.collect_reference_candidates(func, found);
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                if let Some(origins) = self.returned_value_origins(value) {
                    for origin in origins { add(found, origin); }
                    return;
                }
                // A consumed receiver passes on any loans in its value. Only
                // a borrowed (or unresolved) receiver also lends its own slot.
                if !self.type_registry.consumes_receiver(unparen(value).span)
                    && let Some(sources) = self.borrowed_call_input_origins(receiver, AutoBorrowRoot::SharedOnly)
                {
                    for source in sources { add(found, source); }
                }
                self.collect_reference_candidates(receiver, found);
                for arg in args {
                    self.collect_reference_candidates(arg, found);
                }
            }
            ExprKind::Unary {
                op:
                    UnOp::Ref
                    | UnOp::RefMut
                    | UnOp::RefChecked
                    | UnOp::RefCheckedMut
                    | UnOp::RefUnsafe
                    | UnOp::RefUnsafeMut,
                expr,
            } => {
                self.collect_borrowed_place_candidates(expr, found);
            }
            ExprKind::Index { index, .. } if matches!(index.kind, ExprKind::Range { .. }) => {
                self.collect_borrowed_place_candidates(value, found);
            }
            ExprKind::Block(block) | ExprKind::Async(block) | ExprKind::Unsafe(block) => {
                self.collect_block_candidates(block, found)
            }
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.collect_block_candidates(then_branch, found);
                if let Some(tail) = else_branch {
                    self.collect_reference_candidates(tail, found);
                }
            }
            ExprKind::Match { expr, arms } => {
                let mut uncertain = false;
                for arm in arms {
                    if let Some(origins) = self.evaluated_value_origins.get(&arm.body.span).and_then(|observed| observed.origins.as_ref()) {
                        for origin in origins {
                            add(found, origin.clone());
                        }
                    } else {
                        uncertain = true;
                        self.collect_reference_candidates(&arm.body, found);
                    }
                }
                if uncertain {
                    self.collect_reference_candidates(expr, found);
                }
            }
            ExprKind::NamedArg { value, .. } => self.collect_reference_candidates(value, found),
            _ => {}
        }
    }

    /// Field selection implicitly dereferences its receiver. Its value keeps
    /// the referent's payload loans, while an explicit `&receiver.field` still
    /// borrows that field's storage through the reference-formation path.
    fn projected_value_origins(&mut self, projection: &Expr) -> ProjectionOrigins {
        let receiver = match &unparen(projection).kind {
            ExprKind::Field { expr, field } => {
                if let Some((mut place, owner)) = self.call_input_place(expr)
                    && let Some(ty) = self.lookup_field_type(&self.record_conversion_head(&owner), &field.name)
                {
                    // Copying a scalar field observes no stored references, even
                    // if a sibling field in the same record can carry one.
                    let Ok(ty) = self.unifier.try_apply(&ty, field.span) else { return ProjectionOrigins::Unknown };
                    if !self.carries_stored_loan(&ty) { return ProjectionOrigins::Known(List::new()); }
                    place.fields.push(field.name.clone());
                    if !self.check_payload_origin_observation(&place, projection.span) {
                        return ProjectionOrigins::Invalidated;
                    }
                    return self.ctx.env.inline_field_value_origins(&place)
                        .map_or(ProjectionOrigins::Unknown, ProjectionOrigins::Known);
                }
                expr.as_ref()
            }
            ExprKind::TupleIndex { expr, .. } => expr.as_ref(),
            _ => return ProjectionOrigins::Unknown,
        };
        let receiver = unparen(receiver);
        let Some(ty) = self.observed_operand_type(receiver).or_else(|| {
            self.type_registry.lookup_expr(receiver.span).cloned()
        }) else { return ProjectionOrigins::Unknown; };
        let mut origins = List::new();
        self.collect_reference_candidates(receiver, &mut origins);
        self.projected_origins_for_type(ty, origins, receiver.span)
            .map_or(ProjectionOrigins::Unknown, |(_, origins)| ProjectionOrigins::Known(origins))
    }

    pub(super) fn projected_origins_for_type(
        &mut self,
        ty: Type,
        origins: List<RecordReceiver>,
        span: verum_ast::Span,
    ) -> Option<(Type, List<RecordReceiver>)> {
        self.read_value_origins(ty, origins, span, false)
    }

    pub(super) fn read_value_origins(
        &mut self,
        mut ty: Type,
        mut origins: List<RecordReceiver>,
        span: verum_ast::Span,
        single_dereference: bool,
    ) -> Option<(Type, List<RecordReceiver>)> {
        let mut expanded = List::new();
        loop {
            let inner = match ty {
                Type::Reference { inner, .. }
                | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } => inner,
                Type::Refined { base, .. } | Type::CapabilityRestricted { base, .. } => {
                    ty = *base;
                    continue;
                }
                _ => {
                    if expanded.contains(&ty) {
                        return None;
                    }
                    let head = self.place_reference_head(&ty, span).ok()?;
                    if head == ty {
                        return Some((ty, origins));
                    }
                    expanded.push(ty);
                    ty = head;
                    continue;
                }
            };
            let mut payloads = List::new();
            for origin in origins {
                if !self.check_payload_origin_observation(&origin, span) { return None; }
                let stored = self.ctx.env.referent_value_origins(&origin)?;
                for source in stored {
                    if !payloads.contains(&source) {
                        payloads.push(source);
                    }
                }
            }
            origins = payloads;
            ty = *inner;
            if single_dereference { return Some((ty, origins)); }
        }
    }

    fn collect_block_candidates(
        &mut self,
        block: &verum_ast::Block,
        found: &mut List<RecordReceiver>,
    ) {
        // Only the tail reaches the join. Its evaluated origins already
        // include loans carried through local bindings and nested calls.
        if let Some(origins) = block.expr.as_deref().and_then(|tail| self.evaluated_result_origins(tail)) {
            for origin in origins {
                if !found.contains(origin) { found.push(origin.clone()); }
            }
            return;
        }
        // The block's local names have gone out of scope. Include inputs of
        // local initializers rather than treating a local tail name as evidence.
        for stmt in &block.stmts {
            match &stmt.kind {
                StmtKind::Let {
                    value: Some(expr), ..
                }
                | StmtKind::Expr { expr, .. } => {
                    self.collect_reference_candidates(expr, found);
                }
                _ => {}
            }
        }
        if let Some(tail) = &block.expr {
            self.collect_reference_candidates(tail, found);
        }
    }

    /// The storage addressed by an explicit borrow or a `ref` pattern.
    /// This is separate from references already stored inside the value.
    pub(super) fn collect_borrowed_place_candidates(
        &mut self, expr: &Expr, found: &mut List<RecordReceiver>,
    ) {
        if let Some((root, path)) = self.borrow_place(expr) {
            let referents = self.ctx.env.reference_candidates(&root);
            let slot = if !referents.is_empty() && self.projected_referent_mode(expr).is_some() {
                None
            } else {
                self.ctx.env.record_source(&root, false)
            };
            for mut source in slot.into_iter().chain(referents) {
                if let Some(path) = &path {
                    source.fields.extend(path.split('.').map(Into::into));
                }
                if !found.contains(&source) { found.push(source); }
            }
        }
    }

    pub(super) fn retain_origin(
        &mut self,
        origin: &RecordReceiver,
        holder: Option<&str>,
        span: verum_ast::Span,
    ) -> crate::Result<()> {
        let root = self
            .ctx
            .env
            .record_storage_name(origin.binding)
            .ok_or(crate::TypeError::UnknownRecordOrigin { span })?;
        let loan = if origin.fields.is_empty() {
            self.borrow_tracker.borrow_immut(root, span)?
        } else {
            let field = origin
                .fields
                .iter()
                .map(|f| f.as_str())
                .collect::<Vec<_>>()
                .join(".");
            self.borrow_tracker.borrow_field_immut(root, field, span)?
        };
        self.borrow_tracker.clear_last_borrow();
        if let Some(holder) = holder {
            self.borrow_tracker.register_holder(holder, loan);
        }
        Ok(())
    }

    pub(super) fn reference_result_origin(&mut self, expr: &Expr) -> Option<RecordReceiver> {
        match &unparen(expr).kind {
            ExprKind::Path(path) => self.ctx.env.record_source(&path.as_ident()?.name, true),
            ExprKind::Unary {
                op: UnOp::Ref | UnOp::RefMut | UnOp::RefChecked | UnOp::RefCheckedMut,
                expr,
            } => {
                let (origin, ty) = self.borrowed_record_place(expr)?;
                // &r borrows the slot holding r. Reusing r's referent as the
                // identity of that slot would lose one level of indirection.
                self.reference_mode(&ty, expr.span)
                    .ok()?
                    .is_none()
                    .then_some(origin)
            }
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => {
                if let ExprKind::MethodCall { receiver, .. } = &unparen(expr).kind
                    && let Some(place) = self.forwarded_receiver_place(expr.span, receiver)
                {
                    return Some(place);
                }
                let body = self.type_registry.returned_value(expr.span)?;
                if matches!(body.nodes.get(body.result)?, crate::context::ReturnedValueNode::BorrowedField { .. }) {
                    let origins = self.returned_value_origins(unparen(expr))?;
                    let [place] = origins.as_slice() else { return None; };
                    return Some(place.clone());
                }
                let index = body.forwarded_parameter()?;
                let Type::Function { params, return_type, .. } =
                    self.type_registry.lookup_call_target(expr.span)? else { return None };
                if self.unifier.apply(params.get(index)?) != self.unifier.apply(return_type) {
                    return None;
                }
                self.reference_result_origin(args.get(index)?)
            }
            _ => None,
        }
    }

    /// The call owns every argument loan until invocation. In particular a
    /// nested call returning a reference keeps its source borrowed alongside
    /// its siblings. Scalar-returning calls do not extend their input loans.
    pub(super) fn check_call_loans(
        &mut self,
        args: &[Expr],
        params: Option<&[Type]>,
        receiver: Option<(&Expr, bool, Option<&Type>)>,
    ) -> crate::Result<()> {
        struct Loan {
            root: verum_common::Text,
            field: Option<String>,
            mutable: bool,
            span: verum_ast::Span,
        }
        fn overlaps(a: Option<&str>, b: Option<&str>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => {
                    a == b
                        || a.strip_prefix(b).is_some_and(|s| s.starts_with('.'))
                        || b.strip_prefix(a).is_some_and(|s| s.starts_with('.'))
                }
                _ => true,
            }
        }
        let mut loans: Vec<Loan> = Vec::new();
        // An owned temporary is a new receiver place. Only a reference result
        // can carry loans from the call which produced it.
        for (arg, receiver_mode, ty) in receiver
            .into_iter()
            .map(|(r, mode, ty)| (r, Some(mode), ty))
            .chain(
                args.iter()
                    .enumerate()
                    .map(|(i, arg)| (arg, None, params.and_then(|p| p.get(i)))),
            )
        {
            let observed = self.observed_operand_type(arg);
            let mode = ty
                .or(observed.as_ref())
                .map(|ty| self.reference_mode(ty, arg.span))
                .transpose()?
                .flatten();
            let Some(mutable) = receiver_mode
                .or(mode)
                .or_else(|| self.argument_reference_mutability(arg))
            else {
                continue;
            };
            let mut origins = List::new();
            if receiver_mode.is_none() || mode.is_some() {
                self.collect_evaluated_operand_origins(arg, &mut origins);
            }
            let mut storage = Vec::new();
            for origin in origins {
                let name = self
                    .ctx
                    .env
                    .record_storage_name(origin.binding)
                    .ok_or(crate::TypeError::UnknownRecordOrigin { span: arg.span })?;
                let path = (!origin.fields.is_empty()).then(|| {
                    origin
                        .fields
                        .iter()
                        .map(|f| f.as_str())
                        .collect::<Vec<_>>()
                        .join(".")
                });
                storage.push((name, path));
            }
            if storage.is_empty() {
                let place = match &unparen(arg).kind {
                    ExprKind::Unary {
                        op:
                            UnOp::Ref
                            | UnOp::RefMut
                            | UnOp::RefChecked
                            | UnOp::RefCheckedMut
                            | UnOp::RefUnsafe
                            | UnOp::RefUnsafeMut,
                        expr,
                    } => expr.as_ref(),
                    _ => arg,
                };
                if let Some(place) = self.borrow_place(place) {
                    storage.push(place);
                }
            }
            let earlier = loans.len();
            for (root, field) in storage {
                if let Some(other) = loans[..earlier].iter().find(|loan| {
                    loan.root == root
                        && (mutable || loan.mutable)
                        && overlaps(field.as_deref(), loan.field.as_deref())
                }) {
                    return Err(crate::TypeError::BorrowConflict {
                        var: root,
                        existing_borrow_span: other.span,
                        existing_is_mut: other.mutable,
                        new_borrow_span: arg.span,
                        new_is_mut: mutable,
                    });
                }
                loans.push(Loan {
                    root,
                    field,
                    mutable,
                    span: arg.span,
                });
            }
        }
        Ok(())
    }

    fn check_payload_origin_observation(&mut self, source: &RecordReceiver, read_span: verum_ast::Span) -> bool {
        let Some(write_span) = self.ctx.env.unverified_payload_write(source) else {
            return true;
        };
        let mut place = self.ctx.env.record_storage_name(source.binding).unwrap_or_else(|| "value".into());
        for field in &source.fields {
            place.push('.');
            place.push_str(field);
        }
        if !matches!(self.diagnostic_sources().last(),
            Some(crate::TypeError::UnverifiedPayloadLoan { place: p, read_span: r, write_span: w })
                if p == &place && *r == read_span && *w == write_span)
        {
            self.push_diagnostic_for(crate::TypeError::UnverifiedPayloadLoan {
                place, read_span, write_span,
            });
        }
        false
    }

    /// A single registry operation selected from its actual declaration may
    /// publish its operand roles across an import. Retain their order even
    /// when the wrapper has renamed or reordered its formal parameters.
    fn direct_slot_operation(
        &self,
        span: verum_ast::Span,
        arity: usize,
    ) -> Option<(
        &'static verum_vbc::intrinsics::registry::Intrinsic,
        List<usize>,
    )> {
        let (_, source) = self.type_registry.callable_declaration(span)?;
        if !source.contexts.is_empty()
            || source.is_async
            || source.is_generator
            || source.is_meta
            || !source.is_unsafe
            || source.extern_abi.is_some()
            || source.is_variadic
            || !source.attributes.is_empty()
            || source.std_attr.is_some()
            || source.throws_clause.is_some()
            || !source.requires.is_empty()
            || !source.ensures.is_empty()
            || source.params.len() != arity
        {
            return None;
        }
        let (operation, args) = super::callable_properties::direct_intrinsic_body(source)?;
        if args.len() != arity {
            return None;
        }
        let mut positions = List::new();
        for arg in args {
            let position = super::callable_properties::forwarded_parameter_index(source, arg)?;
            if positions.contains(&position) {
                return None;
            }
            positions.push(position);
        }
        Some((operation, positions))
    }

    fn direct_owned_slot_write(
        &self,
        span: verum_ast::Span,
        params: &[Type],
    ) -> Option<[usize; 2]> {
        let (operation, positions) = self.direct_slot_operation(span, 3)?;
        let [owner, index, payload] = positions.as_slice() else {
            return None;
        };
        let Type::Function { return_type, .. } = self.type_registry.lookup_call_target(span)?
        else {
            return None;
        };
        if self.unifier.apply(return_type) != Type::Unit {
            return None;
        }
        let element = self.owned_slot_element(
            params.get(*owner)?,
            operation.owned_slot_write_carrier()?,
            true,
            span,
        )?;
        (self.types_equivalent(&element, params.get(*payload)?)
            && self.types_equivalent(params.get(*index)?, &Type::Int))
        .then_some([*owner, *payload])
    }

    fn owned_slot_element(
        &self,
        ty: &Type,
        carrier: verum_common::well_known_types::WellKnownType,
        exclusive: bool,
        span: verum_ast::Span,
    ) -> Option<Type> {
        let Type::Reference { mutable, inner } = self.place_reference_head(ty, span).ok()? else {
            return None;
        };
        if exclusive && !mutable {
            return None;
        }
        let (name, args) = self.conversion_nominal_head(&inner)?;
        let [element] = args else { return None };
        (self.declared_runtime_storage_carrier(&name)? == carrier).then(|| element.clone())
    }

    fn require_slot_access(&self, operation: &str, span: verum_ast::Span) -> crate::Result<()> {
        if self.in_unsafe_context
            || self.source_type_registration.unsafe_function_body
                == Some(self.borrow_tracker.current_closure_id())
        {
            Ok(())
        } else {
            Err(crate::TypeError::OtherWithCodeSpanned {
                code: "E0444".into(),
                msg: format!("{operation} requires unsafe access to storage").into(),
                span,
            })
        }
    }

    pub(super) fn slot_backing_place(&mut self, owner: &Expr) -> Option<RecordReceiver> {
        self.reference_result_origin(owner).or_else(|| self.call_input_place(owner).map(|place| place.0))
    }

    pub(super) fn index_slot_snapshot(
        &mut self, backing: &RecordReceiver, element: &Type, span: verum_ast::Span,
    ) -> Option<(RecordReceiver, List<RecordReceiver>)> {
        let origins = self.stored_slot_origins(backing, element, span)?;
        Some((backing.clone(), origins))
    }

    /// Iterating an owned List or fixed array yields its stored elements.
    /// The temporary loan that keeps iteration valid belongs to the loop.
    pub(super) fn owned_iteration_item_origins(
        &mut self, source: &Expr, iterable: &Type, item: &Type,
    ) -> Option<List<RecordReceiver>> {
        use verum_common::well_known_types::{WellKnownProtocol, WellKnownType};
        let iterable = self.unifier.apply(iterable);
        let element = match &iterable {
            Type::Array { element, .. } => element.as_ref(),
            _ => {
                let (owner, [element]) = self.conversion_nominal_head(&iterable)? else { return None };
                if self.declared_runtime_storage_carrier(&owner) != Some(WellKnownType::List) {
                    return None;
                }
                // A structural iterator or a specialized implementation with
                // the same concrete Item can yield a loan of its own cursor.
                let protocol = Self::text_to_path(&self.source_operator_name(WellKnownProtocol::IntoIterator)?);
                let checker = self.protocol_checker.read();
                let (implementation, _) = checker.find_impl_with_substitution(&iterable, &protocol)?;
                let (module, _, _) = checker.method_source_for_implementation(implementation, &"into_iter".into())?;
                let (_, [Type::Var(parameter)]) = self.conversion_nominal_head(&implementation.for_type)? else { return None };
                if owner.rsplit_once('.')?.0 != module.as_str()
                    || !implementation.parameters.contains(&crate::context::GenericBinder::Type(*parameter))
                    || implementation.ordinary_associated_type(&"Item".into()).ok()?? != &Type::Var(*parameter)
                {
                    return None;
                }
                element
            }
        };
        if !self.types_equivalent(element, item) || !self.complete_slot_value_origins(source) {
            return None;
        }
        Some(self.aggregate_origins(source, &iterable))
    }

    pub(super) fn observe_indexed_slot(
        &mut self, span: verum_ast::Span, element: &Type,
        snapshot: Option<(RecordReceiver, List<RecordReceiver>)>,
    ) {
        let Some((backing, mut origins)) = snapshot else {
            self.record_owned_value_origins(span, None, false);
            return;
        };
        // Include loans a checked write installed in the same storage while
        // evaluating the key.
        let after = self.stored_slot_origins(&backing, element, span);
        if let Some(after) = &after {
            for origin in after {
                if !origins.contains(origin) { origins.push(origin.clone()); }
            }
        }
        self.record_owned_value_origins(span, Some(origins), after.is_some());
    }

    /// Read a stored value, independently of how long the reference to its
    /// backing owner lives. An unknown prior mutation keeps the observed
    /// contents unproved. Taking a slot also invalidates structural facts.
    fn slot_value_origins(
        &mut self,
        owner: &Expr,
        element: &Type,
        take: bool,
    ) -> Option<List<RecordReceiver>> {
        let backing = self.slot_backing_place(owner)?;
        if take {
            self.refinement_evidence
                .invalidate_write(std::slice::from_ref(&backing));
        }
        self.stored_slot_origins(&backing, element, owner.span)
    }

    fn stored_slot_origins(&mut self, backing: &RecordReceiver, element: &Type, span: verum_ast::Span) -> Option<List<RecordReceiver>> {
        if !self.carries_stored_loan(element) {
            return Some(List::new());
        }
        if !self.check_payload_origin_observation(backing, span)
            || !backing.fields.is_empty()
            || !self.ctx.env.storage_has_payload_origins(backing.binding)
        {
            return None;
        }
        self.ctx.env.collection_element_origins(backing)
    }

    pub(super) fn imported_slot_read_origins(
        &mut self,
        span: verum_ast::Span,
        args: &[Expr],
        params: &[Type],
        result: &Type,
    ) -> Option<List<RecordReceiver>> {
        let (operation, positions) = self.direct_slot_operation(span, 2)?;
        let (carrier, take) = operation.owned_slot_read_carrier()?;
        let [owner, index] = positions.as_slice() else {
            return None;
        };
        if args
            .iter()
            .any(|argument| self.deref_adjustments.contains_key(&argument.span))
        {
            return None;
        }
        let element = self.owned_slot_element(params.get(*owner)?, carrier, take, span)?;
        if !self.types_equivalent(&element, result)
            || !self.types_equivalent(params.get(*index)?, &Type::Int)
        {
            return None;
        }
        self.slot_value_origins(args.get(*owner)?, &element, take)
    }

    /// A registered exchange returns the old value already present in its
    /// destination. A reference in that value keeps its original referent.
    /// The mutable borrow of the slot itself lasts only through the call.
    fn exchanged_slot(
        &mut self,
        span: verum_ast::Span,
        args: &[Expr],
        params: &[Type],
        result: &Type,
    ) -> Option<(RecordReceiver, Type, Option<usize>)> {
        let (slot, replacement) = self.exchange_parameters(span)?;
        if args.len() != params.len() || args.iter().any(|arg| self.deref_adjustments.contains_key(&arg.span)) {
            return None;
        }
        let Type::Reference { mutable: true, inner } = self.place_reference_head(params.get(slot)?, span).ok()?
            else { return None; };
        if !self.types_equivalent(&inner, result) || inner.contains_type(|ty|
            matches!(ty, Type::Unknown | Type::Placeholder { .. }))
        {
            return None;
        }
        let argument = unparen(args.get(slot)?);
        let origin = if let ExprKind::Unary { op: UnOp::RefMut, expr } = &argument.kind {
            let place = unparen(expr);
            match &place.kind {
                ExprKind::Path(path) => self.ctx.env.record_source(path.as_ident()?.as_str(), false)?,
                ExprKind::Field { expr: owner, field } => {
                    let (mut origin, ty) = self.call_input_place(owner)?;
                    let field_ty = self.lookup_field_type(&self.record_conversion_head(&ty), &field.name)?;
                    let field_ty = self.unifier.try_apply(&field_ty, field.span).ok()?;
                    if !self.types_equivalent(&field_ty, &inner) { return None; }
                    origin.fields.push(field.name.clone());
                    origin
                }
                _ => return None,
            }
        } else {
            self.reference_result_origin(argument)?
        };
        if !self.ctx.env.storage_has_payload_origins(origin.binding)
            || !self.check_payload_origin_observation(&origin, argument.span)
        {
            return None;
        }
        Some((origin, *inner, replacement))
    }

    pub(super) fn observe_value_exchange(
        &mut self, span: verum_ast::Span, args: &[Expr], params: &[Type], result: &Type,
    ) -> crate::Result<Option<bool>> {
        let Some((destination, element, replacement)) = self.exchanged_slot(span, args, params, result) else {
            return Ok(None);
        };
        let Some(old) = self.ctx.env.referent_value_origins(&destination) else { return Ok(None) };
        // Capture the old contents before retaining any loan in the new value.
        self.record_owned_value_origins(span, Some(old), true);
        let accounted_for = match replacement.and_then(|index| args.get(index)) {
            Some(value) => self.retain_slot_payload_at(destination, value, &element, span)?,
            None => false,
        };
        Ok(Some(accounted_for))
    }

    pub(super) fn check_slot_read(
        &mut self,
        operation: &'static verum_vbc::intrinsics::registry::Intrinsic,
        carrier: verum_common::well_known_types::WellKnownType,
        take: bool,
        args: &[Expr],
        expression: &Expr,
    ) -> crate::Result<Type> {
        let span = expression.span;
        let [_, owner, index] = args else {
            return Err(crate::TypeError::wrong_arg_count(
                operation.name.into(),
                2,
                args.len().saturating_sub(1),
                span,
            ));
        };
        self.require_slot_access(operation.name, span)?;
        self.with_call_place_types(expression, &args[1..], |checker| {
            let owner_type = checker.synth_expr(owner)?.ty;
            let element = checker
                .owned_slot_element(&owner_type, carrier, take, owner.span)
                .ok_or_else(|| crate::TypeError::OtherWithCodeSpanned {
                    code: "E0444".into(),
                    msg: format!(
                        "{} requires {}reference to its declared {} storage",
                        operation.name,
                        if take { "an exclusive " } else { "a " },
                        carrier.as_str()
                    )
                    .into(),
                    span: owner.span,
                })?;
            checker.check_expr(index, &Type::Int)?;
            let parameters = [owner_type, Type::Int];
            checker.check_call_loans(&args[1..], Some(&parameters), None)?;
            if let Some(origins) = checker.slot_value_origins(owner, &element, take) {
                checker.record_owned_value_origins(span, Some(origins), true);
            } else {
                checker.observe_call_result_origins(
                    span,
                    &element,
                    &args[1..],
                    Some(&parameters),
                    None,
                    None,
                    None,
                )?;
            }
            Ok(element)
        })
    }

    pub(super) fn check_slot_intrinsic(
        &mut self,
        operation: &'static verum_vbc::intrinsics::registry::Intrinsic,
        carrier: verum_common::well_known_types::WellKnownType,
        args: &[Expr],
        expression: &Expr,
    ) -> crate::Result<Type> {
        let span = expression.span;
        let [_, owner, index, value] = args else {
            return Err(crate::TypeError::wrong_arg_count(
                operation.name.into(),
                3,
                args.len().saturating_sub(1),
                span,
            ));
        };
        self.require_slot_access(operation.name, span)?;
        let operands = &args[1..];
        self.with_call_place_types(expression, operands, |checker| {
            let owner_type = checker.synth_expr(owner)?.ty;
            let element = checker
                .owned_slot_element(&owner_type, carrier, true, owner.span)
                .ok_or_else(|| crate::TypeError::OtherWithCodeSpanned {
                    code: "E0444".into(),
                    msg: format!(
                        "{} requires an exclusive reference to its declared {} storage",
                        operation.name,
                        carrier.as_str()
                    )
                    .into(),
                    span: owner.span,
                })?;
            checker.check_expr(index, &Type::Int)?;
            checker.check_expr(value, &element)?;
            let parameters = [owner_type, Type::Int, element];
            checker.check_call_loans(operands, Some(&parameters), None)?;
            checker.consume_affine_call_args(operands, &parameters)?;
            if !checker.retain_slot_payload(owner, value, &parameters[2], span)? {
                checker.retain_possible_call_writes(operands, Some(&parameters), None, None)?;
            }
            Ok(Type::Unit)
        })
    }

    pub(super) fn retain_known_slot_write(
        &mut self,
        span: verum_ast::Span,
        args: &[Expr],
        parameters: &[Type],
    ) -> crate::Result<bool> {
        let Some([owner, payload]) = self.direct_owned_slot_write(span, parameters) else {
            return Ok(false);
        };
        if args
            .iter()
            .any(|argument| self.deref_adjustments.contains_key(&argument.span))
        {
            return Ok(false);
        }
        let [Some(owner), Some(value)] = [args.get(owner), args.get(payload)] else {
            return Ok(false);
        };
        self.retain_slot_payload(owner, value, &parameters[payload], span)
    }

    /// A dynamically intercepted method is selected by its nominal runtime
    /// owner. A source extension, protocol member, or signature without the
    /// declared element operand cannot grant the same write contract.
    pub(super) fn retain_runtime_append(
        &mut self,
        span: verum_ast::Span,
        receiver: &Expr,
        method: &verum_ast::Ident,
        args: &[Expr],
    ) -> crate::Result<bool> {
        let [value] = args else { return Ok(false) };
        if self.deref_adjustments.contains_key(&value.span) {
            return Ok(false);
        }
        let Some(Type::Reference { mutable: true, inner: declared_receiver }) =
            self.type_registry.declared_receiver(span)
        else { return Ok(false) };
        let Some((owner, [element])) = self.conversion_nominal_head(declared_receiver) else { return Ok(false) };
        let Some(carrier) = self.declared_runtime_storage_carrier(&owner) else { return Ok(false) };
        if !verum_vbc::intrinsics::registry::appends_owned_element(carrier, method.as_str(), args.len()) {
            return Ok(false);
        }
        let Some((module, source)) = self.type_registry.callable_declaration(span) else { return Ok(false) };
        let Some((type_module, _)) = owner.rsplit_once('.') else { return Ok(false) };
        let Some(verum_ast::expr::ResolvedCallTarget::StaticCall {
            source_method: Some((selected_owner, selected_span)), ..
        }) = self.resolved_call_targets.get(&span) else { return Ok(false) };
        if selected_owner != module || *selected_span != source.span
            || module.as_str() != type_module || source.name.name != method.name
            || source.is_async || source.is_generator || source.is_meta
            || !source.attributes.is_empty() || source.std_attr.is_some() || source.throws_clause.is_some()
            || !source.requires.is_empty() || !source.ensures.is_empty() || !source.generics.is_empty()
            || !matches!(source.params.as_slice(), [first, second]
                if matches!(first.kind, verum_ast::FunctionParamKind::SelfRefMut)
                    && first.attributes.is_empty() && second.attributes.is_empty()
                    && matches!(second.kind, verum_ast::FunctionParamKind::Regular { default_value: None, .. }))
        {
            return Ok(false);
        }
        let Some(Type::Function { params, return_type, .. }) = self.type_registry.lookup_call_target(span) else { return Ok(false) };
        let [parameter] = params.as_slice() else { return Ok(false) };
        if !self.types_equivalent(element, parameter) || self.unifier.apply(return_type) != Type::Unit {
            return Ok(false);
        }
        let element = element.clone();
        self.retain_slot_payload(receiver, value, &element, span)
    }

    fn retain_slot_payload(
        &mut self,
        owner: &Expr,
        value: &Expr,
        element: &Type,
        span: verum_ast::Span,
    ) -> crate::Result<bool> {
        let Some(destination) = self
            .reference_result_origin(owner)
            .or_else(|| self.call_input_place(owner).map(|place| place.0))
        else {
            return Ok(false);
        };
        self.retain_slot_payload_at(destination, value, element, span)
    }

    fn retain_slot_payload_at(
        &mut self, destination: RecordReceiver, value: &Expr, element: &Type, span: verum_ast::Span,
    ) -> crate::Result<bool> {
        // Installing a slot does not read any other slot. An earlier unknown
        // write keeps its evidence boundary until the contents are observed.
        if self.carries_stored_loan(element)
            && !self
                .ctx
                .env
                .storage_has_payload_origins(destination.binding)
        {
            return Ok(false);
        }
        let incoming = if self.carries_stored_loan(element) {
            let Some(origins) = self.observed_operand_origins(value) else {
                return Ok(false);
            };
            // A missing proof of generic payload origins cannot license a
            // previously unknown write simply because no loan was observed.
            if !self.observed_known_owned_payload(value) {
                return Ok(false);
            }
            origins
        } else {
            List::new()
        };
        let Some(region) = self.ctx.env.storage_loan_region(destination.binding) else {
            return Ok(false);
        };
        self.check_input_storage_retention(&incoming, &destination, span)?;
        self.retain_origins_in_region(&incoming, region, span)?;
        self.refinement_evidence
            .invalidate_write(std::slice::from_ref(&destination));
        Ok(self
            .ctx
            .env
            .insert_owned_slot_origins(&destination, &incoming))
    }

    /// An unknown callee can store an incoming loan in each writable carrier.
    /// Retain that loan in the carrier's region before allowing later reads to
    /// reuse its payload facts. Scalar-only destinations cannot retain loans.
    pub(super) fn retain_possible_call_writes(
        &mut self,
        args: &[Expr],
        params: Option<&[Type]>,
        callable: Option<&Expr>,
        receiver: Option<(&Expr, bool, Option<&Type>)>,
    ) -> crate::Result<()> {
        let mut sources = List::new();
        let mut destinations = List::new();
        let mut unknown_write = None;
        let mut unknown_raw_write = None;
        for (argument, receiver_mode, declared, is_callee) in receiver.into_iter()
            .map(|(value, mode, ty)| (value, Some(mode), ty, false))
            .chain(callable.into_iter().map(|value| (value, None, None, true)))
            .chain(args.iter().enumerate().map(|(index, value)|
                (value, None, params.and_then(|params| params.get(index)), false)))
        {
            let observed = self.observed_operand_type(argument)
                .or_else(|| self.type_registry.lookup_expr(argument.span).cloned());
            let Some(actual) = observed.as_ref().or(declared) else {
                if receiver_mode == Some(true) {
                    return Err(crate::TypeError::UnknownRecordOrigin { span: argument.span });
                }
                continue;
            };
            let selected = declared.unwrap_or(actual);
            let head = self.place_reference_head(selected, argument.span)?;
            if let Type::Pointer { mutable: true, inner: element }
                | Type::VolatilePointer { mutable: true, inner: element } = &head
            {
                let targets = self.observed_raw_address(argument);
                if !self.refinement_evidence.is_empty() {
                    self.refinement_evidence.invalidate_write(targets.as_deref().unwrap_or(&[]));
                }
                let carries = self.carries_stored_loan(element);
                if let Some(targets) = targets {
                    let retains_shared = carries && self.may_retain_shared_loan(element);
                    for target in targets {
                        if !sources.contains(&(target.clone(), true)) { sources.push((target.clone(), true)); }
                        if carries { destinations.push((target, argument.span, retains_shared, false)); }
                    }
                } else if carries {
                    unknown_raw_write = Some(argument.span);
                }
                continue;
            }
            let (mode, payload) = match &head {
                Type::Reference { mutable, inner }
                | Type::CheckedReference { mutable, inner }
                | Type::UnsafeReference { mutable, inner } => {
                    // A selected &self method reborrows an exclusive receiver
                    // as shared. The actual variable's stronger permission is
                    // not permission exercised by this invocation.
                    (receiver_mode.or(Some(*mutable)), inner.as_ref())
                }
                _ => (receiver_mode, &head),
            };
            if mode.is_none() && !self.carries_stored_loan(&head) { continue; }
            let mut origins = List::new();
            if mode.is_some() && self.reference_mode(actual, argument.span)?.is_none() {
                if let Some((source, _)) = self.record_place_for_auto_borrow(argument, AutoBorrowRoot::IncludeExclusive) {
                    origins.push(source);
                } else if let Some(candidates) = self.computed_field_borrow_origins(argument) {
                    origins = candidates;
                }
            } else {
                self.collect_evaluated_operand_origins(argument, &mut origins);
                if mode.is_none() && self.reference_mode(actual, argument.span)?.is_some() {
                    if let Some((_, payloads)) = self.projected_origins_for_type(
                        actual.clone(), origins.clone(), argument.span,
                    ) {
                        origins = payloads;
                    }
                }
            }
            // Classify the permission once per operand, not once per origin
            // pair. A shared reborrow never grants exclusive retention.
            let writable_source = mode.unwrap_or_else(|| self.may_store_writable_reference(&head));
            for origin in &origins {
                let source = (origin.clone(), writable_source);
                if !sources.contains(&source) { sources.push(source); }
            }
            let writes_payload = mode == Some(true) && self.carries_stored_loan(payload);
            let writes_through_value = mode.is_none() && writable_source
                && !self.callable_has_closed_environment(argument);
            // Even a scalar referent can change value, although it cannot
            // retain a loan. An opaque closure may mutate captured storage.
            if (mode == Some(true) || writes_through_value) && !self.refinement_evidence.is_empty() {
                if origins.iter().any(|source| self.ctx.env.record_storage_name(source.binding).is_none()) {
                    self.refinement_evidence.invalidate_write(&[]);
                } else {
                    self.refinement_evidence.invalidate_write(&origins);
                }
            }
            if writes_payload || writes_through_value {
                // A known destination needs one bounded layout traversal.
                // An opaque captured destination remains conservative.
                let retains_shared = !writes_payload || self.may_retain_shared_loan(payload);
                if origins.is_empty() {
                    unknown_write = Some(argument.span);
                    // The argument's own opaque payload was actually passed.
                    // It may share writable backing storage after ordinary
                    // value-copy. A callable's environment is separate from
                    // the slot holding the callable value, though: invoking or
                    // passing it does not rewrite that producer binding.
                    let callable_value = |ty: &Type| match ty {
                        Type::Function { .. } => true,
                        Type::Var(var) => self.unifier.rigid_fn_bound_ref(*var).is_some(),
                        _ => false,
                    };
                    if !is_callee && !callable_value(payload) && !callable_value(actual)
                        && let Some((source, _)) = self.call_input_place(argument)
                    {
                        self.ctx.env.invalidate_payload_origins(&source, &[], argument.span);
                    }
                }
                // Only a typed, inline place proves that sibling fields do not
                // overlap this write. Dynamic indexing, raw pointers and unknown
                // captured or reachable storage keep their enclosing write set.
                let inline = if writes_payload {
                    let value = match &unparen(argument).kind {
                        ExprKind::Unary { op: UnOp::RefMut | UnOp::RefCheckedMut, expr } => expr,
                        _ => argument,
                    };
                    self.call_input_place(value).map(|(place, _)| place)
                } else { None };
                for origin in origins {
                    let exact = inline.as_ref() == Some(&origin);
                    destinations.push((origin, argument.span, retains_shared, exact));
                }
            }
        }
        if let Some(span) = unknown_write {
            self.ctx.env.invalidate_unknown_referent_write(span);
        }
        if let Some(span) = unknown_raw_write {
            for (origin, _) in &sources {
                if let Some(var) = self.ctx.env.dropping_loan_storage(origin.binding, true, self.closure_return_depth) {
                    return Err(crate::TypeError::DanglingReference { var, ref_span: span, drop_span: None });
                }
            }
            self.ctx.env.invalidate_unknown_raw_write(span);
        }
        for (destination, span, retains_shared, inline) in destinations {
            let fields = if inline { destination.fields.as_slice() } else { &[] };
            self.ctx.env.invalidate_payload_origins(&destination, fields, span);
            let incoming: List<_> = sources.iter()
                .filter(|(source, writable)| source.binding != destination.binding
                    && (*writable || retains_shared))
                .map(|(source, _)| source.clone()).collect();
            if incoming.is_empty() { continue; }
            self.check_input_storage_retention(&incoming, &destination, span)?;
            let region = self.ctx.env.storage_loan_region(destination.binding)
                .ok_or(crate::TypeError::UnknownRecordOrigin { span })?;
            self.retain_origins_in_region(&incoming, region, span)?;
        }
        Ok(())
    }

    fn argument_reference_mutability(&mut self, expr: &Expr) -> Option<bool> {
        match &unparen(expr).kind {
            ExprKind::Unary {
                op: UnOp::Ref | UnOp::RefChecked | UnOp::RefUnsafe,
                ..
            } => Some(false),
            ExprKind::Unary {
                op: UnOp::RefMut | UnOp::RefCheckedMut | UnOp::RefUnsafeMut,
                ..
            } => Some(true),
            ExprKind::Path(path) => {
                reference_mutability(&self.ctx.env.lookup(&path.as_ident()?.name)?.ty)
            }
            ExprKind::NamedArg { value, .. } => self.argument_reference_mutability(value),
            ExprKind::Call { func, args, .. } => {
                let ExprKind::Path(path) = &unparen(func).kind else {
                    return None;
                };
                let scheme = self.callee_scheme_with_mount_authority(&self.path_to_string(path))?;
                let Type::Function { return_type, .. } = &scheme.ty else {
                    return None;
                };
                reference_mutability(return_type).or_else(|| {
                    self.argument_reference_mutability(args.get(scheme.returned_value.as_ref()?.forwarded_parameter()?)?)
                })
            }
            _ => None,
        }
    }

    pub(super) fn borrowed_record_place(&mut self, expr: &Expr) -> Option<(RecordReceiver, Type)> {
        self.record_place_for_auto_borrow(expr, AutoBorrowRoot::SharedOnly)
    }

    pub(super) fn call_input_place(&mut self, expr: &Expr) -> Option<(RecordReceiver, Type)> {
        self.record_place_for_auto_borrow(expr, AutoBorrowRoot::IncludeExclusive)
    }

    /// A call borrowing an indexed element can return a reference into its
    /// container. The container is a lifetime bound even when the selected
    /// index is dynamic; it does not identify an exact field for a later write.
    fn borrowed_call_input_origins(&mut self, expr: &Expr, access: AutoBorrowRoot) -> Option<List<RecordReceiver>> {
        self.record_place_for_auto_borrow(expr, access).map(|(place, _)| List::from_single(place))
            .or_else(|| self.observed_indexed_owners(expr))
            .or_else(|| self.indexed_container_origins(expr))
    }

    pub(super) fn indexed_container_origins(&mut self, expr: &Expr) -> Option<List<RecordReceiver>> {
        let (indexed, _) = field_root(expr);
        let ExprKind::Index { expr: owner, index } = &indexed.kind else { return None };
        if matches!(unparen(index).kind, ExprKind::Range { .. }) { return None; }
        let place = self.call_input_place(owner);
        let ty = place.as_ref().map(|(_, ty)| ty.clone())
            .or_else(|| self.type_registry.lookup_expr(owner.span).cloned())?;
        let head = self.place_reference_head(&ty, owner.span).ok()?;
        let storage = head.reference_parts().map_or(&head, |(inner, _, _)| inner);
        if storage.sequence_domain().is_none() && self.declared_list_element(storage).is_none() {
            return None;
        }
        if let Some((place, _)) = place { return Some(List::from_single(place)); }
        if head.reference_parts().is_some() {
            let origins = self.aggregate_origins(owner, &ty);
            if !origins.is_empty() { return Some(origins); }
        } else if let Some(origins) = self.observed_indexed_owners(owner) {
            return Some(origins);
        }
        // An owned computed container is live while its element is inspected.
        // No caller storage can extend the lifetime of that temporary.
        if !self.ctx.env.binds_in_current_scope(INDEXED_TEMPORARY) {
            self.ctx.env.insert_mono(INDEXED_TEMPORARY, ty);
            self.ctx.env.set_binding_loan_region(INDEXED_TEMPORARY, self.borrow_tracker.scope_depth() + 1);
        }
        self.ctx.env.record_source(INDEXED_TEMPORARY, false).map(List::from_single)
    }

    fn check_temporary_binding(&mut self, sources: &[RecordReceiver], span: verum_ast::Span) -> crate::Result<()> {
        let region = self.borrow_tracker.scope_depth();
        for source in sources {
            if self.ctx.env.storage_loan_region(source.binding).is_some_and(|origin| origin > region)
                && self.ctx.env.record_storage_name(source.binding).as_deref() == Some(INDEXED_TEMPORARY)
            {
                return Err(crate::TypeError::DanglingReference { var: INDEXED_TEMPORARY.into(), ref_span: span, drop_span: None });
            }
        }
        Ok(())
    }

    pub(super) fn check_indexed_temporary_result(&mut self, expr: &Expr, ty: &Type) -> crate::Result<()> {
        if self.ctx.env.binds_in_current_scope(INDEXED_TEMPORARY)
            && let Some(temporary) = self.ctx.env.record_source(INDEXED_TEMPORARY, false)
            && self.aggregate_origins(expr, ty).iter().any(|source| source.binding == temporary.binding)
        {
            return Err(crate::TypeError::DanglingReference {
                var: INDEXED_TEMPORARY.into(), ref_span: self.escaping_operand_span(expr, &temporary), drop_span: None,
            });
        }
        Ok(())
    }

    /// A computed reference can designate several callers' places. A write
    /// through an inline field can reach any of them. Keep each candidate
    /// without promoting the set to a unique storage identity.
    fn computed_field_borrow_origins(&mut self, place: &Expr) -> Option<List<RecordReceiver>> {
        let (root, fields) = field_root(place);
        if fields.is_empty() { return None; }
        let actual = self.observed_operand_type(root)
            .or_else(|| self.type_registry.lookup_expr(root.span).cloned())?;
        let ty = match self.place_reference_head(&actual, root.span).ok()? {
            Type::Reference { inner, .. } | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => *inner,
            _ => return None,
        };
        if self.reference_mode(&ty, root.span).ok()?.is_some() { return None; }
        let mut origins = self.evaluated_result_origins(root)?.clone();
        self.project_inline_fields(ty, fields, &mut origins)?;
        Some(origins)
    }

    /// An implicit borrow of an inline field names its original storage even
    /// when the root is an exclusive reference. This is location evidence,
    /// not write permission; the caller still checks the selected receiver.
    /// Reference-valued fields and computed roots need their own derivation.
    fn record_place_for_auto_borrow(
        &mut self,
        expr: &Expr,
        access: AutoBorrowRoot,
    ) -> Option<(RecordReceiver, Type)> {
        let (root, fields) = field_root(expr);
        let name = lexical_place_root(root)?;
        let ty = self.ctx.env.lookup(name)?.ty.clone();
        let head = self.place_reference_head(&ty, root.span).ok()?;
        if access == AutoBorrowRoot::SharedOnly && name == "self" && head.reference_parts().is_some() {
            return None;
        }
        let reference = match &head {
            Type::Reference { mutable: false, .. } => true,
            Type::Reference { mutable: true, inner }
            | Type::CheckedReference { mutable: true, inner }
            | Type::UnsafeReference { mutable: true, inner }
                if access == AutoBorrowRoot::IncludeExclusive =>
            {
                // The reference slot is not the storage behind another
                // reference. Do not invent the second indirection's identity.
                if self.reference_mode(inner, root.span).ok()?.is_some() {
                    return None;
                }
                true
            }
            Type::Reference { .. }
            | Type::CheckedReference { .. }
            | Type::UnsafeReference { .. } => return None,
            _ => false,
        };
        let mut owner = self.ctx.env.record_source(name, reference)?;
        let ty = self.project_inline_fields(ty, fields, std::slice::from_mut(&mut owner))?;
        Some((owner, ty))
    }

    fn project_inline_fields(
        &self,
        mut ty: Type,
        fields: List<&verum_ast::Ident>,
        origins: &mut [RecordReceiver],
    ) -> Option<Type> {
        for field in fields.into_iter().rev() {
            ty = self.lookup_field_type(&self.record_conversion_head(&ty), &field.name)?;
            if self.reference_mode(&ty, field.span).ok()?.is_some() {
                return None;
            }
            for origin in &mut *origins { origin.fields.push(field.name.clone()); }
        }
        Some(ty)
    }
}


#[cfg(test)]
#[path = "../../tests/unit/reference_origin.rs"]
mod tests;
