//! Source-owned compile-time evaluation followed by runtime value materialization.
//! A quoted expression is a syntax value; this pass never implicitly splices it.
use super::{ConstValue, MetaContext, MetaError, MetaRegistry};
use std::sync::Arc;
use verum_ast::{
    decl::*,
    expr::*,
    literal::*,
    visitor::{self, Visitor},
    *,
};
use verum_common::{List, Map, Set, Text};

type Key = (Text, Text);
#[derive(Debug, Clone, Default)]
pub(crate) struct SourceConstants {
    declarations: Map<Key, Expr>,
    pub(crate) materializing: bool,
    values: Map<Key, ConstValue>,
    active: Set<Key>,
    runtime_declarations: Set<Key>,
    pub(crate) runtime_bindings: Set<Text>,
}
fn failure(message: impl Into<Text>) -> MetaError {
    MetaError::MetaEvaluationFailed {
        message: message.into(),
    }
}
impl MetaContext {
    /// Lexical locals take precedence. Runtime bindings are barriers, never
    /// an invitation to fall through to a same-spelling module constant.
    pub(crate) fn eval_source_variable(&mut self, name: &Text) -> Result<ConstValue, MetaError> {
        if let Some(value) = self.get(name) {
            return Ok(value);
        }
        if self.source_constants.runtime_bindings.contains(name) {
            return Err(failure(format!(
                "runtime binding `{name}` is unavailable at compile time"
            )));
        }
        let key = (self.current_module.clone(), name.clone());
        if self.source_constants.runtime_declarations.contains(&key) {
            return Err(failure(format!(
                "runtime declaration `{name}` is unavailable at compile time"
            )));
        }
        if let Some(value) = self.source_constants.values.get(&key) {
            return Ok(value.clone());
        }
        let Some(expr) = self.source_constants.declarations.get(&key).cloned() else {
            return Err(failure(format!(
                "Undefined variable: {name} (owner {})",
                self.current_module
            )));
        };
        if !self.source_constants.active.insert(key.clone()) {
            return Err(MetaError::CircularDependency {
                path: format!("{}.{}", key.0, key.1).into(),
            });
        }
        let saved = std::mem::take(&mut self.bindings);
        let blocked = std::mem::take(&mut self.source_constants.runtime_bindings);
        let result = self
            .ast_expr_to_meta_expr(&expr)
            .and_then(|ir| self.eval_meta_expr(&ir));
        self.bindings = saved;
        self.source_constants.runtime_bindings = blocked;
        self.source_constants.active.remove(&key);
        if let Ok(value) = &result {
            self.source_constants.values.insert(key, value.clone());
        }
        result
    }
    pub(crate) fn has_source_variable(&self, name: &str) -> bool {
        self.source_constants
            .runtime_declarations
            .contains(&(self.current_module.clone(), Text::from(name)))
            || self.bindings.contains_key(&Text::from(name))
            || self
                .source_constants
                .runtime_bindings
                .contains(&Text::from(name))
            || self
                .source_constants
                .declarations
                .contains_key(&(self.current_module.clone(), Text::from(name)))
    }
}

/// Preserve the value carrier. Unsupported values fail rather than becoming text
/// or executable syntax. A widthless unsigned value has no unique runtime carrier.
pub fn materialize_runtime_value(value: ConstValue, span: Span) -> Result<Expr, MetaError> {
    let kind = match value {
        ConstValue::Unit => ExprKind::Tuple(List::new()),
        ConstValue::Int(value) if value < 0 => ExprKind::Unary {
            op: UnOp::Neg,
            // Match parsed source: the literal stores the positive token
            // magnitude as a u128 bit pattern, with a separate unary sign.
            // unsigned_abs also represents i128::MIN exactly.
            expr: verum_common::Heap::new(Expr::literal(Literal::int(value.unsigned_abs() as i128, span))),
        },
        ConstValue::Int(value) => ExprKind::Literal(Literal::int(value, span)),
        ConstValue::UInt(_) => {
            return Err(failure(
                "unsigned compile-time value has no declared runtime width",
            ));
        }
        ConstValue::Float(value) => ExprKind::Literal(Literal::float(value, span)),
        ConstValue::Bool(value) => ExprKind::Literal(Literal::bool(value, span)),
        ConstValue::Char(value) => ExprKind::Literal(Literal::char(value, span)),
        ConstValue::Text(value) => ExprKind::Literal(Literal::string(value, span)),
        ConstValue::Tuple(values) => ExprKind::Tuple(
            values
                .into_iter()
                .map(|v| materialize_runtime_value(v, span))
                .collect::<Result<_, _>>()?,
        ),
        ConstValue::Array(values) => ExprKind::Array(ArrayExpr::List(
            values
                .into_iter()
                .map(|v| materialize_runtime_value(v, span))
                .collect::<Result<_, _>>()?,
        )),
        ConstValue::Expr(_)
        | ConstValue::Type(_)
        | ConstValue::Pattern(_)
        | ConstValue::Item(_)
        | ConstValue::Items(_) => {
            return Err(failure(
                "@const produced a syntax value; runtime value materialization does not splice syntax",
            ));
        }
        _ => return Err(failure("@const value has no supported runtime AST carrier")),
    };
    Ok(Expr::new(kind, span))
}

/// One registry and constant cache per source roster. Register every owner before
/// lowering any unit, so source order does not select a sibling declaration.
#[derive(Debug, Clone)]
pub struct ConstLoweringError {
    pub owner: Text,
    pub span: Span,
    pub error: MetaError,
}
impl std::fmt::Display for ConstLoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (source owner {}, bytes {}..{})",
            self.error, self.owner, self.span.start, self.span.end
        )
    }
}
impl std::error::Error for ConstLoweringError {}

pub struct ConstLowering {
    context: MetaContext,
    failure_span: Option<Span>,
}
impl ConstLowering {
    pub fn new(mut context: MetaContext) -> Self {
        context.source_constants.materializing = true;
        Self {
            context,
            failure_span: None,
        }
    }
    pub(crate) fn required(module: &Module) -> bool {
        let mut pending = PendingConst(false);
        for item in &module.items {
            pending.visit_item(item);
        }
        pending.0
    }
    pub fn into_context(mut self) -> MetaContext {
        self.context.source_constants.materializing = false;
        self.context
    }
    pub fn register(&mut self, owner: &str, module: &Module) -> Result<(), ConstLoweringError> {
        if self.context.registry.is_none() {
            self.context.registry = Some(Arc::new(MetaRegistry::new()));
        }
        self.register_items(owner, &module.items)
            .map_err(|error| ConstLoweringError {
                owner: owner.into(),
                span: module.span,
                error,
            })
    }
    fn register_items(&mut self, owner: &str, items: &[Item]) -> Result<(), MetaError> {
        for item in items {
            match &item.kind {
                ItemKind::Function(f) if f.is_meta && f.generics.is_empty() => {
                    Arc::make_mut(self.context.registry.as_mut().unwrap())
                        .register_meta_function(&Text::from(owner), f)
                        .map_err(|e| failure(e.to_string()))?
                }
                ItemKind::Function(f) => {
                    self.context
                        .source_constants
                        .runtime_declarations
                        .insert((owner.into(), f.name.as_str().into()));
                }
                ItemKind::Static(c) => {
                    self.context
                        .source_constants
                        .runtime_declarations
                        .insert((owner.into(), c.name.as_str().into()));
                }
                ItemKind::Const(c) if c.generics.is_empty() => {
                    let key = (Text::from(owner), Text::from(c.name.as_str()));
                    if let Some(previous) = self.context.source_constants.declarations.get(&key) {
                        if previous != &c.value {
                            return Err(failure(format!(
                                "conflicting compile-time constant {}.{}",
                                owner, c.name
                            )));
                        }
                    } else {
                        self.context
                            .source_constants
                            .declarations
                            .insert(key, c.value.clone());
                    }
                }
                ItemKind::Const(c) => {
                    // A generic constant still occupies its declaration name.
                    // Its body is deferred; it cannot expose a same-named builtin.
                    self.context
                        .source_constants
                        .runtime_declarations
                        .insert((owner.into(), c.name.as_str().into()));
                }
                ItemKind::Module(m) => {
                    if let Some(items) = &m.items {
                        self.register_items(&child_owner(owner, m.name.as_str()), items)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    pub fn lower(&mut self, owner: &str, module: &mut Module) -> Result<(), ConstLoweringError> {
        for item in &mut module.items {
            self.failure_span = None;
            self.item(owner, item).map_err(|error| ConstLoweringError {
                owner: owner.into(),
                span: self.failure_span.unwrap_or(item.span),
                error,
            })?;
        }
        Ok(())
    }
    fn item(&mut self, owner: &str, item: &mut Item) -> Result<(), MetaError> {
        match &mut item.kind {
            ItemKind::Function(f) => self.function(owner, f)?,
            ItemKind::Const(c) if c.generics.is_empty() => {
                let key = (Text::from(owner), Text::from(c.name.as_str()));
                if let Some(value) = self.context.source_constants.values.get(&key).cloned() {
                    c.value = materialize_runtime_value(value, c.value.span)?;
                } else {
                    self.expr(owner, &mut c.value, &Set::new())?;
                }
                self.context
                    .source_constants
                    .declarations
                    .insert(key, c.value.clone());
            }
            ItemKind::Const(_) => return Ok(()), // instantiation owns open generic obligations
            ItemKind::Static(c) => self.expr(owner, &mut c.value, &Set::new())?,
            ItemKind::Impl(i) if i.generics.is_empty() => {
                for member in &mut i.items {
                    match &mut member.kind {
                        ImplItemKind::Function(f) => self.function(owner, f)?,
                        ImplItemKind::Const { value, .. } => {
                            self.expr(owner, value, &Set::new())?
                        }
                        _ => {}
                    }
                }
            }
            ItemKind::Impl(_) => return Ok(()),
            ItemKind::Module(m) => {
                if let Some(items) = &mut m.items {
                    let owner = child_owner(owner, m.name.as_str());
                    for item in items {
                        self.item(&owner, item)?;
                    }
                }
            }
            _ => {}
        }
        // Types/defaults/proof carriers without a staging rule are explicit
        // refusals. Adding a new AST child cannot silently skip an occurrence.
        let mut pending = PendingConst(false);
        pending.visit_item(item);
        if pending.0 {
            return Err(failure(format!(
                "unmaterialized @const in unsupported declaration carrier (owner {owner})"
            )));
        }
        Ok(())
    }
    fn function(&mut self, owner: &str, f: &mut FunctionDecl) -> Result<(), MetaError> {
        if f.is_meta || !f.generics.is_empty() {
            return Ok(());
        }
        let mut bound = Set::new();
        for param in &mut f.params {
            match &mut param.kind {
                FunctionParamKind::Regular {
                    pattern,
                    default_value,
                    ..
                } => {
                    if let Some(value) = default_value {
                        self.expr(owner, value, &bound)?;
                    }
                    add_pattern(pattern, &mut bound);
                }
                _ => {
                    bound.insert(Text::from("self"));
                }
            }
        }
        if let Some(body) = &mut f.body {
            match body {
                FunctionBody::Block(block) => self.block(owner, block, &bound)?,
                FunctionBody::Expr(expr) => self.expr(owner, expr, &bound)?,
            }
        }
        Ok(())
    }
    fn block(
        &mut self,
        owner: &str,
        block: &mut Block,
        outer: &Set<Text>,
    ) -> Result<(), MetaError> {
        let mut bound = outer.clone();
        for stmt in &mut block.stmts {
            match &mut stmt.kind {
                StmtKind::Let { pattern, value, .. } => {
                    if let Some(value) = value {
                        self.expr(owner, value, &bound)?;
                    }
                    add_pattern(pattern, &mut bound);
                }
                StmtKind::LetElse {
                    pattern,
                    value,
                    else_block,
                    ..
                } => {
                    self.expr(owner, value, &bound)?;
                    self.block(owner, else_block, &bound)?;
                    add_pattern(pattern, &mut bound);
                }
                StmtKind::Expr { expr, .. } => {
                    self.expr(owner, expr, &bound)?;
                    add_condition_bindings(expr, &mut bound);
                }
                StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
                    self.expr(owner, expr, &bound)?
                }
                StmtKind::Provide {
                    context,
                    alias,
                    value,
                } => {
                    self.expr(owner, value, &bound)?;
                    bound.insert(alias.as_ref().unwrap_or(context).clone());
                }
                StmtKind::ProvideScope {
                    context,
                    alias,
                    value,
                    block,
                } => {
                    self.expr(owner, value, &bound)?;
                    let mut inner = bound.clone();
                    inner.insert(alias.as_ref().unwrap_or(context).clone());
                    self.expr(owner, block, &inner)?;
                }
                StmtKind::Item(item) => {
                    let mut pending = PendingConst(false);
                    pending.visit_item(item);
                    if pending.0 {
                        return Err(failure(
                            "@const in a local declaration requires a lexical constant scope",
                        ));
                    }
                    match &item.kind {
                        ItemKind::Const(c) => {
                            bound.insert(Text::from(c.name.as_str()));
                        }
                        ItemKind::Static(c) => {
                            bound.insert(Text::from(c.name.as_str()));
                        }
                        ItemKind::Function(f) => {
                            bound.insert(Text::from(f.name.as_str()));
                        }
                        _ => {}
                    }
                }
                StmtKind::Empty => {}
            }
        }
        if let Some(expr) = &mut block.expr {
            self.expr(owner, expr, &bound)?;
        }
        Ok(())
    }
    fn expr(&mut self, owner: &str, expr: &mut Expr, bound: &Set<Text>) -> Result<(), MetaError> {
        match &mut expr.kind {
            ExprKind::Quote { .. } => return Ok(()),
            ExprKind::MetaFunction { name, args, .. } if name.as_str() == "const" => {
                self.failure_span = Some(expr.span);
                if args.len() != 1 {
                    return Err(MetaError::ArityMismatch {
                        expected: 1,
                        got: args.len(),
                    });
                }
                self.context.current_module = Text::from(owner);
                self.context.bindings.clear();
                self.context.source_constants.runtime_bindings = bound.clone();
                self.context.current_stage = 1;
                let ir = self.context.ast_expr_to_meta_expr(&args[0])?;
                let value = self.context.eval_meta_expr(&ir)?;
                *expr = materialize_runtime_value(value, expr.span)?;
                self.failure_span = None;
                return Ok(());
            }
            ExprKind::Block(block) | ExprKind::Async(block) | ExprKind::Unsafe(block) => {
                return self.block(owner, block, bound);
            }
            ExprKind::Binary {
                op: BinOp::And,
                left: lhs,
                right: rhs,
            } => {
                self.expr(owner, lhs, bound)?;
                let mut inner = bound.clone();
                add_condition_bindings(lhs, &mut inner);
                return self.expr(owner, rhs, &inner);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut inner = bound.clone();
                for condition in &mut condition.conditions {
                    match condition {
                        ConditionKind::Expr(e) => {
                            self.expr(owner, e, &inner)?;
                            add_condition_bindings(e, &mut inner);
                        }
                        ConditionKind::Let { pattern, value } => {
                            self.expr(owner, value, &inner)?;
                            add_pattern(pattern, &mut inner);
                        }
                    }
                }
                self.block(owner, then_branch, &inner)?;
                if let Some(e) = else_branch {
                    self.expr(owner, e, bound)?;
                }
                return Ok(());
            }
            ExprKind::Closure { params, body, .. } => {
                let mut inner = bound.clone();
                for param in params {
                    add_pattern(&param.pattern, &mut inner);
                }
                return self.expr(owner, body, &inner);
            }
            ExprKind::Match { expr: value, arms } => {
                self.expr(owner, value, bound)?;
                for arm in arms {
                    let mut inner = bound.clone();
                    add_pattern(&arm.pattern, &mut inner);
                    if let Some(guard) = &mut arm.guard {
                        self.expr(owner, guard, &inner)?;
                    }
                    if let Some(with) = &mut arm.with_clause {
                        for e in with {
                            self.expr(owner, e, &inner)?;
                        }
                    }
                    self.expr(owner, &mut arm.body, &inner)?;
                }
                return Ok(());
            }
            ExprKind::For {
                pattern,
                iter,
                body,
                invariants,
                decreases,
                ..
            }
            | ExprKind::ForAwait {
                pattern,
                async_iterable: iter,
                body,
                invariants,
                decreases,
                ..
            } => {
                self.expr(owner, iter, bound)?;
                let mut inner = bound.clone();
                add_pattern(pattern, &mut inner);
                for e in invariants.iter_mut().chain(decreases.iter_mut()) {
                    self.expr(owner, e, &inner)?;
                }
                return self.block(owner, body, &inner);
            }
            ExprKind::Loop {
                body, invariants, ..
            } => {
                for e in invariants {
                    self.expr(owner, e, bound)?;
                }
                return self.block(owner, body, bound);
            }
            ExprKind::While {
                condition,
                body,
                invariants,
                decreases,
                ..
            } => {
                self.expr(owner, condition, bound)?;
                let mut inner = bound.clone();
                add_condition_bindings(condition, &mut inner);
                for e in invariants.iter_mut().chain(decreases.iter_mut()) {
                    self.expr(owner, e, &inner)?;
                }
                return self.block(owner, body, &inner);
            }

            _ => {}
        }
        if let Some(binder) = verum_ast::visit_mut::binder_name(&expr.kind) {
            let mut pending = PendingConst(false);
            pending.visit_expr(expr);
            if pending.0 {
                return Err(failure(format!(
                    "@const inside unsupported scoped carrier `{binder}` (owner {owner})"
                )));
            }
            return Ok(());
        }
        let mut result = Ok(());
        verum_ast::visit_mut::each_child_expr_mut(expr, &mut |child| {
            if result.is_ok() {
                result = self.expr(owner, child, bound);
            }
        });
        result
    }
}
fn child_owner(parent: &str, child: &str) -> Text {
    if parent.is_empty() {
        Text::from(child)
    } else {
        format!("{parent}.{child}").into()
    }
}
fn add_condition_bindings(expr: &Expr, names: &mut Set<Text>) {
    struct Bindings<'a>(&'a mut Set<Text>);
    impl Visitor for Bindings<'_> {
        fn visit_expr(&mut self, expr: &Expr) {
            match &expr.kind {
                ExprKind::Is { pattern, .. } => add_pattern(pattern, self.0),
                ExprKind::Quote { .. } | ExprKind::Closure { .. } | ExprKind::Block(_) => return,
                _ => visitor::walk_expr(self, expr),
            }
        }
    }
    Bindings(names).visit_expr(expr);
}
fn add_pattern(pattern: &Pattern, names: &mut Set<Text>) {
    struct Names<'a>(&'a mut Set<Text>);
    impl Visitor for Names<'_> {
        fn visit_expr(&mut self, _: &Expr) {} // View/guard expressions introduce no pattern bindings.
        fn visit_pattern(&mut self, p: &Pattern) {
            match &p.kind {
                PatternKind::Ident { name, .. } | PatternKind::TypeTest { binding: name, .. } => {
                    self.0.insert(Text::from(name.as_str()));
                }
                PatternKind::Record { fields, .. }
                | PatternKind::Variant {
                    data: Some(VariantPatternData::Record { fields, .. }),
                    ..
                } => {
                    for field in fields {
                        if field.pattern.is_none() {
                            self.0.insert(field.name.as_str().into());
                        }
                    }
                }
                PatternKind::Stream {
                    rest: Some(name), ..
                } => {
                    self.0.insert(name.as_str().into());
                }
                _ => {}
            }
            visitor::walk_pattern(self, p);
        }
    }
    Names(names).visit_pattern(pattern);
}
struct PendingConst(bool);
impl Visitor for PendingConst {
    fn visit_function(&mut self, f: &FunctionDecl) {
        if !f.is_meta && f.generics.is_empty() {
            visitor::walk_function(self, f);
        }
    }
    fn visit_impl(&mut self, i: &ImplDecl) {
        if i.generics.is_empty() {
            visitor::walk_impl(self, i);
        }
    }
    fn visit_item(&mut self, item: &Item) {
        if matches!(&item.kind, ItemKind::Const(c) if !c.generics.is_empty()) {
            return;
        }
        visitor::walk_item(self, item);
    }
    fn visit_expr(&mut self, expr: &Expr) {
        if matches!(&expr.kind, ExprKind::Quote { .. }) {
            return;
        }
        if matches!(&expr.kind, ExprKind::MetaFunction { name, .. } if name.as_str() == "const") {
            self.0 = true;
            return;
        }
        visitor::walk_expr(self, expr);
    }
}
