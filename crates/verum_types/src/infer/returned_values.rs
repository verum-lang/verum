//! Source-owned result derivations for direct values and structural projections.
use super::TypeChecker;
use crate::context::{InputFieldWrite, ReturnedValue, ReturnedValueNode};
use crate::ty::{RecordReceiver, Type};
use verum_ast::{
    BinOp, Expr, ExprKind, FunctionBody, FunctionDecl, FunctionParamKind, LiteralKind, PatternKind,
    StmtKind, TypeDeclBody,
};
use verum_common::{List, Map, Shared, Text};

struct ReturnedBodyTypes {
    nodes: List<Type>,
    writes: List<Type>,
}

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
                    op,
                    left,
                    right,
                } = &unparen(expr).kind
                else {
                    return None;
                };
                if !op.is_assignment() {
                    return None;
                }
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
                let mut value = append_expression(checker, right, &bindings, &mut nodes)?;
                if *op != BinOp::Assign {
                    let left = append_expression(checker, left, &bindings, &mut nodes)?;
                    let right = value;
                    value = nodes.len();
                    nodes.push(ReturnedValueNode::Compound { op: *op, left, right });
                }
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

/// Select a sum constructor in the declaration's lexical scope. A value with
/// the same spelling is a value receiver, not a constructor declaration.
fn variant_constructor(
    checker: &TypeChecker,
    receiver: &Expr,
    name: &Text,
    count: usize,
    bindings: &Map<Text, usize>,
) -> Option<Text> {
    let mut suffix = List::new();
    let mut root = unparen(receiver);
    while let ExprKind::Field { expr, field } = &root.kind {
        suffix.push(field.name.clone());
        root = unparen(expr);
    }
    let ExprKind::Path(path) = &root.kind else { return None; };
    let verum_ast::ty::PathSegment::Name(first) = path.segments.first()? else { return None; };
    if bindings.contains_key(&first.name) || checker.ctx.env.is_bound_value(&first.name) {
        return None;
    }
    let mut path = checker.path_to_string(path);
    for segment in suffix.iter().rev() {
        path = format!("{path}.{segment}").into();
    }
    let (declaration, owner) = checker.declaring_source_type_declaration(&path)?;
    let TypeDeclBody::Variant(variants) = &declaration.body else { return None; };
    let variant = variants.iter().find(|variant| variant.name.name == *name)?;
    if !variant.generic_params.is_empty() || variant.where_clause.is_some()
        || !variant.attributes.is_empty() || variant.path_endpoints.is_some()
    {
        return None;
    }
    match &variant.data {
        None if count == 0 => (),
        Some(verum_ast::decl::VariantData::Tuple(fields)) if fields.len() == count => (),
        _ => return None,
    }
    Some(TypeChecker::source_type_declaration_key(&owner, &declaration.name.name))
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
        Variant(Text, Text, usize),
        Call(verum_ast::Span, bool, usize),
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
                ExprKind::Call { func, args, type_args }
                    if type_args.is_empty() && matches!(unparen(func).kind, ExprKind::Path(_)) =>
                {
                    pending.push(Step::Call(expression.span, false, args.len()));
                    pending.extend(args.iter().rev().map(Step::Enter));
                    continue;
                }
                ExprKind::MethodCall { receiver, method, args, type_args }
                    if type_args.is_empty() =>
                {
                    if let Some(owner) = variant_constructor(checker, receiver, &method.name, args.len(), bindings) {
                        pending.push(Step::Variant(owner, method.name.clone(), args.len()));
                        pending.extend(args.iter().rev().map(Step::Enter));
                    } else {
                        pending.push(Step::Call(expression.span, true, args.len()));
                        pending.extend(args.iter().rev().map(Step::Enter));
                        pending.push(Step::Enter(receiver));
                    }
                    continue;
                }
                ExprKind::Field { expr, field } => {
                    if let Some(owner) = variant_constructor(checker, expr, &field.name, 0, bindings) {
                        ReturnedValueNode::Variant { owner, name: field.name.clone(), fields: List::new() }
                    } else {
                        pending.push(Step::Field(&field.name));
                        pending.push(Step::Enter(expr));
                        continue;
                    }
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
            Step::Call(span, has_receiver, count) => {
                let start = values.len().checked_sub(count)?;
                let arguments = values.split_off(start);
                let receiver = if has_receiver { Some(values.pop()?) } else { None };
                ReturnedValueNode::SelectedCall { span, receiver, arguments }
            }
            Step::Variant(owner, name, count) => {
                let start = values.len().checked_sub(count)?;
                ReturnedValueNode::Variant { owner, name, fields: values.split_off(start) }
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
            // A selected callable contract is not evidence about a closure's
            // environment. The capture consumer keeps its original boundary.
            && !nodes.iter().any(|node| matches!(node, ReturnedValueNode::SelectedCall { .. }))
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

        let declared_receiver = self.type_registry.declared_receiver(span).cloned();
        let facts = self.returned_body_types(
            &derivation,
            &params,
            declared_receiver.as_ref(),
            Some(&return_type),
            span,
        )?;
        // An assignment destroys the old value as well as constructing its
        // replacement. Nominal shape alone is not a lifecycle contract.
        for ty in &facts.writes {
            if !self.write_value_has_structural_cleanup(ty, span) { return None; }
        }

        let mut values: List<List<RecordReceiver>> = List::new();
        for (index, node) in derivation.nodes.iter().enumerate() {
            let ty = facts.nodes.get(index)?;
            let origins = match node {
                ReturnedValueNode::Parameter(parameter) => {
                    self.closed_call_input_origins(args.get(*parameter)?, ty)?
                }
                ReturnedValueNode::Receiver => self.closed_call_input_origins(receiver?, ty)?,
                ReturnedValueNode::Literal(_) | ReturnedValueNode::Compound { .. }
                | ReturnedValueNode::SelectedCall { .. } => List::new(),
                ReturnedValueNode::Field { source, .. }
                | ReturnedValueNode::TupleIndex { source, .. } => {
                    // The type derivation admits only direct declared projections.
                    // Copying an owned nominal field may still run user code.
                    if !self.return_value_has_trivial_cleanup(ty) {
                        return None;
                    }
                    let (_, mut origins) = self.projected_origins_for_type(
                        facts.nodes.get(*source)?.clone(),
                        values.get(*source)?.clone(),
                        span,
                    )?;
                    if !self.carries_stored_loan(ty) {
                        origins.clear();
                    }
                    origins
                }
                ReturnedValueNode::Tuple(parts)
                | ReturnedValueNode::Variant { fields: parts, .. } => {
                    let mut origins = List::new();
                    for part in parts {
                        for origin in values.get(*part)? {
                            if !origins.contains(origin) {
                                origins.push(origin.clone());
                            }
                        }
                    }
                    origins
                }
                ReturnedValueNode::Record { fields, .. } => {
                    let mut origins = List::new();
                    for (_, part) in fields {
                        for origin in values.get(*part)? {
                            if !origins.contains(origin) {
                                origins.push(origin.clone());
                            }
                        }
                    }
                    origins
                }
            };
            values.push(origins);
        }
        for local in &derivation.locals {
            if !self.return_value_has_trivial_cleanup(facts.nodes.get(*local)?) {
                return None;
            }
        }
        values.get(derivation.result).cloned()
    }

    /// Complete a source signature before caller instantiation. An unknown
    /// callable is never solved from the caller's desired result: only the
    /// exact declaration's closed body and its own parameter types contribute.
    pub(super) fn structural_source_result(
        &mut self,
        body: &ReturnedValue,
        parameters: &[Type],
        span: verum_ast::Span,
    ) -> Option<Type> {
        let facts = self.returned_body_types(body, parameters, None, None, span)?;
        facts.nodes.get(body.result).cloned()
    }

    /// One flat type derivation serves source result inference and call-site
    /// payload reasoning. This supplies type facts, not permission to copy,
    /// mutate or drop a value; the ownership consumer checks those separately.
    fn returned_body_types(
        &mut self,
        body: &ReturnedValue,
        parameters: &[Type],
        receiver: Option<&Type>,
        result: Option<&Type>,
        span: verum_ast::Span,
    ) -> Option<ReturnedBodyTypes> {
        let mut expected: List<Option<Type>> = (0..body.nodes.len()).map(|_| None).collect();
        *expected.get_mut(body.result)? = result.map(|ty| self.unifier.apply(ty));
        let mut writes = List::new();
        for write in &body.writes {
            let input = match body.nodes.get(write.input)? {
                ReturnedValueNode::Parameter(index) => parameters.get(*index)?,
                ReturnedValueNode::Receiver => receiver?,
                _ => return None,
            };
            let mut field = match self.place_reference_head(input, span).ok()? {
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
                // A reference-valued intermediate names another storage edge.
                // Neither custom Deref nor a computed place is a direct field.
                if self.reference_mode(&field, span).ok()?.is_some() {
                    return None;
                }
                field = self.lookup_field_type(&field, name)?;
            }
            let field = self.unifier.apply(&field);
            let slot = expected.get_mut(write.value)?;
            if slot.as_ref().is_some_and(|previous| previous != &field) {
                return None;
            }
            *slot = Some(field.clone());
            writes.push(field);
        }
        // Push declared result/field constraints into structural constructors.
        // A constructor without an inferable nominal application stays unknown.
        for (index, node) in body.nodes.iter().enumerate().rev() {
            let Some(ty) = expected[index].as_ref() else {
                continue;
            };
            let components: List<(usize, Type)> = match node {
                ReturnedValueNode::Tuple(parts) => {
                    let types = match ty {
                        Type::Unit if parts.is_empty() => List::new(),
                        Type::Tuple(types) if types.len() == parts.len() => types.clone(),
                        _ => return None,
                    };
                    parts.iter().copied().zip(types).collect()
                }
                ReturnedValueNode::Compound { left, right, .. } => {
                    List::from_iter([(*left, ty.clone()), (*right, ty.clone())])
                }
                ReturnedValueNode::Variant { owner, name, fields } => {
                    if self.conversion_nominal_head(ty)?.0 != *owner { return None; }
                    let Type::Variant(variants) = self.expand_generic_to_variant(ty).ok()? else { return None; };
                    let payload = variants.get(name)?;
                    let types = match (fields.len(), payload) {
                        (0, Type::Unit) => List::new(),
                        (1, payload) => List::from_iter([payload.clone()]),
                        (count, Type::Tuple(parts)) if count == parts.len() => parts.clone(),
                        _ => return None,
                    };
                    fields.iter().copied().zip(types).collect()
                }
                ReturnedValueNode::Record { owner, fields } => {
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
                let slot = expected.get_mut(component)?;
                if slot.as_ref().is_some_and(|known| known != &ty) {
                    return None;
                }
                *slot = Some(ty);
            }
        }

        let mut nodes: List<Type> = List::new();
        for (index, node) in body.nodes.iter().enumerate() {
            let ty = match node {
                ReturnedValueNode::Parameter(parameter) => {
                    self.unifier.apply(parameters.get(*parameter)?)
                }
                ReturnedValueNode::Receiver => self.unifier.apply(receiver?),
                ReturnedValueNode::Literal(ty) => ty.clone(),
                ReturnedValueNode::Field { source, name } => {
                    let (head, _) = self.projected_origins_for_type(
                        nodes.get(*source)?.clone(),
                        List::new(),
                        span,
                    )?;
                    self.unifier.apply(&self.lookup_field_type(&head, name)?)
                }
                ReturnedValueNode::TupleIndex { source, index } => {
                    let (head, _) = self.projected_origins_for_type(
                        nodes.get(*source)?.clone(),
                        List::new(),
                        span,
                    )?;
                    let Type::Tuple(parts) = head else {
                        return None;
                    };
                    parts.get(*index)?.clone()
                }
                ReturnedValueNode::Tuple(parts) => {
                    if parts.is_empty() {
                        Type::Unit
                    } else {
                        Type::Tuple(
                            parts
                                .iter()
                                .map(|part| nodes.get(*part).cloned())
                                .collect::<Option<List<_>>>()?,
                        )
                    }
                }
                ReturnedValueNode::Record { .. } | ReturnedValueNode::Variant { .. } => expected[index].as_ref()?.clone(),
                ReturnedValueNode::SelectedCall { span, receiver, arguments } => {
                    let arguments = arguments.iter().map(|index| nodes.get(*index).cloned())
                        .collect::<Option<List<_>>>()?;
                    let receiver = match receiver { Some(index) => Some(nodes.get(*index)?), None => None };
                    self.source_scalar_call_result(*span, &arguments, receiver)?
                }
                ReturnedValueNode::Compound { op, left, right } => {
                    let left = nodes.get(*left)?;
                    let right = nodes.get(*right)?;
                    if left != right { return None; }
                    match (op, left) {
                        (BinOp::AddAssign | BinOp::SubAssign | BinOp::MulAssign
                            | BinOp::DivAssign | BinOp::RemAssign, Type::Int | Type::Float)
                        | (BinOp::BitAndAssign | BinOp::BitOrAssign | BinOp::BitXorAssign
                            | BinOp::ShlAssign | BinOp::ShrAssign, Type::Int) => left.clone(),
                        // A user operator may execute arbitrary writes. Its
                        // spelling and a scalar result do not prove the body.
                        _ => return None,
                    }
                }

            };
            if expected[index]
                .as_ref()
                .is_some_and(|expected| expected != &ty)
            {
                return None;
            }
            nodes.push(ty);
        }
        Some(ReturnedBodyTypes { nodes, writes })
    }

    /// Reuse the body attached to the exact selected source call. A scalar
    /// result or a property bound alone cannot account for implicit cleanup.
    /// This leaf proof deliberately refuses nested calls, so the shared flat
    /// derivation never recursively walks a source call graph.
    fn source_scalar_call_result(
        &mut self,
        span: verum_ast::Span,
        arguments: &[Type],
        receiver: Option<&Type>,
    ) -> Option<Type> {
        let body = self.type_registry.returned_value(span)?.clone();
        if !body.writes.is_empty()
            || body.nodes.iter().any(|node| matches!(node, ReturnedValueNode::SelectedCall { .. }))
        { return None; }
        let declared_receiver = self.type_registry.declared_receiver(span).cloned();
        if receiver.is_some() != declared_receiver.is_some()
            || (receiver.is_none() && self.type_registry.explicit_input_target(span).is_none())
        { return None; }
        let Type::Function { params, return_type, contexts, .. } =
            self.type_registry.lookup_call_target(span)?.clone() else { return None; };
        if params.len() != arguments.len()
            || contexts.as_ref().is_some_and(|contexts| !contexts.is_empty())
        { return None; }
        for (actual, declared) in arguments.iter().zip(&params)
            .chain(receiver.zip(declared_receiver.as_ref()))
        {
            let actual = self.place_reference_head(actual, span).ok()?;
            let declared = self.place_reference_head(declared, span).ok()?;
            let autoref = match &declared {
                Type::Reference { inner, .. } | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } => inner.as_ref() == &actual,
                _ => false,
            };
            if actual != declared && !autoref { return None; }
        }
        let facts = self.returned_body_types(&body, &params, declared_receiver.as_ref(), Some(&return_type), span)?;
        // Covers unused parameters and intermediate/local values too: a hidden
        // user Drop or value copy is not proved by a scalar terminal result.
        if facts.nodes.iter().any(|ty| !self.return_value_has_trivial_cleanup(ty)) {
            return None;
        }
        let result = facts.nodes.get(body.result)?;
        matches!(result, Type::Unit | Type::Never | Type::Bool | Type::Int | Type::Float | Type::Char)
            .then(|| result.clone())
    }

    /// A scalar-only sum with no selected destructor has structural cleanup.
    /// This is deliberately narrower than "contains no loan": an owned record
    /// or an unknown lifecycle can run user code even when its result is Unit.
    fn write_value_has_structural_cleanup(&mut self, ty: &Type, span: verum_ast::Span) -> bool {
        let mut pending = List::from_iter([self.unifier.apply(ty)]);
        // This is a proof-search budget; exhaustion retains the unknown-write
        // rule, rather than claiming that a deeper type is reference-free.
        let mut remaining = 4096usize;
        while let Some(ty) = pending.pop() {
            let Some(next) = remaining.checked_sub(1) else { return false; };
            remaining = next;
            match ty {
                Type::Unit | Type::Never | Type::Bool | Type::Int | Type::Float | Type::Char => (),
                Type::Named { .. } | Type::Generic { .. } => {
                    let Some((name, _)) = self.conversion_nominal_head(&ty) else { return false; };
                    let is_sum = self.source_type_registration.completed_declaration(&name)
                        .is_some_and(|declaration| matches!(declaration.body, TypeDeclBody::Variant(_)));
                    if !is_sum { return false; }
                    if !self.write_value_has_no_selected_drop(&ty, span) { return false; }
                    let Ok(Type::Variant(variants)) = self.expand_generic_to_variant(&ty) else { return false; };
                    pending.extend(variants.into_values());
                }
                _ => return false,
            }
            if pending.len() > remaining { return false; }
        }
        true
    }

    fn write_value_has_no_selected_drop(&mut self, ty: &Type, span: verum_ast::Span) -> bool {
        use verum_common::well_known_types::WellKnownProtocol;
        if self.load_source_receiver_methods(ty).is_err()
            || self.load_source_operator_implementation(ty, WellKnownProtocol::Drop, span).is_err()
        { return false; }
        let Some(protocol) = self.source_operator_name(WellKnownProtocol::Drop) else { return false; };
        self.protocol_checker.read().find_impl(ty, &Self::text_to_path(&protocol)).is_none()
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
