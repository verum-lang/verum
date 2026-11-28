//! Resolve carried associated bindings in the consumer's nominal pool.

use super::VbcCodegen;
use crate::{
    mono::TypeSubstitution,
    types::{TypeId, TypeRef},
};
use verum_common::{List, Map, Set};

/// A written protocol qualifier must be elaborated as an exact application.
/// The current source lowerer cannot produce that witness, so it may not erase
/// the qualifier and publish an unqualified projection instead.
pub(super) fn require_projection_application_evidence(ty: &verum_ast::Type) -> super::CodegenResult<()> {
    if matches!(&ty.kind, verum_ast::TypeKind::Qualified { trait_ref, .. } if !trait_ref.segments.is_empty()) {
        return Err(super::CodegenError::with_span(super::CodegenErrorKind::InvalidSourceSignature(
            "qualified associated projection requires declaration-owned protocol application elaboration".into()), ty.span));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/codegen/associated_member_selection.rs"]
mod associated_member_selection;

/// Source identities for fully specialized implementation heads. The ABI maps both
/// pointer-width integers to PTR; it cannot distinguish their implementations.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ConcreteImplType {
    Resolved(TypeRef),
    PointerWidthInteger { signed: bool },
    Applied(TypeId, Vec<Self>),
}

impl ConcreteImplType {
    fn disjoint(&self, other: &Self, codegen: &VbcCodegen) -> bool {
        match (self, other) {
            (Self::Resolved(ty), head) | (head, Self::Resolved(ty)) => head.contradicts(ty, codegen),
            (Self::Applied(a, xs), Self::Applied(b, ys)) =>
                a != b || xs.iter().zip(ys).any(|(x, y)| x.disjoint(y, codegen)),
            (Self::PointerWidthInteger { signed: a }, Self::PointerWidthInteger { signed: b }) =>
                a != b,
            // A pointer-width scalar cannot have a nominal type application.
            (Self::PointerWidthInteger { .. }, Self::Applied(..))
            | (Self::Applied(..), Self::PointerWidthInteger { .. }) => true,
        }
    }

    fn contradicts(&self, expected: &TypeRef, codegen: &VbcCodegen) -> bool {
        match self {
            Self::Resolved(actual) => codegen.declared_types_disjoint(actual, expected),
            // The source guarantees a scalar on every target. Its bit width is irrelevant.
            Self::PointerWidthInteger { .. } => matches!(expected,
                TypeRef::Instantiated { .. } | TypeRef::Reference { .. } | TypeRef::Tuple(_)
                    | TypeRef::Array { .. } | TypeRef::Slice(_) | TypeRef::Function { .. }),
            Self::Applied(base, args) => match expected {
                TypeRef::Concrete(id) => base != id,
                TypeRef::Instantiated {
                    base: other,
                    args: required,
                } => {
                    base != other
                        || args
                            .iter()
                            .zip(required.iter())
                            .any(|(a, b)| a.contradicts(b, codegen))
                }
                _ => false,
            },
        }
    }
}

enum MemberEquation {
    Known(TypeRef),
    Conditional {
        source: verum_ast::source_implementation::AssociatedImplementationSource,
        target: TypeRef,
        carried: Option<TypeRef>,
    },
}

impl VbcCodegen {
    /// An associated equation belongs to its complete implementation head.
    /// The nominal table deliberately omits equations of concrete instances;
    /// it cannot decide this query, nor prove an implementation's prerequisites.
    pub(super) fn selected_implementation_member(
        &self,
        actual: &TypeRef,
        protocol: verum_common::well_known_types::WellKnownProtocol,
        member: &str,
    ) -> Option<TypeRef> {
        let selected = self.nominal_protocol_role(actual, protocol)?;
        self.selected_protocol_implementation_member(actual, selected, &[], member)
    }

    fn nominal_protocol_role(
        &self, actual: &TypeRef, protocol: verum_common::well_known_types::WellKnownProtocol,
    ) -> Option<TypeId> {
        let owner = self.type_by_id(actual.base_type_id()?)?;
        let mut protocols = owner.protocols.iter().filter(|row|
            self.declared_protocol_role(TypeId(row.protocol.0)) == Some(protocol))
            .map(|row| TypeId(row.protocol.0));
        let selected = protocols.next()?;
        if protocols.any(|other| other != selected) { return None; }
        Some(selected)
    }

    /// A conditional assignment is usable only after checking the exact
    /// retained implementation and its declaration-local projection owners.
    pub(super) fn proven_implementation_member(
        &mut self, actual: &TypeRef, role: verum_common::well_known_types::WellKnownProtocol, member: &str,
    ) -> Option<TypeRef> {
        let protocol = self.nominal_protocol_role(actual, role)?;
        let mut remaining = crate::types::MAX_TYPE_REF_NODES;
        self.proven_protocol_member(actual, protocol, &[], member, 0, &mut remaining)
    }

    pub(super) fn proven_protocol_member(
        &mut self, actual: &TypeRef, protocol: TypeId, args: &[TypeRef], member: &str,
        depth: usize, remaining: &mut usize,
    ) -> Option<TypeRef> {
        *remaining = remaining.checked_sub(1)?;
        if depth >= crate::types::MAX_TYPE_REF_DEPTH { return None; }
        match self.implementation_member_candidate(actual, protocol, args, member)? {
            MemberEquation::Known(value) => Some(value),
            MemberEquation::Conditional { source, target, carried } => {
                let proof = self.proven_member_bounds(&source, actual, protocol, args).ok()??;
                if let Some(carried) = carried && carried != proof.bindings.apply(&target).ok()? {
                    return None;
                }
                self.instantiate_member_equation(&target, &proof.bindings, &proof.bounds, depth + 1, remaining)
            }
        }
    }

    fn instantiate_member_equation(
        &mut self, ty: &TypeRef, bindings: &TypeSubstitution,
        bounds: &Map<TypeRef, List<super::protocol_results::BoundProtocol>>,
        depth: usize, remaining: &mut usize,
    ) -> Option<TypeRef> {
        *remaining = remaining.checked_sub(1)?;
        if depth >= crate::types::MAX_TYPE_REF_DEPTH { return None; }
        match ty {
            TypeRef::AssociatedProjection { base, qualifier: None, assoc, args } if args.is_empty() => {
                // Choose the member in the implementation's original binder
                // space. Two unrelated parameters may later become the same type.
                let protocol = self.bound_member_owner(bounds.get(base.as_ref())?.clone(), base, assoc)?;
                let application = bindings.apply(&protocol).ok()?;
                let actual = self.instantiate_member_equation(base, bindings, bounds, depth + 1, remaining)?;
                if let Some(value) = self.declared_bound_associated_target(&actual, &application, assoc) {
                    return Some(value);
                }
                let (owner, arguments) = match &application {
                    TypeRef::Concrete(owner) => (*owner, &[][..]),
                    TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
                    _ => return None,
                };
                self.proven_protocol_member(&actual, owner, arguments, assoc, depth + 1, remaining)
            }
            TypeRef::AssociatedProjection { .. } | TypeRef::RecordFieldProjection { .. } => None,
            TypeRef::Generic(_) | TypeRef::Dynamic(_) | TypeRef::Rank2Function { .. } => bindings.apply(ty).ok(),
            _ => super::type_equalities::map_children(ty, 0, &mut |part, _| {
                self.instantiate_member_equation(part, bindings, bounds, depth + 1, remaining)
                    .ok_or_else(|| super::CodegenError::internal("unresolved implementation equation"))
            }).ok(),
        }
    }

    /// The same retained implementation proof also serves ordinary declared
    /// protocol applications; a member spelling alone cannot select an owner.
    pub(super) fn selected_protocol_implementation_member(
        &self, actual: &TypeRef, protocol: TypeId, protocol_arguments: &[TypeRef], member: &str,
    ) -> Option<TypeRef> {
        match self.implementation_member_candidate(actual, protocol, protocol_arguments, member)? {
            MemberEquation::Known(value) => Some(value),
            MemberEquation::Conditional { .. } => None,
        }
    }

    fn implementation_member_candidate(
        &self, actual: &TypeRef, protocol: TypeId, protocol_arguments: &[TypeRef], member: &str,
    ) -> Option<MemberEquation> {
        use super::{formal_parameters::DeclarationTypeScope, source_substitution::SourceTypeSubstitution};
        use crate::{mono::match_declared_types, types::TypeParamId};
        use verum_ast::{decl::{ImplItemKind, ImplKind}, ty::GenericParamKind};

        let (id, arguments) = match actual {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let owner = self.type_by_id(id)?;
        if owner.type_params.len() != arguments.len() || !self.declared_type_in_scope(actual) {
            return None;
        }
        let empty = SourceTypeSubstitution::default();
        let mut seen = Set::new();
        let mut selected = None;
        for row in owner.protocols.iter().filter(|row| TypeId(row.protocol.0) == protocol)
        {
            let source = row.source_implementation.as_ref()?;
            let repeated_source = !seen.insert(source);
            let implementation = source.declaration().ok()?;
            let ImplKind::Protocol { protocol: path, protocol_args, for_type } = &implementation.kind else {
                return None;
            };
            if self.nominal_type_id_in_scope(&Self::format_type_path(path), source.origin_module())
                    != Some(TypeId(row.protocol.0))
            { return None; }
            let mut names = Vec::new();
            let mut conditional = false;
            for parameter in &implementation.generics {
                match &parameter.kind {
                    GenericParamKind::Type { name, bounds, .. } => {
                        if names.len() >= usize::from(u16::MAX) { return None; }
                        names.push(name.name.to_string());
                        conditional |= !bounds.is_empty();
                    }
                    GenericParamKind::Lifetime { .. } => {}
                    _ => return None,
                }
            }
            let parameters = self.impl_generic_scope_for_owner(Some(id), &names);
            if parameters.len() != names.len() { return None; }
            let ids: List<_> = parameters.values().copied().map(TypeParamId).collect();
            let lexical = DeclarationTypeScope {
                owner: source.origin_module(), substitution: &empty, receiver: None,
            };
            let head = self.semantic_formal_type_in_declaration(
                for_type, &parameters, &ids, &Default::default(), &lexical,
            )?;
            let mut bindings = TypeSubstitution::empty();
            if !match_declared_types(&head, actual, &mut bindings)
                || bindings.apply(&head).ok()?.ne(actual)
            { continue; }
            let receiver = super::ImplReceiverWitness { ty: head, semantic: true };
            let lexical = DeclarationTypeScope { receiver: Some(&receiver), ..lexical };
            let mut declared_arguments = List::new();
            for argument in protocol_args {
                let argument = argument.type_for_declared_slot()?;
                let declared = self.semantic_formal_type_in_declaration(
                    &argument, &parameters, &ids, &Default::default(), &lexical,
                )?;
                declared_arguments.push(bindings.apply(&declared).ok()?);
            }
            // The source may omit protocol arguments supplied by its declared
            // defaults. Complete that same telescope with the actual receiver
            // before comparing applications; spelling arity is not identity.
            let declared_arguments = self.complete_nominal_arguments(
                protocol, declared_arguments.into(), implementation.span, &[], Some(actual),
            ).ok()?;
            if declared_arguments != protocol_arguments { continue; }
            conditional |= implementation.generic_where_clause.as_ref().is_some_and(|clause| !clause.predicates.is_empty())
                || implementation.meta_where_clause.as_ref().is_some_and(|clause| !clause.predicates.is_empty());
            if implementation.specialize_attr.is_some() || (!conditional && ids.iter().any(|id| bindings.get(*id).is_none()))
            { return None; }
            let mut assignments = implementation.items.iter().filter_map(|item| match &item.kind {
                ImplItemKind::Type { name, ty, type_params } if name.name.as_str() == member =>
                    Some((ty, type_params)),
                _ => None,
            });
            let (body, family) = assignments.next()?;
            if !family.is_empty() || assignments.next().is_some()
                || (selected.is_some() && !repeated_source) { return None; }
            let body = self.semantic_formal_type_in_declaration(
                body, &parameters, &ids, &Default::default(), &lexical,
            )?;
            // A retained unconditional row must agree with its source. It is
            // never a replacement for a missing or contradictory source head.
            let mut carried = row.associated_types.iter().filter(|(name, _)|
                self.ctx.strings.get(name.0 as usize).is_some_and(|name| name == member));
            let carried = match carried.next() {
                Some((_, witness)) if carried.next().is_none() =>
                    Some(TypeSubstitution::new(&owner.type_params, arguments).apply(witness).ok()?),
                Some(_) => return None,
                None => None,
            };
            selected = Some(if conditional {
                MemberEquation::Conditional { source: source.clone(), target: body, carried }
            } else {
                let value = bindings.apply(&body).ok()?;
                if !self.declared_type_in_scope(&value) || carried.is_some_and(|other| other != value) { return None; }
                MemberEquation::Known(value)
            });
        }
        selected
    }

    /// Capture the nominal declarations a foreign implementor will need when
    /// checking inherited default signatures and bodies. Resolve them as
    /// ordinary signatures in this file, including their generic binders.
    pub(super) fn retain_default_method_types(&mut self, files: &[&verum_ast::Module]) {
        use verum_ast::{ItemKind, TypeDeclBody, Visitor};
        use verum_common::{Map, Text};
        struct Names<'a> {
            cg: &'a VbcCodegen,
            scope: &'a std::collections::HashMap<String, u16>,
            ids: &'a [crate::types::TypeParamId],
            bindings: &'a mut Map<Text, Option<Text>>,
        }
        impl Visitor for Names<'_> {
            fn visit_type(&mut self, ty: &verum_ast::Type) {
                if let verum_ast::TypeKind::Path(path) = &ty.kind {
                    if let Some(TypeRef::Concrete(id)) = self.cg.semantic_formal_type(ty, self.scope, self.ids)
                        && let Some(descriptor) = self.cg.type_by_id(id)
                        && descriptor.kind != crate::types::TypeKind::Primitive
                        && let Some(owner) = self.cg.type_declaration_identity(descriptor)
                    {
                        self.bindings.insert(path.to_string().replace("::", ".").into(), Some(owner));
                    }
                } else {
                    verum_ast::visitor::walk_type(self, ty);
                }
            }
        }
        for file in files {
            let owner = self.source_file_owner(file, &self.config.module_name);
            let previous = self.ctx.current_source_module.replace(owner.clone());
            let mut bindings = Map::new();
            for item in &file.items {
                let ItemKind::Type(ty) = &item.kind else { continue };
                if !matches!(ty.body, TypeDeclBody::Protocol(_)) { continue; }
                let protocol_owner = crate::module::qualify_module_name(&owner, ty.name.name.as_str());
                let Some(protocol) = self.protocol_registry.get(&protocol_owner) else { continue };
                let parent_names: Vec<_> = ty.generics.iter().filter_map(|parameter| {
                    use verum_ast::ty::GenericParamKind;
                    match &parameter.kind {
                        GenericParamKind::Type { name, .. } | GenericParamKind::HigherKinded { name, .. } =>
                            Some(name.name.to_string()),
                        _ => None,
                    }
                }).collect();
                for method in protocol.default_methods.values() {
                    let (mut scope, shadow) = Self::declaration_generic_param_maps(method, &parent_names, &parent_names);
                    let ids = Self::ordered_generic_param_ids(&scope, &shadow);
                    scope.extend(shadow);
                    Names { cg: self, scope: &scope, ids: &ids, bindings: &mut bindings }.visit_function(method);
                }
            }
            self.ctx.current_source_module = previous;
            if !bindings.is_empty() {
                let lexical = self.source_type_mounts.entry(owner.into()).or_default();
                for (name, binding) in bindings { lexical.entry(name).or_insert_with(|| binding.into()); }
            }
        }
    }

    fn concrete_impl_type(&self, ty: &verum_ast::Type, depth: usize) -> Option<ConcreteImplType> {
        use verum_ast::ty::{GenericArg, TypeKind};
        use verum_common::well_known_types::type_names;
        if depth >= 64 {
            return None;
        }
        if let Some(ty) = self.semantic_formal_type(ty, &std::collections::HashMap::new(), &[]) {
            return (!ty.is_generic()).then_some(ConcreteImplType::Resolved(ty));
        }
        match &ty.kind {
            TypeKind::Path(path) if path.is_single() => {
                let name = path.to_string();
                (type_names::is_pointer_width_integer_type(&name)
                    && self.nominal_type_id(&name) == Some(TypeId::PTR))
                .then(|| ConcreteImplType::PointerWidthInteger {
                    signed: type_names::is_signed_integer_type(&name),
                })
            }
            TypeKind::Generic { base, args } => {
                let ConcreteImplType::Resolved(TypeRef::Concrete(id)) =
                    self.concrete_impl_type(base, depth + 1)?
                else {
                    return None;
                };
                let args = args
                    .iter()
                    .map(|arg| match arg {
                        GenericArg::Type(arg) => self.concrete_impl_type(arg, depth + 1),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(ConcreteImplType::Applied(id, args))
            }
            _ => None,
        }
    }

    /// `Range<Int>::Item = Int` belongs to one instantiation, so it cannot be
    /// published as an unconditional associated binding on `Range<T>`.
    pub(super) fn record_concrete_impl_bindings(
        &mut self,
        implementation: &verum_ast::decl::ImplDecl,
        selected_items: &[&verum_ast::decl::ImplItem],
    ) -> Option<ConcreteImplType> {
        use verum_ast::decl::{ImplItemKind, ImplKind};
        let ImplKind::Protocol {
            for_type, protocol, ..
        } = &implementation.kind
        else {
            return None;
        };
        if !implementation.generics.is_empty() {
            return None;
        }
        if !matches!(&for_type.kind, verum_ast::ty::TypeKind::Generic { .. }) {
            return None;
        }
        let receiver = self.concrete_impl_type(for_type, 0)?;
        let protocol_name = protocol.to_string().replace("::", ".");
        let protocol = crate::types::ProtocolId(self.nominal_type_id(&protocol_name)?.0);
        let facts: std::collections::HashMap<_, _> = selected_items
            .iter()
            .filter_map(|item| {
                let ImplItemKind::Type { name, ty, .. } = &item.kind else {
                    return None;
                };
                Some((name.name.to_string(), self.concrete_impl_type(ty, 0)?))
            })
            .collect();
        self.concrete_impl_bindings
            .entry((receiver.clone(), protocol))
            .and_modify(|previous| {
                previous.retain(|member, binding| facts.get(member) == Some(binding))
            })
            .or_insert(facts);
        Some(receiver)
    }

    /// A default on one closed specialization cannot use the protocol
    /// implementations of its siblings. A generic or unavailable head leaves
    /// the condition open for the ordinary obligation checker.
    pub(super) fn default_self_bound_contradicts(
        &mut self,
        function: &verum_ast::FunctionDecl,
        receiver: TypeId,
        instance: Option<&ConcreteImplType>,
    ) -> super::CodegenResult<bool> {
        use verum_ast::{decl::ImplKind, ty::{TypeBoundKind, WherePredicateKind}};
        let (Some(clause), Some(instance)) = (&function.generic_where_clause, instance) else {
            return Ok(false);
        };
        for predicate in &clause.predicates {
            let WherePredicateKind::Type { ty, bounds } = &predicate.kind else { continue };
            if !Self::ast_type_is_bare_self(ty) { continue; }
            for bound in bounds {
                let path = match &bound.kind {
                    TypeBoundKind::Protocol(path) => path,
                    TypeBoundKind::GenericProtocol(ty) => Self::protocol_application_parts(ty)?.0,
                    _ => continue,
                };
                let Some(required) = self.nominal_type_id(&Self::format_type_path(path)) else {
                    continue;
                };
                let Some(descriptor) = self.type_by_id(receiver) else { continue };
                let sources: Vec<_> = descriptor.protocols.iter()
                    .filter(|row| row.protocol.0 == required.0)
                    .map(|row| row.source_implementation.clone()).collect();
                if sources.is_empty() { continue; }
                let mut disjoint = true;
                for source in sources {
                    let Some(source) = source else { disjoint = false; break };
                    let declaration = source.declaration().map_err(|error| super::CodegenError::with_span(
                        super::CodegenErrorKind::InvalidSourceSignature(error.to_string()), bound.span,
                    ))?;
                    let ImplKind::Protocol { for_type, .. } = &declaration.kind else {
                        return Err(super::CodegenError::internal("protocol evidence requires an implementation head"));
                    };
                    if !declaration.generics.is_empty() { disjoint = false; break; }
                    let head = self.with_protocol_source_scope(
                        Some(source.origin_module().to_owned()), Default::default(), None,
                        |cg| Ok(cg.concrete_impl_type(for_type, 0)),
                    )?;
                    if head.as_ref().is_none_or(|head| !head.disjoint(instance, self)) {
                        disjoint = false;
                        break;
                    }
                }
                if disjoint && !self.has_blanket_for_protocol(required)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn has_blanket_for_protocol(&mut self, required: TypeId) -> super::CodegenResult<bool> {
        use verum_ast::decl::ImplKind;
        for index in 0..self.blanket_impls.len() {
            let blanket = &self.blanket_impls[index];
            let ImplKind::Protocol { protocol, .. } = &blanket.declaration.kind else { continue };
            let path = Self::format_type_path(protocol);
            let selected = self.with_protocol_source_scope(
                blanket.source_module.clone(), Default::default(), None,
                |cg| Ok(cg.nominal_type_id(&path)),
            )?;
            if selected == Some(required) { return Ok(true); }
        }
        Ok(false)
    }

    /// A method slot may cover several specializations of the same nominal receiver.
    /// Omit it only when one explicit associated equality conflicts with all their bindings.
    pub(super) fn default_binding_contradicts(
        &self,
        function: &verum_ast::FunctionDecl,
        receiver_name: &str,
        receiver: crate::types::TypeId,
        protocol: crate::types::ProtocolId,
        instance: Option<&ConcreteImplType>,
    ) -> bool {
        use verum_ast::ty::{TypeBoundKind, TypeKind, WherePredicateKind};
        let Some(clause) = &function.generic_where_clause else {
            return false;
        };
        let Some(descriptor) = self.type_by_id(receiver) else {
            return false;
        };
        let (mut scope, shadow) =
            self.function_generic_param_maps(function, Some(receiver_name), &[]);
        let declared_ids = Self::ordered_generic_param_ids(&scope, &shadow);
        scope.extend(shadow);

        clause.predicates.iter().any(|predicate| {
            let WherePredicateKind::Type { ty, bounds } = &predicate.kind else {
                return false;
            };
            let member = match &ty.kind {
                TypeKind::Qualified {
                    self_ty,
                    trait_ref,
                    assoc_name,
                } if trait_ref.segments.is_empty() && Self::ast_type_is_bare_self(self_ty) => {
                    &assoc_name.name
                }
                TypeKind::AssociatedType { base, assoc } if Self::ast_type_is_bare_self(base) => {
                    &assoc.name
                }
                _ => return false,
            };
            bounds.iter().any(|bound| {
                let TypeBoundKind::Equality(required) = &bound.kind else {
                    return false;
                };
                let Some(required) = self.semantic_formal_type(required, &scope, &declared_ids)
                else {
                    return false;
                };
                if let Some(facts) = instance.and_then(|instance| {
                    self.concrete_impl_bindings
                        .get(&(instance.clone(), protocol))
                }) {
                    return facts
                        .get(member.as_str())
                        .is_some_and(|actual| actual.contradicts(&required, self));
                }
                let mut implementations = descriptor
                    .protocols
                    .iter()
                    .filter(|p| p.protocol == protocol)
                    .peekable();
                implementations.peek().is_some()
                    && implementations.all(|p| {
                        let mut bindings = p
                            .associated_types
                            .iter()
                            .filter(|(name, _)| {
                                self.ctx.strings.get(name.0 as usize).map(String::as_str)
                                    == Some(member.as_str())
                            })
                            .peekable();
                        bindings.peek().is_some()
                            && bindings.all(|(_, ty)| self.declared_types_disjoint(ty, &required))
                    })
            })
        })
    }

    /// Prove impossibility from declared constructors. Open parameters and
    /// projections remain unknown; a nominal alias first expands through its
    /// own declaration. This is a negative applicability proof, not a choice
    /// of witnesses for an otherwise unproved conditional method.
    pub(super) fn declared_types_disjoint(&self, left: &TypeRef, right: &TypeRef) -> bool {
        use crate::types::{MAX_TYPE_REF_DEPTH, MAX_TYPE_REF_NODES};
        let mut pending = List::new();
        pending.push((left.clone(), right.clone(), 0usize));
        let mut remaining = MAX_TYPE_REF_NODES;
        while let Some((left, right, depth)) = pending.pop() {
            let Some(next) = remaining.checked_sub(1) else { return false; };
            remaining = next;
            if depth >= MAX_TYPE_REF_DEPTH { return false; }
            let (Some(left), Some(right)) = (self.expand_declared_alias(left), self.expand_declared_alias(right))
                else { return false; };
            if left == right { continue; }
            let mut components = |a: &[TypeRef], b: &[TypeRef]| {
                pending.extend(a.iter().cloned().zip(b.iter().cloned()).map(|(a, b)| (a, b, depth + 1)));
            };
            match (&left, &right) {
                (TypeRef::Generic(_) | TypeRef::AssociatedProjection { .. } | TypeRef::RecordFieldProjection { .. }, _)
                | (_, TypeRef::Generic(_) | TypeRef::AssociatedProjection { .. } | TypeRef::RecordFieldProjection { .. }) => {},
                // ABI placeholders cannot certify a source type constructor.
                (TypeRef::Concrete(id), _) | (_, TypeRef::Concrete(id))
                    if matches!(*id, TypeId::PTR | TypeId::RESERVED) => {},
                (TypeRef::Concrete(a), TypeRef::Concrete(b))
                | (TypeRef::Concrete(a), TypeRef::Instantiated { base: b, .. })
                | (TypeRef::Instantiated { base: a, .. }, TypeRef::Concrete(b)) => {
                    if a != b { return true; }
                }
                (TypeRef::Instantiated { base: a, args: aa }, TypeRef::Instantiated { base: b, args: ba }) => {
                    if a != b || aa.len() != ba.len() { return true; }
                    components(aa, ba);
                }
                (TypeRef::Reference { inner: a, mutability: am, tier: at },
                 TypeRef::Reference { inner: b, mutability: bm, tier: bt }) => {
                    if am != bm || at != bt { return true; }
                    pending.push((a.as_ref().clone(), b.as_ref().clone(), depth + 1));
                }
                (TypeRef::Tuple(a), TypeRef::Tuple(b)) => {
                    if a.len() != b.len() { return true; }
                    components(a, b);
                }
                (TypeRef::Array { element: a, length: al }, TypeRef::Array { element: b, length: bl }) => {
                    if al != bl { return true; }
                    pending.push((a.as_ref().clone(), b.as_ref().clone(), depth + 1));
                }
                (TypeRef::Slice(a), TypeRef::Slice(b)) => pending.push((a.as_ref().clone(), b.as_ref().clone(), depth + 1)),
                (TypeRef::Function { is_async: aa, params: ap, return_type: ar, contexts: ac },
                 TypeRef::Function { is_async: ba, params: bp, return_type: br, contexts: bc }) => {
                    if aa != ba || ac != bc || ap.len() != bp.len() { return true; }
                    components(ap, bp);
                    pending.push((ar.as_ref().clone(), br.as_ref().clone(), depth + 1));
                }
                (TypeRef::Dynamic(_) | TypeRef::Rank2Function { .. }, _)
                | (_, TypeRef::Dynamic(_) | TypeRef::Rank2Function { .. }) => {},
                // The remaining rigid constructors differ (for example an
                // owned record and a reference), independently of their fields.
                _ => return true,
            }
        }
        false
    }

    /// Select one projection after its children have been normalized. The
    /// descriptor owns both the parameter roster and all candidate equations;
    /// incompatible same-named bindings cannot be selected by iteration order.
    pub(super) fn declared_associated_projection_target(&self, ty: &TypeRef) -> Option<TypeRef> {
        let TypeRef::AssociatedProjection { base, qualifier: None, assoc, args } = ty else { return None; };
        if !args.is_empty() { return None; }
        let (id, arguments) = match base.as_ref() {
            TypeRef::Concrete(id) => (*id, &[][..]),
            TypeRef::Instantiated { base, args } => (*base, args.as_slice()),
            _ => return None,
        };
        let descriptor = self.type_by_id(id)?;
        if descriptor.type_params.len() != arguments.len() { return None; }
        let substitution = TypeSubstitution::new(&descriptor.type_params, arguments);
        let mut binding = None;
        for implementation in &descriptor.protocols {
            for (name, target) in &implementation.associated_types {
                if self.ctx.strings.get(name.0 as usize).map(String::as_str) != Some(assoc.as_str()) { continue; }
                let target = substitution.apply(target).ok()?;
                if binding.as_ref().is_some_and(|previous| previous != &target) { return None; }
                binding = Some(target);
            }
        }
        binding
    }

    /// Instantiate declaration-owned associated bindings. A projection does
    /// not carry a protocol ID, so incompatible same-named bindings remain
    /// unknown; neither protocol iteration order nor a method spelling wins.
    pub(super) fn resolve_declared_associated_types(&self, ty: &TypeRef) -> Option<TypeRef> {
        fn resolve(
            codegen: &VbcCodegen,
            ty: &TypeRef,
            active: &mut Set<TypeRef>,
            depth: usize,
        ) -> Option<TypeRef> {
            // The depth bound also stops expanding cycles such as
            // Grow<T>.Argument -> Grow<Grow<T>>.Argument.
            if depth >= 64 {
                return None;
            }
            let nested = |ty, active: &mut Set<TypeRef>| resolve(codegen, ty, active, depth + 1);
            match ty {
                TypeRef::RecordFieldProjection { .. } => None,
                TypeRef::AssociatedProjection { base, qualifier, assoc, args } => {
                    if qualifier.is_some() || !args.is_empty() { return None; }
                    if !active.insert(ty.clone()) {
                        return None;
                    }
                    let result = (|| {
                        let base = nested(base, active)?;
                        let projection = TypeRef::AssociatedProjection {
                            base: Box::new(base), qualifier: None, assoc: assoc.clone(), args: args.clone(),
                        };
                        let binding = codegen.declared_associated_projection_target(&projection)?;
                        resolve(codegen, &binding, active, depth + 1)
                    })();
                    active.remove(ty);
                    result
                }
                TypeRef::Instantiated { base, args } => Some(TypeRef::Instantiated {
                    base: *base,
                    args: args
                        .iter()
                        .map(|arg| nested(arg, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                }),
                TypeRef::Function {
                    is_async,
                    params,
                    return_type,
                    contexts,
                } => Some(TypeRef::Function {
                    is_async: *is_async,
                    params: params
                        .iter()
                        .map(|param| nested(param, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                    return_type: Box::new(nested(return_type, active)?),
                    contexts: contexts.clone(),
                }),
                TypeRef::Reference {
                    inner,
                    mutability,
                    tier,
                } => Some(TypeRef::Reference {
                    inner: Box::new(nested(inner, active)?),
                    mutability: *mutability,
                    tier: *tier,
                }),
                TypeRef::Tuple(items) => Some(TypeRef::Tuple(
                    items
                        .iter()
                        .map(|item| nested(item, active))
                        .collect::<Option<List<_>>>()?
                        .into(),
                )),
                TypeRef::Array { element, length } => Some(TypeRef::Array {
                    element: Box::new(nested(element, active)?),
                    length: *length,
                }),
                TypeRef::Slice(inner) => Some(TypeRef::Slice(Box::new(nested(inner, active)?))),
                TypeRef::Dynamic(object) => object.try_map(|ty| resolve(codegen, ty, active, depth + 1).ok_or(()))
                    .ok().map(TypeRef::Dynamic),
                // Rank-2 signatures are not certified as ordinary callables.
                TypeRef::Rank2Function { .. } => None,
                TypeRef::Concrete(_) | TypeRef::Generic(_) | TypeRef::ConstValue(_) | TypeRef::ConstSequence(_) => {
                    Some(ty.clone())
                }
            }
        }
        resolve(self, ty, &mut Set::new(), 0)
    }
}
