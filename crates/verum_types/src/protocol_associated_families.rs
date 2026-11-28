//! Family application uses its declared local telescope, never RHS variable order.
use super::{
    AssociatedType, ConstraintEnvironment, ProtocolBound, ProtocolChecker, ProtocolImpl, WhereClause,
    implementation_match::CandidateMatch,
};
use crate::{
    advanced_protocols::AssociatedTypeKind,
    context::GenericBinder,
    ty::{Substitution, Type, ValueTerm},
};
use verum_common::{List, Text};

#[path = "protocol_associated_components.rs"]
mod components;
use components::ComponentPlan;

/// A resolved family application also records completed local arguments. A
/// symbolic application must retain these arguments when emitted or substituted.
pub(super) struct AssignmentFailure {
    pub reason: Text,
    pub bound: Option<Text>,
    pub actual: Option<Type>,
}
impl From<Text> for AssignmentFailure {
    fn from(reason: Text) -> Self {
        Self {
            reason,
            bound: None,
            actual: None,
        }
    }
}
impl From<&str> for AssignmentFailure {
    fn from(reason: &str) -> Self {
        Text::from(reason).into()
    }
}

struct DeclarationApplication {
    owner: Text,
    declaration: AssociatedType,
    arguments: List<Type>,
}

/// Borrowed declaration authority for a source protocol whose methods are
/// still being elaborated. It is never inserted into the shared registry.
#[derive(Clone, Copy)]
pub(crate) struct AssociatedDeclarationHeader<'a> {
    pub identity: &'a Text,
    pub constraints: &'a super::ProtocolConstraints,
    pub associated: &'a verum_common::Map<Text, AssociatedType>,
    pub supers: &'a [ProtocolBound],
}

pub struct FamilyApplication {
    pub arguments: List<Type>,
    pub result: Type,
    components: Option<ComponentPlan>,
}

impl ProtocolChecker {
    pub(crate) fn same_associated_owner(&self, left: &ProtocolBound, right: &ProtocolBound) -> bool {
        self.paths_equal(&left.protocol, &right.protocol)
            && left.args.len() == right.args.len()
            && left.args.iter().zip(&right.args).all(|(a, b)| self.types_structurally_equal(a, b))
    }

    /// Select a member in the declaration's scope before substituting its
    /// receiver. Two distinct parameters may later become the same concrete
    /// type while projecting members of different protocols.
    pub(crate) fn associated_projection_owner(
        &self,
        receiver: &Type,
        name: &Text,
        assumptions: &[WhereClause],
    ) -> Result<Option<ProtocolBound>, Text> {
        let mut owner: Option<ProtocolBound> = None;
        for clause in assumptions {
            if !self.types_structurally_equal(receiver, &clause.ty) { continue; }
            for bound in &clause.bounds {
                if bound.is_negative { continue; }
                let key = self.make_protocol_key(&bound.protocol);
                let Some(selected) = self.associated_declaration_application(
                    &key, &bound.args, receiver, name, &mut Vec::new(),
                )? else { continue; };
                let protocol = verum_ast::Path::new(
                    selected.owner.as_str().split('.').map(|part|
                        verum_ast::PathSegment::Name(verum_ast::Ident::new(part, bound.protocol.span))
                    ).collect(), bound.protocol.span,
                );
                let candidate = ProtocolBound::positive(protocol, selected.arguments);
                if let Some(previous) = &owner {
                    if !self.same_associated_owner(previous, &candidate) {
                        return Err("associated projection has competing declaration owners".into());
                    }
                } else { owner = Some(candidate); }
            }
        }
        Ok(owner)
    }

    /// A bound on an associated type is guaranteed by its declaring protocol,
    /// including when the receiver is rigid and its implementation is unknown.
    pub(super) fn ordinary_projection_bounds(
        &self, receiver: &Type, name: &str, environment: ConstraintEnvironment<'_>,
    ) -> Result<List<ProtocolBound>, Text> {
        let name = Text::from(name);
        let Some(owner) = self.associated_projection_owner(receiver, &name, environment.protocols)? else {
            return Ok(List::new());
        };
        let key = self.make_protocol_key(&owner.protocol);
        let protocol = self.get_protocol(&key)
            .ok_or_else(|| Text::from("associated projection owner is unavailable"))?;
        let declaration = protocol.associated_types.get(&name)
            .ok_or_else(|| Text::from("associated member is absent from its declaring protocol"))?;
        if !matches!(declaration.kind, AssociatedTypeKind::Regular) || declaration.bounds.is_empty() {
            return Ok(List::new());
        }
        protocol.constraints.application(receiver, &owner.args)?;
        let (_, scope) = self.complete_associated_family_arguments_in(
            declaration, receiver, &owner.args, &[], environment,
        )?;
        declaration.bounds.iter().map(|bound| {
            bound.instantiate_family_result(declaration.binders.self_type, &scope, |member| {
                let member = Text::from(member);
                let declared = self.associated_declaration_application(
                    &key, &owner.args, receiver, &member, &mut Vec::new(),
                )?.ok_or_else(|| Text::from("associated result bound names an undeclared sibling"))?;
                let selected = self.associated_projection_owner(receiver, &member, environment.protocols)?
                    .ok_or_else(|| Text::from("associated result bound has no selected sibling"))?;
                if self.make_protocol_key(&selected.protocol) != declared.owner
                    || selected.args.len() != declared.arguments.len()
                    || !selected.args.iter().zip(&declared.arguments)
                        .all(|(actual, expected)| self.types_structurally_equal(actual, expected))
                    || !matches!(declared.declaration.kind, AssociatedTypeKind::Regular)
                {
                    return Err("associated result bound requires its declaring sibling projection".into());
                }
                self.resolve_associated_family(receiver, &member, &[], environment.protocols)?
                    .map(|family| family.result)
                    .ok_or_else(|| Text::from("associated result bound lacks its sibling projection"))
            })
        }).collect()
    }

    /// Reduce through exactly the selected owner application, retaining all
    /// implementation and family domain checks of ordinary family resolution.
    pub(crate) fn resolve_owned_associated_family(
        &self,
        receiver: &Type,
        owner: &ProtocolBound,
        name: &Text,
        arguments: &[Type],
        assumptions: &[WhereClause],
    ) -> Result<Option<FamilyApplication>, Text> {
        self.resolve_owned_associated_family_in(
            receiver, owner, name, arguments,
            ConstraintEnvironment::new(assumptions, &Default::default()),
        )
    }

    pub(crate) fn resolve_owned_associated_family_in(
        &self, receiver: &Type, owner: &ProtocolBound, name: &Text,
        arguments: &[Type], environment: ConstraintEnvironment<'_>,
    ) -> Result<Option<FamilyApplication>, Text> {
        self.select_owned_family(receiver, owner, name, arguments, environment)?
            .map(|application| self.complete_family_components(application, environment)).transpose()
    }

    fn select_owned_family(
        &self, receiver: &Type, owner: &ProtocolBound, name: &Text,
        arguments: &[Type], environment: ConstraintEnvironment<'_>,
    ) -> Result<Option<FamilyApplication>, Text> {
        let key = self.make_protocol_key(&owner.protocol);
        let Some(declaration) = self.associated_declaration_application(
            &key, &owner.args, receiver, name, &mut Vec::new(),
        )? else { return Err("selected protocol does not declare this associated member".into()); };
        if let Some(application) = self.assumed_family(
            receiver, name, arguments, environment.protocols, None, Some(&declaration),
        )? { return Ok(Some(application)); }
        self.select_family_implementation(
            self.impls_by_protocol.get_key_value(&key),
            receiver, name, arguments, environment, Some((&owner.args, &declaration)), None,
        )
    }

    /// A dynamic object's own interface fixes an associated member. Resolve it
    /// through the same declaration and family domain checks as a rigid premise.
    pub(super) fn resolve_dynamic_associated_family(
        &self,
        receiver: &Type,
        protocol: &verum_ast::Path,
        arguments: Option<&[Type]>,
        name: &Text,
    ) -> Result<Type, Text> {
        let mut remaining = super::obligations::PROTOCOL_OBLIGATION_BUDGET;
        let bounds = self.dynamic_protocol_premises(receiver, &mut remaining)?;
        let mut selected = None;
        for bound in &bounds {
            if !self.paths_equal(&bound.protocol, protocol)
                || arguments.is_some_and(|arguments| {
                    arguments.len() != bound.args.len()
                        || !arguments
                            .iter()
                            .zip(&bound.args)
                            .all(|(a, b)| self.types_structurally_equal(a, b))
                })
            {
                continue;
            }
            if let Some(previous) = selected {
                if !self.same_associated_owner(previous, bound) {
                    return Err("dynamic member has competing protocol applications".into());
                }
            }
            selected = Some(bound);
        }
        let selected = selected.ok_or_else(|| {
            Text::from("dynamic receiver does not establish the selected protocol")
        })?;
        let selected = self
            .associated_declaration_application(
                &self.make_protocol_key(&selected.protocol),
                &selected.args,
                receiver,
                name,
                &mut Default::default(),
            )?
            .ok_or_else(|| Text::from("selected protocol has no such associated member"))?;
        // The validated closure already transports inherited equalities to
        // their declaring application. Select that application directly, so
        // an ambiguous member on a different ancestor cannot replace it.
        let premises: List<_> = bounds
            .into_iter()
            .filter(|bound| {
                self.make_protocol_key(&bound.protocol) == selected.owner
                    && bound.args.len() == selected.arguments.len()
                    && bound
                        .args
                        .iter()
                        .zip(&selected.arguments)
                        .all(|(a, b)| self.types_structurally_equal(a, b))
            })
            .map(|bound| WhereClause {
                ty: receiver.clone(),
                bounds: List::from_iter([bound]),
            })
            .collect();
        self.resolve_associated_family(receiver, name, &[], &premises)?
            .map(|application| application.result)
            .ok_or_else(|| Text::from("dynamic receiver lacks the selected associated equality"))
    }

    fn same_family_application(&self, left: &FamilyApplication, right: &FamilyApplication) -> bool {
        left.arguments.len() == right.arguments.len()
            && left.arguments.iter().zip(&right.arguments).all(|(a, b)| self.types_structurally_equal(a, b))
            && self.types_structurally_equal(&left.result, &right.result)
            && match (&left.components, &right.components) {
                (Some(left), Some(right)) => left.agrees_with(right, self),
                (None, None) => true,
                _ => false,
            }
    }

    /// Equations supplied by actual declaration premises for one selected
    /// associated member. A same-spelling member on another protocol is not
    /// evidence: both routes must reach the same declaration-owned family.
    pub(super) fn associated_premise_equations(
        &self,
        receiver: &Type,
        required: &ProtocolBound,
        name: &Text,
        assumptions: &[WhereClause],
    ) -> Result<List<(Type, Type)>, Text> {
        let mut equations = List::new();
        if required.is_negative { return Ok(equations); }
        let expected = self.checked_equality_declaration(receiver, required, name)?;
        let value = required.associated_types.get(name)
            .ok_or_else(|| Text::from("associated equation lacks its required value"))?;
        for clause in assumptions {
            if !self.types_structurally_equal(receiver, &clause.ty) { continue; }
            for bound in &clause.bounds {
                if bound.is_negative { continue; }
                let key = self.make_protocol_key(&bound.protocol);
                let Some(selected) = self.associated_declaration_application(
                    &key, &bound.args, receiver, name, &mut Vec::new(),
                )? else { continue; };
                if selected.owner != expected.owner
                    || selected.arguments.len() != expected.arguments.len()
                    || !selected.arguments.iter().zip(&expected.arguments)
                        .all(|(actual, expected)| self.types_structurally_equal(actual, expected))
                { continue; }
                self.checked_equality_declaration(receiver, bound, name)?;
                let actual = if let Some(actual) = bound.associated_types.get(name) {
                    actual.clone()
                } else {
                    // A declared member proves its own projection, even when
                    // its value is abstract. The checked family resolver keeps
                    // ambiguous owners and defaults from becoming equalities.
                    self.resolve_associated_family(receiver, name, &[], assumptions)?
                        .ok_or_else(|| Text::from("associated premise lacks a declared member"))?
                        .result
                };
                equations.push((value.clone(), actual));
            }
        }
        Ok(equations)
    }

    /// Preserve an explicit inherited equality only when both applications
    /// select the exact same member owner. Fixed parent equalities must agree.
    pub(super) fn inherit_associated_equalities(
        &self,
        receiver: &Type,
        source: &ProtocolBound,
        inherited: &mut ProtocolBound,
    ) -> Result<(), Text> {
        for (name, value) in &source.associated_types {
            let key = self.make_protocol_key(&inherited.protocol);
            let Some(target) = self.associated_declaration_application(
                &key, &inherited.args, receiver, name, &mut Vec::new(),
            )? else { continue; };
            let selected = self.checked_equality_declaration(receiver, source, name)?;
            if target.owner != selected.owner || target.arguments != selected.arguments { continue; }
            if let Some(prior) = inherited.associated_types.get(name) {
                if !self.types_structurally_equal(prior, value) {
                    return Err("inherited associated equality conflicts with its declared parent".into());
                }
            } else {
                inherited.associated_types.insert(name.clone(), value.clone());
            }
        }
        Ok(())
    }

    fn checked_equality_declaration(
        &self, receiver: &Type, bound: &ProtocolBound, name: &Text,
    ) -> Result<DeclarationApplication, Text> {
        let key = self.make_protocol_key(&bound.protocol);
        let protocol = self.get_protocol(&key)
            .ok_or_else(|| Text::from("associated equality protocol is unavailable"))?;
        let super::ProtocolConstraints::Declared { parameters, .. } = &protocol.constraints else {
            return Err("associated equality lacks its declaration-owned telescope".into());
        };
        if parameters.len() != bound.args.len() {
            return Err("associated equality protocol arguments are incomplete".into());
        }
        let selected = self.associated_declaration_application(
            &key, &bound.args, receiver, name, &mut Vec::new(),
        )?.ok_or_else(|| Text::from("associated equality names an undeclared family"))?;
        let (arguments, _) = self.complete_associated_family_arguments(
            &selected.declaration, receiver, &selected.arguments, &[], &[],
        )?;
        if !arguments.is_empty() {
            return Err("an applied associated family requires a parameterized equality".into());
        }
        Ok(selected)
    }

    /// Resolve only when a declaration and all potentially applicable bindings
    /// agree. Rigid symbolic receiver coverage is retained as a later obligation.
    pub fn resolve_associated_family(
        &self,
        receiver: &Type,
        name: &Text,
        arguments: &[Type],
        assumptions: &[WhereClause],
    ) -> Result<Option<FamilyApplication>, Text> {
        self.resolve_associated_family_with_header(receiver, name, arguments, assumptions, None)
    }

    pub(crate) fn resolve_associated_family_with_header(
        &self, receiver: &Type, name: &Text, arguments: &[Type],
        assumptions: &[WhereClause], header: Option<AssociatedDeclarationHeader<'_>>,
    ) -> Result<Option<FamilyApplication>, Text> {
        if let Some(application) = self.assumed_family(receiver, name, arguments, assumptions, header, None)? {
            return Ok(Some(application));
        }
        self.resolve_family_implementations(
            &self.impls_by_protocol, receiver, name, arguments,
            ConstraintEnvironment::new(assumptions, &Default::default()), None, header,
        )
    }

    fn assumed_family(
        &self, receiver: &Type, name: &Text, arguments: &[Type], assumptions: &[WhereClause],
        header: Option<AssociatedDeclarationHeader<'_>>, owner: Option<&DeclarationApplication>,
    ) -> Result<Option<FamilyApplication>, Text> {
        let mut symbolic: Option<(Text, List<Type>, FamilyApplication, bool)> = None;
        for assumption in assumptions {
            if !self.types_structurally_equal(&assumption.ty, receiver) {
                continue;
            }
            for bound in &assumption.bounds {
                if bound.is_negative {
                    continue;
                }
                let key = self.make_protocol_key(&bound.protocol);
                let Some(selected) = self.associated_declaration_application_in_scope(
                    &key,
                    &bound.args,
                    receiver,
                    name,
                    &mut Vec::new(),
                    header,
                )?
                else {
                    continue;
                };
                if owner.is_some_and(|owner| owner.owner != selected.owner
                    || owner.arguments.len() != selected.arguments.len()
                    || !owner.arguments.iter().zip(&selected.arguments).all(|(left, right)| self.types_structurally_equal(left, right)))
                {
                    continue;
                }
                let (completed, _) = self.complete_associated_family_arguments(
                    &selected.declaration,
                    receiver,
                    &selected.arguments,
                    arguments,
                    assumptions,
                )?;
                // A default member may be overridden by an implementation.
                // Only an equality in the selected receiver's actual bound
                // fixes its result in universally quantified code.
                let equality = bound.associated_types.get(name);
                if equality.is_some() && !completed.is_empty() {
                    return Err("an applied associated family requires a parameterized equality".into());
                }
                let projection = Type::Generic {
                    name: format!("::{name}").into(),
                    args: List::from_iter([receiver.clone()]),
                };
                let application = FamilyApplication {
                    components: None,
                    result: equality.cloned().unwrap_or_else(|| {
                        if completed.is_empty() { projection } else {
                            Type::TypeApp { constructor: Box::new(projection), args: completed.clone() }
                        }
                    }),
                    arguments: completed,
                };
                if let Some((prior, args, prior_application, prior_equality)) = &mut symbolic {
                    if prior != &selected.owner || args != &selected.arguments {
                        return Err("symbolic family has competing declaration owners".into());
                    }
                    if equality.is_some() {
                        if *prior_equality && !self.types_structurally_equal(
                            &prior_application.result, &application.result,
                        ) {
                            return Err("symbolic family has conflicting associated equalities".into());
                        }
                        *prior_application = application;
                        *prior_equality = true;
                    }
                } else {
                    symbolic = Some((selected.owner, selected.arguments, application, equality.is_some()));
                }
            }
        }
        Ok(symbolic.map(|(_, _, application, _)| application))
    }

    /// A known owner narrows the index search. Every remaining candidate must
    /// prove its domain in the caller's scope and agree on the same application.
    fn resolve_family_implementations<'a>(
        &self,
        indices: impl IntoIterator<Item = (&'a Text, &'a super::ImplOverlapIndex)>,
        receiver: &Type, name: &Text, arguments: &[Type], environment: ConstraintEnvironment<'_>,
        owner: Option<(&[Type], &DeclarationApplication)>,
        header: Option<AssociatedDeclarationHeader<'_>>,
    ) -> Result<Option<FamilyApplication>, Text> {
        self.select_family_implementation(indices, receiver, name, arguments, environment, owner, header)?
            .map(|application| self.complete_family_components(application, environment)).transpose()
    }

    fn select_family_implementation<'a>(
        &self,
        indices: impl IntoIterator<Item = (&'a Text, &'a super::ImplOverlapIndex)>,
        receiver: &Type, name: &Text, arguments: &[Type], environment: ConstraintEnvironment<'_>,
        owner: Option<(&[Type], &DeclarationApplication)>,
        header: Option<AssociatedDeclarationHeader<'_>>,
    ) -> Result<Option<FamilyApplication>, Text> {
        let mut receiver_scope = None;
        let mut selected: Option<(Text, List<Type>, FamilyApplication)> = None;
        for (key, index) in indices {
            if owner.is_none() && !self.has_associated_declaration(key, name, false, &mut Vec::new()) {
                continue;
            }
            for index in index.candidates(receiver, self.requires_unfiltered_type_candidates(receiver)) {
                let implementation = &self.impls[index];
                let substitution = match self.match_implementation_candidate_in_environment(
                    implementation, receiver, owner.map(|(args, _)| args), true, &environment,
                ) {
                    CandidateMatch::Disjoint => continue,
                    CandidateMatch::Unknown => return Err(format!(
                        "cannot establish the receiver domain of a competing family implementation of `{}` for `{}` while resolving `{}.{name}`",
                        implementation.protocol, implementation.for_type, receiver,
                    ).into()),
                    CandidateMatch::Matched(substitution) => substitution,
                };
                let inferred;
                let declaration = match owner {
                    Some((_, declaration)) => declaration,
                    None => {
                        let owner_arguments: List<_> = implementation.protocol_args.iter()
                            .map(|arg| arg.try_apply_subst(&substitution)).collect::<Result<_, _>>()?;
                        let Some(found) = self.associated_declaration_application_in_scope(
                            key, &owner_arguments, receiver, name, &mut Vec::new(), header,
                        )? else { continue };
                        for argument in &found.arguments {
                            let (variables, values) = receiver_scope
                                .get_or_insert_with(|| (receiver.free_vars(), receiver.value_ids()));
                            if argument.has_value_inference()
                                || argument.free_vars().iter().any(|id| !variables.contains(id))
                                || argument.value_ids().iter().any(|id| !values.contains(id))
                            {
                                return Err("family owner application is not determined by its receiver".into());
                            }
                        }
                        inferred = found;
                        &inferred
                    }
                };
                let application = self.instantiate_declared_family(
                    implementation,
                    &declaration.declaration,
                    receiver,
                    name,
                    &declaration.arguments,
                    arguments,
                    &substitution,
                    environment,
                )?;
                if let Some((prior_protocol, prior_owner, prior)) = &mut selected {
                    if prior_protocol != &declaration.owner
                        || prior_owner != &declaration.arguments
                        || !self.same_family_application(prior, &application)
                    {
                        return Err(
                            "associated family application has competing declaration bindings"
                                .into(),
                        );
                    }
                    if let (Some(kept), Some(additional)) = (&mut prior.components, application.components) {
                        for bound in additional.result_bounds {
                            if !kept.result_bounds.contains(&bound) { kept.result_bounds.push(bound); }
                        }
                    }
                } else {
                    selected = Some((declaration.owner.clone(), declaration.arguments.clone(), application));
                }
            }
        }
        Ok(selected.map(|(_, _, application)| application))
    }

    /// Complete and prove the declared domain before using any implementation body.
    pub fn complete_associated_family_arguments(
        &self,
        declaration: &AssociatedType,
        receiver: &Type,
        owner_arguments: &[Type],
        supplied: &[Type],
        assumptions: &[WhereClause],
    ) -> Result<(List<Type>, Substitution), Text> {
        self.complete_associated_family_arguments_in(
            declaration, receiver, owner_arguments, supplied,
            ConstraintEnvironment::new(assumptions, &Default::default()),
        )
    }

    fn complete_associated_family_arguments_in(
        &self,
        declaration: &AssociatedType,
        receiver: &Type,
        owner_arguments: &[Type],
        supplied: &[Type],
        environment: ConstraintEnvironment<'_>,
    ) -> Result<(List<Type>, Substitution), Text> {
        if declaration.binders.local.len() != declaration.type_params.len() {
            return Err("associated family lacks its declaration-owned local telescope".into());
        }
        if supplied.len() > declaration.binders.local.len() {
            return Err("too many associated family arguments".into());
        }
        if matches!(declaration.kind, AssociatedTypeKind::HigherKinded { .. }) {
            return Err(
                "associated family constructors require exact declared kind evidence".into(),
            );
        }
        let mut declaration_scope = Substitution::new();
        let self_variable = declaration
            .binders
            .self_type
            .ok_or_else(|| Text::from("associated family lacks its declaring Self binder"))?;
        declaration_scope.insert(self_variable, receiver.clone());
        for (position, binder) in &declaration.binders.owner {
            let argument = owner_arguments
                .get(*position)
                .ok_or_else(|| Text::from("associated family owner application is incomplete"))?;
            binder.bind_argument_normalized( argument, &mut declaration_scope,
                |ty| self.normalize_projection_type(ty))?;
        }
        let mut arguments = List::new();
        for (position, (parameter, binder)) in declaration
            .type_params
            .iter()
            .zip(&declaration.binders.local)
            .enumerate()
        {
            let argument = match supplied.get(position) {
                Some(argument) => argument.clone(),
                None => parameter
                    .default
                    .as_ref()
                    .ok_or_else(|| Text::from("missing required associated family argument"))?
                    .try_apply_subst(&declaration_scope)?,
            };
            binder.bind_argument_normalized( &argument, &mut declaration_scope,
                |ty| self.normalize_projection_type(ty))?;
            arguments.push(argument);
        }
        for (parameter, argument) in declaration.type_params.iter().zip(&arguments) {
            let bounds: List<_> = parameter
                .bounds
                .iter()
                .map(|bound| bound.try_apply_subst(&declaration_scope))
                .collect::<Result<_, _>>()?;
            self.check_universal_bounds_in(argument, &bounds, environment)?;
        }
        for clause in &declaration.where_clauses {
            let position = declaration
                .type_params
                .iter()
                .position(|parameter| parameter.name == clause.param)
                .ok_or_else(|| {
                    Text::from("family where clause is outside its declared telescope")
                })?;
            let bounds: List<_> = clause
                .constraints
                .iter()
                .map(|bound| bound.try_apply_subst(&declaration_scope))
                .collect::<Result<_, _>>()?;
            self.check_universal_bounds_in(&arguments[position], &bounds, environment)?;
        }
        Ok((arguments, declaration_scope))
    }

    fn instantiate_declared_family(
        &self,
        implementation: &ProtocolImpl,
        declaration: &AssociatedType,
        receiver: &Type,
        name: &Text,
        owner_arguments: &[Type],
        supplied: &[Type],
        implementation_scope: &Substitution,
        environment: ConstraintEnvironment<'_>,
    ) -> Result<FamilyApplication, Text> {
        let (arguments, declaration_scope) = self.complete_associated_family_arguments_in(
            declaration,
            receiver,
            owner_arguments,
            supplied,
            environment,
        )?;
        self.check_implementation_callable_bounds_in_environment(implementation, implementation_scope, &environment)?;
        // Head matching is not evidence for conditional implementations. The
        // ordinary inference matcher may defer constraints; this boundary may not.
        for clause in &implementation.where_clauses {
            let subject = clause.ty.try_apply_subst(implementation_scope)?;
            let bounds: List<_> = clause
                .bounds
                .iter()
                .map(|bound| bound.try_apply_subst(implementation_scope))
                .collect::<Result<_, _>>()?;
            self.check_universal_bounds_in(&subject, &bounds, environment)?;
        }
        let protocol = ProtocolBound::positive(
            implementation.protocol.clone(),
            implementation
                .protocol_args
                .iter()
                .map(|argument| argument.try_apply_subst(implementation_scope))
                .collect::<Result<_, _>>()?,
        );
        self.check_universal_bounds_in(receiver, &[protocol], environment)?;
        let (result, components) = if let Some(body) = implementation.associated_types.get(name) {
            let (body, scope) = body.application_scope(&arguments, implementation_scope)?;
            let result = body.try_apply_subst(&scope)?;
            let components = self.family_component_plan(body, &scope, implementation)?;
            (result, components)
        } else if let Some(body) = &declaration.default {
            (body.try_apply_subst(&declaration_scope)?, None)
        } else {
            return Err("associated family has no assignment or declared default".into());
        };
        let mut allowed = receiver.free_vars();
        let mut allowed_values = receiver.value_ids();
        for argument in owner_arguments.iter().chain(&arguments) {
            allowed.extend(argument.free_vars());
            allowed_values.extend(argument.value_ids());
        }
        // The exact implementation match also solves declaration parameters
        // from receiver-owned equations, e.g. S.Element = T for Wrapper<S>.
        // Only images of its quantified binders can contribute result types.
        for binder in &implementation.parameters {
            if let GenericBinder::Type(variable) = binder
                && let Some(argument) = implementation_scope.get(variable)
            {
                allowed.extend(argument.free_vars());
            }
        }
        if result
            .free_vars()
            .iter()
            .any(|variable| !allowed.contains(variable))
        {
            return Err("associated family result contains an unbound declaration variable".into());
        }
        if result.has_value_inference() || result.value_ids().iter().any(|id| !allowed_values.contains(id)) {
            return Err("associated family result contains an unbound declaration value".into());
        }
        let bounds = self.family_result_bounds(
            implementation, name, declaration, &declaration_scope, implementation_scope,
        )?;
        let mut application = FamilyApplication { arguments, result, components };
        if let Some(plan) = &mut application.components {
            plan.result_bounds = bounds;
        } else {
            self.check_universal_bounds_in(&application.result, &bounds, environment)?;
        }
        Ok(application)
    }

    fn family_result_bounds(
        &self, implementation: &ProtocolImpl, name: &Text, declaration: &AssociatedType,
        declaration_scope: &Substitution, implementation_scope: &Substitution,
    ) -> Result<List<ProtocolBound>, Text> {
        declaration.bounds.iter().map(|bound| {
            bound.instantiate_family_result(declaration.binders.self_type, declaration_scope, |member| {
                // The implementation still owns its parameters here. Select the
                // defining protocol before instantiating any concrete arguments.
                let key = self.make_protocol_key(&implementation.protocol);
                let owner = self.associated_declaration_application(
                    &key, &implementation.protocol_args, &implementation.for_type, name, &mut Vec::new(),
                )?.ok_or_else(|| Text::from("associated result bound has no declaring protocol"))?;
                self.source_impl_projection(
                    implementation, &owner.owner, &owner.arguments, &Text::from(member),
                    &[], &implementation.where_clauses,
                )?.try_apply_subst(implementation_scope)
            })
        }).collect()
    }
}
impl ProtocolChecker {
    /// Check a definition universally over its own local telescope. The selected
    /// declaration supplies the domain; the implementation may not narrow it.
    pub(super) fn check_associated_assignment(
        &self,
        implementation: &ProtocolImpl,
        name: &Text,
        assignment: &super::AssociatedTypeAssignment,
    ) -> Result<(), AssignmentFailure> {
        let key = self.make_protocol_key(&implementation.protocol);
        let selected = self
            .associated_declaration_application(
                &key,
                &implementation.protocol_args,
                &implementation.for_type,
                name,
                &mut Vec::new(),
            )?
            .ok_or_else(|| Text::from("associated assignment has no declaring protocol"))?;
        let declaration = selected.declaration;
        let owner_arguments = selected.arguments;
        if declaration.refinement.is_some() {
            return Err("associated result refinement requires a checked refinement proof".into());
        }
        if assignment.parameters().len() != declaration.type_params.len() {
            return Err("associated assignment local arity differs from its declaration".into());
        }
        let mut scope = Substitution::new();
        if let Some(variable) = declaration.binders.self_type {
            scope.insert(variable, implementation.for_type.clone());
        } else if !declaration.binders.local.is_empty()
            || !declaration.binders.owner.is_empty()
            || !declaration.type_params.is_empty()
        {
            return Err("associated assignment lacks its declaration telescope".into());
        }
        for (position, binder) in &declaration.binders.owner {
            let argument = owner_arguments
                .get(*position)
                .ok_or_else(|| Text::from("associated owner application is incomplete"))?;
            binder.bind_argument_normalized( argument, &mut scope,
                |ty| self.normalize_projection_type(ty))?;
        }
        if declaration.binders.local.len() != assignment.parameters().len() {
            return Err("associated assignment lacks its declared local telescope".into());
        }
        let mut arguments = List::new();
        let mut implementation_locals = Substitution::new();
        for (declared, implemented) in declaration.binders.local.iter().zip(assignment.parameters()) {
            let argument = match (declared, implemented) {
                (GenericBinder::Type(_), GenericBinder::Type(variable)) => Type::Var(*variable),
                (GenericBinder::Value(domain), GenericBinder::Value(Type::Meta {
                    name, value: ValueTerm::Parameter(id), ..
                })) => {
                    let Type::Meta { ty, refinement, value: ValueTerm::Parameter(_), .. } = domain.try_apply_subst(&scope)? else {
                        return Err("associated assignment has no declared value domain".into());
                    };
                    // Use the declaration's domain with the implementation's
                    // rigid identity. A narrower implementation must fail.
                    Type::Meta { name: name.clone(), ty, refinement, value: ValueTerm::Parameter(*id) }
                }
                _ => return Err("associated assignment parameter role differs from its declaration".into()),
            };
            implemented.bind_argument_normalized( &argument, &mut implementation_locals,
                |ty| self.normalize_projection_type(ty))?;
            declared.bind_argument_normalized( &argument, &mut scope,
                |ty| self.normalize_projection_type(ty))?;
            arguments.push(argument);
        }
        let mut assumptions = implementation.where_clauses.clone();
        for (parameter, argument) in declaration.type_params.iter().zip(&arguments) {
            assumptions.push(WhereClause {
                ty: argument.clone(),
                bounds: parameter
                    .bounds
                    .iter()
                    .map(|bound| bound.try_apply_subst(&scope))
                    .collect::<Result<_, _>>()?,
            });
        }
        for clause in &declaration.where_clauses {
            let position = declaration
                .type_params
                .iter()
                .position(|parameter| parameter.name == clause.param)
                .ok_or_else(|| Text::from("family where clause has no declared subject"))?;
            assumptions.push(WhereClause {
                ty: arguments[position].clone(),
                bounds: clause
                    .constraints
                    .iter()
                    .map(|bound| bound.try_apply_subst(&scope))
                    .collect::<Result<_, _>>()?,
            });
        }
        let result = assignment.instantiate(&arguments, &Substitution::new())?;
        let bounds = self.family_result_bounds(
            implementation, name, &declaration, &scope, &Substitution::new(),
        )?;
        for bound in bounds {
            self.check_universal_bounds(&result, std::slice::from_ref(&bound), &assumptions)
                .map_err(|reason| AssignmentFailure {
                    reason,
                    bound: Some(self.extract_protocol_name(&bound.protocol)),
                    actual: Some(result.clone()),
                })?;
        }
        Ok(())
    }

    /// Preserve each super-protocol's actual owner application when selecting a
    /// family. A diamond must agree on the declaration and its arguments.
    fn associated_declaration_application(
        &self,
        key: &Text,
        arguments: &[Type],
        receiver: &Type,
        name: &Text,
        active: &mut Vec<Text>,
    ) -> Result<Option<DeclarationApplication>, Text> {
        self.associated_declaration_application_in_scope(key, arguments, receiver, name, active, None)
    }

    fn associated_declaration_application_in_scope(
        &self, key: &Text, arguments: &[Type], receiver: &Type, name: &Text,
        active: &mut Vec<Text>, header: Option<AssociatedDeclarationHeader<'_>>,
    ) -> Result<Option<DeclarationApplication>, Text> {
        if active.len() >= 64 || active.contains(key) {
            return Err("recursive associated declaration inheritance has no finite owner".into());
        }
        let (constraints, associated, supers) = if let Some(header) = header.filter(|header| header.identity == key) {
            (header.constraints, header.associated, header.supers)
        } else {
            let protocol = self.protocols.get(key)
                .ok_or_else(|| Text::from("associated declaration owner is unavailable"))?;
            (&protocol.constraints, &protocol.associated_types, protocol.super_protocols.as_slice())
        };
        if let Some(declaration) = associated.get(name) {
            return Ok(Some(DeclarationApplication {
                owner: key.clone(),
                declaration: declaration.clone(),
                arguments: arguments.iter().cloned().collect(),
            }));
        }
        active.push(key.clone());
        let resolved = (|| {
            let mut selected: Option<DeclarationApplication> = None;
            if supers.is_empty() {
                return Ok(None);
            }
            let super::ProtocolConstraints::Declared {
                parameters,
                ..
            } = constraints
            else {
                return Err("inherited associated declaration lacks its owner telescope".into());
            };
            if parameters.len() != arguments.len() {
                return Err(
                    "inherited associated declaration owner arguments are incomplete".into(),
                );
            }
            let scope = constraints.application(receiver, arguments)?;
            for bound in supers {
                if bound.is_negative {
                    continue;
                }
                let bound = bound.try_apply_subst(&scope)?;
                let parent = self.make_protocol_key(&bound.protocol);
                let Some(candidate) = self.associated_declaration_application_in_scope(
                    &parent,
                    &bound.args,
                    receiver,
                    name,
                    active,
                    header,
                )?
                else {
                    continue;
                };
                if let Some(prior) = &selected {
                    if prior.owner != candidate.owner || prior.arguments != candidate.arguments {
                        return Err(
                            "inherited associated family has competing declaration applications"
                                .into(),
                        );
                    }
                } else {
                    selected = Some(candidate);
                }
            }
            Ok(selected)
        })();
        active.pop();
        resolved
    }
}

impl ProtocolChecker {
    /// Bare projection syntax may omit only arguments that the selected family
    /// declaration actually supplies as defaults. Ordinary associated types keep
    /// their zero-argument resolution path.
    pub fn resolve_omitted_family_arguments(
        &self,
        receiver: &Type,
        name: &Text,
        assumptions: &[WhereClause],
    ) -> Result<Option<FamilyApplication>, Text> {
        let mut is_family = false;
        let callables = Default::default();
        let environment = super::ConstraintEnvironment::new(assumptions, &callables);
        for clause in assumptions {
            if !self.types_structurally_equal(&clause.ty, receiver) {
                continue;
            }
            for bound in &clause.bounds {
                if bound.is_negative {
                    continue;
                }
                let key = self.make_protocol_key(&bound.protocol);
                if let Some(selected) = self.associated_declaration_application(
                    &key,
                    &bound.args,
                    receiver,
                    name,
                    &mut Vec::new(),
                )? {
                    is_family |= !selected.declaration.type_params.is_empty();
                }
            }
        }
        for (key, index) in &self.impls_by_protocol {
            if !self.has_associated_declaration(key, name, true, &mut Vec::new()) {
                continue;
            }
            for index in index.candidates(receiver, self.requires_unfiltered_type_candidates(receiver)) {
                let implementation = &self.impls[index];
                let substitution = match self.match_implementation_candidate_in_environment(
                    implementation, receiver, None, true, &environment,
                ) {
                    CandidateMatch::Disjoint => continue,
                    CandidateMatch::Unknown => return Err("cannot determine omitted family arguments for an overlapping implementation".into()),
                    CandidateMatch::Matched(substitution) => substitution,
                };
                let args: List<_> = implementation
                    .protocol_args
                    .iter()
                    .map(|ty| ty.try_apply_subst(&substitution))
                    .collect::<Result<_, _>>()?;
                if let Some(selected) = self.associated_declaration_application(
                    key,
                    &args,
                    receiver,
                    name,
                    &mut Vec::new(),
                )? {
                    is_family |= !selected.declaration.type_params.is_empty();
                }
            }
        }
        if !is_family {
            return Ok(None);
        }
        self.resolve_associated_family(receiver, name, &[], assumptions)
    }
}

impl ProtocolChecker {
    fn has_associated_declaration(
        &self,
        key: &Text,
        name: &Text,
        require_family: bool,
        visited: &mut Vec<Text>,
    ) -> bool {
        if visited.len() >= 64 || visited.contains(key) {
            return false;
        }
        let Some(protocol) = self.protocols.get(key) else {
            return false;
        };
        if let Some(declaration) = protocol.associated_types.get(name) {
            return !require_family || !declaration.type_params.is_empty();
        }
        visited.push(key.clone());
        protocol
            .super_protocols
            .iter()
            .filter(|bound| !bound.is_negative)
            .any(|bound| {
                self.has_associated_declaration(
                    &self.make_protocol_key(&bound.protocol),
                    name,
                    require_family,
                    visited,
                )
            })
    }
}

impl ProtocolChecker {
    /// Resolve a projection while checking this exact implementation, before it
    /// is published as evidence. Its owner telescope and assignments are already
    /// elaborated; no lookup may select a different protocol on the receiver.
    pub(crate) fn source_impl_projection(
        &self,
        implementation: &ProtocolImpl,
        method_owner: &Text,
        method_arguments: &[Type],
        name: &Text,
        supplied: &[Type],
        assumptions: &[WhereClause],
    ) -> Result<Type, Text> {
        let key = self.extract_protocol_name(&implementation.protocol);
        let selected = self
            .associated_declaration_application(
                method_owner,
                method_arguments,
                &implementation.for_type,
                name,
                &mut Vec::new(),
            )?
            .ok_or_else(|| Text::from("method projection is absent from its declaring protocol"))?;
        let environment = ConstraintEnvironment::new(assumptions, &implementation.type_param_fn_bounds);
        if selected.owner != key {
            // An inherited member belongs to the instantiated ancestor. Its
            // conditional implementation must hold under this implementation's
            // premises, including when the receiver is still generic.
            let application = self.resolve_family_implementations(
                self.impls_by_protocol.get_key_value(&selected.owner),
                &implementation.for_type, name, supplied, environment,
                Some((&selected.arguments, &selected)), None,
            )?;
            return application.map(|family| family.result).ok_or_else(|| {
                Text::from("method projection has no established ancestor implementation")
            });
        }
        let candidate = self.associated_declaration_application(
            &key,
            &implementation.protocol_args,
            &implementation.for_type,
            name,
            &mut Vec::new(),
        )?;
        let candidate_owns_assignment = candidate.is_some_and(|candidate| {
            candidate.owner == selected.owner
                && candidate.arguments.len() == selected.arguments.len()
                && candidate
                    .arguments
                    .iter()
                    .zip(&selected.arguments)
                    .all(|(a, b)| self.types_structurally_equal(a, b))
        });
        let (arguments, scope) = self.complete_associated_family_arguments_in(
            &selected.declaration,
            &implementation.for_type,
            &selected.arguments,
            supplied,
            environment,
        )?;
        if candidate_owns_assignment
            && let Some(assignment) = implementation.associated_types.get(name)
        {
            return assignment.instantiate(&arguments, &Substitution::new());
        }
        if let Some(default) = &selected.declaration.default {
            default.try_apply_subst(&scope)
        } else {
            Err(
                "method projection has no established associated assignment or declared default"
                    .into(),
            )
        }
    }
}
