//! Source-owned result derivations for direct values and structural projections.
use super::TypeChecker;
use crate::context::{BorrowedVariantCase, CounterAccess, CounterAddress, InputFieldWrite, ReturnedValue, ReturnedValueNode};
use crate::ty::{RecordReceiver, Type};
use verum_ast::{
    BinOp, Expr, ExprKind, FunctionBody, FunctionDecl, FunctionParamKind, LiteralKind, PatternKind,
    StmtKind, TypeDeclBody,
};
use verum_common::{List, Map, Set, Shared, Text};

const MAX_RETURNED_VALUE_STEPS: usize = 4096;

struct ScalarCallProof {
    parameters: List<Type>,
    receiver: Option<Type>,
    result: Type,
    structural: bool,
}

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

/// Weakening exclusivity preserves the referent, not the permission to write.
/// Nominal identity and reference tier must remain exact; this is not a Deref
/// conversion, a new borrow of the reference slot, or type unification.
fn is_shared_reborrow(actual: &Type, selected: &Type) -> bool {
    match (actual, selected) {
        (Type::Reference { mutable: true, inner: actual },
         Type::Reference { mutable: false, inner: selected })
        | (Type::CheckedReference { mutable: true, inner: actual },
           Type::CheckedReference { mutable: false, inner: selected })
        | (Type::UnsafeReference { mutable: true, inner: actual },
           Type::UnsafeReference { mutable: false, inner: selected }) => actual == selected,
        _ => false,
    }
}

/// A closed structural body: explicit values and direct field assignments.
/// Branches, loops and selected calls need the same typed proof at their use;
/// unknown control flow and hidden inputs keep the conservative caller rule.
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
    if function.params.len() > MAX_RETURNED_VALUE_STEPS { return None; }
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
    let control = value.filter(|value| matches!(unparen(value).kind,
        ExprKind::If { .. } | ExprKind::While { .. } | ExprKind::Block(_))
        && control_tail_is_unit(value));
    append_statements(checker, statements, control, &mut bindings, &mut nodes, &mut locals, &mut writes)?;
    let value = value.filter(|_| control.is_none());
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

/// Do not invent a Unit result for an unannotated value-producing branch.
/// This proof only flattens control flow whose tail is structurally Unit.
fn control_tail_is_unit(value: &Expr) -> bool {
    let mut pending = List::from_iter([value]);
    let mut remaining = MAX_RETURNED_VALUE_STEPS;
    while let Some(value) = pending.pop() {
        let Some(next) = remaining.checked_sub(1) else { return false; };
        remaining = next;
        match &unparen(value).kind {
            ExprKind::While { .. } => (),
            ExprKind::Block(block) => {
                if let Some(value) = &block.expr { pending.push(value); }
            }
            ExprKind::If { then_branch, else_branch, .. } => {
                if let Some(value) = &then_branch.expr { pending.push(value); }
                if let Some(value) = else_branch { pending.push(value); }
            }
            ExprKind::Binary { op, .. } if op.is_assignment() => (),
            ExprKind::Break { value: None, .. } | ExprKind::Continue { .. } => (),
            _ => return false,
        }
        if pending.len() > remaining { return false; }
    }
    true
}

/// Flatten all possible scalar writes, preserving lexical scopes. A loop does
/// not need unrolling: every admitted write preserves the same stored loans.
/// Unknown control flow, mutable locals and early value returns stay unknown.
fn append_statements(
    checker: &TypeChecker,
    statements: &[verum_ast::Stmt],
    tail: Option<&Expr>,
    bindings: &mut Map<Text, usize>,
    nodes: &mut List<ReturnedValueNode>,
    locals: &mut List<usize>,
    writes: &mut List<InputFieldWrite>,
) -> Option<()> {
    enum Work<'a> {
        Statement(&'a verum_ast::Stmt),
        Expression(&'a Expr),
        Block(&'a verum_ast::Block, usize),
        Restore(Map<Text, usize>, usize),
    }
    let mut pending = List::new();
    if let Some(tail) = tail { pending.push(Work::Expression(tail)); }
    if statements.len() > MAX_RETURNED_VALUE_STEPS { return None; }
    pending.extend(statements.iter().rev().map(Work::Statement));
    let mut loops = 0;
    let mut remaining = MAX_RETURNED_VALUE_STEPS;
    while let Some(work) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        if pending.len() > remaining || nodes.len() > MAX_RETURNED_VALUE_STEPS { return None; }
        match work {
            Work::Restore(previous, depth) => { *bindings = previous; loops = depth; }
            Work::Block(block, depth) => {
                remaining = remaining.checked_sub(bindings.len())?;
                if block.stmts.len() > remaining { return None; }
                pending.push(Work::Restore(bindings.clone(), loops));
                loops = depth;
                if let Some(tail) = &block.expr { pending.push(Work::Expression(tail)); }
                pending.extend(block.stmts.iter().rev().map(Work::Statement));
            }
            Work::Statement(statement) => {
                if !statement.attributes.is_empty() { return None; }
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
                        let result = append_expression(checker, value, bindings, nodes)?;
                        bindings.insert(name.name.clone(), result);
                        locals.push(result);
                    }
                    StmtKind::Expr { expr, .. } => pending.push(Work::Expression(expr)),
                    _ => return None,
                }
            }
            Work::Expression(expression) => match &unparen(expression).kind {
                ExprKind::Block(block) => pending.push(Work::Block(block, loops)),
                ExprKind::If { condition, then_branch, else_branch } => {
                    if condition.conditions.len() > remaining { return None; }
                    for condition in &condition.conditions {
                        let verum_ast::expr::ConditionKind::Expr(condition) = condition else { return None; };
                        let source = append_expression(checker, condition, bindings, nodes)?;
                        nodes.push(ReturnedValueNode::Condition(source));
                    }
                    if let Some(branch) = else_branch { pending.push(Work::Expression(branch)); }
                    pending.push(Work::Block(then_branch, loops));
                }
                ExprKind::While { label: None, condition, body, invariants, decreases }
                    if invariants.is_empty() && decreases.is_empty() =>
                {
                    let source = append_expression(checker, condition, bindings, nodes)?;
                    nodes.push(ReturnedValueNode::Condition(source));
                    pending.push(Work::Block(body, loops + 1));
                }
                ExprKind::Break { label: None, value: None } | ExprKind::Continue { label: None }
                    if loops > 0 => (),
                ExprKind::Binary { op, left, right } if op.is_assignment() => {
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
                    let input = append_expression(checker, root, bindings, nodes)?;
                    if !matches!(
                        nodes.get(input)?,
                        ReturnedValueNode::Parameter(_) | ReturnedValueNode::Receiver
                    ) {
                        return None;
                    }
                    let mut value = append_expression(checker, right, bindings, nodes)?;
                    if *op != BinOp::Assign {
                        let left = append_expression(checker, left, bindings, nodes)?;
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
                _ => {
                    let value = append_expression(checker, expression, bindings, nodes)?;
                    locals.push(value);
                }
            },
        }
    }
    Some(())
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
    declared_tuple_variant_owner(checker, &path, name, count)
}

fn declared_tuple_variant_owner(checker: &TypeChecker, path: &str, name: &Text, count: usize) -> Option<Text> {
    let (declaration, owner) = checker.declaring_source_type_declaration(path)?;
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

/// A borrowed sum lends its payload in place. Recognize all returning arms
/// and inspect the complete body of every terminal arm. Computed patterns,
/// guards, local cleanup and unproved calls keep the unknown-write rule.
fn borrowed_variant_match(
    checker: &TypeChecker,
    scrutinee: &Expr,
    arms: &[verum_ast::pattern::MatchArm],
    bindings: &Map<Text, usize>,
    nodes: &[ReturnedValueNode],
) -> Option<ReturnedValueNode> {
    use verum_ast::{UnOp, VariantPatternData};
    let ExprKind::Unary { op, expr } = &unparen(scrutinee).kind else { return None; };
    let mutable = match op { UnOp::Ref => false, UnOp::RefMut => true, _ => return None };
    let mut place = unparen(expr);
    let mut fields = List::new();
    while let ExprKind::Field { expr, field } = &place.kind {
        if fields.len() >= MAX_RETURNED_VALUE_STEPS { return None; }
        fields.push(field.name.clone());
        place = unparen(expr);
    }
    fields.reverse();
    let ExprKind::Path(path) = &place.kind else { return None; };
    let input = match path.segments.as_slice() {
        [verum_ast::PathSegment::SelfValue] => *bindings.get(&Text::from("self"))?,
        [verum_ast::PathSegment::Name(name)] => *bindings.get(&name.name)?,
        _ => return None,
    };
    if !matches!(nodes.get(input)?, ReturnedValueNode::Parameter(_) | ReturnedValueNode::Receiver) {
        return None;
    }
    let mut remaining = MAX_RETURNED_VALUE_STEPS.checked_sub(arms.len())?;
    let mut owner = None;
    let mut cases = List::new();
    let mut traps = Set::new();
    let mut terminal_wildcard = false;
    for arm in arms {
        if terminal_wildcard || arm.guard.is_some() || arm.with_clause.is_some() || !arm.attributes.is_empty() {
            return None;
        }
        let mut body = unparen(&arm.body);
        while let ExprKind::Block(block) = &body.kind {
            remaining = remaining.checked_sub(1)?;
            if !block.stmts.is_empty() { return None; }
            body = unparen(block.expr.as_deref()?);
        }
        let (name, parts) = match &arm.pattern.kind {
            PatternKind::Wildcard => (None, &[][..]),
            PatternKind::Variant { path, data } => {
                let (variant, prefix) = path.segments.split_last()?;
                let verum_ast::PathSegment::Name(variant) = variant else { return None; };
                if prefix.is_empty() { return None; }
                let mut sum = path.clone();
                sum.segments.pop();
                let parts = match data {
                    None => &[][..],
                    Some(VariantPatternData::Tuple(parts)) => parts.as_slice(),
                    _ => return None,
                };
                let key = declared_tuple_variant_owner(checker, &checker.path_to_string(&sum), &variant.name, parts.len())?;
                if owner.as_ref().is_some_and(|previous| previous != &key) { return None; }
                owner = Some(key);
                (Some(variant.name.clone()), parts)
            }
            _ => return None,
        };
        remaining = remaining.checked_sub(parts.len())?;
        let mut bound = Map::new();
        for (index, part) in parts.iter().enumerate() {
            match &part.kind {
                PatternKind::Wildcard => (),
                PatternKind::Ident { name, by_ref: false, mutable: false, subpattern: None } => {
                    if bound.insert(name.name.clone(), index).is_some() { return None; }
                }
                _ => return None,
            }
        }
        let returned_field = match &body.kind {
            ExprKind::Path(path) => Some(*bound.get(&path.as_ident()?.name)?),
            ExprKind::Call { func, args, type_args } if type_args.is_empty() => {
                let ExprKind::Path(path) = &unparen(func).kind else { return None; };
                let callee = &path.as_ident()?.name;
                if bound.contains_key(callee) || bindings.contains_key(callee)
                    || checker.ctx.env.lookup(callee)?.builtin_trap_arity != Some(args.len())
                    || (traps.insert(callee.clone()) && !checker.source_allows_builtin_trap(callee))
                    || args.iter().any(|arg| !matches!(&unparen(arg).kind,
                        ExprKind::Literal(literal) if matches!(literal.kind, LiteralKind::Text(_)))) {
                    return None;
                }
                None
            }
            _ => return None,
        };
        if let Some(name) = name {
            cases.push(BorrowedVariantCase { name, arity: parts.len(), returned_field });
        } else {
            terminal_wildcard = true;
        }
    }
    cases.iter().any(|case| case.returned_field.is_some()).then_some(())?;
    Some(ReturnedValueNode::BorrowedVariant { input, fields, mutable, owner: owner?, cases, terminal_wildcard })
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
        Binary(BinOp),
        Choice,
        Index,
    }
    let mut pending = List::from_iter([Step::Enter(value)]);
    let mut values = List::new();
    let mut remaining = MAX_RETURNED_VALUE_STEPS;
    while let Some(step) = pending.pop() {
        remaining = remaining.checked_sub(1)?;
        if pending.len() > remaining || nodes.len() >= MAX_RETURNED_VALUE_STEPS { return None; }
        let node = match step {
            Step::Enter(expression) => match &unparen(expression).kind {
                ExprKind::Match { expr, arms } => borrowed_variant_match(checker, expr, arms, bindings, nodes)?,
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
                ExprKind::Block(block) if block.stmts.is_empty() => {
                    pending.push(Step::Enter(block.expr.as_deref()?));
                    continue;
                }
                ExprKind::If { condition, then_branch, else_branch: Some(other) }
                    if then_branch.stmts.is_empty() =>
                {
                    let [verum_ast::expr::ConditionKind::Expr(test)] = condition.conditions.as_slice() else {
                        return None;
                    };
                    pending.push(Step::Choice);
                    pending.push(Step::Enter(other));
                    pending.push(Step::Enter(then_branch.expr.as_deref()?));
                    pending.push(Step::Enter(test));
                    continue;
                }
                ExprKind::Literal(literal) => {
                    match &literal.kind {
                        LiteralKind::Int(_) | LiteralKind::Float(_) => {}
                        LiteralKind::Bool(_) | LiteralKind::Char(_) | LiteralKind::Text(_) => {}
                        _ => return None,
                    }
                    ReturnedValueNode::Literal(checker.infer_literal(literal))
                }
                ExprKind::Unsafe(block) if block.stmts.is_empty() => {
                    let ExprKind::MetaFunction { name, args, parenthesized: true } =
                        &unparen(block.expr.as_deref()?).kind else { return None; };
                    let operation = super::callable_properties::intrinsic_operation_name(&name.name, args)?;
                    let operation = verum_vbc::intrinsics::lookup_intrinsic(&operation)?;
                    if !matches!(operation.intrinsic.strategy, verum_vbc::intrinsics::registry::CodegenStrategy::RetainCopy) {
                        return None;
                    }
                    let [_, handle, counter] = args.as_slice() else { return None; };
                    let binding = |value: &Expr| {
                        let ExprKind::Path(path) = &unparen(value).kind else { return None; };
                        match path.segments.as_slice() {
                            [verum_ast::PathSegment::SelfValue] => bindings.get(&Text::from("self")).copied(),
                            [verum_ast::PathSegment::Name(name)] => bindings.get(&name.name).copied(),
                            _ => None,
                        }
                    };
                    let handle = binding(handle)?;
                    let (mut place, address) = match &unparen(counter).kind {
                        ExprKind::Unary { op: verum_ast::UnOp::RefMut, expr } =>
                            (expr.as_ref(), CounterAddress::MutableBorrow),
                        _ => (counter, CounterAddress::Argument),
                    };
                    let mut access = List::new();
                    loop {
                        remaining = remaining.checked_sub(1)?;
                        match &unparen(place).kind {
                            ExprKind::Field { expr, field } => {
                                access.push(CounterAccess::Field(field.name.clone()));
                                place = expr;
                            }
                            ExprKind::Unary { op: verum_ast::UnOp::Deref, expr } => {
                                access.push(CounterAccess::Dereference);
                                place = expr;
                            }
                            _ => break,
                        }
                    }
                    access.reverse();
                    ReturnedValueNode::RetainedCopy { handle, counter: binding(place)?, access, address }
                }
                ExprKind::Binary { op, left, right } if !op.is_assignment() => {
                    pending.push(Step::Binary(*op));
                    pending.push(Step::Enter(right));
                    pending.push(Step::Enter(left));
                    continue;
                }
                ExprKind::Index { expr, index } => {
                    pending.push(Step::Index);
                    pending.push(Step::Enter(index));
                    pending.push(Step::Enter(expr));
                    continue;
                }
                ExprKind::Call { func, args, type_args }
                    if type_args.is_empty() && matches!(unparen(func).kind, ExprKind::Path(_)) =>
                {
                    if args.len() > remaining { return None; }
                    pending.push(Step::Call(expression.span, false, args.len()));
                    pending.extend(args.iter().rev().map(Step::Enter));
                    continue;
                }
                ExprKind::MethodCall { receiver, method, args, type_args }
                    if type_args.is_empty() =>
                {
                    if args.len() > remaining { return None; }
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
                    if parts.len() > remaining { return None; }
                    pending.push(Step::Tuple(parts.len()));
                    pending.extend(parts.iter().rev().map(Step::Enter));
                    continue;
                }
                ExprKind::Record {
                    path,
                    fields,
                    base: None,
                } => {
                    if fields.len() > remaining { return None; }
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
            Step::Choice => {
                let otherwise = values.pop()?;
                let consequent = values.pop()?;
                ReturnedValueNode::Choice { condition: values.pop()?, arms: [consequent, otherwise] }
            }
            Step::Binary(op) => {
                let right = values.pop()?;
                let left = values.pop()?;
                ReturnedValueNode::ScalarBinary { op, left, right }
            }
            Step::Index => {
                let index = values.pop()?;
                let source = values.pop()?;
                ReturnedValueNode::Index { source, index }
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
    /// A callable's signature does not describe its captured environment.
    /// Module functions have no caller-local captures; their exact declaration
    /// must be selected before using that fact. A literal closure instead needs
    /// the existing structural proof that it mentions only its own parameters.
    pub(super) fn callable_has_closed_environment(&mut self, value: &Expr) -> bool {
        if let ExprKind::Path(path) = &unparen(value).kind {
            let Some(verum_ast::ty::PathSegment::Name(root)) = path.segments.first() else {
                return false;
            };
            // Parameters, local aliases and nested functions cannot acquire a
            // module declaration's environment proof through shared spelling.
            let name = self.path_to_string(path);
            if self.ctx.env.is_locally_bound(root.name.as_str())
                || self.ctx.env.is_locally_bound(name.as_str())
            {
                return false;
            }
            let Some(scheme) = self.callee_scheme_with_mount_authority(name.as_str()) else {
                return false;
            };
            let Some((_, declaration)) = &scheme.callable_source else {
                return false;
            };
            return scheme.impl_self_type.is_none()
                && declaration.contexts.is_empty()
                && declaration.params.iter().all(|parameter| matches!(
                    &parameter.kind, FunctionParamKind::Regular { .. }
                ));
        }
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
        ).or_else(|| {
            tracing::trace!(target: "verum_types::ownership", ?span,
                receiver = ?declared_receiver, "selected body has no complete value derivation");
            None
        })?;
        // An assignment destroys the old value as well as constructing its
        // replacement. Nominal shape alone is not a lifecycle contract.
        for ty in &facts.writes {
            if !self.write_value_has_structural_cleanup(ty, span) {
                tracing::trace!(target: "verum_types::ownership", ?span, ?ty,
                    "replaced value has no proved structural cleanup");
                return None;
            }
        }
        // A synchronous owned parameter returned as a whole transfers its
        // cleanup obligation to the result. Other inputs and local values
        // still need their own cleanup proof. A field or copied local is not
        // this calling-convention boundary.
        let transferred = derivation.forwarded_parameter()
            .filter(|index| params.get(*index).is_some_and(|parameter|
                self.types_equivalent(parameter, &return_type)));

        // Only result dependencies carry origins out of this invocation.
        // The complete typed graph and every write were checked above; all
        // inputs and local/field cleanup are still checked below. Querying an
        // unrelated intermediate's payload is not a read of the returned value
        // and must not create diagnostics while attempting this optional proof.
        let mut needed: List<bool> = (0..derivation.nodes.len()).map(|_| false).collect();
        *needed.get_mut(derivation.result)? = true;
        for (index, node) in derivation.nodes.iter().enumerate().rev() {
            if !needed[index] || !self.carries_stored_loan(facts.nodes.get(index)?) {
                needed[index] = false;
                continue;
            }
            let mut mark = |source: usize| -> Option<()> {
                *needed.get_mut(source)? = true;
                Some(())
            };
            match node {
                ReturnedValueNode::Field { source, .. }
                | ReturnedValueNode::TupleIndex { source, .. } => mark(*source)?,
                ReturnedValueNode::RetainedCopy { handle, .. } => mark(*handle)?,
                ReturnedValueNode::BorrowedVariant { input, .. } => mark(*input)?,
                ReturnedValueNode::Choice { arms, .. } => {
                    for arm in arms { mark(*arm)?; }
                }
                ReturnedValueNode::Tuple(parts)
                | ReturnedValueNode::Variant { fields: parts, .. } => {
                    for part in parts { mark(*part)?; }
                }
                ReturnedValueNode::Record { fields, .. } => {
                    for (_, part) in fields { mark(*part)?; }
                }
                _ => (),
            }
        }
        let mut values: List<List<RecordReceiver>> = List::new();
        for (index, node) in derivation.nodes.iter().enumerate() {
            let ty = facts.nodes.get(index)?;
            let mut origins = List::new();
            let mut include = |part: usize| -> Option<()> {
                for origin in values.get(part)? {
                    if !origins.contains(origin) { origins.push(origin.clone()); }
                }
                Some(())
            };
            let origins = match node {
                ReturnedValueNode::Parameter(parameter) => {
                    self.closed_call_input_origins(args.get(*parameter)?, ty,
                        transferred == Some(*parameter))?
                }
                ReturnedValueNode::Receiver => self.closed_call_input_origins(receiver?, ty, false)?,
                ReturnedValueNode::RetainedCopy { handle, .. } => {
                    self.read_value_origins(
                        facts.nodes.get(*handle)?.clone(), values.get(*handle)?.clone(), span, true,
                    )?.1
                }
                ReturnedValueNode::BorrowedVariant { input, fields, cases, .. } => {
                    // The reference addresses storage inside the input. Following the
                    // references stored in the record would substitute their lifetimes.
                    for base in values.get(*input)? {
                        for position in cases.iter().filter_map(|case| case.returned_field) {
                            let mut place = base.clone();
                            place.fields.extend(fields.iter().cloned());
                            place.fields.push(position.to_string().into());
                            if !origins.contains(&place) { origins.push(place); }
                        }
                    }
                    origins
                }
                ReturnedValueNode::Literal(_) | ReturnedValueNode::Compound { .. }
                | ReturnedValueNode::ScalarBinary { .. } | ReturnedValueNode::Condition(_)
                | ReturnedValueNode::Index { .. } | ReturnedValueNode::SelectedCall { .. } => List::new(),
                ReturnedValueNode::Field { source, .. }
                | ReturnedValueNode::TupleIndex { source, .. } => {
                    // The type derivation admits only direct declared projections.
                    // Copying an owned nominal field may still run user code.
                    if !self.return_value_has_trivial_cleanup(ty) {
                        return None;
                    }
                    if needed[index] {
                        self.projected_origins_for_type(
                            facts.nodes.get(*source)?.clone(),
                            values.get(*source)?.clone(),
                            span,
                        )?.1
                    } else {
                        List::new()
                    }
                }
                ReturnedValueNode::Tuple(parts)
                | ReturnedValueNode::Variant { fields: parts, .. } => {
                    for part in parts { include(*part)?; }
                    origins
                }
                ReturnedValueNode::Record { fields, .. } => {
                    for (_, part) in fields { include(*part)?; }
                    origins
                }
                ReturnedValueNode::Choice { arms, .. } => {
                    for arm in arms { include(*arm)?; }
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
        let calls = self.scalar_call_proofs(body)?;
        self.returned_body_types_with_calls(body, parameters, receiver, result, span, &calls)
    }

    fn returned_body_types_with_calls(
        &mut self, body: &ReturnedValue, parameters: &[Type], receiver: Option<&Type>,
        result: Option<&Type>, span: verum_ast::Span, calls: &Map<verum_ast::Span, ScalarCallProof>,
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
                ReturnedValueNode::Choice { condition, arms } => List::from_iter([
                    (*condition, Type::Bool), (arms[0], ty.clone()), (arms[1], ty.clone()),
                ]),
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
                ReturnedValueNode::BorrowedVariant { input, fields, mutable, owner, cases, terminal_wildcard } => {
                    let root = self.place_reference_head(nodes.get(*input)?, span).ok()?;
                    let Type::Reference { mutable: writable, inner } = root else { return None; };
                    if *mutable && !writable { return None; }
                    let mut sum = *inner;
                    for field in fields {
                        if self.reference_mode(&sum, span).ok()?.is_some() { return None; }
                        sum = self.unifier.apply(&self.lookup_field_type(&sum, field)?);
                    }
                    if &self.conversion_nominal_head(&sum)?.0 != owner { return None; }
                    let Type::Variant(variants) = self.expand_generic_to_variant(&sum).ok()? else { return None; };
                    let mut covered = Set::new();
                    let mut result = None;
                    for case in cases {
                        if !covered.insert(&case.name) { return None; }
                        let payload = variants.get(&case.name)?;
                        if let Some(position) = case.returned_field {
                            let field = match (case.arity, position, payload) {
                                (1, 0, payload) => payload,
                                (arity, index, Type::Tuple(parts)) if arity == parts.len() => parts.get(index)?,
                                _ => return None,
                            };
                            let borrow = Type::reference(*mutable, field.clone());
                            if result.as_ref().is_some_and(|other| other != &borrow) { return None; }
                            result = Some(borrow);
                        }
                    }
                    if !terminal_wildcard && covered.len() != variants.len() { return None; }
                    result?
                }
                ReturnedValueNode::RetainedCopy { handle, counter, access, address } => {
                    // The registered operation copies the borrowed handle and writes only
                    // its scalar counter. Resolve each place component in the selected
                    // signature. Calls and user-defined dereference have no such proof.
                    let mut counter_type = nodes.get(*counter)?.clone();
                    let mut writable = self.reference_mode(&counter_type, span).ok()? == Some(true);
                    for step in access {
                        let head = self.place_reference_head(&counter_type, span).ok()?;
                        let permission = self.reference_mode(&head, span).ok()?;
                        writable &= permission != Some(false);
                        counter_type = match step {
                            CounterAccess::Dereference => {
                                permission?;
                                self.read_value_origins(head, List::new(), span, true)?.0
                            }
                            CounterAccess::Field(name) => {
                                let head = if permission.is_some() {
                                    self.read_value_origins(head, List::new(), span, true)?.0
                                } else { head };
                                if self.reference_mode(&head, span).ok()?.is_some() { return None; }
                                self.unifier.apply(&self.lookup_field_type(&head, name)?)
                            }
                        };
                    }
                    match address {
                        CounterAddress::Argument => {
                            if self.reference_mode(&counter_type, span).ok()? != Some(true) { return None; }
                            counter_type = self.read_value_origins(counter_type, List::new(), span, true)?.0;
                        }
                        CounterAddress::MutableBorrow if !writable => return None,
                        CounterAddress::MutableBorrow => (),
                    }
                    if counter_type != Type::u32_refined(0) { return None; }
                    let handle_type = nodes.get(*handle)?;
                    if self.reference_mode(handle_type, span).ok()?.is_none() { return None; }
                    self.read_value_origins(handle_type.clone(), List::new(), span, true)?.0
                }
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
                    self.scalar_call_result(calls.get(span)?, *span, &arguments, receiver)?
                }
                ReturnedValueNode::Choice { condition, arms } => {
                    let first = nodes.get(arms[0])?;
                    if nodes.get(*condition)? != &Type::Bool || first != nodes.get(arms[1])? {
                        return None;
                    }
                    first.clone()
                }
                ReturnedValueNode::Condition(source) => {
                    if nodes.get(*source)? != &Type::Bool { return None; }
                    Type::Unit
                }
                ReturnedValueNode::Index { source, index } => {
                    if nodes.get(*index)? != &Type::Int { return None; }
                    let source = self.place_reference_head(nodes.get(*source)?, span).ok()?;
                    let inner = match source {
                        Type::Reference { inner, .. } | Type::CheckedReference { inner, .. } => *inner,
                        _ => return None,
                    };
                    let element = match inner {
                        Type::Slice { element } | Type::Array { element, .. } => *element,
                        _ => return None,
                    };
                    if !self.forwarding_scalar(&element) { return None; }
                    element
                }
                ReturnedValueNode::ScalarBinary { op, left, right } => {
                    let left = nodes.get(*left)?;
                    let right = nodes.get(*right)?;
                    if left != right || !self.forwarding_scalar(left) { return None; }
                    match op {
                        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => Type::Bool,
                        BinOp::And | BinOp::Or if *left == Type::Bool => Type::Bool,
                        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem
                            if !matches!(left, Type::Bool | Type::Char) => left.clone(),
                        _ => return None,
                    }
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

    /// Validate the selected source-call graph in postorder. Cycles, missing
    /// declarations and exhausted proof resources retain the unknown-write rule.
    /// The operation graph is flat; neither syntax nor call depth grows the stack.
    fn scalar_call_proofs(&mut self, root: &ReturnedValue) -> Option<Map<verum_ast::Span, ScalarCallProof>> {
        enum Work {
            Enter(verum_ast::Span),
            Finish(verum_ast::Span, Shared<ReturnedValue>, ScalarCallProof),
        }
        let mut pending = List::new();
        if root.nodes.len() > MAX_RETURNED_VALUE_STEPS { return None; }
        for node in &root.nodes {
            if let ReturnedValueNode::SelectedCall { span, .. } = node { pending.push(Work::Enter(*span)); }
        }
        let mut proofs = Map::new();
        let mut active = Set::new();
        let mut remaining = MAX_RETURNED_VALUE_STEPS;
        while let Some(work) = pending.pop() {
            remaining = remaining.checked_sub(1)?;
            if pending.len() > remaining { return None; }
            match work {
                Work::Enter(span) => {
                    if proofs.contains_key(&span) { continue; }
                    if !active.insert(span) { return None; }
                    let Type::Function { params, return_type, contexts, .. } =
                        self.type_registry.lookup_call_target(span)?.clone() else { return None; };
                    if contexts.as_ref().is_some_and(|contexts| !contexts.is_empty()) { return None; }
                    let structural = self.type_registry.structural_call_target(span)
                        .map(|(_, receiver)| receiver.clone());
                    let receiver = structural.clone().or_else(|| self.type_registry.declared_receiver(span).cloned());
                    let proof = ScalarCallProof {
                        parameters: params, receiver, result: self.unifier.apply(&return_type),
                        structural: structural.is_some(),
                    };
                    if !matches!(proof.result, Type::Unit | Type::Never)
                        && !self.forwarding_scalar(&proof.result) { return None; }
                    if proof.parameters.iter().any(|ty| !self.return_value_has_trivial_cleanup(ty)) { return None; }
                    if structural.is_some() {
                        // This receipt is issued only by the structural operation
                        // resolver. A same-shaped callable or property is no receipt.
                        proofs.insert(span, proof);
                        active.remove(&span);
                        continue;
                    }
                    if proof.receiver.is_none() && self.type_registry.explicit_input_target(span).is_none() { return None; }
                    let body = self.type_registry.returned_value(span)?.clone();
                    remaining = remaining.checked_sub(body.nodes.len())?;
                    let calls: List<_> = body.nodes.iter().filter_map(|node| match node {
                        ReturnedValueNode::SelectedCall { span, .. } => Some(*span), _ => None,
                    }).collect();
                    pending.push(Work::Finish(span, body, proof));
                    pending.extend(calls.into_iter().map(Work::Enter));
                }
                Work::Finish(span, body, proof) => {
                    let facts = self.returned_body_types_with_calls(
                        &body, &proof.parameters, proof.receiver.as_ref(), Some(&proof.result), span, &proofs,
                    )?;
                    if facts.nodes.iter().any(|ty| !self.return_value_has_trivial_cleanup(ty)) { return None; }
                    for write in &facts.writes {
                        if !self.write_value_has_structural_cleanup(write, span) { return None; }
                    }
                    proofs.insert(span, proof);
                    active.remove(&span);
                }
            }
        }
        Some(proofs)
    }

    fn scalar_call_result(
        &self, proof: &ScalarCallProof, span: verum_ast::Span, arguments: &[Type], receiver: Option<&Type>,
    ) -> Option<Type> {
        if proof.parameters.len() != arguments.len() || receiver.is_some() != proof.receiver.is_some() { return None; }
        for (actual, declared) in arguments.iter().zip(&proof.parameters)
            .chain(receiver.zip(proof.receiver.as_ref()))
        {
            let mut actual = self.place_reference_head(actual, span).ok()?;
            let declared = self.place_reference_head(declared, span).ok()?;
            if proof.structural {
                // Structural receiver selection may cross ordinary reference
                // layers, but never executes a user Deref conversion.
                for _ in 0..64 {
                    if actual == declared { break; }
                    actual = match actual {
                        Type::Reference { inner, .. } | Type::CheckedReference { inner, .. } => *inner,
                        _ => return None,
                    };
                }
            }
            let autoref = match &declared {
                Type::Reference { inner, .. } | Type::CheckedReference { inner, .. }
                | Type::UnsafeReference { inner, .. } => inner.as_ref() == &actual,
                _ => false,
            };
            if actual != declared && !autoref && !is_shared_reborrow(&actual, &declared) {
                return None;
            }
        }
        Some(proof.result.clone())
    }

    /// A sum of primitive carriers with no selected destructor has structural cleanup.
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
            let primitive = ty.primitive_name().or_else(|| {
                if !self.has_registered_scalar_storage(&ty) { return None; }
                match &ty {
                    Type::Named { path, .. } => path.as_ident().map(|name| name.name.as_str()),
                    Type::Generic { name, .. } => Some(name.as_str()),
                    _ => None,
                }
            }).and_then(verum_vbc::TypeId::from_well_known_scalar_name);
            // A cleanup footprint describes the operation actually executed,
            // independently of whether copying needs fresh owned storage.
            if primitive.and_then(verum_vbc::drop_plan::builtin_drop_plan)
                .is_some_and(|plan| matches!(plan,
                    verum_vbc::drop_plan::DropPlan::Ignore | verum_vbc::drop_plan::DropPlan::Text))
            { continue; }
            match ty {
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
        transferred: bool,
    ) -> Option<List<RecordReceiver>> {
        // Unused owned parameters still execute cleanup; the one whole
        // parameter transferred to the result does not.
        if (!transferred && !self.return_value_has_trivial_cleanup(ty))
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
        if actual != selected && !autoref && !is_shared_reborrow(&actual, &selected) {
            tracing::trace!(target: "verum_types::ownership", span = ?argument.span,
                ?actual, ?selected, "call input has no declaration-preserving adaptation");
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
        } else if transferred {
            // Transfer preserves payload loans; it does not prove that an
            // owned value is independent of its input storage. This operand
            // was evaluated before subsequent arguments and callee writes.
            self.collect_evaluated_operand_origins(argument, &mut origins);
        }
        Some(origins)
    }

    fn return_value_has_trivial_cleanup(&self, ty: &Type) -> bool {
        self.forwarding_scalar(&self.unifier.apply(ty)) || matches!(
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
