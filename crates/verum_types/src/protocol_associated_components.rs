//! Select each projected component in its declaration before two parameters
//! can be instantiated with the same receiver. Component images are inserted
//! simultaneously, using the ordinary capture-avoiding type substitution.
use super::{
    ConstraintEnvironment, FamilyApplication, ProtocolBound, ProtocolChecker, ProtocolImpl,
};
use crate::protocol::obligations::PROTOCOL_OBLIGATION_BUDGET;
use crate::{
    context::GenericBinder,
    ty::{Substitution, Type, TypeVar},
};
use verum_common::{List, Map, Text};

#[derive(PartialEq)]
struct Projection {
    receiver: Type,
    owner: ProtocolBound,
    member: Text,
}

pub(super) struct ComponentPlan {
    skeleton: Type,
    pending: Vec<(TypeVar, Projection)>,
    pub(super) result_bounds: List<ProtocolBound>,
}

impl ComponentPlan {
    pub(super) fn agrees_with(&self, other: &Self, checker: &ProtocolChecker) -> bool {
        if self.pending.len() != other.pending.len() {
            return false;
        }
        let mut slots = Substitution::new();
        for ((left, a), (right, b)) in self.pending.iter().zip(&other.pending) {
            if a != b {
                return false;
            }
            slots.insert(*right, Type::Var(*left));
        }
        other
            .skeleton
            .try_apply_subst(&slots)
            .is_ok_and(|skeleton| checker.types_structurally_equal(&self.skeleton, &skeleton))
    }
}

impl ProtocolChecker {
    pub(super) fn family_component_plan(
        &self,
        body: &Type,
        scope: &Substitution,
        implementation: &ProtocolImpl,
    ) -> Result<Option<ComponentPlan>, Text> {
        if implementation.where_clauses.is_empty()
            || !body.contains_type(|ty| ty.ordinary_projection().is_some())
        {
            return Ok(None);
        }
        let mut occupied = body.substitution_variables()?;
        occupied.extend(scope.keys().copied());
        for image in scope.values().chain(scope.value_bindings.values()) {
            occupied.extend(image.substitution_variables()?);
        }
        let mut fresh = || loop {
            let variable = TypeVar::fresh();
            if occupied.insert(variable) {
                break variable;
            }
        };
        let mut marking = Substitution::new();
        let mut parameter_slots = Map::new();
        let mut application = scope.clone();
        for parameter in &implementation.parameters {
            let GenericBinder::Type(parameter) = parameter else {
                continue;
            };
            let original = Type::Var(*parameter);
            if !implementation
                .where_clauses
                .iter()
                .any(|clause| clause.ty == original)
            {
                continue;
            }
            let marker = fresh();
            marking.insert(*parameter, Type::Var(marker));
            parameter_slots.insert(marker, *parameter);
            application.insert(marker, scope.get(parameter).cloned().unwrap_or(original));
        }
        if parameter_slots.is_empty() {
            return Ok(None);
        }
        // Mark free parameters using substitution so an inner quantifier
        // rebinding the same variable cannot authorize a projected component.
        let mut skeleton = body.try_apply_subst(&marking)?;
        let mut pending = Vec::new();
        skeleton.try_replace_types(|node| {
            let Some((Type::Var(marker), name)) = node.ordinary_projection() else {
                return Ok(None);
            };
            let Some(parameter) = parameter_slots.get(marker) else {
                return Ok(None);
            };
            let member = Text::from(name);
            let Some(owner) = self.associated_projection_owner(
                &Type::Var(*parameter),
                &member,
                &implementation.where_clauses,
            )?
            else {
                return Ok(None);
            };
            let slot = fresh();
            let receiver = application[marker].clone();
            pending.push((
                slot,
                Projection {
                    receiver,
                    owner: owner.try_apply_subst(scope)?,
                    member,
                },
            ));
            Ok::<_, Text>(Some(Type::Var(slot)))
        })?;
        if pending.is_empty() {
            return Ok(None);
        }
        Ok(Some(ComponentPlan {
            skeleton: skeleton.try_apply_subst(&application)?,
            pending,
            result_bounds: List::new(),
        }))
    }

    pub(super) fn complete_family_components(
        &self,
        application: FamilyApplication,
        environment: ConstraintEnvironment<'_>,
    ) -> Result<FamilyApplication, Text> {
        if application.components.is_none() {
            return Ok(application);
        }
        struct Frame {
            application: FamilyApplication,
            selections: Substitution,
            parent: Option<(TypeVar, Projection)>,
        }
        let mut stack = vec![Frame {
            application,
            selections: Substitution::new(),
            parent: None,
        }];
        let mut remaining = PROTOCOL_OBLIGATION_BUDGET;
        loop {
            let frame = stack.last_mut().expect("active family application");
            if let Some((slot, selection)) = frame
                .application
                .components
                .as_mut()
                .and_then(|plan| plan.pending.pop())
            {
                remaining = remaining.checked_sub(1).ok_or_else(|| {
                    Text::from("associated component resolution budget exhausted")
                })?;
                if stack.iter().any(|frame| {
                    frame
                        .parent
                        .as_ref()
                        .is_some_and(|(_, active)| active == &selection)
                }) {
                    return Err(format!(
                        "recursive associated component has no finite result: {}.{} on {}",
                        selection.owner.protocol, selection.member, selection.receiver
                    )
                    .into());
                }
                let application = self
                    .select_owned_family(
                        &selection.receiver,
                        &selection.owner,
                        &selection.member,
                        &[],
                        environment,
                    )?
                    .ok_or_else(|| {
                        Text::from(format!(
                            "cannot establish projected component {}.{} on {}",
                            selection.owner.protocol, selection.member, selection.receiver
                        ))
                    })?;
                stack.push(Frame {
                    application,
                    selections: Substitution::new(),
                    parent: Some((slot, selection)),
                });
            } else {
                let Frame {
                    mut application,
                    selections,
                    parent,
                } = stack.pop().expect("completed family application");
                if let Some(plan) = application.components.take() {
                    application.result = plan.skeleton.try_apply_subst(&selections)?;
                    self.check_universal_bounds_in(
                        &application.result,
                        &plan.result_bounds,
                        environment,
                    )?;
                }
                if let Some((slot, selection)) = parent {
                    if application.result.ordinary_projection()
                        == Some((&selection.receiver, selection.member.as_str()))
                        && !self
                            .associated_projection_owner(
                                &selection.receiver,
                                &selection.member,
                                environment.protocols,
                            )?
                            .as_ref()
                            .is_some_and(|owner| {
                                self.same_associated_owner(owner, &selection.owner)
                            })
                    {
                        return Err(format!(
                            "cannot retain the declared projection {}.{} for {} in the caller",
                            selection.owner.protocol, selection.member, selection.receiver
                        )
                        .into());
                    }
                    stack
                        .last_mut()
                        .expect("parent family application")
                        .selections
                        .insert(slot, application.result);
                } else {
                    return Ok(application);
                }
            }
        }
    }
}
