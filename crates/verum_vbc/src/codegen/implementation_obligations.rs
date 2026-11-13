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
            if depth >= 64 || nodes > 4096 {
                return Err(invalid(
                    "implementation obligation type budget exceeded",
                    span,
                ));
            }
            let mut push = |ty| pending.push((ty, depth + 1));
            match ty {
                TypeRef::Instantiated { args, .. } | TypeRef::Tuple(args) => {
                    for ty in args {
                        push(ty);
                    }
                }
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
                    for ty in params {
                        push(ty);
                    }
                    push(return_type);
                }
                TypeRef::Reference { inner, .. } | TypeRef::Slice(inner) => push(inner),
                TypeRef::Array { element, .. } => push(element),
                TypeRef::AssociatedProjection {
                    base,
                    qualifier,
                    args,
                    ..
                } => {
                    push(base);
                    for ty in args {
                        push(ty);
                    }
                    if let Some(application) = qualifier {
                        for ty in &application.args {
                            push(ty);
                        }
                    }
                }
                TypeRef::Concrete(_)
                | TypeRef::Generic(_)
                | TypeRef::ConstValue(_)
                | TypeRef::ConstSequence(_)
                | TypeRef::RecordFieldProjection { .. } => {}
            }
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
                TypeBoundKind::Equality(ty) | TypeBoundKind::GenericProtocol(ty)
                    if matches!(ty.kind, TypeKind::Function { .. }) =>
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

    /// Match a header in its own binder space, then resolve every premise in
    /// the actual receiver's space. Equal numeric binder IDs are not equality.
    fn implementation_alternative(
        &mut self,
        header: &ImplementationHeader,
        goal: &Goal,
        assumptions: &Premises,
    ) -> CodegenResult<Maybe<Premises>> {
        self.with_protocol_source_scope(
            Some(header.source.origin_module().into()),
            Default::default(),
            None,
            |cg| {
                let declaration = &header.declaration;
                if !matches!(declaration.kind, ImplKind::Protocol { .. }) {
                    return Ok(None);
                }
                let (_, _, names) = cg.resolve_impl_generics(declaration);
                let scope = cg.declared_impl_generic_scope(&declaration.kind, &names);
                // An alternative belongs to its own declaration. The querying
                // implementation cannot supply or erase its value binders.
                let source_bindings = cg.implementation_value_scope(declaration, &scope)?;
                cg.current_source_substitution = source_bindings.clone();
                let receiver = cg.resolve_impl_receiver(declaration, &scope, &names)?;
                let mut bindings = TypeSubstitution::empty();
                if !receiver.semantic
                    || !match_declared_types(&receiver.ty, &goal.subject, &mut bindings)
                {
                    return Ok(None);
                }
                let mut matched = None;
                for head in cg.implementation_heads(declaration, &receiver, &scope)? {
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
                if scope.values().any(|id| bindings.get(TypeParamId(*id)).is_none()) {
                    let symbolic = cg.implementation_premises(declaration, &scope)?;
                    infer_implementation_bindings(&symbolic, assumptions, &mut bindings, declaration.span)?;
                }
                let mut substitution = SourceTypeSubstitution::default();
                for (name, id) in &scope {
                    let Some(actual) = bindings.get(TypeParamId(*id)) else {
                        return Ok(Some(Premises {
                            unknown: Some(format!(
                                "implementation has an undetermined parameter '{name}' in '{}' for '{}'",
                                header.source.origin_module(),
                                cg.extract_impl_type_name(&declaration.kind)
                                    .unwrap_or_else(|| format!("{:?}", receiver.ty)),
                            ).into()),
                            ..Default::default()
                        }));
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
                        Ok(Some(premises))
                    },
                )
            },
        )
    }

    pub(super) fn blanket_requirements_hold(
        &mut self,
        blanket: &super::BlanketImpl,
        receiver: &ImplReceiverWitness,
        implementation: &ImplDecl,
        scope: &std::collections::HashMap<String, u16>,
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
        self.prove_implementation_goals(requirements.goals, &assumptions, blanket.declaration.span)
    }

    /// Build a finite AND/OR obligation graph and compute its least fixed point.
    /// Cycles cannot prove themselves. Exhaustion is a diagnostic, never success.
    fn prove_implementation_goals(
        &mut self,
        required: List<Goal>,
        assumptions: &Premises,
        span: Span,
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
        let mut headers = self.implementation_headers.clone();
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
                choices.push(List::new());
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
                if !headers.iter().any(|header| header.source == source) {
                    let declaration =
                        Arc::new(source.declaration().map_err(|error| invalid(error, span))?);
                    headers.push(ImplementationHeader {
                        source,
                        declaration,
                    });
                }
            }
            for header in &headers {
                let Some(premises) = self.implementation_alternative(header, &goal, assumptions)? else {
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
                if match_evidence(signature, actual, &mut candidate, span)? {
                    retain_evidence(&mut selected, candidate, span)?;
                }
            }
            if let Some(candidate) = selected { *bindings = candidate; }
        }
        for goal in &requirements.goals {
            if goal.associated.is_empty() { continue; }
            let Some(subject) = determined_subject(&goal.subject, bindings) else { continue };
            let mut selected = None;
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
