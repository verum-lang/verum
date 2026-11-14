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

pub(super) fn intrinsic_requirement(name: &str) -> Option<PropertyRequirement> {
    match lookup_intrinsic(name).map(|info| info.intrinsic.call_bound()) {
        Some(IntrinsicCallBound::Pure) => Some(PropertyRequirement::AtMost(PropertySet::pure())),
        Some(IntrinsicCallBound::ValueConstruction) => Some(PropertyRequirement::value_construction()),
        Some(IntrinsicCallBound::Synchronous) => Some(PropertyRequirement::source_callable(false)),
        _ => AtomicAccess::from_intrinsic(name).map(AtomicAccess::properties),
    }
}

/// Only a bodyless declaration can inherit its implementation's bound.
/// Defaults, guards and context lookup can execute additional operations.
fn intrinsic_declaration_bound(function: &FunctionDecl) -> Option<PropertyRequirement> {
    if function.body.is_some() {
        return None;
    }
    let name = crate::core_pipeline::extract_intrinsic_name(function)?;
    if has_implicit_call_computation(function) {
        return Some(PropertyRequirement::Unrestricted);
    }
    Some(intrinsic_requirement(&name).unwrap_or(PropertyRequirement::Unrestricted))
}

fn has_implicit_call_computation(function: &FunctionDecl) -> bool {
    let has_defaults = function.params.iter().any(|param| matches!(
        &param.kind, verum_ast::FunctionParamKind::Regular { default_value: Some(_), .. }
    ));
    function.is_async || function.is_generator || function.extern_abi.is_some()
        || !function.contexts.is_empty() || function.std_attr.is_some()
        || !function.requires.is_empty() || !function.ensures.is_empty() || has_defaults
}

#[derive(Default)]
struct SourceCalls {
    unresolved: List<verum_ast::Path>,
    bindings: verum_common::Set<Text>,
    nested_item: bool,
}

// Only lexical paths can supply declaration evidence before expression
// inference. Value receivers and computed callees remain unknown. Walk field
// chains iteratively so qualified calls do not amplify the host stack.
fn source_call_path(expression: &Expr) -> Option<verum_ast::Path> {
    use verum_ast::ty::PathSegment;
    let (mut receiver, method) = match &expression.kind {
        ExprKind::Call { func, .. } => (func.as_ref(), None),
        ExprKind::MethodCall { receiver, method, .. } => (receiver.as_ref(), Some(method)),
        _ => return None,
    };
    let mut suffix = List::new();
    if let Some(method) = method { suffix.push(method.clone()); }
    while let ExprKind::Field { expr, field } = &receiver.kind {
        suffix.push(field.clone());
        receiver = expr;
    }
    let ExprKind::Path(path) = &receiver.kind else { return None; };
    let mut path = path.clone();
    path.segments.extend(suffix.into_iter().rev().map(PathSegment::Name));
    path.span = expression.span;
    Some(path)
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
                        && let Some(path) = source_call_path(expression) =>
                    {
                        source.unresolved.push(path);
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

    /// A selected structural view reads its receiver and constructs fresh
    /// metadata, possibly allocating or failing. It neither writes through that
    /// view nor invokes user code. Operand computations are joined separately.
    pub(super) fn record_structural_call(&mut self, span: Span, params: List<Type>, result: Type) {
        self.type_registry.register_call_target(span, Type::function_with_requirement(
            params, result, PropertyRequirement::value_construction(),
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

    /// A complete, statement-free forwarding expression has no hidden local
    /// work. This is a restricted proof form, not a second expression inferrer.
    fn source_forwarding_expression(function: &FunctionDecl) -> Option<&Expr> {
        use verum_ast::decl::FunctionBody;
        match function.body.as_ref()? {
            FunctionBody::Block(block) if block.stmts.is_empty() => block.expr.as_deref(),
            FunctionBody::Expr(expression) => Some(expression),
            _ => None,
        }
    }

    fn direct_intrinsic_operands(expression: &Expr) -> Option<(PropertyRequirement, &[Expr])> {
        let ExprKind::MetaFunction { name, args, parenthesized: true } = &expression.kind
            else { return None; };
        let operation = intrinsic_operation_name(&name.name, args)?;
        let registered = lookup_intrinsic(&operation)?;
        let operands = args.as_slice().get(1..)?;
        if operands.len() != usize::from(registered.intrinsic.param_count) { return None; }
        let bound = intrinsic_requirement(&operation)?;
        bound.finite_footprint()?;
        Some((bound, operands))
    }

    /// Numeric conversions are supplied by the language registrar. A foreign
    /// nominal with a scalar-looking name cannot authorize its own conversion.
    fn forwarding_scalar(&self, ty: &Type) -> bool {
        match ty {
            Type::Int | Type::Float | Type::Bool | Type::Char => true,
            Type::Named { path, args } if args.is_empty() => path.as_ident().is_some_and(|name| {
                verum_common::well_known_types::type_names::is_numeric_type(&name.name)
                    && self.ctx.builtin_type_definition(&name.name).is_some_and(|declared|
                        self.unifier.normalize_aliases(declared) == *ty)
            }),
            _ => false,
        }
    }

    /// Only declared references are forwarded; copying owned operands or
    /// adapting a reference could execute Clone/Deref. Generic identities come
    /// from the declaration frame, never from arbitrary unification variables.
    /// The only composed body supported here is a call to an intrinsic leaf,
    /// optionally followed by an ordinary checked scalar conversion.
    fn source_intrinsic_forwarding_bound(
        &mut self,
        function: &FunctionDecl,
    ) -> Option<PropertyRequirement> {
        use verum_ast::{decl::FunctionParamKind, ty::{GenericParamKind, PathSegment}};
        use crate::context::GenericBinder;
        if has_implicit_call_computation(function)
            || function.throws_clause.is_some()
            || function.generics.iter().any(|parameter| !matches!(parameter.kind,
                GenericParamKind::Type { .. })) { return None; }
        let expression = Self::source_forwarding_expression(function)?;
        let (operation, cast) = match &expression.kind {
            ExprKind::Cast { expr, ty } => (expr.as_ref(), Some(ty)),
            _ => (expression, None),
        };
        let direct = Self::direct_intrinsic_operands(operation);
        let mut selected = None;
        let operands = if let Some((_, operands)) = &direct {
            if cast.is_some() { return None; }
            *operands
        } else {
            let ExprKind::Call { func, args, type_args } = &operation.kind else { return None; };
            if !type_args.is_empty() { return None; }
            let ExprKind::Path(path) = &func.kind else { return None; };
            let callee = path.as_ident()?;
            let (owner, declaration) = self.source_function_for_properties(
                &self.current_module_path, &callee.name,
            )?;
            // Do not recursively speculate through arbitrary source calls.
            // A leaf is either the declared intrinsic or its exact operation.
            if intrinsic_declaration_bound(&declaration).is_none()
                && Self::source_forwarding_expression(&declaration)
                    .and_then(Self::direct_intrinsic_operands).is_none() { return None; }
            selected = Some((callee.name.clone(), owner, declaration));
            args.as_slice()
        };
        let declared_variables: Set<TypeVar> = self.decl_param_frames.last().into_iter()
            .flat_map(|frame| frame.values()).filter_map(|ty| match GenericBinder::declared(ty) {
                Ok(GenericBinder::Type(variable)) => Some(variable), _ => None,
            }).collect();
        let known = |ty: &Type| ty.free_vars().iter().all(|var| declared_variables.contains(var))
            && !ty.contains_type(|part| matches!(part, Type::Unknown | Type::Placeholder { .. }));
        let mut parameters = Map::new();
        for parameter in &function.params {
            let (name, ty) = match &parameter.kind {
                FunctionParamKind::Regular { pattern, ty, .. } => {
                    let PatternKind::Ident { name, by_ref: false, subpattern: None, .. }
                        = &pattern.kind else { return None; };
                    (name.name.clone(), self.declared_type(ty).ok()?)
                }
                receiver => {
                    let subject = self.current_self_type.clone()?;
                    let ty = match receiver {
                        FunctionParamKind::SelfRef => Type::reference(false, subject),
                        FunctionParamKind::SelfRefMut => Type::reference(true, subject),
                        FunctionParamKind::SelfRefChecked => Type::checked_reference(false, subject),
                        FunctionParamKind::SelfRefCheckedMut => Type::checked_reference(true, subject),
                        FunctionParamKind::SelfRefUnsafe => Type::unsafe_reference(false, subject),
                        FunctionParamKind::SelfRefUnsafeMut => Type::unsafe_reference(true, subject),
                        _ => return None,
                    };
                    (Text::from("self"), ty)
                }
            };
            ty.reference_parts()?;
            if !known(&ty) || parameters.insert(name, ty).is_some() { return None; }
        }
        let mut actual = List::new();
        for operand in operands {
            let ExprKind::Path(path) = &operand.kind else { return None; };
            let name = match path.segments.as_slice() {
                [PathSegment::Name(name)] => name.name.clone(),
                [PathSegment::SelfValue] => Text::from("self"),
                _ => return None,
            };
            actual.push(self.unifier.normalize_aliases(parameters.get(&name)?));
        }
        let output = function.return_type.as_ref()
            .map(|ty| self.declared_type(ty)).transpose().ok()?.unwrap_or(Type::Unit);
        if !known(&output) { return None; }
        if let Some((bound, _)) = direct { return Some(bound); }
        let (callee, owner, declaration) = selected?;
        // A parameter owns its spelling, even when a module has that function.
        if parameters.contains_key(&callee) { return None; }
        let scheme = self.with_source_signature_scope(&owner, |checker| {
            let previous_self = checker.current_self_type.clone();
            checker.set_current_self_type(None);
            checker.push_decl_param_frame(indexmap::IndexMap::new());
            let scheme = checker.source_function_scheme(&declaration).ok();
            checker.pop_decl_param_frame();
            checker.set_current_self_type(previous_self);
            scheme
        })?;
        let instance = scheme.try_instantiate().ok()?;
        if !instance.type_bounds.is_empty() || !instance.protocol_bounds.is_empty()
            || !instance.where_clauses.is_empty() || !instance.value_parameters.is_empty()
            || !scheme.universe_params.is_empty() { return None; }
        let Type::Function { params, return_type, property_requirement: bound, .. } = instance.ty
            else { return None; };
        bound.finite_footprint()?;
        if params.len() != actual.len() { return None; }
        // Speculative matching cannot specialize a caller's declared variable.
        // The final exact equality also refuses coercions accepted by unification.
        let mut matcher = crate::unify::Unifier::new();
        matcher.enter_rigid_scope(declared_variables.iter()
            .map(|var| (*var, Text::from("declared parameter"), false, None)));
        let expected: List<_> = params.iter().map(|ty| self.unifier.normalize_aliases(ty)).collect();
        for (expected, actual) in expected.iter().zip(&actual) {
            matcher.unify(expected, actual, operation.span).ok()?;
        }
        if expected.iter().zip(&actual).any(|(expected, actual)| matcher.apply(expected) != *actual) {
            return None;
        }
        let result = matcher.apply(&self.unifier.normalize_aliases(&return_type));
        if !known(&result) { return None; }
        let output = self.unifier.normalize_aliases(&output);
        if let Some(cast) = cast {
            let target = self.declared_type(cast).ok()?;
            if !self.forwarding_scalar(&result) || !self.forwarding_scalar(&target)
                || self.unifier.normalize_aliases(&target) != output { return None; }
            self.check_cast(&result, &target, expression.span, Some(operation)).ok()?;
            Some(bound.join(&PropertyRequirement::value_construction()))
        } else {
            (result == output).then_some(bound)
        }
    }

    pub(super) fn source_callable_requirement(
        &mut self,
        function: &FunctionDecl,
    ) -> PropertyRequirement {
        if let Some(bound) = intrinsic_declaration_bound(function) {
            return bound;
        }
        if let Some(bound) = self.source_intrinsic_forwarding_bound(function) {
            return bound;
        }
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
        if let Some(bound) = intrinsic_declaration_bound(function) {
            return !bound.may_be_async();
        }
        // The ABI is carried by the declaration, including across imports and
        // metadata. Foreign calls return on the calling stack. They may perform
        // IO, mutate and fail; synchronous is not a promise of purity.
        if function.extern_abi.is_some() {
            return true;
        }
        let Some(body) = &function.body else { return false; };
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
                .any(|path| matches!(path.segments.first(), Some(verum_ast::ty::PathSegment::Name(name))
                    if source.bindings.contains(&name.name)))
        {
            return false;
        }
        active.push(key);
        let proven = source.unresolved.iter().all(|path|
            self.source_call_is_sync(owner, path, active));
        active.pop();
        proven
    }

    fn source_call_is_sync(
        &mut self,
        owner: &Text,
        path: &verum_ast::Path,
        active: &mut List<(Text, Span)>,
    ) -> bool {
        use verum_ast::{TypeDeclBody, ty::PathSegment};
        if let Some(name) = path.as_ident() {
            if let Some((target, declaration)) = self.source_function_for_properties(owner, &name.name) {
                return self.source_body_is_sync(&declaration, &target, active);
            }
            if self.source_receiver_shadowing(owner, &name.name).0 { return false; }
            return self.with_source_signature_scope(owner, |checker|
                checker.declaring_source_type_declaration(&name.name))
                .is_some_and(|(declaration, _)| matches!(declaration.body, TypeDeclBody::Tuple(_)));
        }
        let Some(PathSegment::Name(member)) = path.segments.last() else { return false; };
        let mut receiver = path.clone();
        receiver.segments.pop();
        // Resolve the written module path exactly once, using the same relative
        // path semantics as mounts. A found namespace owns a failed lookup too.
        let module_path = verum_modules::imports::path_to_module_path_resolved(
            &receiver, &verum_modules::ModulePath::from_str(owner));
        let (value_bound, lexical_bound) = match receiver.segments.first() {
            Some(PathSegment::Name(name)) => self.source_receiver_shadowing(owner, &name.name),
            _ => (false, false),
        };
        if value_bound { return false; }
        let module = if lexical_bound { None } else {
            self.module_registry.read().get_by_path_aliased(&module_path.to_string())
        };
        if let Some(module) = module {
            if !module.exports.is_visible_from_path(&member.name,
                &verum_modules::ModulePath::from_str(owner)) { return false; }
            let Some(export) = module.exports.get(&member.name) else { return false; };
            let origin = self.module_registry.read().get(export.source_module);
            let Some(origin) = origin else { return false; };
            let original = export.original_name.as_ref().unwrap_or(&member.name);
            let declaration = origin.ast.items.iter()
                .filter(|item| self.cfg_evaluator.should_include(&item.attributes))
                .find_map(|item| self.imported_function(item, original))
                .filter(|function| self.cfg_evaluator.should_include(&function.attributes)).cloned();
            return declaration.is_some_and(|function|
                self.source_body_is_sync(&function, &origin.path.to_string().into(), active));
        }
        // A nominal receiver selects its own declaration and implementation,
        // never a same-named method belonging to the importing module.
        let Some(name) = receiver.as_ident() else { return false; };
        let selected = self.with_source_signature_scope(owner, |checker|
            checker.declaring_source_type_declaration(name.name.as_str()));
        let Some((declaration, target)) = selected else { return false; };
        match self.source_static_function_for_properties(&target, &declaration.name.name, &member.name) {
            Ok(Some(function)) => {
                if target != *owner && function.visibility != verum_ast::Visibility::Public { return false; }
                return self.source_body_is_sync(&function, &target, active);
            }
            Err(()) => return false,
            Ok(None) => {},
        }
        matches!(&declaration.body, TypeDeclBody::Variant(variants) | TypeDeclBody::Inductive(variants)
            if variants.iter().any(|variant| variant.name.name == member.name
                && self.cfg_evaluator.should_include(&variant.attributes)))
    }

    // A local binding owns its spelling. In particular an imported namespace,
    // type, or global value cannot borrow the contract of an unrelated global
    // module with the same name. Ordinary typed calls still resolve these
    // bindings; preliminary declaration analysis remains conservative.
    fn source_receiver_shadowing(&self, owner: &Text, name: &Text) -> (bool, bool) {
        use super::source_declarations::{source_mount_targets, MountTargetKind};
        let registry = self.module_registry.read();
        let registered = registry.get_by_path_aliased(owner);
        let module = self.source_type_registration.local_declarations.iter().rev()
            .find(|(path, _)| path == owner).map(|(_, module)| module.as_ref())
            .or_else(|| registered.as_ref().map(|module| &module.ast));
        let Some(module) = module else { return (false, false); };
        let mut bound = false;
        let mut value = false;
        let header = module.file_header_index();
        for (index, item) in module.items.iter().enumerate() {
            if Some(index) == header
                || !self.cfg_evaluator.should_include(&item.attributes) { continue; }
            match &item.kind {
                ItemKind::Const(value) if &value.name.name == name => return (true, true),
                ItemKind::Static(value) if &value.name.name == name => return (true, true),
                ItemKind::Function(value) if &value.name.name == name => return (true, true),
                ItemKind::Type(value) if &value.name.name == name => bound = true,
                ItemKind::Module(value) if &value.name.name == name => bound = true,
                ItemKind::Mount(mount) => source_mount_targets(mount,
                    &verum_modules::ModulePath::from_str(owner), name, |target, kind| {
                        let kind = match kind {
                            MountTargetKind::Named => {
                                bound = true;
                                if registry.get_by_path_aliased(&target.to_string()).is_some() { return; }
                                target.parent().and_then(|parent| registry.get_by_path_aliased(&parent.to_string()))
                                    .and_then(|module| module.exports.get(target.name()?).map(|export| export.kind))
                            }
                            MountTargetKind::Glob => registry.get_by_path_aliased(&target.to_string())
                                .and_then(|module| module.exports.get(name).map(|export| export.kind)),
                        };
                        if let Some(kind) = kind {
                            bound = true;
                            value |= !matches!(kind, verum_modules::ExportKind::Type | verum_modules::ExportKind::Module);
                        }
                    }),
                _ => {},
            }
        }
        (value, bound)
    }

    fn source_static_function_for_properties(
        &self, owner: &Text, receiver: &Text, name: &Text,
    ) -> std::result::Result<Option<FunctionDecl>, ()> {
        use verum_ast::{decl::{ImplKind, ImplItemKind, FunctionParamKind}, TypeKind};
        let registry = self.module_registry.read();
        let registered = registry.get_by_path_aliased(owner);
        let module = self.source_type_registration.local_declarations.iter().rev()
            .find(|(path, _)| path == owner).map(|(_, module)| module.as_ref())
            .or_else(|| registered.as_ref().map(|module| &module.ast)).ok_or(())?;
        let mut selected = None;
        for item in &module.items {
            if !self.cfg_evaluator.should_include(&item.attributes) { continue; }
            let ItemKind::Impl(implementation) = &item.kind else { continue; };
            // A generic or constrained implementation requires ordinary typed
            // selection. Do not invent its applicability from a spelling.
            let ImplKind::Inherent(target) = &implementation.kind else { continue; };
            let TypeKind::Path(target) = &target.kind else { continue; };
            if !target.as_ident().is_some_and(|name| &name.name == receiver) { continue; }
            for method in &implementation.items {
                let ImplItemKind::Function(function) = &method.kind else { continue; };
                if &function.name.name != name || !self.cfg_evaluator.should_include(&method.attributes)
                    || !self.cfg_evaluator.should_include(&function.attributes) { continue; }
                if selected.is_some() || !implementation.generics.is_empty()
                    || implementation.generic_where_clause.is_some()
                    || implementation.meta_where_clause.is_some()
                    || implementation.specialize_attr.is_some()
                    || function.params.iter()
                    .any(|param| !matches!(param.kind, FunctionParamKind::Regular { .. })) { return Err(()); }
                selected = Some(function.clone());
            }
        }
        Ok(selected)
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
