//! A reference result can identify a carrier only with a derivation for its
//! referent. Returning the same reference type does not prove pointer identity.
use super::TypeChecker;
use crate::context::ReferenceOrigin;
use crate::ty::{RecordReceiver, Type};
use verum_ast::{
    Expr, ExprKind, FunctionBody, FunctionDecl, FunctionParamKind, PatternKind, StmtKind, UnOp,
};

fn unparen(mut expr: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expr.kind {
        expr = inner;
    }
    expr
}

fn reference_mutability(ty: &Type) -> Option<bool> {
    match ty {
        Type::Reference { mutable, .. }
        | Type::CheckedReference { mutable, .. }
        | Type::UnsafeReference { mutable, .. } => Some(*mutable),
        _ => None,
    }
}

/// A body which returns a parameter unchanged preserves that operand's origin.
/// Signatures, dynamic calls and bodies with other control flow carry no such rule.
pub(super) fn forwarded_parameter(decl: &FunctionDecl) -> Option<usize> {
    fn name(expr: &Expr) -> Option<&verum_common::Text> {
        let ExprKind::Path(path) = &unparen(expr).kind else {
            return None;
        };
        Some(&path.as_ident()?.name)
    }
    let (statements, value) = match decl.body.as_ref()? {
        FunctionBody::Block(block) => match block.expr.as_deref() {
            Some(value) => (block.stmts.as_slice(), value),
            None => {
                let (last, prefix) = block.stmts.split_last()?;
                let StmtKind::Expr { expr, .. } = &last.kind else {
                    return None;
                };
                let ExprKind::Return(Some(value)) = &expr.kind else {
                    return None;
                };
                (prefix, value.as_ref())
            }
        },
        FunctionBody::Expr(expr) => (&[][..], expr),
    };
    let mut result = name(value)?;
    for stmt in statements.iter().rev() {
        if !stmt.attributes.is_empty() {
            return None;
        }
        let StmtKind::Let {
            pattern,
            ty: None,
            value: Some(value),
        } = &stmt.kind
        else {
            return None;
        };
        let PatternKind::Ident {
            name: binding,
            mutable: false,
            by_ref: false,
            subpattern: None,
        } = &pattern.kind
        else {
            return None;
        };
        let source = name(value)?;
        if result == &binding.name {
            result = source;
        }
    }
    let mut forwarded = None;
    for (index, param) in decl.params.iter().enumerate() {
        let FunctionParamKind::Regular { pattern, .. } = &param.kind else {
            return None;
        };
        let PatternKind::Ident {
            name,
            by_ref: false,
            subpattern: None,
            ..
        } = &pattern.kind
        else {
            return None;
        };
        if &name.name == result {
            if forwarded.is_some() {
                return None;
            }
            forwarded = Some(index);
        }
    }
    forwarded
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
        if origins.is_empty() { return Ok(()); }
        let (holder, _) = self.borrow_place(destination)
            .ok_or(crate::TypeError::UnknownRecordOrigin { span: destination.span })?;
        let region = self.ctx.env.binding_loan_region(&holder)
            .ok_or(crate::TypeError::UnknownRecordOrigin { span: destination.span })?;
        for origin in origins {
            let root = self.ctx.env.record_storage_name(origin.binding)
                .ok_or(crate::TypeError::UnknownRecordOrigin { span: destination.span })?;
            if self.ctx.env.storage_loan_region(origin.binding).is_some_and(|source| source > region) {
                return Err(crate::TypeError::DanglingReference {
                    var: root, ref_span: destination.span, drop_span: destination.span,
                });
            }
            let field = (!origin.fields.is_empty()).then(|| origin.fields.join("."));
            if !self.borrow_tracker.retain_place_loans(&root, field.as_deref(), region) {
                self.retain_origin(origin, None, destination.span)?;
                self.borrow_tracker.retain_place_loans(&root, field.as_deref(), region);
            }
        }
        self.ctx.env.assign_reference_origins(&holder, origins);
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

    pub(super) fn aggregate_origins(&mut self, value: &Expr, ty: &Type) -> Vec<RecordReceiver> {
        let mut found = Vec::new();
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
        Ok((!sources.is_empty()).then(|| ReferenceOrigin::Candidates(sources.into())))
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
        let mut candidates = Vec::new();
        self.collect_reference_candidates(value, &mut candidates);
        if shared {
            for source in &candidates {
                self.retain_origin(source, Some(holder), value.span)?;
            }
        }
        Ok((!candidates.is_empty())
            .then(|| ReferenceOrigin::Candidates(candidates.into_boxed_slice())))
    }

    fn collect_reference_candidates(&mut self, value: &Expr, found: &mut Vec<RecordReceiver>) {
        fn add(found: &mut Vec<RecordReceiver>, source: RecordReceiver) {
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
                self.collect_reference_candidates(expr, found)
            }
            ExprKind::Call { args, .. } => {
                for arg in args {
                    self.collect_reference_candidates(arg, found);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
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
                if let Some((root, path)) = self.borrow_place(expr) {
                    let referents = self.ctx.env.reference_candidates(&root);
                    // Taking the address of a field of *r can borrow each
                    // possible referent. Taking &r borrows the slot holding r.
                    let slot = if !referents.is_empty() && self.borrows_referent_field(expr) {
                        None
                    } else {
                        self.ctx.env.record_source(&root, false)
                    };
                    for mut source in slot.into_iter().chain(referents) {
                        if let Some(path) = &path {
                            source.fields.extend(path.split('.').map(Into::into));
                        }
                        add(found, source);
                    }
                }
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
                self.collect_reference_candidates(expr, found);
                for arm in arms {
                    self.collect_reference_candidates(&arm.body, found);
                }
            }
            ExprKind::NamedArg { value, .. } => self.collect_reference_candidates(value, found),
            _ => {}
        }
    }

    fn collect_block_candidates(
        &mut self,
        block: &verum_ast::Block,
        found: &mut Vec<RecordReceiver>,
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
            ExprKind::Call { func, args, .. } => {
                let ExprKind::Path(path) = &unparen(func).kind else {
                    return None;
                };
                let name = self.path_to_string(path);
                let index = self
                    .callee_scheme_with_mount_authority(&name)?
                    .returned_parameter?;
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
        if args.len() + usize::from(receiver.is_some()) < 2 {
            return Ok(());
        }
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
            let mut origins = Vec::new();
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
                    self.argument_reference_mutability(args.get(scheme.returned_parameter?)?)
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
        match &unparen(expr).kind {
            ExprKind::Path(path) => {
                let name = &path.as_ident()?.name;
                let ty = self.ctx.env.lookup(name)?.ty.clone();
                let head = self.place_reference_head(&ty, expr.span).ok()?;
                let reference = matches!(head, Type::Reference { mutable: false, .. });
                if matches!(
                    head,
                    Type::Reference { mutable: true, .. }
                        | Type::CheckedReference { .. }
                        | Type::UnsafeReference { .. }
                ) {
                    return None;
                }
                Some((self.ctx.env.record_source(name, reference)?, ty))
            }
            ExprKind::Field {
                expr: parent,
                field,
            } => {
                let (mut owner, ty) = self.borrowed_record_place(parent)?;
                let field_ty =
                    self.lookup_field_type(&self.record_conversion_head(&ty), &field.name)?;
                if matches!(
                    self.place_reference_head(&field_ty, expr.span).ok()?,
                    Type::Reference { .. }
                        | Type::CheckedReference { .. }
                        | Type::UnsafeReference { .. }
                ) {
                    return None;
                }
                owner.fields.push(field.name.clone());
                Some((owner, field_ty))
            }
            _ => None,
        }
    }
}
