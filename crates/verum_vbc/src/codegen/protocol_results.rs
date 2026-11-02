//! Return declarations of async methods on constrained receiver parameters.
use super::{CodegenError, CodegenResult, VbcCodegen};
use crate::types::TypeRef;
use verum_ast::{Expr, ExprKind};
use verum_common::Maybe;

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
        if self.current_protocol_bounds.is_empty() || !type_args.is_empty() {
            return None;
        }
        let TypeRef::Generic(id) = self.bound_receiver_declaration(receiver)? else {
            return None;
        };
        let mut protocols = self.current_protocol_bounds.get(&id)?.clone();
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
                    .is_some_and(|name| name == method.as_str())
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
                if params.len() != args.len() {
                    continue;
                }
                if output.is_some() {
                    return Some(Err(CodegenError::internal(format!(
                        "ambiguous protocol method `{}` on the awaited receiver",
                        method.as_str()
                    ))));
                }
                if !dispatch.is_async
                    || dispatch.has_generics
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
                output = Some(
                    crate::mono::TypeSubstitution::new(&declaration.type_params, arguments)
                        .apply(return_type),
                );
            }
            for parent in &declaration.protocols {
                protocols.push(TypeRef::Concrete(crate::types::TypeId(parent.protocol.0)));
            }
        }
        output.map(Ok)
    }
}
