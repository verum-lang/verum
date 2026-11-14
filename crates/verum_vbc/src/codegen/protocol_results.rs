//! Method declarations selected through protocol objects and constrained type parameters.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::TypeRef;
use verum_ast::{Expr, ExprKind};
use verum_common::{List, Maybe};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct BoundProtocol {
    application: TypeRef,
    associated: std::collections::BTreeMap<String, TypeRef>,
}

impl From<TypeRef> for BoundProtocol {
    fn from(application: TypeRef) -> Self {
        Self { application, associated: Default::default() }
    }
}

#[derive(Clone, Copy)]
enum Receiver<'a> {
    Value(Option<&'a TypeRef>),
    Static(&'a TypeRef),
    Operator { value: &'a TypeRef, owner: crate::types::TypeId },
}

pub(super) struct ProtocolMethod {
    pub owner: crate::types::TypeId,
    pub output: TypeRef,
    inputs: Vec<TypeRef>,
    pub is_async: bool,
}

impl VbcCodegen {
    /// Resolve an object's complete protocol applications without reparsing
    /// display names or assuming that a particular wrapper owns the object.
    pub(super) fn declared_dynamic_method(
        &self, object: &crate::types::DynamicProtocolType, method: &str, arity: usize,
    ) -> CodegenResult<Maybe<ProtocolMethod>> {
        use crate::types::{TypeId, TypeParamId};
        let invalid = |message: &str| CodegenError::new(
            CodegenErrorKind::InvalidSourceSignature(message.into()));
        let mut outer_ids = List::new();
        let mut opening = crate::mono::TypeSubstitution::empty();
        for &id in self.ctx.current_generic_param_ids.values() {
            let shifted = id.checked_add(1).ok_or_else(|| invalid(
                "dynamic method argument exceeds the binder index space"))?;
            outer_ids.push(TypeParamId(shifted));
            opening.bind(TypeParamId(shifted), TypeRef::Generic(TypeParamId(id)));
        }
        // The hidden subject is not the object interface itself. Calls that
        // expose it require an existential opening witness; never substitute
        // a caller's same-numbered parameter or invent an erased Self value.
        let open = |ty: &TypeRef| -> CodegenResult<TypeRef> {
            if !super::formal_parameters::resolved(ty, &outer_ids, 0) {
                return Err(invalid(
                    "dynamic method exposes an unresolved existential subject"));
            }
            opening.apply(ty).map_err(|error| invalid(&error.to_string()))
        };
        let protocols = object.bounds.iter().map(|bound| {
            let args = bound.args.iter().map(&open).collect::<CodegenResult<List<_>>>()?;
            let application = if args.is_empty() { TypeRef::Concrete(TypeId(bound.protocol.0)) }
                else { TypeRef::Instantiated { base: TypeId(bound.protocol.0), args: args.into() } };
            let associated = object.bindings.iter().filter(|binding| &binding.owner == bound)
                .map(|binding| open(&binding.value).map(|ty| (binding.member.to_string(), ty)))
                .collect::<CodegenResult<_>>()?;
            Ok(BoundProtocol { application, associated })
        }).collect::<CodegenResult<List<_>>>()?;
        self.protocol_method(protocols.into(), method, arity, Receiver::Value(None)).transpose()
    }

    /// Retain only positive, nominal constraints on this scope's type binders.
    pub(super) fn collect_receiver_protocols(
        &self,
        generics: &[verum_ast::ty::GenericParam],
        where_clause: Option<&verum_ast::ty::WhereClause>,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId],
        output: &mut std::collections::HashMap<crate::types::TypeParamId, Vec<BoundProtocol>>,
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
                let subject = TypeRef::Generic(crate::types::TypeParamId(id));
                let bound = (|| {
                    let (head, args) = match &ty.kind {
                        TypeKind::Generic { base, args } => (base.as_ref(), args.as_slice()),
                        _ => (ty, &[][..]),
                    };
                    let protocol = self.semantic_formal_type(head, scope, declared_ids)?.base_type_id()?;
                    let descriptor = self.type_by_id(protocol)?;
                    if descriptor.kind != crate::types::TypeKind::Protocol { return None; }
                    if !args.iter().any(|arg| matches!(arg, verum_ast::GenericArg::Binding(_)))
                        && let Some(application) = self.semantic_formal_type(ty, scope, declared_ids)
                    {
                        let arity = match &application { TypeRef::Instantiated { args, .. } => args.len(), _ => 0 };
                        if arity == descriptor.type_params.len() { return Some(BoundProtocol::from(application)); }
                    }
                    let mut positional = Vec::new();
                    let mut associated = std::collections::BTreeMap::new();
                    for arg in args {
                        match arg {
                            verum_ast::GenericArg::Binding(equation) => {
                                let rhs = self.semantic_formal_type(&equation.ty, scope, declared_ids)?;
                                if associated.insert(equation.name.name.to_string(), rhs).is_some() { return None; }
                            }
                            _ => {
                                let arg = arg.type_for_declared_slot()?;
                                positional.push(self.semantic_formal_type(&arg, scope, declared_ids)?);
                            }
                        }
                    }
                    let args = self.complete_nominal_arguments(protocol, positional, ty.span, &[], Some(&subject)).ok()?;
                    let application = if args.is_empty() { TypeRef::Concrete(protocol) }
                        else { TypeRef::Instantiated { base: protocol, args } };
                    Some(BoundProtocol { application, associated })
                })();
                if let Some(bound) = bound { output.entry(crate::types::TypeParamId(id)).or_default().push(bound); }

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
            self.protocol_method(protocols, method.as_str(), args.len(), Receiver::Value(Some(&TypeRef::Generic(id))))?
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
            .map(|name| self.nominal_type_id(name).map(|id| BoundProtocol::from(TypeRef::Concrete(id))))
            .collect::<Option<Vec<_>>>()?;
        self.protocol_method(protocols, method, arity, Receiver::Value(None))
    }

    /// Synchronous static members return a type instantiated with the actual
    /// caller's binder, including occurrences of Self inside the result.
    pub(super) fn bounded_static_output(
        &self, id: crate::types::TypeParamId, member: &str, arity: usize,
    ) -> Option<TypeRef> {
        let bounds = self.current_protocol_bounds.get(&id)?.clone();
        let selected = self.protocol_method(bounds, member, arity, Receiver::Static(&TypeRef::Generic(id)))?.ok()?;
        (!selected.is_async).then_some(selected.output)
    }

    pub(super) fn bounded_binary_output(&self, left: &TypeRef, protocol: &str, method: &str, right: &TypeRef) -> Option<TypeRef> {
        let TypeRef::Generic(id) = left else { return None };
        let protocols = self.current_protocol_bounds.get(id)?.clone();
        let owner = self.nominal_type_id(protocol)?;
        let selected = self.protocol_method(protocols, method, 1, Receiver::Operator { value: left, owner })?.ok()?;
        (!selected.is_async && selected.inputs.as_slice() == std::slice::from_ref(right))
            .then_some(selected.output)
    }

    fn protocol_method(
        &self,
        mut protocols: Vec<BoundProtocol>,
        method: &str,
        arity: usize,
        receiver: Receiver,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        let mut visited = Vec::new();
        let mut output = None;
        while let Some(protocol) = protocols.pop() {
            if visited.contains(&protocol) {
                continue;
            }
            visited.push(protocol.clone());
            let (owner, arguments) = match &protocol.application {
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
                !matches!(receiver, Receiver::Operator { owner: required, .. } if required != owner) &&
                self.ctx
                    .strings
                    .get(member.name.0 as usize)
                    .is_some_and(|name| name == method)
            }) {
                let dispatch = member.method_dispatch?;
                if matches!(receiver, Receiver::Static(_))
                    != (dispatch.receiver == verum_ast::method_dispatch::ReceiverKind::None) {
                    continue;
                }
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
                if dispatch.has_generics { return None; }
                let mut ids: Vec<_> = declaration
                    .type_params
                    .iter()
                    .map(|param| param.id)
                    .collect();
                let mut substitution = crate::mono::TypeSubstitution::new(&declaration.type_params, arguments);
                let actual = match receiver {
                    Receiver::Value(value) => value,
                    Receiver::Static(ty) | Receiver::Operator { value: ty, .. } => Some(ty),
                };
                if let Some(actual) = actual
                    && let Some(self_id) = declaration.source_declaration.as_ref()
                        .and_then(|source| crate::types::TypeParamId::protocol_self(source.generics.len()))
                {
                    ids.push(self_id);
                    substitution.bind(self_id, actual.clone());
                }
                if !super::formal_parameters::resolved(return_type, &ids, 0) {
                    return None;
                }
                let instantiated = match substitution.apply(return_type)
                {
                    Ok(output) => output,
                    Err(error) => return Some(Err(CodegenError::internal(error.to_string()))),
                };
                let specialize = |ty: TypeRef| {
                    if let TypeRef::AssociatedProjection { base, qualifier: None, assoc, args } = &ty
                        && args.is_empty() && actual == Some(&**base)
                        && let Some(binding) = protocol.associated.get(assoc)
                    { return binding.clone(); }
                    ty
                };
                let inputs = params.iter().map(|ty| substitution.apply(ty).ok().map(&specialize))
                    .collect::<Option<Vec<_>>>()?;
                output = Some(ProtocolMethod {
                    owner,
                    inputs,
                    output: specialize(instantiated),
                    is_async: dispatch.is_async,
                });
            }
            for parent in &declaration.protocols {
                protocols.push(BoundProtocol::from(TypeRef::Concrete(crate::types::TypeId(parent.protocol.0))));
            }
        }
        output.map(Ok)
    }
}
