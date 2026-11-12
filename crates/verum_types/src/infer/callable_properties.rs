//! Computational bounds come from the callable actually selected by inference.
use super::*;
use crate::computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet};
use verum_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use verum_ast::{FunctionDecl, ItemKind, Pattern, PatternKind};

use super::atomic_access::AtomicAccess;

#[derive(Default)]
struct SourceCalls {
    unresolved: verum_common::Set<Text>,
    bindings: verum_common::Set<Text>,
    nested_item: bool,
}

struct Calls<'a> {
    checker: &'a TypeChecker,
    bound: PropertyRequirement,
    source: Option<&'a mut SourceCalls>,
}
impl Visitor for Calls<'_> {
    fn visit_stmt(&mut self, statement: &verum_ast::Stmt) {
        if self.checker.cfg_evaluator.should_include(&statement.attributes) {
            walk_stmt(self, statement);
        }
    }

    fn visit_expr(&mut self, expression: &Expr) {
        match &expression.kind {
            ExprKind::Closure { .. } => return,
            ExprKind::MetaFunction { name, args, .. } if name.name == "intrinsic" => {
                let requirement = args.first().and_then(|arg| match &arg.kind {
                    ExprKind::Literal(verum_ast::Literal {
                        kind: verum_ast::LiteralKind::Text(name),
                        ..
                    }) => match name.as_str() {
                        // The panic instruction raises synchronously. Catching it
                        // does not require a suspension-capable callback.
                        "panic" => Some(PropertyRequirement::source_callable(false)),
                        name => AtomicAccess::from_intrinsic(name).map(AtomicAccess::properties),
                    },
                    _ => None,
                });
                if let Some(requirement) = requirement {
                    self.bound = self.bound.join(&requirement);
                } else {
                    // Type checking the operands does not establish what the
                    // operation can do. Preserve unknown in checked bodies too.
                    self.bound = PropertyRequirement::Unrestricted;
                }
            }
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } => {
                let target = self
                    .checker
                    .type_registry
                    .lookup_call_target(expression.span)
                    .map(|ty| self.checker.unifier.apply(ty));
                let requirement = match target.as_ref() {
                    Some(Type::Function {
                        properties: Some(known),
                        ..
                    }) => PropertyRequirement::AtMost(known.clone()),
                    Some(Type::Function {
                        property_requirement,
                        ..
                    }) => property_requirement.clone(),
                    None if let Some(source) = self.source.as_deref_mut()
                        && let ExprKind::Call { func, .. } = &expression.kind
                        && let ExprKind::Path(path) = &func.kind
                        && let Some(name) = path.as_ident() =>
                    {
                        source.unresolved.insert(name.name.clone());
                        PropertyRequirement::AtMost(PropertySet::pure())
                    }
                    _ => PropertyRequirement::Unrestricted,
                };
                if self.source.is_none() && requirement.may_be_async() {
                    tracing::debug!(target: "verum_types::callable_contract", owner = %self.checker.current_module_path,
                        span = ?expression.span, callable = ?target, "call has no synchronous contract");
                }
                self.bound = self.bound.join(&requirement);
            }
            _ => (),
        }
        walk_expr(self, expression);
    }
    fn visit_pattern(&mut self, pattern: &Pattern) {
        if let Some(source) = self.source.as_deref_mut() {
            match &pattern.kind {
                PatternKind::Ident { name, .. } => {
                    source.bindings.insert(name.name.clone());
                }
                PatternKind::Record { fields, .. }
                | PatternKind::Variant {
                    data: Some(verum_ast::pattern::VariantPatternData::Record { fields, .. }),
                    ..
                } => {
                    source.bindings.extend(
                        fields
                            .iter()
                            .filter(|field| field.pattern.is_none())
                            .map(|field| field.name.name.clone()),
                    );
                }
                PatternKind::View { .. } | PatternKind::Active { .. } => {
                    self.bound = PropertyRequirement::Unrestricted
                }
                _ => {}
            }
        }
        walk_pattern(self, pattern);
    }
    fn visit_item(&mut self, _: &verum_ast::Item) {
        if let Some(source) = self.source.as_deref_mut() {
            source.nested_item = true;
        }
    }
}

impl TypeChecker {
    /// A selected structural operation contains no user callback. Its arguments
    /// are still traversed normally, so their properties cannot disappear.
    pub(super) fn record_structural_call(&mut self, span: Span, params: List<Type>, result: Type) {
        self.type_registry.register_call_target(span, Type::function_with_requirement(
            params, result, PropertyRequirement::source_callable(false),
        ));
    }

    pub(super) fn callable_body_bound(&mut self, body: &Expr) -> PropertyRequirement {
        let local = self.property_inferrer.infer_local_operations(body);
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
            source: None,
        };
        calls.visit_expr(body);
        calls.bound
    }

    pub(super) fn callable_function_body_bound(
        &mut self,
        body: &verum_ast::FunctionBody,
    ) -> PropertyRequirement {
        let local = self.property_inferrer.infer_function_body(body);
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
            source: None,
        };
        match body {
            verum_ast::FunctionBody::Block(block) => calls.visit_block(block),
            verum_ast::FunctionBody::Expr(expr) => calls.visit_expr(expr),
        }
        calls.bound
    }

    pub(super) fn source_callable_requirement(
        &mut self,
        function: &FunctionDecl,
    ) -> PropertyRequirement {
        let declared = PropertyRequirement::declared_function(function.is_pure, function.is_meta);
        if declared.may_be_async()
            && self.source_body_is_sync(function, &self.current_module_path.clone(), &mut List::new())
        {
            return PropertyRequirement::source_callable(false);
        }
        declared
    }

    fn source_body_is_sync(
        &mut self,
        function: &FunctionDecl,
        owner: &Text,
        active: &mut List<(Text, Span)>,
    ) -> bool {
        if function.is_async || function.is_generator {
            return false;
        }
        // The ABI is carried by the declaration, including across imports and
        // metadata. Foreign calls return on the calling stack. They may perform
        // IO, mutate and fail; synchronous is not a promise of purity.
        if function.extern_abi.is_some() {
            return true;
        }
        let Some(body) = &function.body else {
            return false;
        };
        let key = (owner.clone(), function.span);
        if active.len() >= 32 || active.contains(&key) {
            return false;
        }
        let mut local = self.property_inferrer.infer_function_body(body);
        for param in &function.params {
            if let verum_ast::FunctionParamKind::Regular {
                default_value: Some(value),
                ..
            } = &param.kind
            {
                local = local.union(&self.property_inferrer.infer_local_operations(value));
            }
        }
        let mut source = SourceCalls::default();
        let mut calls = Calls {
            checker: self,
            bound: PropertyRequirement::AtMost(local),
            source: Some(&mut source),
        };
        for param in &function.params {
            if let verum_ast::FunctionParamKind::Regular {
                pattern,
                default_value,
                ..
            } = &param.kind
            {
                calls.visit_pattern(pattern);
                if let Some(value) = default_value {
                    calls.visit_expr(value);
                }
            }
        }
        match body {
            verum_ast::FunctionBody::Block(block) => calls.visit_block(block),
            verum_ast::FunctionBody::Expr(expr) => calls.visit_expr(expr),
        }
        if calls.bound.may_be_async() {
            return false;
        }
        if source.unresolved.is_empty() {
            return true;
        }
        if source.nested_item
            || source
                .unresolved
                .iter()
                .any(|name| source.bindings.contains(name))
        {
            return false;
        }
        active.push(key);
        let proven = source.unresolved.iter().all(|name| {
            self.source_function_for_properties(owner, name)
                .is_some_and(|(target, declaration)| {
                    self.source_body_is_sync(&declaration, &target, active)
                })
        });
        active.pop();
        proven
    }

    /// Before a source method has been checked, only direct function bindings
    /// in its own module can complete the proof. Dynamic or ambiguous targets
    /// retain their open contract.
    fn source_function_for_properties(
        &self,
        owner: &Text,
        name: &Text,
    ) -> Option<(Text, FunctionDecl)> {
        use super::source_declarations::{MountTargetKind, source_mount_targets};
        if let Some((_, module)) = self.source_type_registration.local_declarations.iter().rev()
            .find(|(path, _)| path == owner)
        {
            if let Some(declaration) = module.items.iter().find_map(|item| self.imported_function(item, name)) {
                return Some((owner.clone(), declaration.clone()));
            }
            if module.items.iter().any(|item| match &item.kind {
                ItemKind::Const(value) => &value.name.name == name,
                ItemKind::Static(value) => &value.name.name == name,
                _ => false,
            }) { return None; }
        }
        let registry = self.module_registry.read();
        let module = self.get_module_with_path_aliases(owner, &registry)?;
        let local = |module: &verum_modules::ModuleInfo, name: &Text| {
            module.ast.items.iter().find_map(|item| self.imported_function(item, name)).cloned()
        };
        if let Some(function) = local(&module, name) {
            return Some((module.path.to_string().into(), function));
        }
        if module.ast.items.iter().any(|item| match &item.kind {
            ItemKind::Const(value) => &value.name.name == name,
            ItemKind::Static(value) => &value.name.name == name,
            _ => false,
        }) {
            return None;
        }
        let mut named = Vec::new();
        let mut glob = Vec::new();
        for item in &module.ast.items {
            if let ItemKind::Mount(mount) = &item.kind {
                source_mount_targets(mount, &module.path, name, |target, kind| match kind {
                    MountTargetKind::Named => {
                        if let (Some(parent), Some(leaf)) = (target.parent(), target.name()) {
                            named.push((parent, leaf.clone()));
                        }
                    }
                    MountTargetKind::Glob => glob.push((target, name.clone())),
                });
            }
        }
        let explicit = !named.is_empty();
        let mut selected = None;
        for (path, leaf) in if explicit { named } else { glob } {
            let target = self.get_module_with_path_aliases(&path.to_string(), &registry)?;
            let Some(export) = target.exports.get(&leaf) else {
                if explicit {
                    return None;
                }
                continue;
            };
            if export.kind != verum_modules::ExportKind::Function
                || export.visibility != verum_ast::Visibility::Public
            {
                return None;
            }
            let origin = registry.get(export.source_module)?;
            let original = export.original_name.as_ref().unwrap_or(&leaf);
            let declaration = local(&origin, original)?;
            let key = (Text::from(origin.path.to_string()), declaration);
            if selected.as_ref().is_some_and(|previous| previous != &key) {
                return None;
            }
            selected = Some(key);
        }
        selected
    }

    pub(super) fn callable_bound_with_marker(
        bound: PropertyRequirement,
        is_async: bool,
    ) -> PropertyRequirement {
        if is_async {
            bound.join(&PropertyRequirement::AtMost(PropertySet::single(
                ComputationalProperty::Async,
            )))
        } else {
            bound
        }
    }
}
