//! Implementation-owned associated type expressions. A family's local binders
//! belong to its definition, independently of later protocol elaboration.
use crate::ty::{Substitution, Type, TypeVar};
use verum_common::{List, Set, Text};

#[derive(Debug, Clone, PartialEq)]
pub struct AssociatedTypeAssignment {
    parameters: List<TypeVar>,
    body: Type,
}

impl AssociatedTypeAssignment {
    pub fn ordinary(body: Type) -> Self {
        Self {
            parameters: List::new(),
            body,
        }
    }

    pub fn family(parameters: List<TypeVar>, body: Type) -> Result<Self, Text> {
        let unique: Set<_> = parameters.iter().copied().collect();
        if unique.len() != parameters.len() {
            return Err("associated family repeats a local declaration binder".into());
        }
        Ok(Self { parameters, body })
    }

    pub fn parameters(&self) -> &[TypeVar] {
        self.parameters.as_slice()
    }

    /// Legacy zero-argument consumers cannot observe an unapplied family body.
    pub fn ordinary_body(&self) -> Option<&Type> {
        self.parameters.is_empty().then_some(&self.body)
    }

    pub fn free_owner_variables(&self) -> Set<TypeVar> {
        let mut variables = self.body.free_vars();
        for parameter in &self.parameters {
            variables.remove(parameter);
        }
        variables
    }

    pub fn instantiate(&self, arguments: &[Type], owners: &Substitution) -> Result<Type, Text> {
        if arguments.len() != self.parameters.len() {
            return Err(
                "associated family arguments do not match the implementation telescope".into(),
            );
        }
        let mut substitution = owners.clone();
        for (parameter, argument) in self.parameters.iter().zip(arguments) {
            substitution.insert(*parameter, argument.clone());
        }
        self.body.try_apply_subst(&substitution)
    }

    /// Transport only owner substitutions; local parameters remain bound here.
    pub fn try_apply_owner_subst(&self, owners: &Substitution) -> Result<Self, Text> {
        let Type::Forall { vars, body } = (Type::Forall {
            vars: self.parameters.clone(),
            body: Box::new(self.body.clone()),
        })
        .try_apply_subst(owners)?
        else {
            return Err("associated assignment substitution lost its binding telescope".into());
        };
        Ok(Self {
            parameters: vars,
            body: *body,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_substitution_cannot_capture_a_family_local() {
        let owner = TypeVar::fresh();
        let local = TypeVar::fresh();
        let assignment = AssociatedTypeAssignment::family(
            List::from_iter([local]),
            Type::Tuple(List::from_iter([Type::Var(owner), Type::Var(local)])),
        )
        .unwrap();
        let mut substitution = Substitution::new();
        substitution.insert(owner, Type::Var(local));
        let transported = assignment.try_apply_owner_subst(&substitution).unwrap();
        assert_ne!(transported.parameters()[0], local);
        assert!(transported.free_owner_variables().contains(&local));
        assert_eq!(
            transported
                .instantiate(&[Type::bool()], &Substitution::new())
                .unwrap(),
            Type::Tuple(List::from_iter([Type::Var(local), Type::bool()]))
        );
    }
    #[test]
    fn family_body_requires_all_of_its_local_arguments() {
        let local = TypeVar::fresh();
        let family =
            AssociatedTypeAssignment::family(List::from_iter([local]), Type::Var(local)).unwrap();
        assert!(family.ordinary_body().is_none());
        assert!(family.instantiate(&[], &Substitution::new()).is_err());
        assert!(
            family
                .instantiate(&[Type::bool(), Type::bool()], &Substitution::new())
                .is_err()
        );
    }
}
