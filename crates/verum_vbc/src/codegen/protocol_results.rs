//! Method declarations selected through protocol objects and constrained type parameters.
use super::{CodegenError, CodegenResult, VbcCodegen};
use crate::types::TypeRef;
use verum_ast::{Expr, ExprKind};
use verum_common::Maybe;

pub(super) struct ProtocolMethod {
    pub owner: crate::types::TypeId,
    pub output: TypeRef,
    is_async: bool,
}

impl VbcCodegen {
    /// Retain only positive, nominal constraints on this scope's type binders.
    pub(super) fn collect_receiver_protocols(
        &self,
        generics: &[verum_ast::ty::GenericParam],
        where_clause: Option<&verum_ast::ty::WhereClause>,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId],
        output: &mut std::collections::HashMap<crate::types::TypeParamId, Vec<TypeRef>>,
    ) {
        use verum_ast::ty::{
            GenericParamKind, TypeBound, TypeBoundKind, TypeKind, WherePredicateKind,
        };
        let mut collect = |name: &str, bounds: &[TypeBound]| {
            let Some(&id) = scope.get(name) else { return };
            for bound in bounds {
                let path;
                let ty = match &bound.kind {
                    TypeBoundKind::Protocol(p) => {
                        path = verum_ast::Type::new(TypeKind::Path(p.clone()), bound.span);
                        &path
                    }
                    TypeBoundKind::GenericProtocol(ty) => ty,
                    _ => continue,
                };
                if let Some(protocol) = self.semantic_formal_type(ty, scope, declared_ids)
                    && protocol
                        .base_type_id()
                        .and_then(|base| self.type_by_id(base))
                        .is_some_and(|desc| desc.kind == crate::types::TypeKind::Protocol)
                {
                    output
                        .entry(crate::types::TypeParamId(id))
                        .or_default()
                        .push(protocol);
                }
            }
        };
        for param in generics {
            if let GenericParamKind::Type { name, bounds, .. } = &param.kind {
                collect(name.as_str(), bounds);
            }
        }
        for predicate in where_clause
            .into_iter()
            .flat_map(|clause| &clause.predicates)
        {
            if let WherePredicateKind::Type { ty, bounds } = &predicate.kind
                && let Some(name) = Self::for_type_generic_param_name(ty)
            {
                collect(&name, bounds);
            }
        }
    }

    /// An async protocol method declares the value produced by await. Resolve
    /// the member through the receiver's own bounds before crossing the await.
    pub(super) fn bound_async_method_output(&self, call: &Expr) -> Maybe<CodegenResult<TypeRef>> {
        let ExprKind::MethodCall {
            receiver,
            method,
            type_args,
            args,
        } = &call.kind
        else {
            return None;
        };
        if !type_args.is_empty() {
            return None;
        }
        let selected = if let Some(name) = self.extract_expr_type_name(receiver)
            && Self::dyn_receiver_bounds(&name).is_some()
        {
            self.dynamic_method(&name, method.as_str(), args.len())?
        } else {
            let TypeRef::Generic(id) = self.bound_receiver_declaration(receiver)? else {
                return None;
            };
            let protocols = self.current_protocol_bounds.get(&id)?.clone();
            self.protocol_method(protocols, method.as_str(), args.len())?
        };
        match selected {
            Ok(member) if member.is_async => Some(Ok(member.output)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        }
    }

    /// A dynamic receiver with no usable source declaration has no known
    /// result. Other receiver families retain their own method-resolution path.
    pub(super) fn dynamic_call_result(
        &self,
        receiver: &str,
        method: &str,
        arity: usize,
        type_args: &[verum_ast::ty::GenericArg],
    ) -> Option<Option<TypeRef>> {
        Self::dyn_receiver_bounds(receiver)?;
        let result = if type_args.is_empty() {
            self.dynamic_method(receiver, method, arity)
                .and_then(Result::ok)
                .filter(|member| !member.is_async)
                .map(|member| member.output)
        } else {
            None
        };
        Some(result)
    }

    /// Look up a dynamic call in the receiver's declared protocol hierarchy.
    /// The selected owner also supplies the runtime dispatch key.
    pub(super) fn dynamic_method(
        &self,
        receiver: &str,
        method: &str,
        arity: usize,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        let protocols = Self::dyn_receiver_bounds(receiver)?
            .split('+')
            .map(|name| self.nominal_type_id(name).map(TypeRef::Concrete))
            .collect::<Option<Vec<_>>>()?;
        self.protocol_method(protocols, method, arity)
    }

    fn protocol_method(
        &self,
        mut protocols: Vec<TypeRef>,
        method: &str,
        arity: usize,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        let mut visited = Vec::new();
        let mut output = None;
        while let Some(protocol) = protocols.pop() {
            if visited.contains(&protocol) {
                continue;
            }
            visited.push(protocol.clone());
            let (owner, arguments) = match &protocol {
                TypeRef::Concrete(owner) => (*owner, &[][..]),
                TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                _ => return None,
            };
            let declaration = self.type_by_id(owner)?;
            if declaration.kind != crate::types::TypeKind::Protocol
                || arguments.len() != declaration.type_params.len()
            {
                return None;
            }
            for member in declaration.variants.iter().filter(|member| {
                self.ctx
                    .strings
                    .get(member.name.0 as usize)
                    .is_some_and(|name| name == method)
            }) {
                let dispatch = member.method_dispatch?;
                let TypeRef::Function {
                    params,
                    return_type,
                    ..
                } = member.payload.as_ref()?
                else {
                    return None;
                };
                if params.len() != arity {
                    continue;
                }
                if output.is_some() {
                    return Some(Err(CodegenError::internal(format!(
                        "ambiguous protocol method `{method}` on the receiver"
                    ))));
                }
                if dispatch.has_generics
                    || dispatch.receiver == verum_ast::method_dispatch::ReceiverKind::None
                {
                    return None;
                }
                let ids: Vec<_> = declaration
                    .type_params
                    .iter()
                    .map(|param| param.id)
                    .collect();
                if !super::formal_parameters::resolved(return_type, &ids, 0) {
                    return None;
                }
                let instantiated = match crate::mono::TypeSubstitution::new(&declaration.type_params, arguments)
                    .apply(return_type)
                {
                    Ok(output) => output,
                    Err(error) => return Some(Err(CodegenError::internal(error.to_string()))),
                };
                output = Some(ProtocolMethod {
                    owner,
                    output: instantiated,
                    is_async: dispatch.is_async,
                });
            }
            for parent in &declaration.protocols {
                protocols.push(TypeRef::Concrete(crate::types::TypeId(parent.protocol.0)));
            }
        }
        output.map(Ok)
    }
}
