//! Declaration-owned implementation entailment. A trigger schedules a query;
//! it is never evidence for the other premises of an implementation.
use super::source_substitution::SourceTypeSubstitution;
use super::{CodegenError, CodegenErrorKind, CodegenResult, ImplReceiverWitness, VbcCodegen};
use crate::mono::{TypeSubstitution, match_declared_types};
use crate::types::{TypeId, TypeParamId, TypeRef};
use std::sync::Arc;
use verum_ast::{
    Span, Type,
    decl::{ImplDecl, ImplItemKind, ImplKind},
    source_implementation::AssociatedImplementationSource,
    ty::{GenericArg, GenericParamKind, TypeBound, TypeBoundKind, TypeKind, WherePredicateKind},
};
use verum_common::{List, Map, Maybe, Text};

#[derive(Clone)]
pub(super) struct ImplementationHeader {
    source: AssociatedImplementationSource,
    declaration: Arc<ImplDecl>,
}

/// Preparation shared only while one implementation's default methods are
/// selected. That pass queues bodies without changing declaration authority.
/// Caller premises, proof graphs and imported evidence remain query-local.
#[derive(Default)]
pub(super) struct ImplementationPreparation {
    headers: Maybe<List<PreparedHeader>>,
}

struct PreparedHeader {
    header: ImplementationHeader,
    protocol_heads: ProtocolHeadFilter,
    implementation: Maybe<PreparedImplementation>,
}

impl PreparedHeader {
    fn new(header: ImplementationHeader) -> Self {
        Self { header, protocol_heads: ProtocolHeadFilter::Unprepared, implementation: None }
    }
}

#[derive(Clone)]
enum ProtocolHeadFilter {
    Unprepared,
    // Incomplete source identity cannot justify excluding this header.
    Unknown,
    Declared(List<TypeId>),
}

/// Declaration-only inventory for one prepared source graph. This does not
/// retain a receiver, a caller premise, an imported witness or a proof result.
#[derive(Default)]
pub(super) struct ProtocolHeadPreparation {
    heads: Map<AssociatedImplementationSource, ProtocolHeadFilter>,
}

struct PreparedImplementation {
    scope: std::collections::HashMap<String, u16>,
    source_bindings: SourceTypeSubstitution,
    receiver: ImplReceiverWitness,
    // Inheritance is elaborated only after the exact receiver can match.
    heads: Maybe<List<Goal>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Goal {
    subject: TypeRef,
    protocol: TypeId,
    arguments: List<TypeRef>,
    associated: List<(Text, TypeRef)>,
}

impl Goal {
    fn validate_size(&self, span: Span) -> CodegenResult<()> {
        // Validate before hashing, cloning or declaration matching. A small
        // source rule can grow a type at every proof step without growing ASTs.
        let mut pending: List<_> = std::iter::once(&self.subject)
            .chain(&self.arguments)
            .chain(self.associated.iter().map(|(_, ty)| ty))
            .map(|ty| (ty, 0))
            .collect();
        let mut nodes = 0;
        while let Some((ty, depth)) = pending.pop() {
            nodes += 1;
            if depth >= crate::types::MAX_TYPE_REF_DEPTH || nodes > crate::types::MAX_TYPE_REF_NODES {
                return Err(invalid(
                    "implementation obligation type budget exceeded",
                    span,
                ));
            }
            ty.for_each_child(|child| pending.push((child, depth + 1)));
        }
        Ok(())
    }

    fn entails(&self, required: &Self) -> bool {
        self.subject == required.subject
            && self.protocol == required.protocol
            && self.arguments == required.arguments
            && required
                .associated
                .iter()
                .all(|binding| self.associated.contains(binding))
    }
}

#[derive(Default)]
struct Premises {
    goals: List<Goal>,
    // Each pair is a semantic subject and its declared structural callable bound.
    callables: List<(TypeRef, TypeRef)>,
    impossible: bool,
    unknown: Maybe<Text>,
}

impl Premises {
    fn protocol_bounds(&self) -> Map<TypeRef, List<super::protocol_results::BoundProtocol>> {
        let mut bounds: Map<_, List<_>> = Map::new();
        for goal in &self.goals {
            let application = if goal.arguments.is_empty() { TypeRef::Concrete(goal.protocol) }
                else { TypeRef::instantiated(goal.protocol, goal.arguments.iter().cloned()) };
            let associated = goal.associated.iter().map(|(name, ty)| (name.to_string(), ty.clone())).collect();
            bounds.entry(goal.subject.clone()).or_default().push(super::protocol_results::BoundProtocol { application, associated });
        }
        bounds
    }
}

pub(super) struct ProvenImplementation {
    pub bindings: TypeSubstitution,
    pub bounds: Map<TypeRef, List<super::protocol_results::BoundProtocol>>,
}

impl VbcCodegen {
    /// Collected across the whole source unit before any method is selected.
    pub(super) fn record_implementation_header(
        &mut self,
        declaration: &ImplDecl,
        owner: &str,
    ) -> CodegenResult<()> {
        if !matches!(declaration.kind, ImplKind::Protocol { .. }) {
            return Ok(());
        }
        let selected = self.selected_impl_items(declaration)?;
        let source =
            AssociatedImplementationSource::new_with_items(owner.into(), declaration, &selected)
                .map_err(|message| invalid(message, declaration.span))?;
        if !self
            .implementation_headers
            .iter()
            .any(|header| header.source == source)
        {
            let declaration = source
                .declaration()
                .map_err(|message| invalid(message, declaration.span))?;
            self.implementation_headers.push(ImplementationHeader {
                source,
                declaration: Arc::new(declaration),
            });
        }
        Ok(())
    }

    fn obligation_goal(
        &mut self,
        subject: &ImplReceiverWitness,
        path: &verum_ast::Path,
        args: &[GenericArg],
        scope: &std::collections::HashMap<String, u16>,
        parameters: &[verum_ast::ty::GenericParam],
    ) -> CodegenResult<Goal> {
        let ids: List<_> = scope.values().copied().map(TypeParamId).collect();
        // The subject is the checked source type of this bound, which may
        // differ from the lexical implementation's Self in explicit arguments.
        if !subject.semantic {
            return Err(invalid("protocol obligation lacks its semantic subject", path.span));
        }
        let application = self.resolve_protocol_application(path, args, scope, &ids, parameters, subject)?;
        self.application_goal(subject.ty.clone(), &application, args, scope, path.span)
    }

    fn application_goal(
        &self,
        subject: TypeRef,
        application: &super::protocol_applications::ProtocolApplication,
        args: &[GenericArg],
        scope: &std::collections::HashMap<String, u16>,
        span: Span,
    ) -> CodegenResult<Goal> {
        let protocol = application
            .protocol
            .ok_or_else(|| invalid("unresolved protocol obligation", span))?;
        let mut arguments = List::new();
        if let Some(info) = self.protocol_registry.get(&application.key) {
            for parameter in info.generics.iter() {
                let name = match &parameter.kind {
                    GenericParamKind::Lifetime { .. } | GenericParamKind::Level { .. } => continue,
                    GenericParamKind::Type { name, .. }
                    | GenericParamKind::Const { name, .. }
                    | GenericParamKind::Meta { name, .. }
                    | GenericParamKind::HigherKinded { name, .. }
                    | GenericParamKind::KindAnnotated { name, .. }
                    | GenericParamKind::Context { name } => name,
                };
                let argument = application
                    .substitution
                    .get(name.name.as_str())
                    .ok_or_else(|| {
                        invalid(
                            "protocol obligation lacks its argument witness",
                            parameter.span,
                        )
                    })?;
                if !argument.semantic {
                    return Err(invalid(
                        "protocol obligation has no semantic argument",
                        parameter.span,
                    ));
                }
                arguments.push(argument.ty.clone());
            }
        }
        let mut associated = List::new();
        for arg in args {
            if let GenericArg::Binding(binding) = arg {
                associated.push((
                    binding.name.name.clone(),
                    self.resolve_signature_type_ref(&binding.ty, scope)?,
                ));
            }
        }
        associated.sort_by(|left, right| left.0.cmp(&right.0));
        let goal = Goal {
            subject,
            protocol,
            arguments,
            associated,
        };
        goal.validate_size(span)?;
        Ok(goal)
    }

    fn implementation_heads(
        &mut self,
        declaration: &ImplDecl,
        receiver: &ImplReceiverWitness,
        scope: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<List<Goal>> {
        let ImplKind::Protocol {
            protocol,
            protocol_args,
            ..
        } = &declaration.kind
        else {
            return Ok(List::new());
        };
        let ids: List<_> = scope.values().copied().map(TypeParamId).collect();
        let application = self.with_protocol_source_scope(
            self.ctx.current_source_module.clone(),
            self.current_source_substitution.clone(),
            Some(receiver.clone()),
            |cg| cg.resolve_protocol_application(
                protocol, protocol_args, scope, &ids, &declaration.generics, receiver,
            ),
        )?;
        let mut pending = List::from_iter([application]);
        let mut heads = List::new();
        while let Some(application) = pending.pop() {
            // Associated bindings are proved from the implementation, below.
            let head = self.application_goal(
                receiver.ty.clone(),
                &application,
                &[],
                scope,
                declaration.span,
            )?;
            if heads.contains(&head) {
                continue;
            }
            if heads.len() >= 256 {
                return Err(invalid(
                    "protocol inheritance obligation budget exceeded",
                    declaration.span,
                ));
            }
            heads.push(head);
            if let Some(info) = self.protocol_registry.get(&application.key).cloned() {
                self.with_protocol_source_scope(
                    Some(info.source_module.clone()),
                    application.substitution,
                    Some(receiver.clone()),
                    |cg| {
                        for parent in info.super_protocols.iter() {
                            let (path, args) = Self::protocol_application_parts(parent)?;
                            pending.push(cg.resolve_protocol_application(
                                path,
                                args,
                                &Default::default(),
                                &ids,
                                &info.generics,
                                receiver,
                            )?);
                        }
                        Ok(())
                    },
                )?;
            }
        }
        Ok(heads)
    }

    fn obligation_bounds(
        &mut self,
        subject: &ImplReceiverWitness,
        bounds: &[TypeBound],
        declaration: &ImplDecl,
        scope: &std::collections::HashMap<String, u16>,
        premises: &mut Premises,
    ) -> CodegenResult<()> {
        for bound in bounds {
            match &bound.kind {
                TypeBoundKind::Protocol(path) => premises.goals.push(self.obligation_goal(
                    subject,
                    path,
                    &[],
                    scope,
                    &declaration.generics,
                )?),
                TypeBoundKind::Callable(ty)
                    if matches!(ty.kind, TypeKind::Function { .. } | TypeKind::Rank2Function { .. }) =>
                {
                    // A callable bound is a structural contract, not a nominal protocol.
                    let ids: List<_> = scope.values().copied().map(TypeParamId).collect();
                    let signature = self.resolve_source_type_witness(ty, scope, &ids)?;
                    if subject.semantic && signature.semantic {
                        premises.callables.push((subject.ty.clone(), signature.ty));
                    } else {
                        premises.unknown = Some("callable bound lacks its semantic witness".into());
                    }
                }
                TypeBoundKind::GenericProtocol(ty) => {
                    let (path, args) = Self::protocol_application_parts(ty)?;
                    premises.goals.push(self.obligation_goal(
                        subject,
                        path,
                        args,
                        scope,
                        &declaration.generics,
                    )?);
                }
                TypeBoundKind::Equality(ty) => {
                    let expected = self.resolve_signature_type_ref(ty, scope)?;
                    if !subject.semantic || subject.ty != expected {
                        premises.impossible = true;
                    }
                }
                _ => {
                    premises.unknown =
                        Some("implementation bound requires additional declaration evidence".into())
                }
            }
        }
        Ok(())
    }

    fn implementation_premises(
        &mut self,
        declaration: &ImplDecl,
        scope: &std::collections::HashMap<String, u16>,
    ) -> CodegenResult<Premises> {
        let mut premises = Premises::default();
        let ids: List<_> = scope.values().copied().map(TypeParamId).collect();
        for parameter in &declaration.generics {
            match &parameter.kind {
                GenericParamKind::Type { name, bounds, .. } => {
                    let subject = self.resolve_source_type_witness(
                        &Type::new(
                            TypeKind::Path(verum_ast::Path::from_ident(name.clone())),
                            name.span,
                        ),
                        scope,
                        &ids,
                    )?;
                    self.obligation_bounds(&subject, bounds, declaration, scope, &mut premises)?;
                }
                GenericParamKind::Lifetime { .. } => {}
                _ => {
                    premises.unknown = Some(
                        "implementation parameter requires additional declaration evidence".into(),
                    )
                }
            }
        }
        for clause in declaration
            .generic_where_clause
            .iter()
            .chain(declaration.meta_where_clause.iter())
        {
            for predicate in &clause.predicates {
                match &predicate.kind {
                    WherePredicateKind::Type { ty, bounds } => {
                        let subject = self.resolve_source_type_witness(ty, scope, &ids)?;
                        self.obligation_bounds(&subject, bounds, declaration, scope, &mut premises)?;
                    }
                    _ => {
                        premises.unknown =
                            Some("implementation refinement has not been discharged".into())
                    }
                }
            }
        }
        if declaration.specialize_attr.is_some() {
            premises.unknown =
                Some("implementation specialization requires its selection evidence".into());
        }
        Ok(premises)
    }

    /// A head mismatch is independent of receiver and argument substitution.
    /// Follow only canonical protocol declarations in their own source scopes.
    /// This does not prove an application: every potentially matching header
    /// still enters the original receiver, argument and premise matcher.
    fn implementation_protocol_heads(
        &mut self,
        header: &ImplementationHeader,
    ) -> CodegenResult<ProtocolHeadFilter> {
        if let Some(heads) = self.implementation_head_preparation.as_ref()
            .and_then(|prepared| prepared.heads.get(&header.source))
        {
            return Ok(heads.clone());
        }
        let heads = self.resolve_implementation_protocol_heads(header)?;
        if let Some(prepared) = self.implementation_head_preparation.as_mut() {
            prepared.heads.insert(header.source.clone(), heads.clone());
        }
        Ok(heads)
    }

    fn resolve_implementation_protocol_heads(
        &mut self,
        header: &ImplementationHeader,
    ) -> CodegenResult<ProtocolHeadFilter> {
        let ImplKind::Protocol { protocol, .. } = &header.declaration.kind else {
            return Ok(ProtocolHeadFilter::Unknown);
        };
        let first = self.with_protocol_source_scope(
            Some(header.source.origin_module().into()), Default::default(), None,
            |cg| Ok(cg.protocol_declaration_identity(protocol)),
        )?;
        let mut pending = List::from_iter([first]);
        let mut heads = List::new();
        while let Some((id, key)) = pending.pop() {
            let Some(id) = id else { return Ok(ProtocolHeadFilter::Unknown) };
            if heads.contains(&id) { continue; }
            // The normal application traversal owns diagnostics. A head-only
            // budget or an incomplete declaration merely defers to that path.
            if heads.len() >= 256 { return Ok(ProtocolHeadFilter::Unknown); }
            if self.type_by_id(id).is_none_or(|descriptor|
                descriptor.kind != crate::types::TypeKind::Protocol)
            {
                return Ok(ProtocolHeadFilter::Unknown);
            }
            let Some(info) = self.protocol_registry.get(key.as_str()).cloned().filter(|info|
                key.as_str() == crate::module::qualify_module_name(&info.source_module, &info.name))
            else { return Ok(ProtocolHeadFilter::Unknown) };
            heads.push(id);
            let complete = self.with_protocol_source_scope(
                Some(info.source_module.clone()), Default::default(), None,
                |cg| {
                    for parent in info.super_protocols.iter() {
                        let Ok((path, _)) = Self::protocol_application_parts(parent) else {
                            return Ok(false);
                        };
                        if pending.len() >= 256 { return Ok(false); }
                        pending.push(cg.protocol_declaration_identity(path));
                    }
                    Ok(true)
                },
            )?;
            if !complete { return Ok(ProtocolHeadFilter::Unknown); }
        }
        Ok(ProtocolHeadFilter::Declared(heads))
    }

    /// Match a header in its own binder space, then resolve every premise in
    /// the actual receiver's space. Equal numeric binder IDs are not equality.
    fn implementation_alternative(
        &mut self,
        header: &ImplementationHeader,
        prepared: &mut Maybe<PreparedImplementation>,
        goal: &Goal,
        assumptions: &Premises,
    ) -> CodegenResult<Maybe<(Premises, TypeSubstitution)>> {
        self.with_protocol_source_scope(
            Some(header.source.origin_module().into()),
            Default::default(),
            None,
            |cg| {
                let declaration = &header.declaration;
                if !matches!(declaration.kind, ImplKind::Protocol { .. }) {
                    return Ok(None);
                }
                if prepared.is_none() {
                    let (_, _, names) = cg.resolve_impl_generics(declaration);
                    let scope = cg.declared_impl_generic_scope(&declaration.kind, &names);
                    let source_bindings = cg.implementation_value_scope(declaration, &scope)?;
                    cg.current_source_substitution = source_bindings.clone();
                    let receiver = cg.resolve_impl_receiver(declaration, &scope, &names)?;
                    *prepared = Some(PreparedImplementation { scope, source_bindings, receiver, heads: None });
                }
                let PreparedImplementation { scope, source_bindings, receiver, heads } = prepared.as_mut().expect("header was prepared above");
                let mut bindings = TypeSubstitution::empty();
                if !receiver.semantic
                    || !match_declared_types(&receiver.ty, &goal.subject, &mut bindings)
                {
                    return Ok(None);
                }
                cg.current_source_substitution = source_bindings.clone();
                let heads = match heads {
                    Some(heads) => heads,
                    slot @ None => slot.insert(cg.implementation_heads(declaration, receiver, scope)?),
                };
                let mut matched = None;
                for head in heads.iter() {
                    let mut candidate = bindings.clone();
                    if head.protocol == goal.protocol
                        && head.arguments.len() == goal.arguments.len()
                        && head
                            .arguments
                            .iter()
                            .zip(&goal.arguments)
                            .all(|(formal, actual)| {
                                match_declared_types(formal, actual, &mut candidate)
                            })
                    {
                        matched = Some(candidate);
                        break;
                    }
                }
                let Some(mut bindings) = matched else {
                    return Ok(None);
                };
                // The receiver and protocol arguments can leave parameters that
                // are functionally determined by a callable bound, e.g. the
                // result T of F: fn(Input) -> T. Resolve the declaration's
                // symbolic requirements before crossing into the caller space.
                let mut concrete_callables = List::new();
                if scope.values().any(|id| bindings.get(TypeParamId(*id)).is_none()) {
                    let mut symbolic = cg.implementation_premises(declaration, scope)?;
                    let bounds = symbolic.protocol_bounds();
                    for (subject, signature) in &mut symbolic.callables {
                        if determined_subject(subject, &bindings).is_some_and(crate::mono::is_concrete_type) {
                            *signature = cg.qualify_bound_projections(signature, &bounds)
                                .ok_or_else(|| invalid("callable input has no declaring protocol", declaration.span))?;
                        }
                    }
                    infer_implementation_bindings(cg, &symbolic, assumptions, &mut bindings, declaration.span)?;
                    for (index, (subject, signature)) in symbolic.callables.iter().enumerate() {
                        if let Some(actual) = determined_subject(subject, &bindings).filter(|ty| crate::mono::is_concrete_type(ty))
                            && super::constructor_types::fully_bound(signature, &bindings) {
                            let signature = bindings.apply(signature).map_err(|err| invalid(err.to_string(), declaration.span))?;
                            if let Some(resolved) = cg.body_declaration_type(&signature) {
                                concrete_callables.push((index, actual.clone(), resolved));
                            }
                        }
                    }
                }
                let mut substitution = SourceTypeSubstitution::default();
                for (name, id) in scope.iter() {
                    let Some(actual) = bindings.get(TypeParamId(*id)) else {
                        return Ok(Some((Premises {
                            unknown: Some(format!(
                                "implementation has an undetermined parameter '{name}' in '{}' for '{}'",
                                header.source.origin_module(),
                                cg.extract_impl_type_name(&declaration.kind)
                                    .unwrap_or_else(|| format!("{:?}", receiver.ty)),
                            ).into()),
                            ..Default::default()
                        }, bindings)));
                    };
                    let witness = ImplReceiverWitness { ty: actual.clone(), semantic: true };
                    if let Some((_, domain)) = source_bindings.get_value(name) {
                        let domain = bindings.apply(domain)
                            .map_err(|error| invalid(error.to_string(), declaration.span))?;
                        substitution.insert_value(name.clone(), witness, domain);
                    } else {
                        substitution.insert(name.clone(), witness);
                    }
                }
                cg.with_protocol_source_scope(
                    Some(header.source.origin_module().into()),
                    substitution,
                    Some(ImplReceiverWitness {
                        ty: goal.subject.clone(),
                        semantic: true,
                    }),
                    |cg| {
                        let mut premises =
                            cg.implementation_premises(declaration, &Default::default())?;
                        for (index, actual, signature) in concrete_callables {
                            let Some((subject, required)) = premises.callables.get_mut(index) else { return Ok(None) };
                            if *subject != actual { return Ok(None); }
                            *required = signature;
                        }
                        for (name, required) in &goal.associated {
                            let value =
                                declaration.items.iter().find_map(|item| match &item.kind {
                                    ImplItemKind::Type {
                                        name: member, ty, ..
                                    } if member.name == *name => Some(ty),
                                    _ => None,
                                });
                            match value {
                                Some(value) => {
                                    if cg.resolve_signature_type_ref(value, &Default::default())?
                                        != *required
                                    {
                                        premises.impossible = true;
                                    }
                                }
                                None => {
                                    premises.unknown = Some(
                                        "associated equality lacks its implementation binding"
                                            .into(),
                                    )
                                }
                            }
                        }
                        Ok(Some((premises, bindings)))
                    },
                )
            },
        )
    }

    /// Prove this assignment's own source row. Return its symbolic nominal
    /// premises so projections in the equation keep their declaring protocols.
    pub(super) fn proven_member_bounds(
        &mut self, source: &AssociatedImplementationSource, subject: &TypeRef, protocol: TypeId,
        arguments: &[TypeRef],
    ) -> CodegenResult<Maybe<ProvenImplementation>> {
        let declaration = source.declaration().map_err(|reason| invalid(reason, Span::default()))?;
        let span = declaration.span;
        let header = ImplementationHeader { source: source.clone(), declaration: Arc::new(declaration) };
        let goal = Goal { subject: subject.clone(), protocol, arguments: arguments.iter().cloned().collect(), associated: List::new() };
        goal.validate_size(span)?;
        let mut assumptions = Premises::default();
        for (subject, bounds) in &self.current_protocol_bounds {
            let Some(hierarchy) = self.protocol_hierarchy(bounds.clone(), Some(subject)) else { continue; };
            for bound in hierarchy {
                let (protocol, arguments) = match bound.application {
                    TypeRef::Concrete(id) => (id, List::new()),
                    TypeRef::Instantiated { base, args } => (base, args.into()),
                    _ => continue,
                };
                let associated = bound.associated.into_iter().map(|(name, ty)| (name.into(), ty)).collect();
                assumptions.goals.push(Goal { subject: subject.clone(), protocol, arguments, associated });
            }
        }
        assumptions.callables.extend(self.ctx.registers.callable_bound_signatures.iter()
            .map(|(id, signature)| (TypeRef::Generic(*id), signature.clone())));
        let mut prepared = None;
        let Some((required, bindings)) = self.implementation_alternative(&header, &mut prepared, &goal, &assumptions)? else {
            return Ok(None);
        };
        if required.impossible || required.unknown.is_some()
            || !required.callables.iter().all(|bound| callable_is_proven(bound, &assumptions.callables))
            || !self.prove_implementation_goals(required.goals, &assumptions, span, &mut ImplementationPreparation::default())?
        { return Ok(None); }

        let Some(prepared) = prepared else { return Ok(None); };
        let premises = self.with_protocol_source_scope(
            Some(source.origin_module().into()), prepared.source_bindings, Some(prepared.receiver),
            |cg| cg.implementation_premises(&header.declaration, &prepared.scope),
        )?;
        let bounds = premises.protocol_bounds();
        Ok(Some(ProvenImplementation { bindings, bounds }))
    }

    pub(super) fn blanket_requirements_hold(
        &mut self,
        blanket: &super::BlanketImpl,
        receiver: &ImplReceiverWitness,
        implementation: &ImplDecl,
        scope: &std::collections::HashMap<String, u16>,
        preparation: &mut ImplementationPreparation,
    ) -> CodegenResult<bool> {
        let source_bindings = self.implementation_value_scope(implementation, scope)?;
        let assumptions = self.with_protocol_source_scope(
            self.ctx.current_source_module.clone(), source_bindings, Some(receiver.clone()),
            |cg| cg.implementation_premises(implementation, scope),
        )?;
        let requirements = self.with_protocol_source_scope(
            blanket.source_module.clone(),
            SourceTypeSubstitution::receiver(blanket.receiver_parameter.clone(), receiver.clone()),
            Some(receiver.clone()),
            |cg| cg.implementation_premises(&blanket.declaration, &Default::default()),
        )?;
        if requirements.impossible {
            return Ok(false);
        }
        if let Some(reason) = requirements.unknown {
            return Err(invalid(reason, blanket.declaration.span));
        }
        if !requirements.callables.iter().all(|bound| callable_is_proven(bound, &assumptions.callables)) {
            return Err(invalid("unproved callable bound", blanket.declaration.span));
        }
        self.prove_implementation_goals(
            requirements.goals, &assumptions, blanket.declaration.span, preparation,
        )
    }

    /// Build a finite AND/OR obligation graph and compute its least fixed point.
    /// Cycles cannot prove themselves. Exhaustion is a diagnostic, never success.
    fn prove_implementation_goals(
        &mut self,
        required: List<Goal>,
        assumptions: &Premises,
        span: Span,
        preparation: &mut ImplementationPreparation,
    ) -> CodegenResult<bool> {
        const MAX_GOALS: usize = 4096;
        const MAX_EDGES: usize = 32768;
        while self.obligation_blanket_cursor < self.blanket_impls.len() {
            let blanket = self.blanket_impls[self.obligation_blanket_cursor].clone();
            let owner = blanket
                .source_module
                .unwrap_or_else(|| self.config.module_name.clone());
            self.with_protocol_source_scope(Some(owner.clone()), Default::default(), None, |cg| {
                cg.record_implementation_header(&blanket.declaration, &owner)
            })?;
            self.obligation_blanket_cursor += 1;
        }
        // The caller owns this preparation for one method-selection pass.
        // Registration and alias changes happen outside that lifetime. Only
        // the existing declaration roster is shared between its queries.
        let headers = preparation.headers.get_or_insert_with(|| {
            self.implementation_headers.iter().cloned()
                .map(PreparedHeader::new).collect()
        });
        // Descriptor-provided evidence is visible only after this proof reaches
        // its subject. Retaining that roster would broaden later proof inputs.
        let mut imported_headers = List::new();
        let mut goals = required.clone();
        let mut indices: Map<Goal, usize> = goals
            .iter()
            .cloned()
            .enumerate()
            .map(|(id, goal)| (goal, id))
            .collect();
        let mut alternatives: List<List<List<usize>>> = List::new();
        let mut unknown = List::new();
        let mut edge_count = 0usize;
        let mut cursor = 0;
        while cursor < goals.len() {
            let goal = goals[cursor].clone();
            let mut choices = List::new();
            let mut reason = None;
            if assumptions.goals.iter().any(|fact| fact.entails(&goal)) {
                // This exact proposition is a lexical premise, including
                // its protocol arguments and requested associated equalities.
                // Its proof is complete without reopening other declarations.
                choices.push(List::new());
                alternatives.push(choices);
                unknown.push(None);
                cursor += 1;
                continue;
            }
            let imported: List<_> = goal
                .subject
                .base_type_id()
                .and_then(|id| self.type_by_id(id))
                .into_iter()
                .flat_map(|descriptor| &descriptor.protocols)
                .filter(|row| row.protocol.0 == goal.protocol.0)
                .map(|row| row.source_implementation.clone())
                .collect();
            for source in imported {
                let Some(source) = source else {
                    reason = Some(Text::from(
                        "imported implementation lacks its source contract",
                    ));
                    continue;
                };
                if !headers.iter().chain(&imported_headers)
                    .any(|prepared| prepared.header.source == source)
                {
                    let declaration =
                        Arc::new(source.declaration().map_err(|error| invalid(error, span))?);
                    imported_headers.push(PreparedHeader::new(ImplementationHeader { source, declaration }));
                }
            }
            for prepared in headers.iter_mut().chain(&mut imported_headers) {
                let PreparedHeader { header, protocol_heads, implementation } = prepared;
                if matches!(protocol_heads, ProtocolHeadFilter::Unprepared) {
                    *protocol_heads = self.implementation_protocol_heads(header)?;
                }
                if matches!(protocol_heads, ProtocolHeadFilter::Declared(heads) if !heads.contains(&goal.protocol)) {
                    continue;
                }
                let alternative = self.implementation_alternative(header, implementation, &goal, assumptions)
                    .map_err(|mut error| {
                        use crate::mono::SubstitutionError::{TypeDepthLimit, TypeNodeLimit};
                        if matches!(error.kind, CodegenErrorKind::SourceSubstitution(TypeDepthLimit | TypeNodeLimit)) {
                            let context = "implementation obligation type budget exceeded";
                            error.context = Some(match error.context.take() {
                                Some(source) => format!("{context} while checking {source}"),
                                None => context.to_owned(),
                            });
                            error.span.get_or_insert(span);
                        }
                        error
                    })?;
                let Some((premises, _)) = alternative else {
                    continue;
                };
                if premises.impossible {
                    continue;
                }
                if let Some(message) = premises.unknown {
                    reason = Some(message);
                    continue;
                }
                if !premises.callables.iter().all(|bound| callable_is_proven(bound, &assumptions.callables)) {
                    reason = Some("unproved callable bound".into());
                    continue;
                }
                let mut dependencies = List::new();
                for premise in premises.goals {
                    let id = if let Some(&id) = indices.get(&premise) {
                        id
                    } else {
                        if goals.len() >= MAX_GOALS {
                            return Err(invalid(
                                "implementation obligation goal budget exceeded",
                                span,
                            ));
                        }
                        let id = goals.len();
                        indices.insert(premise.clone(), id);
                        goals.push(premise);
                        id
                    };
                    dependencies.push(id);
                    edge_count += 1;
                    if edge_count > MAX_EDGES {
                        return Err(invalid(
                            "implementation obligation edge budget exceeded",
                            span,
                        ));
                    }
                }
                choices.push(dependencies);
            }
            alternatives.push(choices);
            unknown.push(reason);
            cursor += 1;
        }
        // Each edge is consumed once; long chains do not repeatedly rescan
        // the entire graph. No recursion is proportional to proof depth.
        let mut dependents: List<List<usize>> = (0..goals.len()).map(|_| List::new()).collect();
        let mut choices = List::new();
        let mut ready = List::new();
        for (goal, alternatives) in alternatives.iter().enumerate() {
            for premises in alternatives {
                let choice = choices.len();
                choices.push((goal, premises.len()));
                if premises.is_empty() {
                    ready.push(goal);
                }
                for dependency in premises {
                    dependents[*dependency].push(choice);
                }
            }
        }
        let mut proven = List::from_iter((0..goals.len()).map(|_| false));
        while let Some(goal) = ready.pop() {
            if proven[goal] {
                continue;
            }
            proven[goal] = true;
            for &choice in &dependents[goal] {
                choices[choice].1 -= 1;
                if choices[choice].1 == 0 {
                    ready.push(choices[choice].0);
                }
            }
        }
        if required.iter().all(|goal| proven[indices[goal]]) {
            return Ok(true);
        }
        if let Some(reason) = unknown.into_iter().flatten().next() {
            return Err(invalid(reason, span));
        }
        Ok(false)
    }
}

fn callable_is_proven(bound: &(TypeRef, TypeRef), assumptions: &[(TypeRef, TypeRef)]) -> bool {
    bound.0 == bound.1 || assumptions.contains(bound)
}

/// Match the declaration's original pattern in its own binder space. Actual
/// generics belong to the caller. Reconstructing the exact witness excludes
/// representation-only coercions from the semantic proof.
fn match_evidence(
    pattern: &TypeRef,
    actual: &TypeRef,
    candidate: &mut TypeSubstitution,
    span: Span,
) -> CodegenResult<bool> {
    if !match_declared_types(pattern, actual, candidate) {
        return Ok(false);
    }
    let instantiated = candidate.apply(pattern).map_err(|error| invalid(error.to_string(), span))?;
    Ok(instantiated == *actual)
}

fn determined_subject<'a>(subject: &'a TypeRef, bindings: &'a TypeSubstitution) -> Option<&'a TypeRef> {
    match subject {
        TypeRef::Generic(id) => bindings.get(*id),
        _ if !subject.is_generic() => Some(subject),
        _ => None,
    }
}

fn retain_evidence(
    selected: &mut Maybe<TypeSubstitution>,
    candidate: TypeSubstitution,
    span: Span,
) -> CodegenResult<()> {
    if selected.as_ref().is_some_and(|previous| previous != &candidate) {
        return Err(invalid("conflicting implementation parameter witnesses", span));
    }
    *selected = Some(candidate);
    Ok(())
}

/// Functions determine their signatures. A protocol application determines
/// its associated outputs once its subject and every argument are known. Each
/// pass either binds a new implementation parameter or reaches a fixed point.
fn infer_implementation_bindings(
    cg: &VbcCodegen,
    requirements: &Premises,
    assumptions: &Premises,
    bindings: &mut TypeSubstitution,
    span: Span,
) -> CodegenResult<()> {
    loop {
        let before = bindings.len();
        for (subject, signature) in &requirements.callables {
            let Some(subject) = determined_subject(subject, bindings) else { continue };
            let actuals = std::iter::once(subject).filter(|ty| matches!(ty, TypeRef::Function { .. }))
                .chain(assumptions.callables.iter().filter(|(ty, _)| ty == subject).map(|(_, actual)| actual));
            let mut selected = None;
            for actual in actuals {
                let mut candidate = bindings.clone();
                let matched = if crate::mono::is_concrete_type(actual) {
                    cg.bind_callable_operand(signature, actual, &mut candidate)
                } else { match_evidence(signature, actual, &mut candidate, span)? };
                if matched {
                    retain_evidence(&mut selected, candidate, span)?;
                }
            }
            if let Some(candidate) = selected { *bindings = candidate; }
        }
        for goal in &requirements.goals {
            if goal.associated.is_empty() { continue; }
            let Some(subject) = determined_subject(&goal.subject, bindings) else { continue };
            let mut selected = None;
            // A nominal subject can supply the same evidence as a caller
            // premise. Only a fully selected, unconditional source equation
            // may introduce a binder here. The instantiated premises are
            // still discharged by the obligation graph.
            if goal.arguments.iter().all(|ty| super::constructor_types::fully_bound(ty, bindings)) {
                let args: List<_> = goal.arguments.iter().map(|ty| bindings.apply(ty))
                    .collect::<Result<_, _>>().map_err(|error| invalid(error.to_string(), span))?;
                let mut candidate = bindings.clone();
                let mut complete = true;
                for (member, pattern) in &goal.associated {
                    let Some(actual) = cg.selected_protocol_implementation_member(subject, goal.protocol, &args, member) else {
                        complete = false;
                        break;
                    };
                    if !match_evidence(pattern, &actual, &mut candidate, span)? { complete = false; break; }
                }
                if complete { selected = Some(candidate); }
            }
            for fact in &assumptions.goals {
                if fact.protocol != goal.protocol || fact.subject != *subject
                    || fact.arguments.len() != goal.arguments.len() { continue; }
                let mut candidate = bindings.clone();
                // Protocol arguments select an application. An implementation
                // may provide P<Int> and P<Bool> for the very same subject.
                let mut same_application = true;
                for (pattern, actual) in goal.arguments.iter().zip(&fact.arguments) {
                    if !match_evidence(pattern, actual, &mut candidate, span)? || candidate != *bindings {
                        same_application = false;
                        break;
                    }
                }
                if !same_application { continue; }
                let mut matched = false;
                for (member, pattern) in &goal.associated {
                    if let Some((_, actual)) = fact.associated.iter().find(|(name, _)| name == member) {
                        if !match_evidence(pattern, actual, &mut candidate, span)? {
                            matched = false;
                            break;
                        }
                        matched = true;
                    }
                }
                if matched { retain_evidence(&mut selected, candidate, span)?; }
            }
            if let Some(candidate) = selected { *bindings = candidate; }
        }
        if bindings.len() == before { return Ok(()); }
    }
}

fn invalid(message: impl Into<Text>, span: Span) -> CodegenError {
    CodegenError::with_span(
        CodegenErrorKind::InvalidSourceSignature(message.into().into_string()),
        span,
    )
}
