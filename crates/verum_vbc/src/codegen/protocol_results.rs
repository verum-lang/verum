//! Method declarations selected through protocol objects and constrained type parameters.
use super::{CodegenError, CodegenErrorKind, CodegenResult, VbcCodegen};
use crate::types::{TypeId, TypeRef};
use verum_ast::{Expr, ExprKind};
use verum_common::{List, Map, Maybe};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct BoundProtocol {
    pub(super) application: TypeRef,
    pub(super) associated: std::collections::BTreeMap<String, TypeRef>,
}

impl From<TypeRef> for BoundProtocol {
    fn from(application: TypeRef) -> Self {
        Self { application, associated: Default::default() }
    }
}

#[derive(Clone, Copy)]
enum Receiver<'a> {
    Value(Option<&'a TypeRef>),
    Static(&'a TypeRef, Option<StaticMethodCall<'a>>),
    Operator { value: &'a TypeRef, owner: crate::types::TypeId },
}

#[derive(Clone, Copy)]
struct StaticMethodCall<'a> {
    operands: &'a [Expr],
    explicit: &'a [verum_ast::ty::GenericArg],
    depth: usize,
}

pub(super) struct ProtocolMethod {
    pub owner: crate::types::TypeId,
    pub application: TypeRef,
    pub slot: u32,
    pub receiver: verum_ast::method_dispatch::ReceiverKind,
    pub output: TypeRef,
    pub inputs: Vec<TypeRef>,
    pub method_arguments: Vec<TypeRef>,
    pub is_async: bool,
}

impl VbcCodegen {
    /// In a protocol default, a bare `Self` in expression position denotes the
    /// implementing type. The lexical protocol selects the associated function.
    pub(super) fn default_static_member(
        &self,
        receiver: &Expr,
        method: &str,
        arity: usize,
    ) -> CodegenResult<Maybe<ProtocolMethod>> {
        if !matches!(&receiver.kind, ExprKind::Path(path)
            if path.as_ident().is_some_and(|name| name.as_str() == "Self"))
        {
            return Ok(None);
        }
        let Some(scope) = &self.current_deferred_impl else {
            return Ok(None);
        };
        let Some(application) = &scope.protocol else {
            return Ok(None);
        };
        if application.protocol.and_then(|id| self.type_by_id(id)).is_none() {
            return Ok(None);
        }
        let declared = self.declared_protocol_application(application).ok_or_else(||
            CodegenError::with_span(CodegenErrorKind::InvalidSourceSignature(format!(
                "incomplete protocol application for Self.{method}"
            )), receiver.span))?;
        self.protocol_method(
            List::from_iter([declared.into()]),
            method,
            arity,
            Receiver::Static(&scope.receiver.ty, None),
        )
        .transpose()
    }

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
        self.protocol_method(protocols, method, arity, Receiver::Value(None)).transpose()
    }

    /// Retain positive, nominal constraints on their complete declared subjects.
    /// A projection or a conditional nominal Self has the same authority as a
    /// bare type parameter; sharing a member name does not establish a bound.
    pub(super) fn collect_receiver_protocols(
        &self,
        generics: &[verum_ast::ty::GenericParam],
        where_clause: Option<&verum_ast::ty::WhereClause>,
        scope: &std::collections::HashMap<String, u16>,
        declared_ids: &[crate::types::TypeParamId],
        output: &mut Map<TypeRef, List<BoundProtocol>>,
    ) {
        let declaration = super::formal_parameters::DeclarationTypeScope {
            owner: self.lexical_type_scope(), substitution: &self.current_source_substitution,
            receiver: self.current_impl_receiver.as_ref(),
        };
        self.visit_declared_protocol_bounds(generics, where_clause, scope, declared_ids, &declaration,
            |subject| self.protocol_bound_subject(subject), |subject, mut bound| {
            for target in bound.associated.values_mut() {
                let Some(known) = self.protocol_bound_subject(target) else { return; };
                *target = known;
            }
            output.entry(subject.clone()).or_default().push(bound);
        });
    }

    pub(super) fn visit_declared_protocol_bounds(
        &self, generics: &[verum_ast::ty::GenericParam], where_clause: Option<&verum_ast::ty::WhereClause>,
        scope: &std::collections::HashMap<String, u16>, declared_ids: &[crate::types::TypeParamId],
        declaration: &super::formal_parameters::DeclarationTypeScope<'_>,
        subject_in_scope: impl Fn(&TypeRef) -> Option<TypeRef>,
        mut visit: impl FnMut(&TypeRef, BoundProtocol),
    ) {
        use verum_ast::ty::{
            GenericParamKind, TypeBound, TypeBoundKind, TypeKind, WherePredicateKind,
        };
        let mut collect = |subject: TypeRef, bounds: &[TypeBound]| {
            let Some(subject) = subject_in_scope(&subject) else { return; };
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
                if let Some(bound) = self.declaration_protocol_bound(ty, &subject, scope, declared_ids, declaration) {
                    visit(&subject, bound);
                }
            }
        };
        for param in generics {
            if let GenericParamKind::Type { name, bounds, .. } = &param.kind
                && let Some(&id) = scope.get(name.as_str()) {
                collect(TypeRef::Generic(crate::types::TypeParamId(id)), bounds);
            }
        }
        for predicate in where_clause
            .into_iter()
            .flat_map(|clause| &clause.predicates)
        {
            if let WherePredicateKind::Type { ty, bounds } = &predicate.kind
                && let Some(subject) = self.semantic_formal_type_in_declaration(ty, scope, declared_ids, &Default::default(), declaration)
            {
                collect(subject, bounds);
            }
        }
    }

    /// Elaborate a bound in its declaring telescope. Its associated bindings
    /// are still callee terms here; caller premises must not rewrite them.
    fn declaration_protocol_bound(
        &self, ty: &verum_ast::Type, subject: &TypeRef,
        scope: &std::collections::HashMap<String, u16>,
        ids: &[crate::types::TypeParamId],
        declaration: &super::formal_parameters::DeclarationTypeScope<'_>,
    ) -> Maybe<BoundProtocol> {
        use verum_ast::{GenericArg, TypeKind};
        let semantic = |ty: &verum_ast::Type| self.semantic_formal_type_in_declaration(
            ty, scope, ids, &Default::default(), declaration);
        let (head, args) = match &ty.kind {
            TypeKind::Generic { base, args } => (base.as_ref(), args.as_slice()),
            _ => (ty, &[][..]),
        };
        let protocol = semantic(head)?.base_type_id()?;
        let descriptor = self.type_by_id(protocol)?;
        if descriptor.kind != crate::types::TypeKind::Protocol { return None; }
        if !args.iter().any(|arg| matches!(arg, GenericArg::Binding(_)))
            && let Some(application) = semantic(ty)
        {
            let arity = match &application { TypeRef::Instantiated { args, .. } => args.len(), _ => 0 };
            if arity == descriptor.type_params.len() { return Some(application.into()); }
        }
        let mut positional = Vec::new();
        let mut associated = std::collections::BTreeMap::new();
        for arg in args {
            match arg {
                GenericArg::Binding(equation) => {
                    if associated.insert(equation.name.name.to_string(), semantic(&equation.ty)?).is_some() {
                        return None;
                    }
                }
                _ => positional.push(semantic(&arg.type_for_declared_slot()?)?),
            }
        }
        let args = self.complete_nominal_arguments(protocol, positional, ty.span, &[], Some(subject)).ok()?;
        let application = if args.is_empty() { TypeRef::Concrete(protocol) }
            else { TypeRef::Instantiated { base: protocol, args } };
        Some(BoundProtocol { application, associated })
    }

    /// Receiver-owned associated equations determine implementation parameters
    /// omitted from Self's nominal arguments. A free function's own bounds use
    /// the same operation. Method-local bounds have a separate lexical owner;
    /// they are not reinterpreted as bounds from the implementation's source.
    pub(super) fn complete_declared_associated_arguments(
        &self, function: &super::FunctionInfo, descriptor: &crate::module::FunctionDescriptor,
        bindings: &mut crate::mono::TypeSubstitution,
    ) -> Maybe<()> {
        use super::{constructor_types::fully_bound, formal_parameters::DeclarationTypeScope};
        use verum_ast::ty::{GenericParamKind, TypeBoundKind};
        let Some(roster) = self.callable_generic_roster(function) else { return Some(()); };
        let scope = self.callable_owner_generic_scope(function, descriptor)?;
        let retained: List<_> = roster.iter().filter(|parameter|
            parameter.witness_slot.is_some_and(|id| scope.get(&parameter.name) == Some(&id.0))).collect();
        let ids: List<_> = scope.values().copied().map(crate::types::TypeParamId).collect();
        let owner = match descriptor.origin_module {
            Some(id) => self.ctx.strings.get(id.0 as usize)?.as_str(),
            None => self.config.module_name.as_str(),
        };
        let receiver = descriptor.source_receiver.as_ref().map(|ty|
            super::ImplReceiverWitness { ty: ty.clone(), semantic: true });
        let empty = super::source_substitution::SourceTypeSubstitution::default();
        let declaration = DeclarationTypeScope { owner, substitution: &empty, receiver: receiver.as_ref() };
        for parameter in retained {
            let Some(source) = parameter.source.as_deref() else { continue; };
            let GenericParamKind::Type { bounds, .. } = &source.kind else { continue; };
            let Some(id) = parameter.witness_slot else { continue; };
            let Some(subject) = bindings.get(id).cloned() else { continue; };
            for bound in bounds {
                let TypeBoundKind::GenericProtocol(ty) = &bound.kind else { continue; };
                if !matches!(&ty.kind, verum_ast::TypeKind::Generic { args, .. }
                    if args.iter().any(|arg| matches!(arg, verum_ast::GenericArg::Binding(_))))
                { continue; }
                let required = self.declaration_protocol_bound(
                    ty, &TypeRef::Generic(id), &scope, &ids, &declaration)?;
                if !fully_bound(&required.application, bindings) { continue; }
                let application = bindings.apply(&required.application).ok()?;
                for (member, formal) in required.associated {
                    let actual = self.declared_bound_associated_target(&subject, &application, &member)?;
                    if !self.bind_source_declaration(&formal, &actual, bindings) { return None; }
                }
            }
        }
        Some(())
    }

    pub(super) fn bound_member_owner(
        &self, bounds: List<BoundProtocol>, subject: &TypeRef, member: &str,
    ) -> Maybe<TypeRef> {
        let hierarchy = self.protocol_hierarchy(bounds, Some(subject))?;
        let mut member_owner = None;
        for bound in &hierarchy {
            let owner = self.type_by_id(bound.application.base_type_id()?)?;
            if crate::associated_types::declared_members(owner)?.any(|entry| entry == (member, 0)) {
                if member_owner.as_ref().is_some_and(|prior| prior != &bound.application) { return None; }
                member_owner = Some(bound.application.clone());
            }
        }
        member_owner
    }

    /// Infer an unqualified nominal member's owner only when its declaration
    /// is unique and its application requires no missing protocol arguments.
    fn nominal_member_owner(&self, subject: &TypeRef, member: &str) -> Maybe<crate::types::ProtocolApplication> {
        let mut selected = None;
        for row in &self.type_by_id(subject.base_type_id()?)?.protocols {
            let owner = self.type_by_id(TypeId(row.protocol.0))?;
            let Some((_, arity)) = crate::associated_types::declared_members(owner)?.find(|(name, _)| *name == member)
                else { continue; };
            if arity != 0 || !owner.type_params.is_empty() || selected.is_some_and(|prior| prior != row.protocol) {
                return None;
            }
            selected = Some(row.protocol);
        }
        selected.map(|protocol| crate::types::ProtocolApplication { protocol, args: Vec::new() })
    }

    /// Discharge concrete projections before serializing a frame operation.
    /// Open projections keep their exact declaring application for substitution.
    pub(super) fn runtime_member_witness(&mut self, ty: TypeRef) -> TypeRef {
        fn qualify(
            cg: &mut VbcCodegen, ty: &TypeRef, depth: usize, remaining: &mut usize,
        ) -> CodegenResult<TypeRef> {
            if depth >= crate::types::MAX_TYPE_REF_DEPTH || *remaining == 0 {
                return Err(CodegenError::internal("runtime member witness exceeds its type budget"));
            }
            *remaining -= 1;
            let mut ty = super::type_equalities::map_children(ty, 0, &mut |part, binders| {
                if binders == 0 { qualify(cg, part, depth + 1, remaining) } else { Ok(part.clone()) }
            })?;
            if let TypeRef::AssociatedProjection { base, qualifier, assoc, args } = &mut ty && args.is_empty() {
                if qualifier.is_none() {
                    *qualifier = match cg.current_protocol_bounds.get(base.as_ref()) {
                        Some(bounds) => cg.bound_member_owner(bounds.clone(), base, assoc).and_then(|application| {
                            let (owner, args) = match application {
                                TypeRef::Concrete(owner) => (owner, Vec::new()),
                                TypeRef::Instantiated { base, args } => (base, args),
                                _ => return None,
                            };
                            Some(crate::types::ProtocolApplication { protocol: crate::types::ProtocolId(owner.0), args })
                        }),
                        None => cg.nominal_member_owner(base, assoc),
                    };
                }
                if let Some(application) = qualifier && crate::mono::is_concrete_type(base)
                    && let Some(value) = cg.proven_protocol_member(base, TypeId(application.protocol.0),
                        &application.args, assoc, depth + 1, remaining)
                { return Ok(value); }
            }
            Ok(ty)
        }
        if matches!(ty, TypeRef::Generic(_) | TypeRef::Concrete(_)) || crate::mono::is_concrete_type(&ty) {
            return ty;
        }
        let mut remaining = crate::types::MAX_TYPE_REF_NODES;
        qualify(self, &ty, 0, &mut remaining).unwrap_or(ty)
    }

    /// A same-named member of another protocol is not an equality witness.
    /// Keep both the entire application and the caller's actual subject.
    pub(super) fn declared_bound_associated_target(
        &self, subject: &TypeRef, application: &TypeRef, member: &str,
    ) -> Maybe<TypeRef> {
        let subject = self.body_declaration_type(subject)?;
        let application = self.body_declaration_type(application)?;
        let (owner, args) = match &application {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let member_owner = self.bound_member_owner(List::from_iter([application.clone().into()]), &subject, member)?;
        let mut selected = None;
        if let Some(bounds) = self.current_protocol_bounds.get(&subject) {
            for bound in bounds {
                let hierarchy = self.protocol_hierarchy(List::from_iter([bound.clone()]), Some(&subject))?;
                if !hierarchy.iter().any(|bound| bound.application == application) { continue; }
                for bound in hierarchy.iter().filter(|bound| bound.application == member_owner) {
                    let Some(target) = bound.associated.get(member) else { continue; };
                    let target = self.body_declaration_type(target)?;
                    if selected.as_ref().is_some_and(|prior| prior != &target) { return None; }
                    selected = Some(target);
                }
            }
        }
        if selected.is_some() { return selected; }
        self.selected_protocol_implementation_member(&subject, owner, args, member)
    }

    /// A retained implementation equation can identify a projected subject
    /// with its declared target. Otherwise the projection remains symbolic:
    /// failure to resolve it supplies neither a replacement type nor a bound.
    fn protocol_bound_subject(&self, subject: &TypeRef) -> Maybe<TypeRef> {
        self.body_declaration_type(subject)
    }

    pub(super) fn declared_bound_method(
        &self, receiver: &TypeRef, method: &str, arity: usize,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        let subject = self.protocol_bound_subject(receiver)?;
        let protocols = self.current_protocol_bounds.get(&subject)?.clone();
        self.protocol_method(protocols, method, arity, Receiver::Value(Some(&subject)))
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
        let selected = if let Some(declaration) = self.source_expression_declaration(receiver, None, 0) {
            match self.declared_receiver_method(&declaration, method.as_str(), args.len() + 1) {
                Ok(Some(super::expressions::index_projection::DeclaredReceiverMethod::Protocol(member))) => Ok(member),
                Ok(_) => return None,
                Err(error) => Err(error),
            }
        } else if let Some(name) = self.extract_expr_type_name(receiver)
            && Self::dyn_receiver_bounds(&name).is_some()
        {
            self.dynamic_method(&name, method.as_str(), args.len())?
        } else {
            return None;
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
            .collect::<Option<List<_>>>()?;
        self.protocol_method(protocols, method, arity, Receiver::Value(None))
    }

    /// Synchronous static members return a type instantiated with the actual
    /// caller's binder, including occurrences of Self inside the result.
    pub(super) fn bounded_static_method(
        &self, receiver: &TypeRef, member: &str, operands: &[Expr],
        explicit: &[verum_ast::ty::GenericArg], depth: usize,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        if depth >= 64 { return None; }
        let subject = self.protocol_bound_subject(receiver)?;
        let bounds = self.current_protocol_bounds.get(&subject)?.clone();
        let call = StaticMethodCall { operands, explicit, depth };
        self.protocol_method(bounds, member, operands.len(), Receiver::Static(&subject, Some(call)))
    }

    pub(super) fn bounded_binary_method(
        &self, left: &TypeRef, protocol: &str, method: &str, right: &TypeRef,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        let subject = self.protocol_bound_subject(left)?;
        let protocols = self.current_protocol_bounds.get(&subject)?.clone();
        let owner = self.nominal_type_id(protocol)?;
        let selected = self.protocol_method(protocols, method, 1, Receiver::Operator { value: left, owner })?;
        match selected {
            Ok(selected) if !selected.is_async && selected.inputs.as_slice() == std::slice::from_ref(right) => Some(Ok(selected)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        }
    }

    pub(super) fn bounded_binary_output(&self, left: &TypeRef, protocol: &str, method: &str, right: &TypeRef) -> Option<TypeRef> {
        self.bounded_binary_method(left, protocol, method, right)?.ok().map(|selected| selected.output)
    }

    /// Traverse the declared applications shared by method and associated-member
    /// selection. Inherited arguments stay in their owner's telescope until the
    /// complete application is substituted; bare parent IDs cannot supply them.
    pub(super) fn protocol_hierarchy(
        &self, protocols: List<BoundProtocol>, subject: Maybe<&TypeRef>,
    ) -> Maybe<List<BoundProtocol>> {
        use super::formal_parameters::DeclarationTypeScope;
        use crate::{mono::TypeSubstitution, types::TypeParamId};
        let mut pending = protocols;
        let mut selected = List::new();
        while let Some(protocol) = pending.pop() {
            if selected.contains(&protocol) { continue; }
            if selected.len() >= 256 || pending.len() >= 256 { return None; }
            let (owner, args) = match &protocol.application {
                TypeRef::Concrete(owner) => (*owner, &[][..]),
                TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                _ => return None,
            };
            let descriptor = self.type_by_id(owner)?;
            if descriptor.kind != crate::types::TypeKind::Protocol
                || args.len() != descriptor.type_params.len() { return None; }
            if let Some(source) = &descriptor.source_declaration {
                let verum_ast::TypeDeclBody::Protocol(body) = &source.body else { return None; };
                let owner = match descriptor.origin_module {
                    Some(id) => self.ctx.strings.get(id.0 as usize)?.as_str(),
                    None => self.config.module_name.as_str(),
                };
                let mut scope = std::collections::HashMap::new();
                let mut ids = List::new();
                for parameter in &descriptor.type_params {
                    let name = self.ctx.strings.get(parameter.name.0 as usize)?.clone();
                    if scope.insert(name, parameter.id.0).is_some() { return None; }
                    ids.push(parameter.id);
                }
                let self_id = TypeParamId::protocol_self(source.generics.len())?;
                ids.push(self_id);
                let receiver = super::ImplReceiverWitness { ty: TypeRef::Generic(self_id), semantic: true };
                let empty = super::source_substitution::SourceTypeSubstitution::default();
                let lexical = DeclarationTypeScope { owner, substitution: &empty, receiver: Some(&receiver) };
                let mut substitution = TypeSubstitution::new(&descriptor.type_params, args);
                if let Some(subject) = subject { substitution.bind(self_id, subject.clone()); }
                for parent in &body.extends {
                    let mut parent = self.declaration_protocol_bound(parent, &receiver.ty, &scope, &ids, &lexical)?;
                    parent.application = substitution.apply(&parent.application).ok()?;
                    for target in parent.associated.values_mut() { *target = substitution.apply(target).ok()?; }
                    for (member, target) in &protocol.associated {
                        if parent.associated.get(member).is_some_and(|prior| prior != target) { return None; }
                        parent.associated.insert(member.clone(), target.clone());
                    }
                    pending.push(parent);
                }
            } else {
                // Old nonparametric descriptors carry exact parent identities.
                // An erased generic application does not acquire arguments here.
                if !descriptor.type_params.is_empty() && !descriptor.protocols.is_empty() { return None; }
                for parent in &descriptor.protocols {
                    let id = crate::types::TypeId(parent.protocol.0);
                    if !self.type_by_id(id)?.type_params.is_empty() { return None; }
                    pending.push(BoundProtocol { application: TypeRef::Concrete(id), associated: protocol.associated.clone() });
                }
            }
            selected.push(protocol);
        }
        Some(selected)
    }

    /// Bind the method's own parameters in its protocol telescope before
    /// substituting caller types. An implementation can choose different names
    /// and witness slots for the same positional parameters.
    fn bind_static_method_arguments(
        &self,
        protocol: &crate::types::TypeDescriptor,
        member: &crate::types::VariantDescriptor,
        name: &str,
        formals: &[TypeRef],
        call: StaticMethodCall<'_>,
        bindings: &mut crate::mono::TypeSubstitution,
    ) -> CodegenResult<Vec<(crate::types::TypeParamId, TypeRef)>> {
        use verum_ast::decl::{ProtocolItemKind, TypeDeclBody};
        use crate::types::TypeParamId;

        let invalid = |reason: &str| CodegenError::new(CodegenErrorKind::TypeInference(
            format!("static protocol method `{name}`: {reason}")));
        let source = protocol.source_declaration.as_ref().ok_or_else(|| invalid("missing protocol source declaration"))?;
        let TypeDeclBody::Protocol(body) = &source.body else { return Err(invalid("invalid protocol declaration")); };
        let declaration = body.items.iter().filter_map(|item| match &item.kind {
            ProtocolItemKind::Function { decl, .. } => Some(decl), _ => None,
        }).nth(member.tag as usize).filter(|decl| decl.name.as_str() == name)
            .ok_or_else(|| invalid("source declaration differs from selected slot"))?;
        let mut owner = std::collections::HashMap::new();
        for parameter in &protocol.type_params {
            let key = self.ctx.strings.get(parameter.name.0 as usize).ok_or_else(|| invalid("missing protocol binder"))?;
            owner.insert(key.to_string(), parameter.id.0);
        }
        let self_id = TypeParamId::protocol_self(source.generics.len()).ok_or_else(|| invalid("invalid Self binder"))?;
        owner.insert("Self".into(), self_id.0);
        if declaration.generics.iter().any(|p| p.is_implicit || !matches!(p.kind, verum_ast::ty::GenericParamKind::Type { .. })) {
            return Err(invalid("method parameter requires a declaration-owned non-type or implicit witness"));
        }
        let (dense, shadow) = Self::extend_function_generic_scope(declaration, owner);
        let slots = Self::explicit_generic_param_ids(declaration, &dense, &shadow)
            .into_iter().collect::<Option<Vec<_>>>().ok_or_else(|| invalid("unsupported method binder"))?;
        if call.explicit.len() > slots.len() { return Err(invalid("too many explicit arguments")); }
        for (id, argument) in slots.iter().zip(call.explicit) {
            let ty = argument.type_for_declared_slot().ok_or_else(|| invalid("expected a declared type argument"))?;
            if !matches!(ty.kind, verum_ast::TypeKind::Inferred) {
                let actual = self.explicit_type_witness(&ty, None).ok_or_else(|| invalid("unresolved explicit argument"))?;
                if !self.bind_source_declaration(&TypeRef::Generic(*id), &actual, bindings) {
                    return Err(invalid("conflicting explicit argument"));
                }
            }
        }
        for (formal, operand) in formals.iter().zip(call.operands) {
            let expected = super::constructor_types::fully_bound(formal, bindings)
                .then(|| bindings.apply(formal).ok()).flatten();
            let actual = self.source_expression_declaration(operand, expected.as_ref(), call.depth + 1);
            if !actual.is_some_and(|actual| self.bind_source_declaration(formal, &actual, bindings)) {
                return Err(invalid("operand does not establish its declared type").at_source(operand.span));
            }
        }
        slots.into_iter().map(|id| {
            bindings.get(id).cloned().map(|ty| (id, ty)).ok_or_else(|| invalid("missing method type argument"))
        }).collect()
    }

    fn protocol_method(
        &self,
        protocols: List<BoundProtocol>,
        method: &str,
        arity: usize,
        receiver: Receiver,
    ) -> Maybe<CodegenResult<ProtocolMethod>> {
        let subject = match receiver {
            Receiver::Value(value) => value,
            Receiver::Static(ty, _) | Receiver::Operator { value: ty, .. } => Some(ty),
        };
        let protocols = self.protocol_hierarchy(protocols, subject)?;
        let mut output = None;
        for protocol in protocols {
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
                if matches!(receiver, Receiver::Static(_, _))
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
                let mut ids: Vec<_> = declaration
                    .type_params
                    .iter()
                    .map(|param| param.id)
                    .collect();
                let mut substitution = crate::mono::TypeSubstitution::new(&declaration.type_params, arguments);
                let actual = match receiver {
                    Receiver::Value(value) => value,
                    Receiver::Static(ty, _) | Receiver::Operator { value: ty, .. } => Some(ty),
                };
                let self_id = declaration.source_declaration.as_ref()
                    .and_then(|source| crate::types::TypeParamId::protocol_self(source.generics.len()));
                let members = if let (Some(actual), Some(self_id)) = (actual, self_id) {
                    ids.push(self_id);
                    substitution.bind(self_id, actual.clone());
                    Some((self_id, &protocol.associated))
                } else {
                    None
                };
                let method_arguments = if dispatch.has_generics {
                    let Receiver::Static(_, Some(call)) = receiver else { return None; };
                    match self.bind_static_method_arguments(declaration, member, method, params, call, &mut substitution) {
                        Ok(arguments) => {
                            ids.extend(arguments.iter().map(|(id, _)| *id));
                            arguments.into_iter().map(|(_, ty)| ty).collect()
                        }
                        Err(error) => return Some(Err(error)),
                    }
                } else if matches!(receiver, Receiver::Static(_, Some(call)) if !call.explicit.is_empty()) {
                    return Some(Err(CodegenError::new(CodegenErrorKind::TypeInference(format!(
                        "non-generic static protocol method `{method}` takes no type arguments"
                    )))));
                } else { Vec::new() };
                if !super::formal_parameters::resolved(return_type, &ids, 0) {
                    return None;
                }
                let specialize = |ty| substitution.apply_with_members(ty, members);
                let instantiated = match specialize(return_type) {
                    Ok(output) => output,
                    Err(error) => return Some(Err(CodegenError::internal(error.to_string()))),
                };
                let inputs = match params.iter().map(specialize).collect::<Result<_, _>>() {
                    Ok(inputs) => inputs,
                    Err(error) => return Some(Err(CodegenError::internal(error.to_string()))),
                };
                output = Some(ProtocolMethod {
                    owner,
                    application: protocol.application.clone(),
                    slot: member.tag,
                    receiver: dispatch.receiver,
                    inputs,
                    method_arguments,
                    output: instantiated,
                    is_async: dispatch.is_async,
                });
            }
        }
        output.map(Ok)
    }
}
