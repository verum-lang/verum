//! Simultaneous term substitution with explicit lexical scopes.
//!
//! Inserted arguments stay in the caller's scope. Local binders that could
//! capture them are renamed before visiting their bodies. A construct without
//! a scope/transport rule returns an error rather than a partially rewritten VC.
use crate::refinement::RefinementError;
use verum_ast::expr::{
    Block, ComprehensionClause, ComprehensionClauseKind, ConditionKind, RecoverBody,
};
use verum_ast::pattern::{FieldPattern, MatchArm, Pattern, PatternKind, VariantPatternData};
use verum_ast::visitor::{self, Visitor};
use verum_ast::{Expr, ExprKind, Ident, Path, PathSegment, Span, StmtKind, Type, TypeKind};
use verum_common::{Map, Maybe, Set, Text};

type Result<T> = std::result::Result<T, RefinementError>;

#[derive(Clone)]
enum Replacement<'a> {
    Argument(&'a Expr),
    Local(Text),
}
type Scope<'a> = Map<Text, Replacement<'a>>;

#[derive(Default)]
struct Names(Set<Text>);
impl Visitor for Names {
    fn visit_expr(&mut self, expr: &Expr) {
        // A shorthand field is a variable occurrence even though it has no
        // child expression in the surface AST.
        if let ExprKind::Record { fields, .. } = &expr.kind {
            for field in fields {
                if field.value.is_none() {
                    self.visit_ident(&field.name);
                }
            }
        }
        visitor::walk_expr(self, expr);
    }

    fn visit_ident(&mut self, ident: &Ident) {
        self.0.insert(ident.name.clone());
    }
    fn visit_path(&mut self, path: &Path) {
        for part in &path.segments {
            match part {
                PathSegment::Name(ident) => self.visit_ident(ident),
                PathSegment::SelfValue => {
                    self.0.insert("self".into());
                }
                _ => {}
            }
        }
    }
}

struct Substitution {
    used: Set<Text>,
    argument_names: Set<Text>,
    next_name: usize,
    depth: usize,
}

pub(crate) fn substitute(expr: &Expr, replacements: &[(Text, &Expr)]) -> Result<Expr> {
    if replacements.is_empty() {
        return Ok(expr.clone());
    }
    let mut names = Names::default();
    names.visit_expr(expr);
    let mut argument_names = Names::default();
    let mut scope = Scope::new();
    for (name, value) in replacements {
        if scope
            .insert(name.clone(), Replacement::Argument(value))
            .is_some()
        {
            return Err(RefinementError::new(
                format!("duplicate substitution for `{name}`").into(),
                expr.span,
            ));
        }
        names.0.insert(name.clone());
        argument_names.visit_expr(value);
    }
    names.0.extend(argument_names.0.iter().cloned());
    let mut substitution = Substitution {
        used: names.0,
        argument_names: argument_names.0,
        next_name: 0,
        depth: 0,
    };
    let mut result = expr.clone();
    substitution.expr(&mut result, &scope)?;
    Ok(result)
}

impl Substitution {
    fn unsupported(&self, form: &str, span: Span) -> RefinementError {
        RefinementError::new(
            format!(
                "cannot transport a refinement through {form}: a checked scope rule is required"
            )
            .into(),
            span,
        )
    }

    fn bind_name(&mut self, name: &mut Text, scope: &mut Scope<'_>, group: &mut Map<Text, Text>) {
        let original = name.clone();
        if let Some(renamed) = group.get(&original) {
            *name = renamed.clone();
            return;
        }
        scope.remove(&original);
        if self.argument_names.contains(&original) {
            let fresh = loop {
                let candidate: Text = format!("__refinement_local_{}", self.next_name).into();
                self.next_name += 1;
                if self.used.insert(candidate.clone()) {
                    break candidate;
                }
            };
            scope.insert(original.clone(), Replacement::Local(fresh.clone()));
            *name = fresh;
        }
        group.insert(original, name.clone());
    }

    fn pattern(
        &mut self,
        pattern: &mut Pattern,
        scope: &mut Scope<'_>,
        group: &mut Map<Text, Text>,
    ) -> Result<()> {
        match &mut pattern.kind {
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                self.bind_name(&mut name.name, scope, group);
                if let Some(inner) = subpattern {
                    self.pattern(inner, scope, group)?;
                }
            }
            PatternKind::Tuple(items)
            | PatternKind::Array(items)
            | PatternKind::And(items)
            | PatternKind::Or(items) => {
                for item in items {
                    self.pattern(item, scope, group)?;
                }
            }
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                for item in before {
                    self.pattern(item, scope, group)?;
                }
                if let Some(inner) = rest {
                    self.pattern(inner, scope, group)?;
                }
                for item in after {
                    self.pattern(item, scope, group)?;
                }
            }
            PatternKind::Record { fields, .. } => self.fields(fields, scope, group)?,
            PatternKind::Variant { data, .. } => match data {
                Some(VariantPatternData::Tuple(items)) => {
                    for item in items {
                        self.pattern(item, scope, group)?;
                    }
                }
                Some(VariantPatternData::Record { fields, .. }) => {
                    self.fields(fields, scope, group)?
                }
                None => {}
            },
            PatternKind::Reference { inner, .. } | PatternKind::Paren(inner) => {
                self.pattern(inner, scope, group)?
            }
            PatternKind::Cons { head, tail } => {
                self.pattern(head, scope, group)?;
                self.pattern(tail, scope, group)?;
            }
            PatternKind::Stream {
                head_patterns,
                rest,
            } => {
                for item in head_patterns {
                    self.pattern(item, scope, group)?;
                }
                if let Some(name) = rest {
                    self.bind_name(&mut name.name, scope, group);
                }
            }
            PatternKind::TypeTest { binding, test_type } => {
                self.annotation(test_type, scope)?;
                self.bind_name(&mut binding.name, scope, group);
            }
            PatternKind::Wildcard
            | PatternKind::Rest
            | PatternKind::Literal(_)
            | PatternKind::Range { .. } => {}
            // These forms evaluate expressions while matching. Their binding
            // order must come from the pattern elaborator, not a structural walk.
            PatternKind::View { .. } | PatternKind::Active { .. } | PatternKind::Guard { .. } => {
                return Err(self.unsupported("an expression-bearing pattern", pattern.span));
            }
        }
        Ok(())
    }

    fn fields(
        &mut self,
        fields: &mut [FieldPattern],
        scope: &mut Scope<'_>,
        group: &mut Map<Text, Text>,
    ) -> Result<()> {
        for field in fields {
            if let Some(pattern) = &mut field.pattern {
                self.pattern(pattern, scope, group)?;
            } else {
                let mut name = field.name.clone();
                self.bind_name(&mut name.name, scope, group);
                if name.name != field.name.name {
                    field.pattern = Some(Pattern::ident(name, false, field.span));
                }
            }
        }
        Ok(())
    }

    fn enter_pattern(&mut self, pattern: &mut Pattern, scope: &mut Scope<'_>) -> Result<()> {
        self.pattern(pattern, scope, &mut Map::new())
    }

    /// A term-only transform has no authority to reinterpret a type slot as
    /// a value slot. Refuse dependent annotations needing such transport.
    fn annotation(&self, ty: &Type, scope: &Scope<'_>) -> Result<()> {
        struct Dependencies<'a, 'b> {
            scope: &'a Scope<'b>,
            bound: Set<Text>,
            found: Option<Span>,
        }
        impl Visitor for Dependencies<'_, '_> {
            fn visit_type(&mut self, ty: &Type) {
                if matches!(
                    ty.kind,
                    TypeKind::Rank2Function { .. }
                        | TypeKind::TypeLambda { .. }
                        | TypeKind::Existential { .. }
                        | TypeKind::Bounded { .. }
                        | TypeKind::DynProtocol { .. }
                        | TypeKind::TypeConstructor { .. }
                ) {
                    // The structural visitor intentionally skips some bounds
                    // and declaration telescopes. Such nodes require an
                    // elaborated type transport rule, not a dependency guess.
                    self.found = Some(ty.span);
                } else if let TypeKind::Refined { base, predicate } = &ty.kind {
                    self.visit_type(base);
                    let saved = self.bound.clone();
                    if let Some(name) = &predicate.binding {
                        self.bound.insert(name.name.clone());
                    } else {
                        self.bound.insert("it".into());
                        self.bound.insert("self".into());
                    }
                    self.visit_expr(&predicate.expr);
                    self.bound = saved;
                } else {
                    visitor::walk_type(self, ty);
                }
            }
            fn visit_path(&mut self, path: &Path) {
                let name = match path.segments.first() {
                    Some(PathSegment::Name(name)) => &name.name,
                    Some(PathSegment::SelfValue) => &Text::from("self"),
                    _ => return,
                };
                if self.scope.contains_key(name) && !self.bound.contains(name) {
                    self.found = Some(path.span);
                }
            }
        }
        let mut dependencies = Dependencies {
            scope,
            bound: Set::new(),
            found: None,
        };
        dependencies.visit_type(ty);
        if let Some(span) = dependencies.found {
            return Err(self.unsupported("a dependent type annotation", span));
        }
        Ok(())
    }

    fn optional_type(&self, ty: &Maybe<Type>, scope: &Scope<'_>) -> Result<()> {
        if let Some(ty) = ty {
            self.annotation(ty, scope)?;
        }
        Ok(())
    }

    fn block(&mut self, block: &mut Block, parent: &Scope<'_>) -> Result<()> {
        let mut scope = parent.clone();
        for statement in &mut block.stmts {
            match &mut statement.kind {
                StmtKind::Let { pattern, ty, value } => {
                    self.optional_type(ty, &scope)?;
                    if let Some(value) = value {
                        self.expr(value, &scope)?;
                    }
                    self.enter_pattern(pattern, &mut scope)?;
                }
                StmtKind::LetElse {
                    pattern,
                    ty,
                    value,
                    else_block,
                } => {
                    self.optional_type(ty, &scope)?;
                    self.expr(value, &scope)?;
                    self.block(else_block, &scope)?;
                    self.enter_pattern(pattern, &mut scope)?;
                }
                StmtKind::Expr { expr, .. } | StmtKind::Defer(expr) | StmtKind::Errdefer(expr) => {
                    self.expr(expr, &scope)?
                }
                StmtKind::Provide { alias, value, .. } => {
                    self.expr(value, &scope)?;
                    if let Some(name) = alias {
                        self.bind_name(name, &mut scope, &mut Map::new());
                    }
                }
                StmtKind::ProvideScope {
                    alias,
                    value,
                    block,
                    ..
                } => {
                    self.expr(value, &scope)?;
                    let mut inner = scope.clone();
                    if let Some(name) = alias {
                        self.bind_name(name, &mut inner, &mut Map::new());
                    }
                    self.expr(block, &inner)?;
                }
                StmtKind::Item(_) => {
                    return Err(self.unsupported("a local declaration", statement.span));
                }
                StmtKind::Empty => {}
            }
        }
        if let Some(tail) = &mut block.expr {
            self.expr(tail, &scope)?;
        }
        Ok(())
    }

    fn condition(&mut self, condition: &mut Expr, scope: &mut Scope<'_>) -> Result<()> {
        match &mut condition.kind {
            ExprKind::Is {
                expr,
                pattern,
                negated: false,
            } => {
                self.expr(expr, scope)?;
                self.enter_pattern(pattern, scope)
            }
            ExprKind::Binary {
                op: verum_ast::BinOp::And,
                left,
                right,
            } => {
                self.condition(left, scope)?;
                self.condition(right, scope)
            }
            _ => self.expr(condition, scope),
        }
    }

    fn arms(&mut self, arms: &mut [MatchArm], parent: &Scope<'_>) -> Result<()> {
        for arm in arms {
            let mut scope = parent.clone();
            self.enter_pattern(&mut arm.pattern, &mut scope)?;
            if let Some(guard) = &mut arm.guard {
                self.expr(guard, &scope)?;
            }
            if let Some(with) = &mut arm.with_clause {
                for expression in with {
                    self.expr(expression, &scope)?;
                }
            }
            self.expr(&mut arm.body, &scope)?;
        }
        Ok(())
    }

    fn recover(&mut self, recover: &mut RecoverBody, parent: &Scope<'_>) -> Result<()> {
        match recover {
            RecoverBody::MatchArms { arms, .. } => self.arms(arms, parent),
            RecoverBody::Closure { param, body, .. } => {
                self.optional_type(&param.ty, parent)?;
                let mut scope = parent.clone();
                self.enter_pattern(&mut param.pattern, &mut scope)?;
                self.expr(body, &scope)
            }
        }
    }

    fn clauses(
        &mut self,
        clauses: &mut [ComprehensionClause],
        scope: &mut Scope<'_>,
    ) -> Result<()> {
        for clause in clauses {
            match &mut clause.kind {
                ComprehensionClauseKind::For { pattern, iter } => {
                    self.expr(iter, scope)?;
                    self.enter_pattern(pattern, scope)?;
                }
                ComprehensionClauseKind::Let { pattern, ty, value } => {
                    self.optional_type(ty, scope)?;
                    self.expr(value, scope)?;
                    self.enter_pattern(pattern, scope)?;
                }
                ComprehensionClauseKind::If(condition) => self.condition(condition, scope)?,
            }
        }
        Ok(())
    }

    fn expr(&mut self, expr: &mut Expr, scope: &Scope<'_>) -> Result<()> {
        if scope.is_empty() {
            return Ok(());
        }
        if self.depth >= 256 {
            return Err(self.unsupported("the expression depth limit", expr.span));
        }
        self.depth += 1;
        let result = self.expr_inner(expr, scope);
        self.depth -= 1;
        result
    }

    fn expr_inner(&mut self, expr: &mut Expr, scope: &Scope<'_>) -> Result<()> {
        match &mut expr.kind {
            ExprKind::Path(path) if path.segments.len() == 1 => {
                let replacement = match &path.segments[0] {
                    PathSegment::Name(name) => scope.get(&name.name),
                    PathSegment::SelfValue => scope.get(&Text::from("self")),
                    _ => return Ok(()),
                };
                if let Some(replacement) = replacement {
                    match replacement {
                        Replacement::Argument(value) => *expr = (*value).clone(),
                        Replacement::Local(name) => {
                            expr.kind =
                                ExprKind::Path(Path::single(Ident::new(name.clone(), expr.span)));
                        }
                    }
                }
                return Ok(());
            }
            ExprKind::Block(block)
            | ExprKind::Async(block)
            | ExprKind::Unsafe(block)
            | ExprKind::Meta(block) => return self.block(block, scope),
            ExprKind::Closure {
                params,
                return_type,
                body,
                ..
            } => {
                let mut inner = scope.clone();
                for parameter in params {
                    self.optional_type(&parameter.ty, &inner)?;
                    self.enter_pattern(&mut parameter.pattern, &mut inner)?;
                }
                self.optional_type(return_type, &inner)?;
                return self.expr(body, &inner);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let mut inner = scope.clone();
                for condition in &mut condition.conditions {
                    match condition {
                        ConditionKind::Expr(expr) => self.condition(expr, &mut inner)?,
                        ConditionKind::Let { pattern, value } => {
                            self.expr(value, &inner)?;
                            self.enter_pattern(pattern, &mut inner)?;
                        }
                    }
                }
                self.block(then_branch, &inner)?;
                if let Some(other) = else_branch {
                    self.expr(other, scope)?;
                }
                return Ok(());
            }
            ExprKind::Match { expr, arms } => {
                self.expr(expr, scope)?;
                return self.arms(arms, scope);
            }
            ExprKind::Forall { bindings, body } | ExprKind::Exists { bindings, body } => {
                let mut inner = scope.clone();
                for binding in bindings {
                    self.optional_type(&binding.ty, &inner)?;
                    if let Some(domain) = &mut binding.domain {
                        self.expr(domain, &inner)?;
                    }
                    self.enter_pattern(&mut binding.pattern, &mut inner)?;
                    if let Some(guard) = &mut binding.guard {
                        self.expr(guard, &inner)?;
                    }
                }
                return self.expr(body, &inner);
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
                self.expr(iter, scope)?;
                let mut inner = scope.clone();
                self.enter_pattern(pattern, &mut inner)?;
                self.block(body, &inner)?;
                for clause in invariants.iter_mut().chain(decreases.iter_mut()) {
                    self.expr(clause, &inner)?;
                }
                return Ok(());
            }
            ExprKind::Loop {
                body, invariants, ..
            } => {
                self.block(body, scope)?;
                for invariant in invariants {
                    self.expr(invariant, scope)?;
                }
                return Ok(());
            }
            ExprKind::While {
                condition,
                body,
                invariants,
                decreases,
                ..
            } => {
                let mut inner = scope.clone();
                self.condition(condition, &mut inner)?;
                self.block(body, &inner)?;
                for clause in invariants.iter_mut().chain(decreases.iter_mut()) {
                    self.expr(clause, &inner)?;
                }
                return Ok(());
            }
            ExprKind::TryRecover { try_block, recover } => {
                self.expr(try_block, scope)?;
                return self.recover(recover, scope);
            }
            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.expr(try_block, scope)?;
                self.recover(recover, scope)?;
                return self.expr(finally_block, scope);
            }
            ExprKind::Select { arms, .. } => {
                for arm in arms {
                    if let Some(future) = &mut arm.future {
                        self.expr(future, scope)?;
                    }
                    let mut inner = scope.clone();
                    if let Some(pattern) = &mut arm.pattern {
                        self.enter_pattern(pattern, &mut inner)?;
                    }
                    if let Some(guard) = &mut arm.guard {
                        self.expr(guard, &inner)?;
                    }
                    self.expr(&mut arm.body, &inner)?;
                }
                return Ok(());
            }
            ExprKind::Nursery {
                options,
                body,
                on_cancel,
                recover,
                ..
            } => {
                if let Some(timeout) = &mut options.timeout {
                    self.expr(timeout, scope)?;
                }
                if let Some(max_tasks) = &mut options.max_tasks {
                    self.expr(max_tasks, scope)?;
                }
                self.block(body, scope)?;
                if let Some(block) = on_cancel {
                    self.block(block, scope)?;
                }
                if let Some(recover) = recover {
                    self.recover(recover, scope)?;
                }
                return Ok(());
            }
            ExprKind::Comprehension {
                expr: body,
                clauses,
            }
            | ExprKind::StreamComprehension {
                expr: body,
                clauses,
            }
            | ExprKind::SetComprehension {
                expr: body,
                clauses,
            }
            | ExprKind::GeneratorComprehension {
                expr: body,
                clauses,
            } => {
                let mut inner = scope.clone();
                self.clauses(clauses, &mut inner)?;
                return self.expr(body, &inner);
            }
            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => {
                let mut inner = scope.clone();
                self.clauses(clauses, &mut inner)?;
                self.expr(key_expr, &inner)?;
                return self.expr(value_expr, &inner);
            }
            ExprKind::Record { fields, .. } => {
                for field in fields {
                    if field.value.is_none() && scope.contains_key(&field.name.name) {
                        field.value = Some(Expr::ident(field.name.clone()));
                    }
                }
            }
            ExprKind::Call { type_args, .. } | ExprKind::MethodCall { type_args, .. } => {
                for argument in type_args {
                    if let verum_ast::GenericArg::Const(value) = argument {
                        self.expr(value, scope)?;
                    }
                }
            }
            ExprKind::Is {
                expr: value,
                pattern,
                ..
            } => {
                self.expr(value, scope)?;
                return self.enter_pattern(pattern, &mut scope.clone());
            }
            ExprKind::Quote { .. }
            | ExprKind::MacroCall { .. }
            | ExprKind::InlineAsm { .. }
            | ExprKind::CalcBlock(_)
            | ExprKind::DestructuringAssign { .. } => {
                return Err(self.unsupported("an unelaborated code or assignment form", expr.span));
            }
            _ => {}
        }
        if let Some(form) = verum_ast::visit_mut::binder_name(&expr.kind) {
            return Err(self.unsupported(form, expr.span));
        }
        // Preserve explicit type arguments and reject transport which needs
        // declaration-owned type/value slot information rather than guessing it.
        struct Annotations<'a, 'b, 'c> {
            substitution: &'a Substitution,
            scope: &'b Scope<'c>,
            error: Option<RefinementError>,
        }
        impl Visitor for Annotations<'_, '_, '_> {
            fn visit_expr(&mut self, _: &Expr) {}
            fn visit_type(&mut self, ty: &Type) {
                if self.error.is_none() {
                    self.error = self.substitution.annotation(ty, self.scope).err();
                }
            }
        }
        let mut annotations = Annotations {
            substitution: self,
            scope,
            error: None,
        };
        visitor::walk_expr(&mut annotations, expr);
        if let Some(error) = annotations.error {
            return Err(error);
        }
        let mut result = Ok(());
        verum_ast::visit_mut::each_child_expr_mut(expr, &mut |child| {
            if result.is_ok() {
                result = self.expr(child, scope);
            }
        });
        result
    }
}
