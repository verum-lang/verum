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
    fn reference_mode(&self, ty: &Type, span: verum_ast::Span) -> crate::Result<Option<bool>> {
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

    /// Unlike a witness identity, a lifetime restriction may conservatively
    /// retain several input loans when a callee can choose its return value.
    pub(super) fn bind_reference_result(
        &mut self,
        value: &Expr,
        holder: &str,
    ) -> crate::Result<Option<ReferenceOrigin>> {
        if let Some(origin) = self.reference_result_origin(value) {
            if matches!(
                &unparen(value).kind,
                ExprKind::Call { .. } | ExprKind::Path(_)
            ) {
                self.retain_origin(&origin, Some(holder), value.span)?;
            }
            return Ok(Some(ReferenceOrigin::Exact(origin)));
        }
        let mut candidates = Vec::new();
        self.collect_reference_candidates(value, &mut candidates);
        for source in &candidates {
            self.retain_origin(source, None, value.span)?;
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
            ExprKind::Call { args, .. } => {
                for arg in args {
                    self.collect_reference_candidates(arg, found);
                }
            }
            ExprKind::MethodCall { receiver, args, .. } => {
                if let Some((source, _)) = self.borrowed_record_place(receiver) {
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
                if let Some((root, path)) = self.borrow_place(expr)
                    && let Some(mut storage) = self.ctx.env.record_source(&root, false)
                {
                    if let Some(path) = path {
                        storage.fields.extend(path.split('.').map(Into::into));
                    }
                    add(found, storage);
                    for referent in self.ctx.env.reference_candidates(&root) {
                        add(found, referent);
                    }
                }
            }
            ExprKind::Block(block) => self.collect_block_candidates(block, found),
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

    fn retain_origin(
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
                op: UnOp::Ref,
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

    fn borrowed_record_place(&mut self, expr: &Expr) -> Option<(RecordReceiver, Type)> {
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
