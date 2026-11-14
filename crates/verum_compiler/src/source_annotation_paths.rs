//! Free source paths of a retained nominal declaration.
//!
//! Archive decoding validates the complete carrier with
//! `validate_source_type_declaration` before this private graph walk. Its shared
//! depth/node budget bounds these small structural visitors. Scopes are lexical:
//! a binding removes paths only in its own body, never throughout a declaration.
use verum_ast::{
    Block, Expr, ExprKind, FunctionDecl, FunctionParam, FunctionParamKind, Pattern, PatternKind,
    Stmt, StmtKind, Type, TypeDecl, TypeKind, Visitor,
    decl::{ProtocolItemKind, RecordField, TypeDeclBody, VariantData},
    expr::{ComprehensionClause, ComprehensionClauseKind, ConditionKind, RecoverBody},
    pattern::{FieldPattern, VariantPatternData},
    ty::{GenericParam, GenericParamKind, Path, PathSegment, TypeBound, TypeBoundKind},
};
use verum_common::{List, Set, Text};

#[derive(Clone, Copy, Default)]
enum Domain {
    #[default]
    Value,
    Type,
}
#[derive(Default)]
struct Scope {
    values: Set<Text>,
    types: Set<Text>,
}
#[derive(Default)]
struct Paths {
    names: Set<Text>,
    scopes: List<Scope>,
    domain: Domain,
}

pub(super) fn free_paths(source: &TypeDecl) -> Set<Text> {
    let mut paths = Paths::default();
    paths.visit_type_decl(source);
    paths.names
}

// Keep rooted path segments; `super.helper` and a bare `helper` are distinct.
fn expression_path(mut expression: &Expr) -> Option<Text> {
    let mut suffix = List::new();
    loop {
        match &expression.kind {
            ExprKind::Field { expr, field } => {
                suffix.push(field.name.as_str());
                expression = expr;
            }
            ExprKind::Path(path) => {
                let mut name = path.to_string().replace("::", ".");
                for field in suffix.into_iter().rev() {
                    name.push('.');
                    name.push_str(field);
                }
                return Some(name.into());
            }
            _ => return None,
        }
    }
}

impl Paths {
    fn scoped(&mut self, visit: impl FnOnce(&mut Self)) {
        self.scopes.push(Scope::default());
        visit(self);
        self.scopes.pop();
    }
    fn bind_value(&mut self, name: &str) {
        self.scopes
            .last_mut()
            .expect("lexical scope")
            .values
            .insert(name.into());
    }
    fn bind_type(&mut self, name: &str) {
        self.bind_value(name);
        self.scopes
            .last_mut()
            .expect("lexical scope")
            .types
            .insert(name.into());
    }
    fn bound(&self, name: &Text) -> bool {
        self.scopes.iter().rev().any(|scope| match self.domain {
            Domain::Value => scope.values.contains(name),
            Domain::Type => scope.types.contains(name),
        })
    }
    fn name(&mut self, name: Text) {
        let head: Text = name.as_str().split('.').next().unwrap_or_default().into();
        if !name.is_empty() && !self.bound(&head) {
            self.names.insert(name);
        }
    }
    fn type_path(&mut self, path: &Path) {
        let saved = self.domain;
        self.domain = Domain::Type;
        self.visit_path(path);
        self.domain = saved;
    }
    fn generics(&mut self, generics: &[GenericParam]) {
        // The source signature's generic telescope is predeclared in its own
        // scope. Meta parameters can appear as a path-shaped generic argument;
        // their declared role remains local even before argument elaboration.
        for parameter in generics {
            match &parameter.kind {
                GenericParamKind::Type { name, .. }
                | GenericParamKind::HigherKinded { name, .. }
                | GenericParamKind::KindAnnotated { name, .. }
                | GenericParamKind::Const { name, .. }
                | GenericParamKind::Meta { name, .. }
                | GenericParamKind::Context { name }
                | GenericParamKind::Level { name } => self.bind_type(&name.name),
                GenericParamKind::Lifetime { .. } => {}
            }
        }
        for parameter in generics {
            match &parameter.kind {
                GenericParamKind::Type {
                    bounds, default, ..
                } => {
                    self.bounds(bounds);
                    if let Some(ty) = default {
                        self.visit_type(ty);
                    }
                }
                GenericParamKind::HigherKinded { bounds, .. }
                | GenericParamKind::KindAnnotated { bounds, .. } => self.bounds(bounds),
                GenericParamKind::Const { ty, .. } => self.visit_type(ty),
                GenericParamKind::Meta { ty, refinement, .. } => {
                    self.visit_type(ty);
                    if let Some(expr) = refinement {
                        self.visit_expr(expr);
                    }
                }
                _ => {}
            }
        }
    }
    fn bounds(&mut self, bounds: &[TypeBound]) {
        let mut pending: List<_> = bounds.iter().collect();
        while let Some(bound) = pending.pop() {
            match &bound.kind {
                TypeBoundKind::Protocol(path) | TypeBoundKind::NegativeProtocol(path) => {
                    self.type_path(path)
                }
                TypeBoundKind::GenericProtocol(ty) | TypeBoundKind::Equality(ty) => {
                    self.visit_type(ty)
                }
                TypeBoundKind::AssociatedTypeBound {
                    type_path, bounds, ..
                } => {
                    self.type_path(type_path);
                    pending.extend(bounds);
                }
                TypeBoundKind::AssociatedTypeEquality {
                    type_path, eq_type, ..
                } => {
                    self.type_path(type_path);
                    self.visit_type(eq_type);
                }
            }
        }
    }
    fn parameter(&mut self, parameter: &FunctionParam) {
        match &parameter.kind {
            FunctionParamKind::Regular {
                pattern,
                ty,
                default_value,
            } => {
                self.visit_type(ty);
                if let Some(value) = default_value {
                    self.visit_expr(value);
                }
                self.bind_pattern(pattern);
            }
            _ => self.bind_value("self"),
        }
    }
    fn fields(&mut self, fields: &[RecordField]) {
        self.scoped(|this| {
            for field in fields {
                this.visit_type(&field.ty);
                if let Some(value) = &field.default_value {
                    this.visit_expr(value);
                }
                // Declaration elaboration publishes preceding Universe fields.
                // A scalar field is NOT an unqualified value binding here:
                // `allowed: Int` must not hide the module's `allowed(it)`.
                if matches!(field.ty.kind, TypeKind::Universe { .. }) {
                    this.bind_type(&field.name.name);
                }
            }
        });
    }
    fn pattern_fields(&mut self, fields: &[FieldPattern]) {
        for field in fields {
            if let Some(pattern) = &field.pattern {
                self.bind_pattern(pattern);
            } else {
                self.bind_value(&field.name.name);
            }
        }
    }
    // Pattern binding publishes values, even for a `Type` annotation. The
    // checker resolves type names from declaration frames/type definitions,
    // not from value schemes. A value annotation does not introduce a type name.
    fn bind_pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Ident {
                name, subpattern, ..
            } => {
                self.bind_value(&name.name);
                if let Some(pattern) = subpattern {
                    self.bind_pattern(pattern);
                }
            }
            PatternKind::Or(patterns) => {
                let mut common: Option<Scope> = None;
                for pattern in patterns {
                    self.scopes.push(Scope::default());
                    self.bind_pattern(pattern);
                    let branch = self.scopes.pop().expect("pattern alternative scope");
                    if let Some(common) = &mut common {
                        common.values.retain(|name| branch.values.contains(name));
                        common.types.retain(|name| branch.types.contains(name));
                    } else {
                        common = Some(branch);
                    }
                }
                if let Some(common) = common {
                    let scope = self.scopes.last_mut().expect("lexical scope");
                    scope.values.extend(common.values);
                    scope.types.extend(common.types);
                }
            }
            PatternKind::Tuple(patterns)
            | PatternKind::Array(patterns)
            | PatternKind::And(patterns) => {
                for pattern in patterns {
                    self.bind_pattern(pattern);
                }
            }
            PatternKind::Slice {
                before,
                rest,
                after,
            } => {
                for pattern in before {
                    self.bind_pattern(pattern);
                }
                if let Some(pattern) = rest {
                    self.bind_pattern(pattern);
                }
                for pattern in after {
                    self.bind_pattern(pattern);
                }
            }
            PatternKind::Record { path, fields, .. } => {
                self.type_path(path);
                self.pattern_fields(fields);
            }
            PatternKind::Variant { path, data } => {
                self.type_path(path);
                match data {
                    Some(VariantPatternData::Tuple(patterns)) => {
                        for pattern in patterns {
                            self.bind_pattern(pattern);
                        }
                    }
                    Some(VariantPatternData::Record { fields, .. }) => self.pattern_fields(fields),
                    None => {}
                }
            }
            PatternKind::Reference { inner, .. } | PatternKind::Paren(inner) => {
                self.bind_pattern(inner)
            }
            PatternKind::View {
                view_function,
                pattern,
            } => {
                self.visit_expr(view_function);
                self.bind_pattern(pattern);
            }
            PatternKind::Active {
                name,
                params,
                bindings,
            } => {
                self.name(name.name.clone());
                for value in params {
                    self.visit_expr(value);
                }
                for pattern in bindings {
                    self.bind_pattern(pattern);
                }
            }
            PatternKind::TypeTest { binding, test_type } => {
                self.visit_type(test_type);
                self.bind_value(&binding.name);
            }
            PatternKind::Stream {
                head_patterns,
                rest,
            } => {
                for pattern in head_patterns {
                    self.bind_pattern(pattern);
                }
                if let Some(rest) = rest {
                    self.bind_value(&rest.name);
                }
            }
            PatternKind::Cons { head, tail } => {
                self.bind_pattern(head);
                self.bind_pattern(tail);
            }
            PatternKind::Guard { pattern, guard } => {
                self.bind_pattern(pattern);
                self.visit_expr(guard);
            }
            PatternKind::Wildcard
            | PatternKind::Rest
            | PatternKind::Literal(_)
            | PatternKind::Range { .. } => {}
        }
    }
    fn recover(&mut self, recover: &RecoverBody) {
        match recover {
            RecoverBody::MatchArms { arms, .. } => {
                for arm in arms {
                    self.scoped(|this| {
                        this.bind_pattern(&arm.pattern);
                        if let Some(guard) = &arm.guard {
                            this.visit_expr(guard);
                        }
                        this.visit_expr(&arm.body);
                    });
                }
            }
            RecoverBody::Closure { param, body, .. } => self.scoped(|this| {
                if let Some(ty) = &param.ty {
                    this.visit_type(ty);
                }
                this.bind_pattern(&param.pattern);
                this.visit_expr(body);
            }),
        }
    }
    fn clauses(&mut self, clauses: &[ComprehensionClause]) {
        for clause in clauses {
            match &clause.kind {
                ComprehensionClauseKind::For { pattern, iter } => {
                    self.visit_expr(iter);
                    self.bind_pattern(pattern);
                }
                ComprehensionClauseKind::Let { pattern, ty, value } => {
                    if let Some(ty) = ty {
                        self.visit_type(ty);
                    }
                    self.visit_expr(value);
                    self.bind_pattern(pattern);
                }
                ComprehensionClauseKind::If(expr) => self.visit_expr(expr),
            }
        }
    }
    fn function(&mut self, function: &FunctionDecl, executable: bool) {
        self.scoped(|this| {
            this.generics(&function.generics);
            for parameter in &function.params {
                this.parameter(parameter);
            }
            if let Some(ty) = &function.return_type {
                this.visit_type(ty);
            }
            if let Some(throws) = &function.throws_clause {
                for ty in &throws.error_types {
                    this.visit_type(ty);
                }
            }
            for context in &function.contexts {
                this.context(context);
            }
            for clause in [&function.generic_where_clause, &function.meta_where_clause]
                .into_iter()
                .flatten()
            {
                this.visit_where_clause(clause);
            }
            for expr in function.decreases.iter().chain(&function.requires) {
                this.visit_expr(expr);
            }
            this.scoped(|this| {
                this.bind_value("result");
                for expr in &function.ensures {
                    this.visit_expr(expr);
                }
            });
            if executable && let Some(body) = &function.body {
                match body {
                    verum_ast::decl::FunctionBody::Block(block) => this.visit_block(block),
                    verum_ast::decl::FunctionBody::Expr(expression) => this.visit_expr(expression),
                }
            }
        });
    }
    fn context(&mut self, context: &verum_ast::context::ContextRequirement) {
        self.type_path(&context.path);
        for ty in &context.args {
            self.visit_type(ty);
        }
        if let Some(condition) = &context.condition {
            self.visit_expr(condition);
        }
        for transform in &context.transforms {
            for argument in &transform.args {
                self.visit_expr(argument);
            }
        }
    }
}

impl Visitor for Paths {
    fn visit_path(&mut self, path: &Path) {
        // Rooted paths keep their owner even when a local shares the leaf.
        if matches!(
            path.segments.first(),
            Some(PathSegment::Cog | PathSegment::Super | PathSegment::Relative)
        ) {
            self.names
                .insert(path.to_string().replace("::", ".").into());
        } else {
            self.name(path.to_string().replace("::", ".").into());
        }
    }
    fn visit_type_decl(&mut self, declaration: &TypeDecl) {
        self.scoped(|this| {
            this.bind_type("Self");
            this.generics(&declaration.generics);
            for parameter in &declaration.value_params {
                this.parameter(parameter);
            }
            for clause in [
                &declaration.generic_where_clause,
                &declaration.meta_where_clause,
            ]
            .into_iter()
            .flatten()
            {
                this.visit_where_clause(clause);
            }
            match &declaration.body {
                TypeDeclBody::Alias(ty) | TypeDeclBody::Newtype(ty) => this.visit_type(ty),
                TypeDeclBody::Record(fields) => this.fields(fields),
                TypeDeclBody::Tuple(types) | TypeDeclBody::SigmaTuple(types) => {
                    for ty in types {
                        this.visit_type(ty);
                    }
                }
                TypeDeclBody::Variant(variants) | TypeDeclBody::Inductive(variants) => {
                    for variant in variants {
                        this.scoped(|this| {
                            this.generics(&variant.generic_params);
                            if let Some(clause) = &variant.where_clause {
                                this.visit_where_clause(clause);
                            }
                            match &variant.data {
                                Some(VariantData::Tuple(types)) => {
                                    for ty in types {
                                        this.visit_type(ty);
                                    }
                                }
                                Some(VariantData::Record(fields)) => this.fields(fields),
                                None => {}
                            }
                            if let Some((from, to)) = &variant.path_endpoints {
                                this.visit_expr(from);
                                this.visit_expr(to);
                            }
                        });
                    }
                }
                TypeDeclBody::Protocol(body) | TypeDeclBody::Coinductive(body) => {
                    for ty in &body.extends {
                        this.visit_type(ty);
                    }
                    if let Some(clause) = &body.generic_where_clause {
                        this.visit_where_clause(clause);
                    }
                    for item in &body.items {
                        match &item.kind {
                            // The retained method carries a signature only. Nested
                            // executable declarations use visit_function, but the
                            // current source validator refuses them in metadata.
                            ProtocolItemKind::Function { decl, .. } => this.function(decl, false),
                            ProtocolItemKind::Type {
                                type_params,
                                bounds,
                                where_clause,
                                default_type,
                                ..
                            } => this.scoped(|this| {
                                this.generics(type_params);
                                this.bounds(bounds);
                                if let Some(clause) = where_clause {
                                    this.visit_where_clause(clause);
                                }
                                if let Some(ty) = default_type {
                                    this.visit_type(ty);
                                }
                            }),
                            ProtocolItemKind::Const { ty, .. } => this.visit_type(ty),
                            ProtocolItemKind::Axiom(axiom) => this.visit_axiom(axiom),
                        }
                    }
                }
                TypeDeclBody::Quotient { base, relation } => {
                    this.visit_type(base);
                    this.visit_expr(relation);
                }
                TypeDeclBody::Unit => {}
            }
        });
    }
    fn visit_axiom(&mut self, axiom: &verum_ast::decl::AxiomDecl) {
        self.scoped(|this| {
            this.generics(&axiom.generics);
            for parameter in &axiom.params {
                this.parameter(parameter);
            }
            if let Some(ty) = &axiom.return_type {
                this.visit_type(ty);
            }
            for clause in [&axiom.generic_where_clause, &axiom.meta_where_clause]
                .into_iter()
                .flatten()
            {
                this.visit_where_clause(clause);
            }
            this.visit_expr(&axiom.proposition);
        });
    }
    fn visit_function(&mut self, function: &FunctionDecl) {
        self.function(function, true);
    }
    fn visit_type(&mut self, ty: &Type) {
        let saved = self.domain;
        self.domain = Domain::Type;
        match &ty.kind {
            TypeKind::Refined { base, predicate } => {
                self.visit_type(base);
                self.scoped(|this| {
                    // These bind the refined value only in this predicate. An
                    // explicit binder is an additional spelling of that value.
                    this.bind_value("it");
                    this.bind_value("self");
                    if let Some(binding) = &predicate.binding {
                        this.bind_value(&binding.name);
                    }
                    this.visit_expr(&predicate.expr);
                });
            }
            TypeKind::Record { fields, row_var } => {
                self.fields(fields);
                if let Some(row) = row_var {
                    self.name(row.name.clone());
                }
            }
            TypeKind::Rank2Function {
                type_params,
                params,
                return_type,
                contexts,
                where_clause,
                ..
            } => self.scoped(|this| {
                this.generics(type_params);
                for ty in params {
                    this.visit_type(ty);
                }
                this.visit_type(return_type);
                for context in &contexts.requirements {
                    this.context(context);
                }
                if let Some(clause) = where_clause {
                    this.visit_where_clause(clause);
                }
            }),
            TypeKind::TypeLambda { params, body } => self.scoped(|this| {
                for name in params {
                    this.bind_type(&name.name);
                }
                this.visit_type(body);
            }),
            TypeKind::Existential { name, bounds } => self.scoped(|this| {
                this.bind_type(&name.name);
                this.bounds(bounds);
            }),
            TypeKind::Function { contexts, .. } => {
                verum_ast::visitor::walk_type(self, ty);
                for context in &contexts.requirements {
                    self.context(context);
                }
            }
            TypeKind::DynProtocol { bounds, bindings } => {
                self.bounds(bounds);
                if let Some(bindings) = bindings {
                    for binding in bindings {
                        self.visit_type(&binding.ty);
                    }
                }
            }
            TypeKind::Bounded { base, bounds } => {
                self.visit_type(base);
                self.bounds(bounds);
            }
            _ => verum_ast::visitor::walk_type(self, ty),
        }
        self.domain = saved;
    }
    fn visit_where_predicate(&mut self, predicate: &verum_ast::ty::WherePredicate) {
        use verum_ast::ty::WherePredicateKind;
        match &predicate.kind {
            WherePredicateKind::Type { ty, bounds } => {
                self.visit_type(ty);
                self.bounds(bounds);
            }
            WherePredicateKind::Value { predicate } => self.scoped(|this| {
                this.bind_value("it");
                this.bind_value("self");
                this.visit_expr(predicate);
            }),
            WherePredicateKind::Ensures { postcondition } => self.scoped(|this| {
                this.bind_value("result");
                this.visit_expr(postcondition);
            }),
            WherePredicateKind::Meta { constraint } => self.visit_expr(constraint),
        }
    }
    fn visit_block(&mut self, block: &Block) {
        self.scoped(|this| verum_ast::visitor::walk_block(this, block));
    }
    fn visit_stmt(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Item(item) => {
                use verum_ast::ItemKind;
                match &item.kind {
                    ItemKind::Function(function) => {
                        // Local declarations bind at their statement, including
                        // recursive uses; earlier sibling expressions stay free.
                        self.bind_value(&function.name.name);
                        self.visit_function(function);
                    }
                    ItemKind::Const(declaration) => {
                        self.visit_type(&declaration.ty);
                        self.visit_expr(&declaration.value);
                        self.bind_value(&declaration.name.name);
                    }
                    ItemKind::Static(declaration) => {
                        self.visit_type(&declaration.ty);
                        self.visit_expr(&declaration.value);
                        self.bind_value(&declaration.name.name);
                    }
                    _ => self.visit_item(item),
                }
            }
            StmtKind::Let { pattern, ty, value } => {
                if let Some(ty) = ty {
                    self.visit_type(ty);
                }
                if let Some(value) = value {
                    self.visit_expr(value);
                }
                self.bind_pattern(pattern);
            }
            StmtKind::LetElse {
                pattern,
                ty,
                value,
                else_block,
            } => {
                if let Some(ty) = ty {
                    self.visit_type(ty);
                }
                self.visit_expr(value);
                self.visit_block(else_block);
                self.bind_pattern(pattern);
            }
            _ => verum_ast::visitor::walk_stmt(self, statement),
        }
    }
    fn visit_expr(&mut self, expression: &Expr) {
        let saved = self.domain;
        self.domain = Domain::Value;
        match &expression.kind {
            ExprKind::MethodCall {
                receiver, method, ..
            } => {
                if let Some(path) = expression_path(receiver) {
                    self.name(format!("{path}.{}", method.name).into());
                }
                verum_ast::visitor::walk_expr(self, expression);
            }
            ExprKind::Field { .. } => {
                if let Some(path) = expression_path(expression) {
                    self.name(path.into());
                }
                verum_ast::visitor::walk_expr(self, expression);
            }
            ExprKind::Closure {
                params,
                return_type,
                body,
                ..
            } => self.scoped(|this| {
                for parameter in params {
                    if let Some(ty) = &parameter.ty {
                        this.visit_type(ty);
                    }
                    this.bind_pattern(&parameter.pattern);
                }
                if let Some(ty) = return_type {
                    this.visit_type(ty);
                }
                this.visit_expr(body);
            }),
            ExprKind::Match { expr, arms } => {
                self.visit_expr(expr);
                for arm in arms {
                    self.scoped(|this| {
                        this.bind_pattern(&arm.pattern);
                        if let Some(guard) = &arm.guard {
                            this.visit_expr(guard);
                        }
                        this.visit_expr(&arm.body);
                    });
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.scoped(|this| {
                    for condition in &condition.conditions {
                        match condition {
                            ConditionKind::Expr(expr) => {
                                if let ExprKind::Is {
                                    expr,
                                    pattern,
                                    negated: false,
                                } = &expr.kind
                                {
                                    this.visit_expr(expr);
                                    this.bind_pattern(pattern);
                                } else {
                                    this.visit_expr(expr);
                                }
                            }
                            ConditionKind::Let { pattern, value } => {
                                this.visit_expr(value);
                                this.bind_pattern(pattern);
                            }
                        }
                    }
                    this.visit_block(then_branch);
                });
                if let Some(expr) = else_branch {
                    self.visit_expr(expr);
                }
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
                self.visit_expr(iter);
                self.scoped(|this| {
                    this.bind_pattern(pattern);
                    for expr in invariants.iter().chain(decreases) {
                        this.visit_expr(expr);
                    }
                    this.visit_block(body);
                });
            }
            ExprKind::Forall { bindings, body } | ExprKind::Exists { bindings, body } => self
                .scoped(|this| {
                    for binding in bindings {
                        if let Some(ty) = &binding.ty {
                            this.visit_type(ty);
                        }
                        if let Some(expr) = &binding.domain {
                            this.visit_expr(expr);
                        }
                        this.bind_pattern(&binding.pattern);
                        if let Some(expr) = &binding.guard {
                            this.visit_expr(expr);
                        }
                    }
                    this.visit_expr(body);
                }),
            ExprKind::Comprehension { expr, clauses }
            | ExprKind::StreamComprehension { expr, clauses }
            | ExprKind::SetComprehension { expr, clauses }
            | ExprKind::GeneratorComprehension { expr, clauses } => self.scoped(|this| {
                this.clauses(clauses);
                this.visit_expr(expr);
            }),
            ExprKind::MapComprehension {
                key_expr,
                value_expr,
                clauses,
            } => self.scoped(|this| {
                this.clauses(clauses);
                this.visit_expr(key_expr);
                this.visit_expr(value_expr);
            }),
            ExprKind::TryRecover { try_block, recover } => {
                self.visit_expr(try_block);
                self.recover(recover);
            }
            ExprKind::TryRecoverFinally {
                try_block,
                recover,
                finally_block,
            } => {
                self.visit_expr(try_block);
                self.recover(recover);
                self.visit_expr(finally_block);
            }
            ExprKind::Select { arms, .. } => {
                for arm in arms {
                    if let Some(expr) = &arm.future {
                        self.visit_expr(expr);
                    }
                    self.scoped(|this| {
                        if let Some(pattern) = &arm.pattern {
                            this.bind_pattern(pattern);
                        }
                        if let Some(expr) = &arm.guard {
                            this.visit_expr(expr);
                        }
                        this.visit_expr(&arm.body);
                    });
                }
            }
            ExprKind::Nursery {
                options,
                body,
                on_cancel,
                recover,
                ..
            } => {
                if let Some(expr) = &options.timeout {
                    self.visit_expr(expr);
                }
                if let Some(expr) = &options.max_tasks {
                    self.visit_expr(expr);
                }
                self.visit_block(body);
                if let Some(block) = on_cancel {
                    self.visit_block(block);
                }
                if let Some(recover) = recover {
                    self.recover(recover);
                }
            }
            ExprKind::Record { path, fields, base } => {
                self.type_path(path);
                for field in fields {
                    if let Some(value) = &field.value {
                        self.visit_expr(value);
                    } else {
                        self.name(field.name.name.clone());
                    }
                }
                if let Some(base) = base {
                    self.visit_expr(base);
                }
            }
            // A stand-alone pattern test does not publish bindings after it.
            ExprKind::Is { expr, pattern, .. } => {
                self.visit_expr(expr);
                self.scoped(|this| this.bind_pattern(pattern));
            }
            _ => verum_ast::visitor::walk_expr(self, expression),
        }
        self.domain = saved;
    }
}
