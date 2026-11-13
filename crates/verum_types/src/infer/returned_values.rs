//! Source-owned result derivations for direct values and structural projections.
use super::TypeChecker;
use crate::context::{ReturnedValue, ReturnedValueNode};
use crate::ty::{RecordReceiver, Type};
use verum_ast::{
    Expr, ExprKind, FunctionBody, FunctionDecl, FunctionParamKind, PatternKind, StmtKind,
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
pub(super) fn returned_value(function: &FunctionDecl) -> Option<Shared<ReturnedValue>> {
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
    for (index, parameter) in function.params.iter().enumerate() {
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
        nodes.push(ReturnedValueNode::Independent);
        return Some(Shared::new(ReturnedValue { nodes, result }));
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
        let result = append_expression(value, &bindings, &mut nodes)?;
        bindings.insert(name.name.clone(), result);
    }
    let result = append_expression(value, &bindings, &mut nodes)?;
    Some(Shared::new(ReturnedValue { nodes, result }))
}

fn append_expression(
    value: &Expr,
    bindings: &Map<Text, usize>,
    nodes: &mut List<ReturnedValueNode>,
) -> Option<usize> {
    enum Step<'a> {
        Enter(&'a Expr),
        Binding(&'a Text),
        Field(&'a Text),
        TupleIndex(usize),
        Aggregate(usize),
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
                ExprKind::Literal(_) => ReturnedValueNode::Independent,
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
                    pending.push(Step::Aggregate(parts.len()));
                    pending.extend(parts.iter().rev().map(Step::Enter));
                    continue;
                }
                ExprKind::Record {
                    fields, base: None, ..
                } => {
                    pending.push(Step::Aggregate(fields.len()));
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
            Step::Aggregate(count) => {
                let start = values.len().checked_sub(count)?;
                ReturnedValueNode::Aggregate(values.split_off(start))
            }
        };
        values.push(nodes.len());
        nodes.push(node);
    }
    (values.len() == 1).then(|| values[0])
}

impl TypeChecker {
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
        let Type::Function { params, .. } = self.type_registry.lookup_call_target(span)?.clone()
        else {
            return None;
        };
        if params.len() != args.len() {
            return None;
        }
        let mut values: List<(Option<Type>, List<RecordReceiver>)> = List::new();
        for node in &derivation.nodes {
            let (ty, origins) = match node {
                ReturnedValueNode::Parameter(index) => {
                    let argument = args.get(*index)?;
                    let ty = params.get(*index)?.clone();
                    let mut origins = List::new();
                    let actual = self
                        .observed_operand_type(argument)
                        .or_else(|| self.type_registry.lookup_expr(argument.span).cloned())?;
                    if self.reference_mode(&ty, argument.span).ok()?.is_some()
                        && self.reference_mode(&actual, argument.span).ok()?.is_none()
                    {
                        origins.push(self.borrowed_record_place(argument)?.0);
                    } else {
                        self.collect_reference_candidates(argument, &mut origins);
                    }
                    (Some(ty), origins)
                }
                ReturnedValueNode::Independent => (None, List::new()),
                ReturnedValueNode::Field { source, name } => {
                    let (ty, origins) = values.get(*source)?.clone();
                    let (receiver, mut origins) =
                        self.projected_origins_for_type(ty?, origins, span)?;
                    let field = self.lookup_field_type(&receiver, name)?;
                    if !self.carries_stored_loan(&field) {
                        origins.clear();
                    }
                    (Some(field), origins)
                }
                ReturnedValueNode::TupleIndex { source, index } => {
                    let (ty, origins) = values.get(*source)?.clone();
                    let (receiver, mut origins) =
                        self.projected_origins_for_type(ty?, origins, span)?;
                    let Type::Tuple(parts) = receiver else {
                        return None;
                    };
                    let field = parts.get(*index)?.clone();
                    if !self.carries_stored_loan(&field) {
                        origins.clear();
                    }
                    (Some(field), origins)
                }
                ReturnedValueNode::Aggregate(parts) => {
                    let mut origins = List::new();
                    for part in parts {
                        for origin in &values.get(*part)?.1 {
                            if !origins.contains(origin) {
                                origins.push(origin.clone());
                            }
                        }
                    }
                    (None, origins)
                }
            };
            values.push((ty, origins));
        }
        Some(values.get(derivation.result)?.1.clone())
    }
}
