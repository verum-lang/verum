//! Family application uses its declared local telescope, never RHS variable order.
use super::{
    AssociatedFamilyBinders, AssociatedType, ProtocolBound, ProtocolChecker, ProtocolImpl, WhereClause,
    implementation_match::CandidateMatch,
};
use crate::{
    advanced_protocols::AssociatedTypeKind,
    context::GenericBinder,
    ty::{Substitution, Type, ValueTerm},
};
use verum_common::{List, Text};

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
}

impl ProtocolChecker {
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
                    if !self.paths_equal(&previous.protocol, &candidate.protocol)
                        || previous.args != candidate.args
                    {
                        return Err("associated projection has competing declaration owners".into());
                    }
                } else { owner = Some(candidate); }
            }
        }
        Ok(owner)
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
        let key = self.make_protocol_key(&owner.protocol);
        let Some(declaration) = self.associated_declaration_application(
            &key, &owner.args, receiver, name, &mut Vec::new(),
        )? else { return Err("selected protocol does not declare this associated member".into()); };
        let Some((implementation, substitution)) = self.find_impl_matching(
            receiver, &owner.protocol, Some(&owner.args),
        ) else { return Ok(None); };
        self.instantiate_declared_family(
            implementation, &declaration.declaration, receiver, name,
            &declaration.arguments, arguments, &substitution, assumptions,
        ).map(Some)
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
        if let Some((_, _, application, _)) = symbolic {
            return Ok(Some(application));
        }
        let mut selected: Option<(Text, List<Type>, FamilyApplication)> = None;
        for (key, index) in &self.impls_by_protocol {
            if !self.has_associated_declaration(key, name, false, &mut Vec::new()) {
                continue;
            }
            for index in index.candidates(receiver, self.requires_unfiltered_type_candidates(receiver)) {
                let implementation = &self.impls[index];
                let substitution = match self.match_implementation_candidate(
                    implementation,
                    receiver,
                    None,
                    true,
                ) {
                    CandidateMatch::Disjoint => continue,
                    CandidateMatch::Unknown => return Err(
                        "cannot establish the receiver domain of a competing family implementation"
                            .into(),
                    ),
                    CandidateMatch::Matched(substitution) => substitution,
                };
                let owner_arguments: List<_> = implementation
                    .protocol_args
                    .iter()
                    .map(|argument| argument.try_apply_subst(&substitution))
                    .collect::<Result<_, _>>()?;
                let Some(selected_declaration) = self.associated_declaration_application_in_scope(
                    key,
                    &owner_arguments,
                    receiver,
                    name,
                    &mut Vec::new(),
                    header,
                )?
                else {
                    continue;
                };
                let owner_arguments = selected_declaration.arguments;
                let receiver_variables = receiver.free_vars();
                let receiver_values = receiver.value_ids();
                if owner_arguments.iter().any(|argument| {
                    argument
                        .free_vars()
                        .iter()
                        .any(|variable| !receiver_variables.contains(variable))
                        || argument.value_ids().iter().any(|id| !receiver_values.contains(id))
                        || argument.has_value_inference()
                }) {
                    return Err("family owner application is not determined by its receiver".into());
                }
                let application = self.instantiate_declared_family(
                    implementation,
                    &selected_declaration.declaration,
                    receiver,
                    name,
                    &owner_arguments,
                    arguments,
                    &substitution,
                    assumptions,
                )?;
                if let Some((prior_protocol, prior_owner, prior)) = &selected {
                    if prior_protocol != &selected_declaration.owner
                        || *prior_owner != owner_arguments
                        || prior.arguments != application.arguments
                        || !self.types_structurally_equal(&prior.result, &application.result)
                    {
                        return Err(
                            "associated family application has competing declaration bindings"
                                .into(),
                        );
                    }
                } else {
                    selected = Some((selected_declaration.owner, owner_arguments, application));
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
            AssociatedFamilyBinders::bind_argument_normalized(binder, argument, &mut declaration_scope,
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
            AssociatedFamilyBinders::bind_argument_normalized(binder, &argument, &mut declaration_scope,
                |ty| self.normalize_projection_type(ty))?;
            arguments.push(argument);
        }
        for (parameter, argument) in declaration.type_params.iter().zip(&arguments) {
            let bounds: List<_> = parameter
                .bounds
                .iter()
                .map(|bound| bound.try_apply_subst(&declaration_scope))
                .collect::<Result<_, _>>()?;
            self.check_universal_bounds(argument, &bounds, assumptions)?;
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
            self.check_universal_bounds(&arguments[position], &bounds, assumptions)?;
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
        assumptions: &[WhereClause],
    ) -> Result<FamilyApplication, Text> {
        let (arguments, declaration_scope) = self.complete_associated_family_arguments(
            declaration,
            receiver,
            owner_arguments,
            supplied,
            assumptions,
        )?;
        self.check_implementation_callable_bounds(implementation, implementation_scope)?;
        // Head matching is not evidence for conditional implementations. The
        // ordinary inference matcher may defer constraints; this boundary may not.
        for clause in &implementation.where_clauses {
            let subject = clause.ty.try_apply_subst(implementation_scope)?;
            let bounds: List<_> = clause
                .bounds
                .iter()
                .map(|bound| bound.try_apply_subst(implementation_scope))
                .collect::<Result<_, _>>()?;
            self.check_universal_bounds(&subject, &bounds, assumptions)?;
        }
        let protocol = ProtocolBound::positive(
            implementation.protocol.clone(),
            implementation
                .protocol_args
                .iter()
                .map(|argument| argument.try_apply_subst(implementation_scope))
                .collect::<Result<_, _>>()?,
        );
        self.check_universal_bounds(receiver, &[protocol], assumptions)?;
        let result = if let Some(body) = implementation.associated_types.get(name) {
            body.instantiate(&arguments, implementation_scope)?
        } else if let Some(body) = &declaration.default {
            body.try_apply_subst(&declaration_scope)?
        } else {
            return Err("associated family has no assignment or declared default".into());
        };
        let mut allowed = receiver.free_vars();
        let mut allowed_values = receiver.value_ids();
        for argument in owner_arguments.iter().chain(&arguments) {
            allowed.extend(argument.free_vars());
            allowed_values.extend(argument.value_ids());
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
        let bounds: List<_> = declaration
            .bounds
            .iter()
            .map(|bound| bound.try_apply_subst(&declaration_scope))
            .collect::<Result<_, _>>()?;
        self.check_universal_bounds(&result, &bounds, assumptions)?;
        Ok(FamilyApplication { arguments, result })
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
            AssociatedFamilyBinders::bind_argument_normalized(binder, argument, &mut scope,
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
            AssociatedFamilyBinders::bind_argument_normalized(implemented, &argument, &mut implementation_locals,
                |ty| self.normalize_projection_type(ty))?;
            AssociatedFamilyBinders::bind_argument_normalized(declared, &argument, &mut scope,
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
        let bounds: List<_> = declaration
            .bounds
            .iter()
            .map(|bound| bound.try_apply_subst(&scope))
            .collect::<Result<_, _>>()?;
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
                self_type,
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
            let mut scope: Substitution = parameters
                .iter()
                .zip(arguments)
                .map(|(variable, argument)| (*variable, argument.clone()))
                .collect();
            scope.insert(*self_type, receiver.clone());
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
                let substitution=match self.match_implementation_candidate(implementation,receiver,None,true) {
                    CandidateMatch::Disjoint=>continue,
                    CandidateMatch::Unknown=>return Err("cannot determine omitted family arguments for an overlapping implementation".into()),
                    CandidateMatch::Matched(substitution)=>substitution,
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
        let candidate = self.associated_declaration_application(
            &key,
            &implementation.protocol_args,
            &implementation.for_type,
            name,
            &mut Vec::new(),
        )?;
        let selected = self
            .associated_declaration_application(
                method_owner,
                method_arguments,
                &implementation.for_type,
                name,
                &mut Vec::new(),
            )?
            .ok_or_else(|| Text::from("method projection is absent from its declaring protocol"))?;
        let candidate_owns_assignment = candidate.is_some_and(|candidate| {
            candidate.owner == selected.owner
                && candidate.arguments.len() == selected.arguments.len()
                && candidate
                    .arguments
                    .iter()
                    .zip(&selected.arguments)
                    .all(|(a, b)| self.types_structurally_equal(a, b))
        });
        let (arguments, scope) = self.complete_associated_family_arguments(
            &selected.declaration,
            &implementation.for_type,
            &selected.arguments,
            supplied,
            assumptions,
        )?;
        if candidate_owns_assignment
            && let Some(assignment) = implementation.associated_types.get(name)
        {
            return assignment.instantiate(&arguments, &Substitution::new());
        }
        if selected.owner != key {
            // An inherited method uses only an established implementation of its
            // exact ancestor application. A child may declare a different C.
            let span = implementation.span;
            let path = verum_ast::Path::new(
                selected
                    .owner
                    .as_str()
                    .split('.')
                    .map(|name| verum_ast::ty::PathSegment::Name(verum_ast::Ident::new(name, span)))
                    .collect(),
                span,
            );
            if let Some((ancestor, substitution)) = self.find_impl_for_instantiation(
                &implementation.for_type,
                &path,
                &selected.arguments,
            ) {
                if let Some(assignment) = ancestor.associated_types.get(name) {
                    return assignment.instantiate(&arguments, &substitution);
                }
            }
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
