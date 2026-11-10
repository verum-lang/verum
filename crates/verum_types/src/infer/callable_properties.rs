//! Computational bounds come from the callable actually selected by inference.
use super::*;
use crate::computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet};
use verum_ast::visitor::{Visitor, walk_expr, walk_pattern};
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
    fn visit_expr(&mut self, expression: &Expr) {
        match &expression.kind {
            ExprKind::Closure { .. } => return,
            ExprKind::MetaFunction { name, args, .. } if name.name == "intrinsic" => {
                let access = args.first().and_then(|arg| match &arg.kind {
                    ExprKind::Literal(verum_ast::Literal {
                        kind: verum_ast::LiteralKind::Text(name),
                        ..
                    }) => AtomicAccess::from_intrinsic(name.as_str()),
                    _ => None,
                });
                if let Some(access) = access {
                    self.bound = self.bound.join(&access.properties());
                } else if self.source.is_some() {
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

    pub(super) fn concrete_method_requirement(
        &mut self,
        method: &FunctionDecl,
    ) -> PropertyRequirement {
        let declared = PropertyRequirement::declared_function(method.is_pure, method.is_meta);
        if declared.may_be_async()
            && self.source_body_is_sync(method, &self.current_module_path.clone(), &mut Vec::new())
        {
            return PropertyRequirement::source_callable(false);
        }
        declared
    }

    fn source_body_is_sync(
        &mut self,
        function: &FunctionDecl,
        owner: &Text,
        active: &mut Vec<(Text, Span)>,
    ) -> bool {
        let Some(body) = &function.body else {
            return false;
        };
        let key = (owner.clone(), function.span);
        if function.is_async || active.len() >= 32 || active.contains(&key) {
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
            self.source_function_for_effect(owner, name)
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
    fn source_function_for_effect(
        &self,
        owner: &Text,
        name: &Text,
    ) -> Option<(Text, FunctionDecl)> {
        use super::source_declarations::{MountTargetKind, source_mount_targets};
        let registry = self.module_registry.read();
        let module = self.get_module_with_path_aliases(owner, &registry)?;
        let local = |module: &verum_modules::ModuleInfo, name: &Text| {
            module.ast.items.iter().find_map(|item| match &item.kind {
                ItemKind::Function(declaration) if &declaration.name.name == name => {
                    Some(declaration.clone())
                }
                _ => None,
            })
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
