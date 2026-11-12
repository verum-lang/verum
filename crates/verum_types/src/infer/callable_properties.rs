//! Computational bounds come from the callable actually selected by inference.
use super::*;
use crate::computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet};
use verum_ast::visitor::{Visitor, walk_expr, walk_pattern, walk_stmt};
use verum_ast::{FunctionDecl, ItemKind, Pattern, PatternKind};
use verum_vbc::intrinsics::{lookup_intrinsic, registry::IntrinsicCallBound};

use super::atomic_access::AtomicAccess;

pub(super) fn intrinsic_operation_name(surface: &str, args: &[Expr]) -> Option<Text> {
    match (surface, &args.first()?.kind) {
        ("intrinsic", ExprKind::Literal(verum_ast::Literal {
            kind: verum_ast::LiteralKind::Text(name), ..
        })) => Some(name.as_str().into()),
        ("vbc", ExprKind::Path(path)) => path.as_ident().map(|name| name.name.clone()),
        _ => None,
    }
}

fn intrinsic_requirement(name: &str) -> Option<PropertyRequirement> {
    match lookup_intrinsic(name).map(|info| info.intrinsic.call_bound()) {
        Some(IntrinsicCallBound::Pure) => Some(PropertyRequirement::AtMost(PropertySet::pure())),
        Some(IntrinsicCallBound::Synchronous) => Some(PropertyRequirement::source_callable(false)),
        _ => AtomicAccess::from_intrinsic(name).map(AtomicAccess::properties),
    }
}

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
            ExprKind::MetaFunction { name, args, .. }
                if matches!(name.name.as_str(), "intrinsic" | "vbc") => {
                let requirement = intrinsic_operation_name(name.name.as_str(), args)
                    .and_then(|name| intrinsic_requirement(name.as_str()));
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
    pub(super) fn record_method_publication(
        &mut self,
        receiver: &Text,
        name: &Text,
        scheme: &crate::context::TypeScheme,
    ) {
        let Some((owner, declaration)) = &scheme.callable_source else { return; };
        let locations = self.source_type_registration.method_publications
            .entry((owner.clone(), declaration.span)).or_default();
        let location = (receiver.clone(), name.clone());
        if !locations.contains(&location) { locations.push(location); }
    }

    pub(super) fn publish_checked_method_contract(
        &mut self,
        implementation: &verum_ast::decl::ImplDecl,
        declaration: &FunctionDecl,
        checked: &Type,
    ) -> Result<()> {
        let owner = self.current_module_path.clone();
        if let Some(locations) = self.source_type_registration.method_publications
            .get(&(owner.clone(), declaration.span))
        {
            let mut methods = self.inherent_methods.write();
            for (receiver, name) in locations {
                if let Some(scheme) = methods.get_mut(receiver).and_then(|entries| entries.get_mut(name)) {
                    scheme.refine_callable_contract(&owner, declaration, checked);
                }
                if let Some(method) = name.strip_prefix("$static$") {
                    self.ctx.env.refine_callable_contract(
                        &format!("{receiver}.{method}"), &owner, declaration, checked,
                    );
                }
            }
        }
        if matches!(implementation.kind, verum_ast::decl::ImplKind::Protocol { .. }) {
            self.protocol_checker.write().refine_source_method_contract(
                &owner, implementation, declaration, checked,
            )?;
        }
        Ok(())
    }

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
            // Bodyless declarations execute the registered operation. An
            // annotation on a source body cannot replace that body's evidence.
            return crate::core_pipeline::extract_intrinsic_name(function)
                .and_then(|name| intrinsic_requirement(name.as_str()))
                .is_some_and(|bound| !bound.may_be_async());
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
        let registry = self.module_registry.read();
        let registered = self.get_module_with_path_aliases(owner, &registry);
        // A currently checked source need not yet be a registry entry. Its
        // complete lexical inventory (including mounts) still owns resolution.
        let module = self.source_type_registration.local_declarations.iter().rev()
            .find(|(path, _)| path == owner).map(|(_, module)| module.as_ref())
            .or_else(|| registered.as_ref().map(|module| &module.ast))?;
        let path = registered.as_ref().map_or_else(
            || verum_modules::ModulePath::from_str(owner), |module| module.path.clone(),
        );
        let included = |item: &&verum_ast::Item| self.cfg_evaluator.should_include(&item.attributes);
        let local = |module: &verum_ast::Module, name: &Text| {
            module.items.iter().filter(included)
                .filter_map(|item| self.imported_function(item, name))
                .find(|function| self.cfg_evaluator.should_include(&function.attributes)).cloned()
        };
        if let Some(function) = local(module, name) {
            return Some((path.to_string().into(), function));
        }
        if module.items.iter().filter(included).any(|item| match &item.kind {
            ItemKind::Const(value) => &value.name.name == name,
            ItemKind::Static(value) => &value.name.name == name,
            _ => false,
        }) {
            return None;
        }
        let mut named = List::new();
        let mut glob = List::new();
        for item in module.items.iter().filter(included) {
            if let ItemKind::Mount(mount) = &item.kind {
                source_mount_targets(mount, &path, name, |target, kind| match kind {
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
            let declaration = local(&origin.ast, original)?;
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
