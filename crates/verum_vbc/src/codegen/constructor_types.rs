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
            TypeRef::Generic(id) => self
                .ctx
                .current_generic_param_ids
                .iter()
                .find_map(|(name, actual)| (*actual == id.0).then(|| name.clone())),
            TypeRef::Instantiated { base, args } => {
                let owner = self.type_ref_to_field_name(&TypeRef::Concrete(*base))?;
                let args = args
                    .iter()
                    .map(|arg| self.declaration_type_name(arg, depth + 1))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!("{owner}<{}>", args.join(", ")))
            }
            TypeRef::Tuple(parts) => Some(format!(
                "({})",
                parts
                    .iter()
                    .map(|part| self.declaration_type_name(part, depth + 1))
                    .collect::<Option<Vec<_>>>()?
                    .join(", ")
            )),
            _ => self.type_ref_to_field_name(ty),
        }
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
            let associated = self
                .future_implementation_for_type(owner)?
                .associated_types
                .iter()
                .find(|(name, _)| {
                    self.ctx
                        .strings
                        .get(name.0 as usize)
                        .is_some_and(|name| name == "Output")
                })?;
            if !self.bind_source_declaration(&associated.1, expected_output, &mut bindings) {
                return None;
            }
        }
        let owner_ids: verum_common::List<_> =
            declaration.type_params.iter().map(|p| p.id).collect();
        for (field, value) in arguments {
            if !super::formal_parameters::resolved(field, &owner_ids, 0) {
                return None;
            }
            let contextual = bindings.apply(field);
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

    fn source_call_declaration(
        &self,
        expr: &Expr,
        expected: Option<&TypeRef>,
        depth: usize,
    ) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
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
                let ExprKind::Path(path) = &receiver.kind else {
                    return None;
                };
                let owner = path.to_string();
                // Only a declared type namespace selects a static callable.
                // A value receiver needs its separate self-argument evidence.
                if self.ctx.lookup_var(&owner).is_some()
                    || self
                        .semantic_type_in_current_scope(&owner)?
                        .base_type_id()
                        .is_none()
                {
                    return None;
                }
                (format!("{owner}.{}", method.name), type_args, args)
            }
            _ => return None,
        };
        let function = self
            .ctx
            .lookup_function_with_arity_in_scope(&name, args.len())?;
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
        if !type_args.is_empty() {
            if type_args.len() != function.type_param_ids.len() {
                return None;
            }
            for (id, argument) in function.type_param_ids.iter().zip(type_args) {
                let verum_ast::ty::GenericArg::Type(argument) = argument else {
                    return None;
                };
                let actual = self
                    .semantic_type_in_current_scope(&Self::extract_type_name_from_ast(argument))?;
                if !self.bind_source_declaration(&TypeRef::Generic(*id), &actual, &mut bindings) {
                    return None;
                }
            }
        }
        if let Some(expected) = expected {
            if !self.bind_source_declaration(return_type, expected, &mut bindings) {
                return None;
            }
        }
        for (formal, argument) in params.iter().zip(args) {
            let contextual = bindings.apply(formal);
            let actual = self.source_expression_declaration(
                argument,
                fully_bound(formal, &bindings).then_some(&contextual),
                depth + 1,
            )?;
            if !self.bind_source_declaration(formal, &actual, &mut bindings) {
                return None;
            }
        }
        fully_bound(return_type, &bindings).then(|| bindings.apply(return_type))
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
        let (_, output) = self
            .future_implementation_for_type(owner)?
            .associated_types
            .iter()
            .find(|(name, _)| {
                self.ctx
                    .strings
                    .get(name.0 as usize)
                    .is_some_and(|name| name == "Output")
            })?;
        let output = TypeSubstitution::new(&declaration.type_params, args).apply(output);
        self.resolve_future_projections(&output, depth + 1)
    }

    fn resolve_future_projections(&self, ty: &TypeRef, depth: usize) -> Option<TypeRef> {
        if depth >= 64 {
            return None;
        }
        Some(match ty {
            TypeRef::AssociatedProjection { base, assoc } if assoc == "Output" => {
                self.associated_future_output(base, depth + 1)?
            }
            TypeRef::Instantiated { base, args } => TypeRef::Instantiated {
                base: *base,
                args: args
                    .iter()
                    .map(|arg| self.resolve_future_projections(arg, depth + 1))
                    .collect::<Option<_>>()?,
            },
            TypeRef::Tuple(parts) => TypeRef::Tuple(
                parts
                    .iter()
                    .map(|part| self.resolve_future_projections(part, depth + 1))
                    .collect::<Option<_>>()?,
            ),
            _ => ty.clone(),
        })
    }
}

fn fully_bound(ty: &TypeRef, bindings: &TypeSubstitution) -> bool {
    match ty {
        TypeRef::Generic(id) => bindings.get(*id).is_some(),
        TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
            args.iter().all(|ty| fully_bound(ty, bindings))
        }
        TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => fully_bound(inner, bindings),
        TypeRef::Array { element, .. } => fully_bound(element, bindings),
        TypeRef::Function {
            params,
            return_type,
            ..
        }
        | TypeRef::Rank2Function {
            params,
            return_type,
            ..
        } => {
            params.iter().all(|ty| fully_bound(ty, bindings)) && fully_bound(return_type, bindings)
        }
        TypeRef::AssociatedProjection { base, .. } => fully_bound(base, bindings),
        _ => true,
    }
}
