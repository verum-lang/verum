//! Source-owned result derivations for direct values and structural projections.
use super::TypeChecker;
use crate::context::{InputFieldWrite, ReturnedValue, ReturnedValueNode};
use crate::ty::{RecordReceiver, Type};
use verum_ast::{
    Expr, ExprKind, FunctionBody, FunctionDecl, FunctionParamKind, LiteralKind, PatternKind,
    StmtKind, TypeDeclBody,
};
use verum_common::{List, Map, Shared, Text};

fn unparen(mut expression: &Expr) -> &Expr {
    while let ExprKind::Paren(inner) = &expression.kind {
        expression = inner;
    }
    expression
}

/// A closed structural body: explicit values and direct field assignments.
/// Calls, control flow and hidden inputs keep the conservative caller rule.
/// Assignment targets are validated against the selected types before use;
/// source syntax alone never proves that a write preserves stored loans.
pub(super) fn returned_value(
    checker: &TypeChecker,
    function: &FunctionDecl,
) -> Option<Shared<ReturnedValue>> {
    if function.is_async
        || function.is_generator
        || function.is_unsafe
        || function.is_meta
        || !function.attributes.is_empty()
        || function.std_attr.is_some()
        || function.extern_abi.is_some()
        || !function.contexts.is_empty()
        || function.is_variadic
        || !function.requires.is_empty()
        || !function.ensures.is_empty()
    {
        return None;
    }
    let mut nodes = List::new();
    let mut bindings = Map::new();
    let mut locals = List::new();
    let mut writes = List::new();
    let mut argument = 0;
    for (index, parameter) in function.params.iter().enumerate() {
        if !parameter.attributes.is_empty() {
            return None;
        }
        let (name, node) = match &parameter.kind {
            FunctionParamKind::Regular {
                pattern,
                default_value: None,
                ..
            } => {
                let PatternKind::Ident {
                    name,
                    by_ref: false,
                    subpattern: None,
                    ..
                } = &pattern.kind
                else {
                    return None;
                };
                let node = ReturnedValueNode::Parameter(argument);
                argument += 1;
                (name.name.clone(), node)
            }
            FunctionParamKind::SelfRef | FunctionParamKind::SelfRefMut if index == 0 => {
                (Text::from("self"), ReturnedValueNode::Receiver)
            }
            _ => return None,
        };
        if bindings.insert(name, nodes.len()).is_some() {
            return None;
        }
        nodes.push(node);
    }
    let (statements, value) = match function.body.as_ref()? {
        FunctionBody::Expr(value) => (&[][..], Some(value)),
        FunctionBody::Block(block) => match block.expr.as_deref() {
            Some(value) => (block.stmts.as_slice(), Some(value)),
            None => match block.stmts.last() {
                Some(last) if last.attributes.is_empty() => match &last.kind {
                    StmtKind::Expr { expr, .. } => match &expr.kind {
                        ExprKind::Return(value) => (
                            &block.stmts.as_slice()[..block.stmts.len() - 1],
                            value.as_deref(),
                        ),
                        _ => (block.stmts.as_slice(), None),
                    },
                    _ => (block.stmts.as_slice(), None),
                },
                _ => (block.stmts.as_slice(), None),
            },
        },
    };
    for statement in statements {
        if !statement.attributes.is_empty() {
            return None;
        }
        match &statement.kind {
            StmtKind::Let {
                pattern,
                ty: None,
                value: Some(value),
            } => {
                let PatternKind::Ident {
                    name,
                    mutable: false,
                    by_ref: false,
                    subpattern: None,
                } = &pattern.kind
                else {
                    return None;
                };
                let result = append_expression(checker, value, &bindings, &mut nodes)?;
                bindings.insert(name.name.clone(), result);
                locals.push(result);
            }
            StmtKind::Expr { expr, .. } => {
                let ExprKind::Binary {
                    op: verum_ast::BinOp::Assign,
                    left,
                    right,
                } = &unparen(expr).kind
                else {
                    return None;
                };
                let mut fields = List::new();
                let mut root = unparen(left);
                while let ExprKind::Field { expr, field } = &root.kind {
                    fields.push(field.name.clone());
                    root = unparen(expr);
                }
                if fields.is_empty() {
                    return None;
                }
                fields.reverse();
                let input = append_expression(checker, root, &bindings, &mut nodes)?;
                if !matches!(
                    nodes.get(input)?,
                    ReturnedValueNode::Parameter(_) | ReturnedValueNode::Receiver
                ) {
                    return None;
                }
                let value = append_expression(checker, right, &bindings, &mut nodes)?;
                writes.push(InputFieldWrite {
                    input,
                    fields,
                    value,
                });
            }
            _ => return None,
        }
    }
    let result = match value {
        Some(value) => append_expression(checker, value, &bindings, &mut nodes)?,
        None => {
            let index = nodes.len();
            nodes.push(ReturnedValueNode::Literal(Type::Unit));
            index
        }
    };
    Some(Shared::new(ReturnedValue {
        nodes,
        result,
        locals,
        writes,
    }))
}

fn append_expression(
    checker: &TypeChecker,
    value: &Expr,
    bindings: &Map<Text, usize>,
    nodes: &mut List<ReturnedValueNode>,
) -> Option<usize> {
    enum Step<'a> {
        Enter(&'a Expr),
        Binding(&'a Text),
        Field(&'a Text),
        TupleIndex(usize),
        Tuple(usize),
        Record(Text, List<Text>),
    }
    let mut pending = List::from_iter([Step::Enter(value)]);
    let mut values = List::new();
    while let Some(step) = pending.pop() {
        let node = match step {
            Step::Enter(expression) => match &unparen(expression).kind {
                ExprKind::Path(path) => {
                    let slot = match path.segments.as_slice() {
                        [verum_ast::ty::PathSegment::Name(name)] => bindings.get(&name.name),
                        [verum_ast::ty::PathSegment::SelfValue] => {
                            bindings.get(&Text::from("self"))
                        }
                        _ => return None,
                    };
                    values.push(*slot?);
                    continue;
                }
                ExprKind::Literal(literal) => {
                    match &literal.kind {
                        LiteralKind::Int(value) if value.suffix.is_none() => {}
                        LiteralKind::Float(value) if value.suffix.is_none() => {}
                        LiteralKind::Bool(_) | LiteralKind::Char(_) | LiteralKind::Text(_) => {}
                        _ => return None,
                    }
                    ReturnedValueNode::Literal(checker.infer_literal(literal))
                }
                ExprKind::Field { expr, field } => {
                    pending.push(Step::Field(&field.name));
                    pending.push(Step::Enter(expr));
                    continue;
                }
                ExprKind::TupleIndex { expr, index } => {
                    pending.push(Step::TupleIndex(*index as usize));
                    pending.push(Step::Enter(expr));
                    continue;
                }
                ExprKind::Tuple(parts) => {
                    pending.push(Step::Tuple(parts.len()));
                    pending.extend(parts.iter().rev().map(Step::Enter));
                    continue;
                }
                ExprKind::Record {
                    path,
                    fields,
                    base: None,
                } => {
                    // Every field must be explicit. A default initializer can
                    // execute a user function before the result is returned.
                    let name = checker.path_to_string(path);
                    let (declaration, owner) = checker.declaring_source_type_declaration(&name)?;
                    let TypeDeclBody::Record(record) = &declaration.body else {
                        return None;
                    };
                    if record.len() != fields.len()
                        || record.iter().any(|declared| {
                            fields
                                .iter()
                                .filter(|field| field.name.name == declared.name.name)
                                .count()
                                != 1
                        })
                    {
                        return None;
                    }
                    let owner = if owner.is_empty() {
                        declaration.name.name.clone()
                    } else {
                        format!("{owner}.{}", declaration.name.name).into()
                    };
                    pending.push(Step::Record(
                        owner,
                        fields.iter().map(|field| field.name.name.clone()).collect(),
                    ));
                    for field in fields.iter().rev() {
                        pending.push(match &field.value {
                            Some(value) => Step::Enter(value),
                            None => Step::Binding(&field.name.name),
                        });
                    }
                    continue;
                }
                _ => return None,
            },
            Step::Binding(name) => {
                values.push(*bindings.get(name)?);
                continue;
            }
            Step::Field(name) => ReturnedValueNode::Field {
                source: values.pop()?,
                name: name.clone(),
            },
            Step::TupleIndex(index) => ReturnedValueNode::TupleIndex {
                source: values.pop()?,
                index,
            },
            Step::Tuple(count) => {
                let start = values.len().checked_sub(count)?;
                ReturnedValueNode::Tuple(values.split_off(start))
            }
            Step::Record(owner, fields) => {
                let start = values.len().checked_sub(fields.len())?;
                ReturnedValueNode::Record {
                    owner,
                    fields: fields.into_iter().zip(values.split_off(start)).collect(),
                }
            }
        };
        values.push(nodes.len());
        nodes.push(node);
    }
    (values.len() == 1).then(|| values[0])
}

impl TypeChecker {
    /// A callable's signature does not describe its captured environment. The
    /// same closed structural language can prove that a literal closure only
    /// mentions its own parameters. Unknown syntax keeps the conservative rule.
    pub(super) fn callable_has_closed_environment(&self, value: &Expr) -> bool {
        let ExprKind::Closure {
            params,
            body,
            contexts,
            async_,
            ..
        } = &unparen(value).kind
        else {
            return false;
        };
        if *async_ || !contexts.is_empty() {
            return false;
        }
        let mut bindings = Map::new();
        let mut nodes = List::new();
        for (index, parameter) in params.iter().enumerate() {
            let PatternKind::Ident {
                name,
                by_ref: false,
                subpattern: None,
                ..
            } = &parameter.pattern.kind
            else {
                return false;
            };
            if bindings.insert(name.name.clone(), nodes.len()).is_some() {
                return false;
            }
            nodes.push(ReturnedValueNode::Parameter(index));
        }
        append_expression(self, body, &bindings, &mut nodes).is_some()
    }

    pub(super) fn returned_value_origins(
        &mut self,
        expression: &Expr,
    ) -> Option<List<RecordReceiver>> {
        let (args, receiver) = match &unparen(expression).kind {
            ExprKind::Call { args, .. } => (args.as_slice(), None),
            ExprKind::MethodCall { args, receiver, .. } => {
                (args.as_slice(), Some(receiver.as_ref()))
            }
            _ => return None,
        };
        self.returned_call_value_origins(expression.span, args, receiver)
    }

    pub(super) fn returned_call_value_origins(
        &mut self,
        span: verum_ast::Span,
        args: &[Expr],
        receiver: Option<&Expr>,
    ) -> Option<List<RecordReceiver>> {
        let derivation = self.type_registry.returned_value(span)?.clone();
        let Type::Function {
            params,
            return_type,
            contexts,
            ..
        } = self.type_registry.lookup_call_target(span)?.clone()
        else {
            return None;
        };
        if params.len() != args.len()
            || contexts
                .as_ref()
                .is_some_and(|contexts| !contexts.is_empty())
        {
            return None;
        }

        // A structural constructor must supply every field of the selected
        // result, with exactly its checked types. This excludes implicit
        // default initializers and user conversions from the closed program.
        let mut expected: List<Option<Type>> = (0..derivation.nodes.len()).map(|_| None).collect();
        expected[derivation.result] = Some(self.unifier.apply(&return_type));
        let declared_receiver = self.type_registry.declared_receiver(span).cloned();
        for write in &derivation.writes {
            let input = match derivation.nodes.get(write.input)? {
                ReturnedValueNode::Parameter(index) => params.get(*index)?,
                ReturnedValueNode::Receiver => declared_receiver.as_ref()?,
                _ => return None,
            };
            let head = self.place_reference_head(input, span).ok()?;
            let mut field = match head {
                Type::Reference {
                    mutable: true,
                    inner,
                }
                | Type::CheckedReference {
                    mutable: true,
                    inner,
                }
                | Type::UnsafeReference {
                    mutable: true,
                    inner,
                } => *inner,
                _ => return None,
            };
            for name in &write.fields {
                // A reference-valued intermediate is another storage edge;
                // custom Deref and computed places need their own proof.
                if self.reference_mode(&field, span).ok()?.is_some() {
                    return None;
                }
                field = self.lookup_field_type(&field, name)?;
            }
            let field = self.unifier.apply(&field);
            // Replacing a nominal value can run Drop/Clone/conversion. Only
            // primitive scalar storage has a closed write operation here.
            if !matches!(
                field,
                Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char
            ) {
                return None;
            }
            let slot = expected.get_mut(write.value)?;
            if slot.as_ref().is_some_and(|previous| previous != &field) {
                return None;
            }
            *slot = Some(field);
        }
        for (index, node) in derivation.nodes.iter().enumerate().rev() {
            let components: List<(usize, Type)> = match node {
                ReturnedValueNode::Tuple(parts) => {
                    let ty = expected[index].as_ref()?;
                    let types = match ty {
                        Type::Unit if parts.is_empty() => List::new(),
                        Type::Tuple(types) if types.len() == parts.len() => types.clone(),
                        _ => return None,
                    };
                    parts.iter().copied().zip(types).collect()
                }
                ReturnedValueNode::Record { owner, fields } => {
                    let ty = expected[index].as_ref()?;
                    if self.conversion_nominal_head(ty)?.0 != *owner {
                        return None;
                    }
                    let declared = self.resolve_to_record_type(ty)?;
                    if declared.len() != fields.len() {
                        return None;
                    }
                    fields
                        .iter()
                        .map(|(name, value)| Some((*value, declared.get(name)?.clone())))
                        .collect::<Option<List<_>>>()?
                }
                _ => List::new(),
            };
            for (component, ty) in components {
                if expected[component]
                    .as_ref()
                    .is_some_and(|known| known != &ty)
                {
                    return None;
                }
                expected[component] = Some(ty);
            }
        }

        let mut values: List<(Type, List<RecordReceiver>)> = List::new();
        for (index, node) in derivation.nodes.iter().enumerate() {
            let (ty, origins) = match node {
                ReturnedValueNode::Parameter(index) => {
                    let argument = args.get(*index)?;
                    let ty = self.unifier.apply(params.get(*index)?);
                    let origins = self.closed_call_input_origins(argument, &ty)?;
                    (ty, origins)
                }
                ReturnedValueNode::Receiver => {
                    let ty = self.unifier.apply(declared_receiver.as_ref()?);
                    let origins = self.closed_call_input_origins(receiver?, &ty)?;
                    (ty, origins)
                }
                ReturnedValueNode::Literal(ty) => (ty.clone(), List::new()),
                ReturnedValueNode::Field { source, name } => {
                    let (ty, origins) = values.get(*source)?.clone();
                    let (receiver, mut origins) =
                        self.projected_origins_for_type(ty, origins, span)?;
                    // This is a direct declared field. A Deref implementation
                    // cannot enter the derivation through this lookup.
                    let field = self.lookup_field_type(&receiver, name)?;
                    // Copying an owned nominal field can dispatch declared
                    // copy behavior. Its resource discipline alone does not
                    // prove that operation cannot write through another input.
                    if !self.return_value_has_trivial_cleanup(&field) {
                        return None;
                    }
                    if !self.carries_stored_loan(&field) {
                        origins.clear();
                    }
                    (field, origins)
                }
                ReturnedValueNode::TupleIndex { source, index } => {
                    let (ty, origins) = values.get(*source)?.clone();
                    let (receiver, mut origins) =
                        self.projected_origins_for_type(ty, origins, span)?;
                    let Type::Tuple(parts) = receiver else {
                        return None;
                    };
                    let field = parts.get(*index)?.clone();
                    if !self.return_value_has_trivial_cleanup(&field) {
                        return None;
                    }
                    if !self.carries_stored_loan(&field) {
                        origins.clear();
                    }
                    (field, origins)
                }
                ReturnedValueNode::Tuple(parts) => {
                    let mut origins = List::new();
                    for part in parts {
                        for origin in &values.get(*part)?.1 {
                            if !origins.contains(origin) {
                                origins.push(origin.clone());
                            }
                        }
                    }
                    (expected[index].as_ref()?.clone(), origins)
                }
                ReturnedValueNode::Record { fields, .. } => {
                    let mut origins = List::new();
                    for (_, part) in fields {
                        for origin in &values.get(*part)?.1 {
                            if !origins.contains(origin) {
                                origins.push(origin.clone());
                            }
                        }
                    }
                    (expected[index].as_ref()?.clone(), origins)
                }
            };
            if expected[index]
                .as_ref()
                .is_some_and(|expected| expected != &ty)
            {
                return None;
            }
            values.push((ty, origins));
        }
        for local in &derivation.locals {
            if !self.return_value_has_trivial_cleanup(&values.get(*local)?.0) {
                return None;
            }
        }
        Some(values.get(derivation.result)?.1.clone())
    }

    fn closed_call_input_origins(
        &mut self,
        argument: &Expr,
        ty: &Type,
    ) -> Option<List<RecordReceiver>> {
        // Even unused owned parameters may execute user cleanup.
        if !self.return_value_has_trivial_cleanup(ty)
            || self.deref_adjustments.contains_key(&argument.span)
        {
            return None;
        }
        let actual = self
            .observed_operand_type(argument)
            .or_else(|| self.type_registry.lookup_expr(argument.span).cloned())?;
        let actual = self.place_reference_head(&actual, argument.span).ok()?;
        let selected = self.place_reference_head(ty, argument.span).ok()?;
        let autoref = match &selected {
            Type::Reference { inner, .. }
            | Type::CheckedReference { inner, .. }
            | Type::UnsafeReference { inner, .. } => inner.as_ref() == &actual,
            _ => false,
        };
        if actual != selected && !autoref {
            return None;
        }
        let mut origins = List::new();
        if autoref {
            origins.push(self.call_input_place(argument)?.0);
        } else if self
            .reference_mode(&selected, argument.span)
            .ok()?
            .is_some()
        {
            origins.push(self.reference_result_origin(argument)?);
        }
        Some(origins)
    }

    fn return_value_has_trivial_cleanup(&self, ty: &Type) -> bool {
        matches!(
            self.unifier.apply(ty),
            Type::Unit
                | Type::Never
                | Type::Bool
                | Type::Int
                | Type::Float
                | Type::Char
                | Type::Reference { .. }
                | Type::CheckedReference { .. }
                | Type::UnsafeReference { .. }
        )
    }
}
