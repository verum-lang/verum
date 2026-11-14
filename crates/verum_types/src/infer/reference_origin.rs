//! A reference result can identify a carrier only with a derivation for its
//! referent. Returning the same reference type does not prove pointer identity.
use super::TypeChecker;
use crate::context::ReferenceOrigin;
use crate::ty::{RecordReceiver, Type};
use verum_common::List;
use verum_ast::{
    Expr, ExprKind, StmtKind, UnOp,
};

fn unparen(mut expr: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expr.kind {
        expr = inner;
    }
    expr
}

#[derive(Clone, Copy, PartialEq)]
enum AutoBorrowRoot {
    SharedOnly,
    IncludeExclusive,
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
    ) -> crate::Result<()> {
        let direct = match &unparen(destination).kind {
            ExprKind::Path(path) => path.as_ident().map(|name| name.name.clone()),
            _ => None,
        };
        if origins.is_empty() {
            if let Some(name) = direct { self.ctx.env.replace_current_binding_origins(&name, origins); }
            return Ok(());
        }
        let (holder, _) = self.borrow_place(destination)
            .ok_or(crate::TypeError::UnknownRecordOrigin { span: destination.span })?;
        let region = self.ctx.env.binding_loan_region(&holder)
            .ok_or(crate::TypeError::UnknownRecordOrigin { span: destination.span })?;
        self.retain_origins_in_region(origins, region, destination.span)?;
        if !direct.is_some_and(|name| self.ctx.env.replace_current_binding_origins(&name, origins)) {
            self.ctx.env.assign_reference_origins(&holder, origins);
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
                    var: root, ref_span: span, drop_span: span,
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
        match (&unparen(value).kind, ty) {
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
        if self.reference_mode(ty, value.span)?.is_some() {
            return Ok(None);
        }
        let sources = self.aggregate_origins(value, ty);
        for source in &sources {
            self.retain_origin(source, None, value.span)?;
        }
        Ok((!sources.is_empty()).then(|| ReferenceOrigin::Candidates(sources.into_boxed_slice())))
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
                .dropping_loan_storage(source.binding, returning)
            {
                return Err(crate::TypeError::DanglingReference {
                    var,
                    ref_span: expr.span,
                    drop_span: expr.span,
                });
            }
        }
        Ok(())
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
            if let Some(previous) = self.match_arm_origins.get_mut(&body.span) {
                *previous = None;
            }
            return;
        }
        let origins = self.match_value_origins(body);
        self.match_arm_origins.entry(body.span).and_modify(|previous| {
            match (previous.as_mut(), origins.as_ref()) {
                (Some(known), Some(observed)) => {
                    for origin in observed {
                        if !known.contains(origin) {
                            known.push(origin.clone());
                        }
                    }
                }
                _ => *previous = None,
            }
        }).or_insert(origins);
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
                    if let Some(sources) = self.projected_value_origins(expr) {
                        for source in sources {
                            if !origins.contains(&source) {
                                origins.push(source);
                            }
                        }
                    } else {
                        pending.push(expr);
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
        if let Some(source) = self.reference_result_origin(value) {
            add(found, source);
            return;
        }
        match &unparen(value).kind {
            ExprKind::Path(path) => {
                if let Some(name) = path.as_ident() {
                    if let Some(place) = self.ctx.env.record_source(&name.name, false) {
                        self.check_payload_origin_observation(&place);
                    }
                    for origin in self.ctx.env.reference_candidates(&name.name) {
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
                if let Some(origins) = self.projected_value_origins(expr) {
                    for origin in origins {
                        add(found, origin);
                    }
                } else {
                    self.collect_reference_candidates(expr, found);
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
            ExprKind::Call { args, .. } => {
                if let Some(origins) = self.returned_value_origins(value) {
                    for origin in origins { add(found, origin); }
                    return;
                }
                for arg in args {
                    self.collect_reference_candidates(arg, found);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                if let Some(origins) = self.returned_value_origins(value) {
                    for origin in origins { add(found, origin); }
                    return;
                }
                // A consumed receiver passes on any loans in its value. Only
                // a borrowed (or unresolved) receiver also lends its own slot.
                if !self.type_registry.consumes_receiver(unparen(value).span)
                    && let Some((source, _)) = self.borrowed_record_place(receiver)
                {
                    add(found, source);
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
            ExprKind::Block(block) | ExprKind::Async(block) => {
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
                    if let Some(origins) = self.match_arm_origins.get(&arm.body.span).and_then(Option::as_ref) {
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
    fn projected_value_origins(&mut self, receiver: &Expr) -> Option<List<RecordReceiver>> {
        let receiver = unparen(receiver);
        let ty = self.observed_operand_type(receiver).or_else(|| {
            self.type_registry.lookup_expr(receiver.span).cloned()
        })?;
        let mut origins = List::new();
        self.collect_reference_candidates(receiver, &mut origins);
        self.projected_origins_for_type(ty, origins, receiver.span).map(|(_, origins)| origins)
    }

    pub(super) fn projected_origins_for_type(
        &mut self,
        ty: Type,
        origins: List<RecordReceiver>,
        span: verum_ast::Span,
    ) -> Option<(Type, List<RecordReceiver>)> {
        self.read_value_origins(ty, origins, span, false)
    }

    fn read_value_origins(
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
                if !self.check_payload_origin_observation(&origin) { return None; }
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
            let slot = if !referents.is_empty() && self.borrows_referent_field(expr) {
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
                let index = self.type_registry.returned_value(expr.span)?.forwarded_parameter()?;
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
        may_write_inputs: bool,
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
                self.collect_reference_candidates(arg, &mut origins);
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
        if may_write_inputs {
            self.retain_possible_call_writes(args, params, receiver)?;
        }
        Ok(())
    }

    fn check_payload_origin_observation(&mut self, source: &RecordReceiver) -> bool {
        let Some(span) = self.ctx.env.unverified_payload_write(source.binding) else {
            return true;
        };
        let name = self.ctx.env.record_storage_name(source.binding).unwrap_or_else(|| "value".into());
        self.push_diagnostic_for(crate::TypeError::OtherWithCodeSpanned {
            code: "E312".into(),
            msg: format!("cannot establish the lifetime of references stored in `{name}` after this mutable call").into(),
            span,
        });
        false
    }

    /// An unknown callee can store an incoming loan in each writable carrier.
    /// Retain that loan in the carrier's region before allowing later reads to
    /// reuse its payload facts. Scalar-only destinations cannot retain loans.
    fn retain_possible_call_writes(
        &mut self,
        args: &[Expr],
        params: Option<&[Type]>,
        receiver: Option<(&Expr, bool, Option<&Type>)>,
    ) -> crate::Result<()> {
        let mut sources = List::new();
        let mut destinations = List::new();
        let mut unknown_write = None;
        for (argument, receiver_mode, declared) in receiver.into_iter()
            .map(|(value, mode, ty)| (value, Some(mode), ty))
            .chain(args.iter().enumerate().map(|(index, value)|
                (value, None, params.and_then(|params| params.get(index)))))
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
            let (mode, payload) = match &head {
                Type::Reference { mutable, inner }
                | Type::CheckedReference { mutable, inner }
                | Type::UnsafeReference { mutable, inner } => (Some(*mutable), inner.as_ref()),
                _ => (receiver_mode, &head),
            };
            if mode.is_none() && !self.carries_stored_loan(&head) { continue; }
            let mut origins = List::new();
            if mode.is_some() && self.reference_mode(actual, argument.span)?.is_none() {
                if let Some((source, _)) = self.record_place_for_auto_borrow(argument, AutoBorrowRoot::IncludeExclusive) {
                    origins.push(source);
                }
            } else {
                self.collect_reference_candidates(argument, &mut origins);
                if mode.is_none() && self.reference_mode(actual, argument.span)?.is_some() {
                    if let Some((_, payloads)) = self.projected_origins_for_type(
                        actual.clone(), origins.clone(), argument.span,
                    ) {
                        origins = payloads;
                    }
                }
            }
            for origin in &origins {
                if !sources.contains(origin) { sources.push(origin.clone()); }
            }
            let writes_payload = mode == Some(true) && self.carries_stored_loan(payload);
            let writes_through_value = mode.is_none()
                && self.may_store_writable_reference(payload)
                && !self.callable_has_closed_environment(argument);
            if writes_payload || writes_through_value {
                if origins.is_empty() {
                    unknown_write = Some(argument.span);
                }
                for origin in origins {
                    destinations.push((origin, argument.span));
                }
            }
        }
        if let Some(span) = unknown_write {
            self.ctx.env.invalidate_unknown_payload_write(span);
        }
        for (destination, span) in destinations {
            self.ctx.env.invalidate_payload_origins(&destination, span);
            let incoming: List<_> = sources.iter()
                .filter(|source| source.binding != destination.binding).cloned().collect();
            if incoming.is_empty() { continue; }
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

    /// The roots of an established field projection are the possible referents
    /// of its receiver. A computed place with no such evidence keeps its slot.
    fn borrows_referent_field(&self, place: &Expr) -> bool {
        let ExprKind::Field { expr, field } = &unparen(place).kind else {
            return false;
        };
        let ExprKind::Path(path) = &unparen(expr).kind else {
            return false;
        };
        let Some(binding) = path.as_ident().and_then(|name| self.ctx.env.lookup(&name.name)) else {
            return false;
        };
        self.reference_mode(&binding.ty, expr.span).is_ok_and(|mode| mode.is_some())
            && self.lookup_field_type(&self.record_conversion_head(&binding.ty), &field.name).is_some()
    }

    pub(super) fn borrowed_record_place(&mut self, expr: &Expr) -> Option<(RecordReceiver, Type)> {
        self.record_place_for_auto_borrow(expr, AutoBorrowRoot::SharedOnly)
    }

    pub(super) fn call_input_place(&mut self, expr: &Expr) -> Option<(RecordReceiver, Type)> {
        self.record_place_for_auto_borrow(expr, AutoBorrowRoot::IncludeExclusive)
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
        let mut fields = List::new();
        let mut root = unparen(expr);
        while let ExprKind::Field { expr, field } = &root.kind {
            fields.push(field);
            root = unparen(expr);
        }
        let ExprKind::Path(path) = &root.kind else { return None };
        let name = match path.segments.as_slice() {
            [verum_ast::ty::PathSegment::Name(name)] => name.name.as_str(),
            [verum_ast::ty::PathSegment::SelfValue]
                if access == AutoBorrowRoot::IncludeExclusive => "self",
            _ => return None,
        };
        let mut ty = self.ctx.env.lookup(name)?.ty.clone();
        let head = self.place_reference_head(&ty, root.span).ok()?;
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
        for field in fields.into_iter().rev() {
            ty = self.lookup_field_type(&self.record_conversion_head(&ty), &field.name)?;
            if self.reference_mode(&ty, field.span).ok()?.is_some() {
                return None;
            }
            owner.fields.push(field.name.clone());
        }
        Some((owner, ty))
    }
}


#[cfg(test)]
#[path = "../../tests/unit/reference_origin.rs"]
mod tests;
