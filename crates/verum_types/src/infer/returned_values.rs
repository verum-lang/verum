//! Source-owned result derivations for direct values and structural projections.
use super::TypeChecker;
use crate::context::{ReturnedValue, ReturnedValueNode};
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

/// Only explicit value construction is admitted here. Calls, mutations,
/// control flow and hidden inputs preserve the conservative caller rule.
/// The flat program shares immutable local aliases and has no recursive nodes.
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
    for (index, parameter) in function.params.iter().enumerate() {
        if !parameter.attributes.is_empty() {
            return None;
        }
        let FunctionParamKind::Regular {
            pattern,
            default_value: None,
            ..
        } = &parameter.kind
        else {
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
        if bindings.insert(name.name.clone(), nodes.len()).is_some() {
            return None;
        }
        nodes.push(ReturnedValueNode::Parameter(index));
    }
    if let Some(FunctionBody::Block(block)) = &function.body
        && block.stmts.is_empty()
        && block.expr.is_none()
    {
        let result = nodes.len();
        nodes.push(ReturnedValueNode::Literal(Type::Unit));
        return Some(Shared::new(ReturnedValue {
            nodes,
            result,
            locals,
        }));
    }
    let (statements, value) = match function.body.as_ref()? {
        FunctionBody::Expr(value) => (&[][..], value),
        FunctionBody::Block(block) => match block.expr.as_deref() {
            Some(value) => (block.stmts.as_slice(), value),
            None => {
                let (last, prefix) = block.stmts.split_last()?;
                if !last.attributes.is_empty() {
                    return None;
                }
                let StmtKind::Expr { expr, .. } = &last.kind else {
                    return None;
                };
                let ExprKind::Return(Some(value)) = &expr.kind else {
                    return None;
                };
                (prefix, value.as_ref())
            }
        },
    };
    for statement in statements {
        if !statement.attributes.is_empty() {
            return None;
        }
        let StmtKind::Let {
            pattern,
            ty: None,
            value: Some(value),
        } = &statement.kind
        else {
            return None;
        };
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
    let result = append_expression(checker, value, &bindings, &mut nodes)?;
    Some(Shared::new(ReturnedValue {
        nodes,
        result,
        locals,
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
                    values.push(*bindings.get(&path.as_ident()?.name)?);
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
        let args = match &unparen(expression).kind {
            ExprKind::Call { args, .. } | ExprKind::MethodCall { args, .. } => args,
            _ => return None,
        };
        self.returned_call_value_origins(expression.span, args)
    }

    pub(super) fn returned_call_value_origins(
        &mut self,
        span: verum_ast::Span,
        args: &[Expr],
    ) -> Option<List<RecordReceiver>> {
        let derivation = self.type_registry.returned_value(span)?.clone();
        let Type::Function {
            params,
            return_type,
            ..
        } = self.type_registry.lookup_call_target(span)?.clone()
        else {
            return None;
        };
        if params.len() != args.len() {
            return None;
        }

        // A structural constructor must supply every field of the selected
        // result, with exactly its checked types. This excludes implicit
        // default initializers and user conversions from the closed program.
        let mut expected: List<Option<Type>> = (0..derivation.nodes.len()).map(|_| None).collect();
        expected[derivation.result] = Some(self.unifier.apply(&return_type));
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
                    // An owned nominal parameter can run user Drop even in an
                    // empty body. Only primitive values and references have a
                    // closed cleanup operation here.
                    if !self.return_value_has_trivial_cleanup(&ty) {
                        return None;
                    }
                    if self.deref_adjustments.contains_key(&argument.span) {
                        return None;
                    }
                    let actual = self
                        .observed_operand_type(argument)
                        .or_else(|| self.type_registry.lookup_expr(argument.span).cloned())?;
                    let actual = self.place_reference_head(&actual, argument.span).ok()?;
                    let selected = self.place_reference_head(&ty, argument.span).ok()?;
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
                        origins.push(self.borrowed_record_place(argument)?.0);
                    } else if self
                        .reference_mode(&selected, argument.span)
                        .ok()?
                        .is_some()
                    {
                        // Candidate input loans of an opaque reference result
                        // do not establish the storage it actually points to.
                        origins.push(self.reference_result_origin(argument)?);
                    }
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
