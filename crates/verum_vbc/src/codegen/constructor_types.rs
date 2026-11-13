//! Constructor declarations bind owner parameters from fields and contextual
//! results. The owner's binder roster, never a same-spelled caller parameter,
//! determines substitution order.
use super::*;
use crate::{
    mono::TypeSubstitution,
    types::{TypeKind, TypeRef},
};
use std::borrow::Cow;
use verum_ast::{Expr, ExprKind};

impl VbcCodegen {
    pub(super) fn declaration_type_name(&self, ty: &TypeRef, depth: usize) -> Option<String> {
        if depth >= 64 {
            return None;
        }
        match ty {
            TypeRef::RecordFieldProjection { .. } => None,
            TypeRef::Generic(id) => self
                .ctx
                .current_generic_param_ids
                .iter()
                .find_map(|(name, actual)| (*actual == id.0).then(|| name.clone())),
            TypeRef::Instantiated { base, args } => {
                let owner = self.render_field_type_ref(&TypeRef::Concrete(*base), true)?;
                let args = args
                    .iter()
                    .map(|arg| self.declaration_type_name(arg, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!("{owner}<{}>", args.join(", ")))
            }
            TypeRef::Reference { inner, mutability, tier } => {
                use crate::types::{CbgrTier, Mutability};
                let tier = match tier {
                    CbgrTier::Tier0 => "",
                    CbgrTier::Tier1 => "checked ",
                    CbgrTier::Tier2 => "unsafe ",
                };
                let access = if *mutability == Mutability::Mutable { "mut " } else { "" };
                Some(format!("&{tier}{access}{}", self.declaration_type_name(inner, depth + 1)?))
            }
            TypeRef::Tuple(parts) => Some(format!(
                "({})",
                parts
                    .iter()
                    .map(|part| self.declaration_type_name(part, depth + 1))
                    .collect::<Option<Vec<_>>>()?
                    .join(", ")
            )),
            _ => self.render_field_type_ref(ty, true),
        }
    }

    /// Type of a field read from a selected sum arm, instantiated in the
    /// receiver's own telescope. This describes the emitted projection.
    pub(super) fn projected_variant_field(
        &self,
        ty: &TypeRef,
        tag: u32,
        field: u32,
    ) -> Option<TypeRef> {
        let (id, args) = match ty {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let owner = self.type_by_id(id)?;
        if owner.kind != crate::types::TypeKind::Sum || owner.type_params.len() != args.len() {
            return None;
        }
        let variant = owner.variants.iter().find(|v| v.tag == tag)?;
        let member = variant
            .fields
            .get(field as usize)?
            .declaration_type
            .as_ref()?;
        let ids: verum_common::List<_> = owner.type_params.iter().map(|param| param.id).collect();
        if !super::formal_parameters::resolved(member, &ids, 0) {
            return None;
        }
        let result = TypeSubstitution::new(&owner.type_params, args)
            .apply(member)
            .ok()?;
        self.declared_type_in_scope(&result).then_some(result)
    }

    pub(super) fn future_implementation_for_type(
        &self,
        owner: crate::types::TypeId,
    ) -> Option<&crate::types::ProtocolImpl> {
        self.type_by_id(owner)?
            .protocols
            .iter()
            .find(|implementation| {
                self.type_by_id(crate::types::TypeId(implementation.protocol.0))
                    .and_then(|protocol| self.ctx.strings.get(protocol.name.0 as usize))
                    .is_some_and(|name| {
                        verum_common::well_known_types::WellKnownProtocol::Future
                            .matches(name.rsplit('.').next().unwrap_or(name))
                    })
            })
    }
    /// Forward interfaces and compiled declarations can both describe one
    /// implementation. Only a declared, unambiguous member is evidence.
    pub(super) fn associated_binding<'a>(
        &'a self,
        implementations: impl Iterator<Item = &'a crate::types::ProtocolImpl>,
        member: &str,
    ) -> Option<&'a TypeRef> {
        let mut bindings = implementations.flat_map(|implementation| &implementation.associated_types)
            .filter(|(name, _)| self.ctx.strings.get(name.0 as usize)
                .is_some_and(|name| name == member))
            .map(|(_, ty)| ty);
        let first = bindings.next()?;
        bindings.all(|other| other == first).then_some(first)
    }

    /// Instantiate an associated type from the selected canonical protocol on
    /// this exact nominal owner, retaining any type arguments of the caller.
    pub(super) fn declared_protocol_member(
        &self, ty: &TypeRef, protocol: verum_common::well_known_types::WellKnownProtocol, member: &str,
    ) -> Option<TypeRef> {
        let (id, args) = match ty {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let owner = self.type_by_id(id)?;
        if owner.type_params.len() != args.len() { return None; }
        let implementations = owner.protocols.iter().filter(|implementation| {
            let Some(declaration) = self.type_by_id(crate::types::TypeId(implementation.protocol.0)) else {
                return false;
            };
            let name = self.ctx.strings.get(declaration.name.0 as usize);
            let module = declaration.origin_module.and_then(|id| self.ctx.strings.get(id.0 as usize));
            name.is_some_and(|name| protocol.matches(name.rsplit('.').next().unwrap_or(name)))
                && module.is_some_and(|module| protocol.canonical_archive_modules().contains(&module.as_str()))
        });
        let target = self.associated_binding(implementations, member)?;
        TypeSubstitution::new(&owner.type_params, args).apply(target).ok()
    }

    fn future_output_declaration(&self, owner: crate::types::TypeId) -> Option<&TypeRef> {
        let protocol = self.future_implementation_for_type(owner)?.protocol;
        self.associated_binding(
            self.type_by_id(owner)?.protocols.iter().filter(|implementation| implementation.protocol == protocol),
            "Output",
        )
    }

    pub(super) fn declared_type_in_scope(&self, ty: &TypeRef) -> bool {
        let ids: verum_common::List<_> = self
            .ctx
            .current_generic_param_ids
            .values()
            .copied()
            .map(crate::types::TypeParamId)
            .collect();
        super::formal_parameters::resolved(ty, &ids, 0)
    }

    /// Match a source declaration after validating the actual caller's scope.
    pub(super) fn bind_source_declaration(
        &self,
        pattern: &TypeRef,
        actual: &TypeRef,
        bindings: &mut TypeSubstitution,
    ) -> bool {
        self.declared_type_in_scope(actual)
            && crate::mono::match_declared_types(pattern, actual, bindings)
    }

    pub(super) fn constructor_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        output: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        let expr = Self::normalized_place(expr);
        let (owner_name, arguments) = match &expr.kind {
            ExprKind::Record {
                path,
                fields,
                base: None,
            } => {
                let name = path.to_string();
                let id = self.semantic_type_in_current_scope(&name)?.base_type_id()?;
                let declaration = self.type_by_id(id)?;
                if declaration.kind != TypeKind::Record || declaration.fields.len() != fields.len()
                {
                    return None;
                }
                let mut arguments = Vec::new();
                for field in &declaration.fields {
                    let name = self.ctx.strings.get(field.name.0 as usize)?;
                    let value = fields
                        .iter()
                        .find(|value| value.name.name.as_str() == name.as_str())?;
                    let value = value.value.as_ref().map(Cow::Borrowed).unwrap_or_else(|| {
                        Cow::Owned(Expr::path(verum_ast::Path::single(value.name.clone())))
                    });
                    arguments.push((field.declaration_type.as_ref()?, value));
                }
                (name, arguments)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                args,
                type_args,
            } if type_args.is_empty() => {
                let ExprKind::Path(path) = &receiver.kind else {
                    return None;
                };
                let name = path.to_string();
                let id = self.semantic_type_in_current_scope(&name)?.base_type_id()?;
                let declaration = self.type_by_id(id)?;
                if declaration.kind != TypeKind::Sum {
                    return None;
                }
                let variant = declaration.variants.iter().find(|variant| {
                    self.ctx
                        .strings
                        .get(variant.name.0 as usize)
                        .is_some_and(|name| name == method.name.as_str())
                })?;
                if variant.fields.len() != args.len() {
                    return None;
                }
                let arguments = variant
                    .fields
                    .iter()
                    .zip(args)
                    .map(|(field, argument)| {
                        Some((field.declaration_type.as_ref()?, Cow::Borrowed(argument)))
                    })
                    .collect::<Option<Vec<_>>>()?;
                (name, arguments)
            }
            ExprKind::Field {
                expr: receiver,
                field,
            } => {
                let ExprKind::Path(path) = &receiver.kind else {
                    return None;
                };
                let name = path.to_string();
                let id = self.semantic_type_in_current_scope(&name)?.base_type_id()?;
                let declaration = self.type_by_id(id)?;
                let variant = declaration.variants.iter().find(|variant| {
                    self.ctx
                        .strings
                        .get(variant.name.0 as usize)
                        .is_some_and(|name| name == field.name.as_str())
                })?;
                if !variant.fields.is_empty() || variant.arity != 0 {
                    return None;
                }
                (name, Vec::new())
            }
            _ => return None,
        };
        let owner = self
            .semantic_type_in_current_scope(&owner_name)?
            .base_type_id()?;
        let declaration = self.type_by_id(owner)?;
        let template = if declaration.type_params.is_empty() {
            TypeRef::Concrete(owner)
        } else {
            TypeRef::Instantiated {
                base: owner,
                args: declaration
                    .type_params
                    .iter()
                    .map(|p| TypeRef::Generic(p.id))
                    .collect(),
            }
        };
        let mut bindings = TypeSubstitution::empty();
        if let Some(expected) = expected {
            if !self.bind_source_declaration(&template, expected, &mut bindings) {
                return None;
            }
        }
        if let Some(expected_output) = output {
            let associated = self.future_output_declaration(owner)?;
            if !self.bind_source_declaration(associated, expected_output, &mut bindings) {
                return None;
            }
        }
        let owner_ids: verum_common::List<_> =
            declaration.type_params.iter().map(|p| p.id).collect();
        for (field, value) in arguments {
            if !super::formal_parameters::resolved(field, &owner_ids, 0) {
                return None;
            }
            let contextual = bindings.apply(field).ok()?;
            // An unbound owner parameter is not a caller parameter, even when
            // its numeric ID coincides. Context only travels after all free
            // owner parameters of the field have acquired actual bindings.
            let known = fully_bound(field, &bindings);
            let actual = self.source_expression_declaration(
                &value,
                known.then_some(&contextual),
                depth + 1,
            )?;
            if !self.bind_source_declaration(field, &actual, &mut bindings) {
                return None;
            }
        }
        let args = declaration
            .type_params
            .iter()
            .map(|p| bindings.get(p.id).cloned())
            .collect::<Option<Vec<_>>>()?;
        let result = if args.is_empty() {
            TypeRef::Concrete(owner)
        } else {
            TypeRef::Instantiated { base: owner, args }
        };
        Some(result)
    }

    pub(super) fn source_expression_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        let expr = Self::normalized_place(expr);
        if let Some(result) = self.lexical_call_result(expr) {
            return result;
        }
        if let Some(ty) = self.constructor_declaration(expr, expected, None, depth + 1) {
            return Some(ty);
        }
        let ty = self
            .source_call_declaration(expr, expected, depth + 1)
            .or_else(|| {
                (!matches!(expr.kind, ExprKind::Record { .. }))
                    .then(|| self.declared_expression_result(expr, None, 0, 0))
                    .flatten()
            })
            .or_else(|| {
                self.extract_expr_type_name(expr)
                    .or_else(|| self.infer_expr_type_name(expr))
                    .or_else(|| Self::literal_default_type_name(expr))
                    .and_then(|name| self.semantic_type_in_current_scope(&name))
            })?;
        if !self.declared_type_in_scope(&ty) {
            return None;
        }
        if let Some(id) = ty.base_type_id() {
            if let Some(owner) = self.type_by_id(id) {
                let arity = match &ty {
                    TypeRef::Instantiated { args, .. } => args.len(),
                    _ => 0,
                };
                if owner.type_params.len() != arity {
                    return None;
                }
            }
        }
        if let Some(expected) = expected {
            if expected != &ty {
                return None;
            }
        }
        Some(ty)
    }

    pub(super) fn admits_constructor_context(expr: &Expr) -> bool {
        matches!(Self::normalized_place(expr).kind,
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } | ExprKind::Record { .. })
    }

    /// A call's expected result can bind its own parameters. Only arguments
    /// fixed by that substitution can supply context to a nested expression.
    pub(super) fn contextual_call_parameters(
        &self, info: &FunctionInfo, explicit: &[verum_ast::ty::GenericArg], operands: &[Expr],
    ) -> Option<Vec<Option<TypeRef>>> {
        if !operands.iter().any(Self::admits_constructor_context) {
            return None;
        }
        let TypeRef::Function { params, return_type, .. } = info.callable_signature.as_ref()? else {
            return None;
        };
        let mut bindings = TypeSubstitution::empty();
        self.bind_source_type_arguments(info, explicit, &mut bindings)?;
        if let Some(expected) = self.ctx.expected_value_type.as_ref()
            && !self.bind_source_declaration(return_type, expected, &mut bindings)
        {
            return None;
        }
        params.iter().map(|formal| {
            if fully_bound(formal, &bindings) {
                Some(Some(bindings.apply(formal).ok()?))
            } else {
                Some(None)
            }
        }).collect()
    }

    /// A projected field belongs to its instantiated record, so its free
    /// arguments are caller witnesses even when none of them is concrete yet.
    pub(super) fn declared_argument_witnesses<'a>(
        &self, id: u32, count: usize, args: impl Iterator<Item = &'a Expr>,
        captured: Option<&[Option<TypeRef>]>,
    ) -> Option<Vec<TypeRef>> {
        let info = self.ctx.lookup_function_by_id(crate::module::FunctionId(id))?;
        self.selected_argument_witnesses(info, count, args, captured)
    }

    /// Synthetic constructors share no executable ID. Bind the callable that
    /// was actually selected, preserving its nominal target and parameters.
    pub(super) fn selected_argument_witnesses<'a>(
        &self, info: &FunctionInfo, count: usize, args: impl Iterator<Item = &'a Expr>,
        captured: Option<&[Option<TypeRef>]>,
    ) -> Option<Vec<TypeRef>> {
        if info.type_param_ids.is_empty() {
            return None;
        }
        // Async, generator and contextual functions do not have an ordinary
        // synchronous function result. Their source parameter declarations
        // still bind generic witnesses, independently of the result carrier.
        let signature = match info.callable_signature.as_ref() {
            Some(TypeRef::Function { params, return_type, .. }) => Some((params, return_type)),
            _ => None,
        };
        let declared = self.ctx.semantic_fn_params.get(&info.id);
        let parameter = |index| signature.and_then(|(params, _)| params.get(index))
            .or_else(|| declared?.get(index).and_then(Option::as_ref));
        if info.param_count as usize != count { return None; }
        let mut bindings = TypeSubstitution::empty();
        if let (Some((_, return_type)), Some(expected)) = (signature, self.ctx.expected_value_type.as_ref()) {
            // A surrounding call can have a different result than this operand.
            // A failed structural match contributes no partial substitutions.
            let mut result_bindings = TypeSubstitution::empty();
            if self.bind_source_declaration(return_type, expected, &mut result_bindings) {
                bindings = result_bindings;
            }
        }
        for (index, arg) in args.enumerate() {
            let formal = parameter(index)?;
            if !formal.is_generic() { continue; }
            let actual = if let Some(actual) = captured.and_then(|arguments| arguments.get(index)).and_then(Option::as_ref) {
                actual.clone()
            } else if matches!(
                Self::normalized_place(arg).kind, ExprKind::Call { .. } | ExprKind::MethodCall { .. }
            ) {
                let expected = fully_bound(formal, &bindings).then(|| bindings.apply(formal).ok()).flatten();
                self.source_expression_declaration(arg, expected.as_ref(), 0)?
            } else {
                self.compiled_callable_signature(arg).cloned().or_else(||
                    self.declared_expression_result(arg, None, 0, 0)).or_else(|| {
                    // A literal binding a still-free formal has the language's
                    // literal type. No register or rendered variable hint is evidence.
                    Self::literal_default_type_name(arg)
                        .and_then(|name| self.semantic_type_in_current_scope(&name))
                })?
            };
            let (mut formal, mut actual) = (formal, &actual);
            if index == 0 && info.param_names.first().is_some_and(|name| name == "self") {
                // A method's receiver can borrow implicitly. Its nested
                // arguments retain all reference and ownership distinctions.
                if let TypeRef::Reference { inner, .. } = formal {
                    formal = inner;
                    if let TypeRef::Reference { inner, .. } = actual { actual = inner; }
                }
            }
            self.bind_declared_operand(info, index, formal, actual, &mut bindings)?;
        }
        info.type_param_ids.iter().map(|id| bindings.get(*id).cloned()).collect()
    }

    fn bind_declared_operand(&self, function: &FunctionInfo, index: usize, formal: &TypeRef,
        actual: &TypeRef, bindings: &mut TypeSubstitution) -> Option<()> {
        self.bind_source_declaration(formal, actual, bindings).then_some(())?;
        if matches!(formal, TypeRef::Generic(_)) && matches!(actual, TypeRef::Function { .. })
            && let Some(bound) = self.callable_parameter_bound(function, index)
        {
            let bound = self.resolve_declared_associated_types(bound).unwrap_or_else(|| bound.clone());
            self.bind_source_declaration(&bound, actual, bindings).then_some(())?;
        }
        Some(())
    }

    /// Only the callable's explicit slots are named at the call site. An
    /// impl receiver can contribute additional parameters to its full roster.
    pub(super) fn bind_source_type_arguments(
        &self,
        function: &FunctionInfo,
        arguments: &[verum_ast::ty::GenericArg],
        bindings: &mut TypeSubstitution,
    ) -> Option<()> {
        let slots = &function.explicit_type_param_ids;
        if arguments.len() > slots.len() {
            return None;
        }
        for (index, (slot, argument)) in slots.iter().zip(arguments).enumerate() {
            let Some(actual) = self.callable_argument_witness(function, index, argument, None).ok()? else { continue; };
            if !self.bind_source_declaration(&TypeRef::Generic((*slot)?), &actual, bindings) {
                return None;
            }
        }
        Some(())
    }

    /// An in-scope callee shadows every module declaration, even when its
    /// result cannot be recovered. The outer option records that distinction.
    pub(super) fn lexical_call_result(&self, expr: &Expr) -> Option<Option<TypeRef>> {
        let ExprKind::Call { func, .. } = &expr.kind else { return None };
        let ExprKind::Path(path) = &Self::normalized_place(func).kind else { return None };
        let name = path.as_ident()?;
        self.ctx.lookup_var(name.as_str())?;
        let result = self.declared_expression_result(func, None, 0, 0).and_then(|ty| match ty {
            TypeRef::Function { return_type, .. } => Some(*return_type),
            TypeRef::Generic(id) => self.ctx.registers.callable_bound_results.get(&id).cloned(),
            _ => None,
        });
        Some(result)
    }

    pub(super) fn source_call_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        if let Some(result) = self.lexical_call_result(expr) {
            return result;
        }
        let (name, type_args, args) = match &expr.kind {
            ExprKind::Call {
                func,
                type_args,
                args,
            } => {
                let ExprKind::Path(path) = &func.kind else {
                    return None;
                };
                (path.to_string(), type_args, args)
            }
            ExprKind::MethodCall {
                receiver,
                method,
                type_args,
                args,
            } => {
                // Dotted free functions and static methods use the same source
                // namespace grammar. Flatten only namespace paths; a value
                // receiver needs a separately established self declaration.
                let owner = self.try_flatten_module_path_resolved(receiver)?.join(".");
                (format!("{owner}.{}", method.name), type_args, args)
            }
            _ => return None,
        };
        let function = self
            .ctx
            .lookup_function_with_arity_in_scope(&name, args.len())?;
        self.source_call_result(function, type_args, args, expected, depth)
    }

    pub(super) fn source_call_result(
        &self,
        function: &FunctionInfo,
        type_args: &[verum_ast::ty::GenericArg],
        args: &[Expr],
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        let TypeRef::Function {
            params,
            return_type,
            ..
        } = function.callable_signature.as_ref()?
        else {
            return None;
        };
        if params.len() != args.len() {
            return None;
        }
        let mut bindings = TypeSubstitution::empty();
        self.bind_source_type_arguments(function, type_args, &mut bindings)?;
        if let Some(expected) = expected {
            if !self.bind_source_declaration(return_type, expected, &mut bindings) {
                return None;
            }
        }
        for (index, (formal, argument)) in params.iter().zip(args).enumerate() {
            let contextual = bindings.apply(formal).ok()?;
            let actual = self.compiled_callable_signature(argument).cloned().or_else(|| self.source_expression_declaration(
                argument,
                fully_bound(formal, &bindings).then_some(&contextual),
                depth + 1,
            ))?;
            self.bind_declared_operand(function, index, formal, &actual, &mut bindings)?;
        }
        fully_bound(return_type, &bindings).then(|| bindings.apply(return_type).ok()).flatten()
    }

    pub(super) fn associated_future_output(&self, ty: &TypeRef, depth: usize) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        let (owner, args) = match ty {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let declaration = self.type_by_id(owner)?;
        if declaration.type_params.len() != args.len() {
            return None;
        }
        let output = self.future_output_declaration(owner)?;
        let output = TypeSubstitution::new(&declaration.type_params, args).apply(output).ok()?;
        self.resolve_declared_projections(&output, depth + 1)
    }

    fn resolve_declared_projections(&self, ty: &TypeRef, depth: usize) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        Some(match ty {
            // Associated-type resolution cannot choose a record receiver.
            TypeRef::RecordFieldProjection { owner, ordinal } => TypeRef::RecordFieldProjection {
                owner: *owner, ordinal: *ordinal,
            },
            TypeRef::AssociatedProjection { base, qualifier, assoc, args } => {
                if qualifier.is_some() || !args.is_empty() { return None; }
                let base = self.resolve_declared_projections(base, depth + 1)?;
                let (owner, args) = match &base {
                    TypeRef::Concrete(id) => (*id, &[][..]),
                    TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                    _ => return None,
                };
                let declaration = self.type_by_id(owner)?;
                if declaration.type_params.len() != args.len() {
                    return None;
                }
                let binding = self.associated_binding(declaration.protocols.iter(), assoc)?;
                let declared = TypeSubstitution::new(&declaration.type_params, args).apply(binding).ok()?;
                self.resolve_declared_projections(&declared, depth + 1)?
            }
            TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
                base: *base,
                args: args
                    .iter()
                    .map(|arg| self.resolve_declared_projections(arg, depth + 1))
                    .collect::<Option<_>>()?,
            },
            TypeRef::Tuple(parts) => TypeRef::Tuple(
                parts
                    .iter()
                    .map(|part| self.resolve_declared_projections(part, depth + 1))
                    .collect::<Option<_>>()?,
            ),
            _ => ty.clone(),
        })
    }
}

pub(super) fn fully_bound(ty: &TypeRef, bindings: &TypeSubstitution) -> bool {
    fully_bound_at(ty, bindings, 0)
}

/// Locally quantified parameters are bound by the enclosing function type.
/// Only free parameters, lowered back into the caller's scope, need witnesses.
fn fully_bound_at(ty: &TypeRef, bindings: &TypeSubstitution, depth: u32) -> bool {
    match ty {
        TypeRef::RecordFieldProjection { .. } => false,
        TypeRef::Generic(id) => {
            let index = u32::from(id.0);
            index < depth || bindings.get(crate::types::TypeParamId((index - depth) as u16)).is_some()
        }
        TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
            args.iter().all(|ty| fully_bound_at(ty, bindings, depth))
        }
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => fully_bound_at(inner, bindings, depth),
        TypeRef::Array { element, .. } => fully_bound_at(element, bindings, depth),
        TypeRef::Function {
            params,
            return_type,
            ..
        } => {
            params.iter().all(|ty| fully_bound_at(ty, bindings, depth))
                && fully_bound_at(return_type, bindings, depth)
        }
        TypeRef::Rank2Function {
            type_param_count,
            params,
            return_type,
            ..
        } => {
            let Some(depth) = depth.checked_add(u32::from(*type_param_count)) else { return false; };
            params.iter().all(|ty| fully_bound_at(ty, bindings, depth))
                && fully_bound_at(return_type, bindings, depth)
        }
        TypeRef::AssociatedProjection { base, qualifier, args, .. } => fully_bound_at(base, bindings, depth)
            && args.iter().all(|argument| fully_bound_at(argument, bindings, depth))
            && qualifier.as_ref().is_none_or(|application| application.args.iter().all(|argument| fully_bound_at(argument, bindings, depth))),
        _ => true,
    }
}

#[cfg(test)]
mod binder_tests {
    use super::*;
    use crate::types::TypeParamId;

    #[test]
    fn negation_does_not_supply_literal_evidence_for_other_domains() {
        for source in ["-true", "-\"text\"", "-2_u16", "-(3_km)", "-x"] {
            let expression = verum_fast_parser::Parser::new(source).parse_expr().unwrap();
            assert_eq!(VbcCodegen::literal_default_type_name(&expression), None, "{source}");
        }
    }

    #[test]
    fn record_projection_requires_a_receiver_not_a_generic_binding() {
        let projection = TypeRef::RecordFieldProjection { owner: TypeId(2048), ordinal: 0 };
        let mut bindings = TypeSubstitution::empty();
        bindings.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
        assert!(!fully_bound(&projection, &bindings));
        let cg = VbcCodegen::new();
        assert_eq!(cg.resolve_declared_projections(&projection, 0), Some(projection.clone()));
        assert!(cg.declaration_type_name(&projection, 0).is_none());
        assert!(!fully_bound(&TypeRef::Rank2Function {
            is_async: false,
            type_param_count: 1, params: vec![TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(projection), contexts: Default::default(),
        }, &bindings));
    }

    #[test]
    fn quantified_local_is_bound_but_free_slot_requires_its_outer_witness() {
        let quantified = |index| TypeRef::Rank2Function {
            is_async: false,
            type_param_count: 1,
            params: vec![TypeRef::Generic(TypeParamId(0))],
            return_type: Box::new(TypeRef::Generic(TypeParamId(index))),
            contexts: Default::default(),
        };
        let mut bindings = TypeSubstitution::empty();
        assert!(fully_bound(&quantified(0), &bindings));
        assert!(!fully_bound(&quantified(1), &bindings));
        bindings.bind(TypeParamId(1), TypeRef::Concrete(TypeId::BOOL));
        assert!(!fully_bound(&quantified(1), &bindings));
        bindings.bind(TypeParamId(0), TypeRef::Concrete(TypeId::INT));
        assert!(fully_bound(&quantified(1), &bindings));
    }
}
