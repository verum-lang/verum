//! Prove that a method only reborrows a stored reference. The selected body
//! may branch or loop. Each use of the exclusive receiver must independently
//! preserve its reference slots. A call on the whole receiver has the same
//! obligation. A call on the referent of a slot receives no address of the slot.
use super::*;
use verum_ast::expr::{ArrayExpr, ConditionKind, UnOp};
use verum_ast::pattern::VariantPatternData;
use verum_ast::{Block, FunctionBody, PathSegment, StmtKind, TypeDeclBody};

fn is_receiver(value: &Expr) -> bool {
    matches!(&value.kind, ExprKind::Path(path) if matches!(path.segments.as_slice(), [PathSegment::SelfValue]))
}

fn rooted_in_receiver(mut value: &Expr) -> bool {
    loop {
        match &value.kind {
            ExprKind::Field { expr, .. }
            | ExprKind::TupleIndex { expr, .. }
            | ExprKind::Index { expr, .. }
            | ExprKind::Paren(expr) => value = expr,
            _ => return is_receiver(value),
        }
    }
}

enum BodyNode<'a> {
    Expression(&'a Expr),
    Block(&'a Block),
    Pattern(&'a Pattern),
}

impl TypeChecker {
    pub(in crate::infer) fn selected_receiver_preserves_stored_loans(
        &mut self,
        span: Span,
    ) -> bool {
        if self.source_type_registration.checking_callable_evidence {
            return false;
        }
        self.receiver_footprint(span).is_some()
    }

    fn receiver_footprint(&mut self, span: Span) -> Option<()> {
        let Type::Reference {
            mutable: true,
            inner,
        } = self.type_registry.declared_receiver(span)?.clone()
        else {
            return None;
        };
        let (record, _) = self.conversion_nominal_head(&inner)?;
        if !matches!(
            self.source_type_registration
                .completed_declaration(&record)?
                .body,
            TypeDeclBody::Record(_)
        ) {
            return None;
        }
        let fields = self.extract_record_fields(&inner).ok()?;
        if !fields.values().any(|field| {
            matches!(field, Type::Reference { mutable: true, inner }
            if !self.may_supply_call_borrow(inner))
        }) {
            return None;
        }
        let (owner, source) = self.receiver_forwarding_target(span, &inner)?;
        let graph = Shared::new(parking_lot::RwLock::new(
            self.module_registry.read().clone(),
        ));
        let mut remaining =
            MAX_SOURCE_CALL_STEPS.checked_sub(source.source.module().items.len())?;
        let mut checker = self.prepare_callable_evidence(&owner, source.source.module(), &graph)?;
        enum Visit {
            Enter(SourceFunction),
            Leave(SourceFunction),
        }
        let mut pending = List::from_single(Visit::Enter(source));
        let mut active = Set::new();
        let mut proved = Map::new();
        while let Some(visit) = pending.pop() {
            remaining = remaining.checked_sub(1)?;
            let selected = match visit {
                Visit::Enter(selected) => selected,
                Visit::Leave(selected) => {
                    active.remove(&selected.identity());
                    proved.insert(selected.identity(), selected);
                    continue;
                }
            };
            if proved.contains_key(&selected.identity()) {
                continue;
            }
            if !active.insert(selected.identity()) {
                return None;
            }
            let item = selected.inherent_method_item()?;
            checker.check_item(&item).ok()?;
            if !checker.take_deferred_errors().is_empty()
                || !checker.diagnostic_sources().is_empty()
                || checker.diagnostics().iter().any(|d| d.is_error())
            {
                return None;
            }
            let calls =
                checker.receiver_body_footprint(&inner, selected.declaration(), &mut remaining)?;
            pending.push(Visit::Leave(selected));
            for call in calls {
                let (nested_owner, nested) = checker.receiver_forwarding_target(call, &inner)?;
                if nested_owner != owner {
                    return None;
                }
                pending.push(Visit::Enter(nested));
            }
        }
        Some(())
    }

    fn receiver_forwarding_target(
        &mut self,
        span: Span,
        receiver: &Type,
    ) -> Option<(Text, SourceFunction)> {
        let Type::Reference { inner, .. } = self.type_registry.declared_receiver(span)? else {
            return None;
        };
        if !self
            .protocol_checker
            .read()
            .types_structurally_equal(inner, receiver)
        {
            return None;
        }
        let Type::Function {
            params,
            return_type,
            ..
        } = self.type_registry.lookup_call_target(span)?.clone()
        else {
            return None;
        };
        if self.may_supply_call_borrow(&return_type) || !self.footprint_cleanup(&return_type, span)
        {
            return None;
        }
        for parameter in &params {
            let value = match parameter {
                Type::Reference {
                    inner,
                    mutable: false,
                } => inner.as_ref(),
                value => value,
            };
            if self.may_supply_call_borrow(value) || !self.footprint_cleanup(value, span) {
                return None;
            }
        }
        let (owner, declaration) = self.type_registry.callable_declaration(span)?;
        if has_implicit_call_computation(declaration)
            || declaration.throws_clause.is_some()
            || !declaration.generics.is_empty()
            || declaration.is_unsafe
            || declaration.is_variadic
        {
            return None;
        }
        Some((
            owner.clone(),
            self.selected_callable_source(span, owner, declaration)?,
        ))
    }

    /// Keep managed values whose cleanup is known, including unused owned
    /// locals. Retiring a reference handle does not drop its referent.
    fn footprint_cleanup(&mut self, ty: &Type, span: Span) -> bool {
        let mut pending = List::from_single(self.unifier.apply(ty));
        let mut remaining = MAX_SOURCE_CALL_STEPS;
        while let Some(ty) = pending.pop() {
            let Some(next) = remaining.checked_sub(1) else {
                return false;
            };
            remaining = next;
            if matches!(
                ty,
                Type::Pointer { .. } | Type::VolatilePointer { .. } | Type::UnsafeReference { .. }
            ) {
                return false;
            }
            if self.return_value_has_trivial_cleanup(&ty)
                || self.write_value_has_structural_cleanup(&ty, span)
            {
                continue;
            }
            match &ty {
                Type::Array {
                    element,
                    size: Some(_),
                } => pending.push(element.as_ref().clone()),
                Type::Tuple(fields) => pending.extend(fields.iter().cloned()),
                Type::Named { .. } | Type::Generic { .. } => {
                    let Some((name, _)) = self.conversion_nominal_head(&ty) else {
                        return false;
                    };
                    let Some(source) = self.source_type_registration.completed_declaration(&name)
                    else {
                        return false;
                    };
                    if source.resource_modifier.is_some()
                        || !matches!(source.body, TypeDeclBody::Record(_))
                        || !self.write_value_has_no_selected_drop(&ty, span)
                    {
                        return false;
                    }
                    let Some(Type::Record(fields)) = self.expand_generic_to_record(&ty) else {
                        return false;
                    };
                    pending.extend(fields.into_values());
                }
                _ => return false,
            }
            if pending.len() > remaining {
                return false;
            }
        }
        true
    }

    fn receiver_body_footprint(
        &mut self,
        receiver_ty: &Type,
        declaration: &FunctionDecl,
        remaining: &mut usize,
    ) -> Option<List<Span>> {
        use BodyNode::{Block as Body, Expression as Value, Pattern as Binding};
        let mut calls = List::new();
        let mut pending = List::from_single(match declaration.body.as_ref()? {
            FunctionBody::Block(block) => Body(block),
            FunctionBody::Expr(expr) => Value(expr),
        });
        while let Some(node) = pending.pop() {
            *remaining = remaining.checked_sub(1)?;
            match node {
                Body(block) => {
                    if block.stmts.len() > *remaining {
                        return None;
                    }
                    for stmt in &block.stmts {
                        if !stmt.attributes.is_empty() {
                            return None;
                        }
                        match &stmt.kind {
                            StmtKind::Let {
                                pattern,
                                value: Some(value),
                                ..
                            } => {
                                let ty = self.type_registry.lookup_expr(value.span)?.clone();
                                if !self.footprint_cleanup(&ty, value.span) {
                                    return None;
                                }
                                pending.extend([Binding(pattern), Value(value)]);
                            }
                            StmtKind::Expr { expr, .. } => pending.push(Value(expr)),
                            StmtKind::Empty => (),
                            _ => return None,
                        }
                    }
                    pending.extend(block.expr.as_deref().map(Value));
                }
                Binding(pattern) => match &pattern.kind {
                    PatternKind::Wildcard
                    | PatternKind::Literal(_)
                    | PatternKind::Ident {
                        by_ref: false,
                        subpattern: None,
                        ..
                    } => (),
                    PatternKind::Tuple(parts)
                    | PatternKind::Variant {
                        data: Some(VariantPatternData::Tuple(parts)),
                        ..
                    } if parts.len() <= *remaining => pending.extend(parts.iter().map(Binding)),
                    PatternKind::Variant { data: None, .. } => (),
                    _ => return None,
                },
                Value(value) => match &value.kind {
                    ExprKind::Path(path)
                        if !path
                            .segments
                            .iter()
                            .any(|part| matches!(part, PathSegment::SelfValue)) =>
                    {
                        ()
                    }
                    ExprKind::Literal(literal)
                        if matches!(
                            literal.kind,
                            verum_ast::LiteralKind::Int(_)
                                | verum_ast::LiteralKind::Float(_)
                                | verum_ast::LiteralKind::Bool(_)
                                | verum_ast::LiteralKind::Char(_)
                                | verum_ast::LiteralKind::ByteChar(_)
                                | verum_ast::LiteralKind::Text(_)
                        ) =>
                    {
                        ()
                    }
                    ExprKind::Continue { .. } => (),
                    ExprKind::Field { expr, field } if is_receiver(expr) => {
                        let ty = self.lookup_field_type(receiver_ty, &field.name)?;
                        if self.may_supply_call_borrow(&ty)
                            || !self.footprint_cleanup(&ty, value.span)
                        {
                            return None;
                        }
                    }
                    ExprKind::Field { expr, .. } | ExprKind::Paren(expr) => {
                        pending.push(Value(expr))
                    }
                    ExprKind::Cast { expr, ty } => {
                        let target = self.declared_type(ty).ok()?;
                        if !self.forwarding_scalar(&target) {
                            return None;
                        }
                        pending.push(Value(expr));
                    }
                    ExprKind::Unary { op, expr } => {
                        if !matches!(
                            op,
                            UnOp::Neg | UnOp::Not | UnOp::BitNot | UnOp::Ref | UnOp::RefMut
                        ) || (matches!(op, UnOp::Ref | UnOp::RefMut) && rooted_in_receiver(expr))
                        {
                            return None;
                        }
                        pending.push(Value(expr));
                    }
                    ExprKind::Binary { op, left, right } => {
                        // Scalar operands cannot supply a reference to a slot of
                        // the receiver, even when the operator returns Bool.
                        if !matches!(op, verum_ast::BinOp::Assign)
                            && self
                                .source_type_registration
                                .scalar_binary_operands
                                .get(&value.span)
                                != Some(&true)
                        {
                            return None;
                        }
                        pending.extend([Value(left), Value(right)]);
                    }
                    ExprKind::Index { expr, index } => {
                        let ty = self
                            .unifier
                            .apply(self.type_registry.lookup_expr(expr.span)?);
                        if !matches!(
                            self.unwrap_reference_type(&ty),
                            Type::Array { .. } | Type::Slice { .. }
                        ) {
                            return None;
                        }
                        pending.extend([Value(expr), Value(index)]);
                    }
                    ExprKind::Array(ArrayExpr::Repeat { value, count }) => {
                        pending.extend([Value(value), Value(count)])
                    }
                    ExprKind::Tuple(parts) | ExprKind::Array(ArrayExpr::List(parts))
                        if parts.len() <= *remaining =>
                    {
                        pending.extend(parts.iter().map(Value))
                    }
                    ExprKind::Range { start, end, .. } => {
                        pending.extend(start.as_deref().map(Value));
                        pending.extend(end.as_deref().map(Value));
                    }
                    ExprKind::Block(block) | ExprKind::Unsafe(block) => pending.push(Body(block)),
                    ExprKind::If {
                        condition,
                        then_branch,
                        else_branch,
                    } if condition.conditions.len() <= *remaining => {
                        for condition in &condition.conditions {
                            let ConditionKind::Expr(test) = condition else {
                                return None;
                            };
                            pending.push(Value(test));
                        }
                        pending.push(Body(then_branch));
                        pending.extend(else_branch.as_deref().map(Value));
                    }
                    ExprKind::Match { expr, arms } if arms.len() <= *remaining => {
                        pending.push(Value(expr));
                        for arm in arms {
                            if !arm.attributes.is_empty() || arm.with_clause.is_some() {
                                return None;
                            }
                            pending.extend([Binding(&arm.pattern), Value(&arm.body)]);
                            pending.extend(arm.guard.as_deref().map(Value));
                        }
                    }
                    ExprKind::Loop {
                        body, invariants, ..
                    } if invariants.is_empty() => pending.push(Body(body)),
                    ExprKind::While {
                        condition,
                        body,
                        invariants,
                        decreases,
                        ..
                    } if invariants.is_empty() && decreases.is_empty() => {
                        pending.extend([Value(condition), Body(body)])
                    }
                    ExprKind::For {
                        iter,
                        body,
                        pattern,
                        invariants,
                        decreases,
                        ..
                    } if invariants.is_empty()
                        && decreases.is_empty()
                        && !rooted_in_receiver(iter) =>
                    {
                        pending.extend([Value(iter), Body(body), Binding(pattern)])
                    }
                    ExprKind::Break { value, .. } | ExprKind::Return(value) => {
                        pending.extend(value.as_deref().map(Value))
                    }
                    ExprKind::Record {
                        fields, base: None, ..
                    } if fields.len() <= *remaining => {
                        let ty = self.type_registry.lookup_expr(value.span)?.clone();
                        if !self.footprint_cleanup(&ty, value.span) {
                            return None;
                        }
                        for field in fields {
                            if !field.attributes.is_empty() {
                                return None;
                            }
                            pending.push(Value(field.value.as_ref()?));
                        }
                    }
                    ExprKind::Call { func, args, .. }
                    | ExprKind::MethodCall {
                        receiver: func,
                        args,
                        ..
                    } => {
                        if args.len() > *remaining || args.iter().any(rooted_in_receiver) {
                            return None;
                        }
                        if !self.is_declared_footprint_operation(value.span) {
                            return None;
                        }
                        if is_receiver(func) && matches!(value.kind, ExprKind::MethodCall { .. }) {
                            calls.push(value.span);
                        } else if let ExprKind::Field { expr, field } = &func.kind
                            && is_receiver(expr)
                            && let Type::Reference {
                                mutable: true,
                                inner,
                            } = self.lookup_field_type(receiver_ty, &field.name)?
                        {
                            let Type::Reference { inner: actual, .. } =
                                self.type_registry.declared_receiver(value.span)?
                            else {
                                return None;
                            };
                            if !self
                                .protocol_checker
                                .read()
                                .types_structurally_equal(actual, &inner)
                                || self.may_supply_call_borrow(&inner)
                                || !self.footprint_cleanup(&inner, value.span)
                                || !self.footprint_call_returns_closed_value(value.span)
                                || self.deref_adjustments.contains_key(&func.span)
                            {
                                return None;
                            }
                        } else {
                            // A sibling field may be read or copied; a method on
                            // it may not expose its address as a new local value.
                            if rooted_in_receiver(func) && !self.closed_field_call(value.span) {
                                return None;
                            }
                            pending.push(Value(func));
                        }
                        pending.extend(args.iter().map(Value));
                    }
                    _ => return None,
                },
            }
            if pending.len() > *remaining {
                return None;
            }
        }
        Some(calls)
    }

    fn footprint_call_returns_closed_value(&self, span: Span) -> bool {
        self.type_registry.lookup_call_target(span).is_some_and(|ty|
            matches!(ty, Type::Function { return_type, .. } if !self.may_supply_call_borrow(&self.unifier.apply(return_type))))
    }

    fn closed_field_call(&self, span: Span) -> bool {
        self.footprint_call_returns_closed_value(span)
            && self
                .type_registry
                .callable_declaration(span)
                .is_some_and(|(_, function)| !function.is_unsafe)
            && !matches!(
                self.type_registry.declared_receiver(span),
                Some(Type::Reference { mutable: true, .. })
            )
    }

    fn is_declared_footprint_operation(&mut self, span: Span) -> bool {
        let Some(Type::Function {
            params,
            return_type,
            ..
        }) = self.type_registry.lookup_call_target(span).cloned()
        else {
            return false;
        };
        if params.iter().any(|parameter| {
            parameter.contains_type(|part| {
                matches!(
                    part,
                    Type::Pointer { .. }
                        | Type::VolatilePointer { .. }
                        | Type::UnsafeReference { .. }
                        | Type::Function { .. }
                )
            })
        }) {
            return false;
        }
        if !self.footprint_cleanup(&return_type, span) {
            return false;
        }
        if self.type_registry.constructed_value(span).is_some()
            || self.type_registry.structural_call_target(span).is_some()
        {
            return true;
        }
        let Some((_, function)) = self.type_registry.callable_declaration(span) else {
            return false;
        };
        function.body.is_some() && !has_implicit_call_computation(function) && !function.is_variadic
    }
}
