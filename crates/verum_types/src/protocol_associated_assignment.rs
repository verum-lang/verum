//! Implementation-owned associated type expressions. A family's local binders
//! belong to its definition, independently of later protocol elaboration.
use crate::{
    context::GenericBinder,
    ty::{Substitution, Type, TypeVar, ValueId, ValueTerm},
};
use verum_common::{List, Set, Text};

#[derive(Debug, Clone, PartialEq)]
pub struct AssociatedTypeAssignment {
    parameters: List<GenericBinder>,
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
        Self::mixed_family(
            parameters.into_iter().map(GenericBinder::Type).collect(),
            body,
        )
    }

    pub fn mixed_family(parameters: List<GenericBinder>, body: Type) -> Result<Self, Text> {
        let mut types = Set::new();
        let mut values = Set::new();
        for parameter in &parameters {
            let unique = match parameter {
                GenericBinder::Type(variable) => types.insert(*variable),
                GenericBinder::Value(Type::Meta {
                    value: ValueTerm::Parameter(id),
                    ..
                }) => values.insert(*id),
                _ => return Err("associated family requires an owned type or value binder".into()),
            };
            if !unique {
                return Err("associated family repeats a local declaration binder".into());
            }
        }
        Ok(Self { parameters, body })
    }

    pub fn parameters(&self) -> &[GenericBinder] {
        self.parameters.as_slice()
    }

    /// Legacy zero-argument consumers cannot observe an unapplied family body.
    pub fn ordinary_body(&self) -> Option<&Type> {
        self.parameters.is_empty().then_some(&self.body)
    }

    pub fn free_owner_variables(&self) -> Set<TypeVar> {
        let mut variables = self.body.free_vars();
        for parameter in &self.parameters {
            if let GenericBinder::Value(value) = parameter {
                variables.extend(value.free_vars());
            }
        }
        for parameter in &self.parameters {
            if let GenericBinder::Type(variable) = parameter {
                variables.remove(variable);
            }
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
        for parameter in &self.parameters {
            match parameter {
                GenericBinder::Type(variable) => {
                    substitution.shift_remove(variable);
                }
                GenericBinder::Value(Type::Meta {
                    value: ValueTerm::Parameter(id),
                    ..
                }) => {
                    substitution.value_bindings.shift_remove(id);
                }
                _ => return Err("associated family requires an owned type or value binder".into()),
            }
        }
        for (parameter, argument) in self.parameters.iter().zip(arguments) {
            parameter.bind_argument( argument, &mut substitution)?;
        }
        self.body.try_apply_subst(&substitution)
    }

    /// Transport only owner substitutions; local parameters remain bound here.
    pub fn try_apply_owner_subst(&self, owners: &Substitution) -> Result<Self, Text> {
        if owners.is_empty() {
            return Ok(self.clone());
        }
        let mut scoped = owners.clone();
        let mut types = List::new();
        let mut free_values = self.body.value_ids();
        for parameter in &self.parameters {
            if let GenericBinder::Value(value) = parameter {
                free_values.extend(value.value_ids());
            }
        }
        for parameter in &self.parameters {
            match parameter {
                GenericBinder::Type(variable) => {
                    scoped.shift_remove(variable);
                    types.push(*variable);
                }
                GenericBinder::Value(Type::Meta {
                    value: ValueTerm::Parameter(id),
                    ..
                }) => {
                    scoped.value_bindings.shift_remove(id);
                    free_values.remove(id);
                }
                _ => return Err("associated family requires an owned type or value binder".into()),
            }
        }
        // As in Type::substitution_under_binders, only a range whose owner
        // actually occurs free in the body or domains can capture a local.
        let free_types = self.free_owner_variables();
        let mut range_values = Set::new();
        for (variable, replacement) in &scoped {
            if free_types.contains(variable) {
                range_values.extend(replacement.value_ids());
            }
        }
        for (id, replacement) in &scoped.value_bindings {
            if free_values.contains(id) {
                range_values.extend(replacement.value_ids());
            }
        }
        let mut renaming = Substitution::new();
        let mut values = indexmap::IndexMap::new();
        for parameter in &self.parameters {
            if let GenericBinder::Value(Type::Meta {
                name,
                ty,
                refinement,
                value: ValueTerm::Parameter(id),
            }) = parameter
            {
                let renamed = Type::Meta {
                    name: name.clone(),
                    ty: ty.clone(),
                    refinement: refinement.clone(),
                    value: ValueTerm::Parameter(if range_values.contains(id) {
                        ValueId::fresh()
                    } else {
                        *id
                    }),
                };
                if range_values.contains(id) {
                    renaming.value_bindings.insert(*id, renamed.clone());
                }
                values.insert(*id, renamed);
            }
        }
        values = values
            .iter()
            .map(|(id, parameter)| {
                let Type::Meta {
                    name,
                    ty,
                    refinement,
                    value,
                } = parameter
                else {
                    unreachable!()
                };
                Ok((
                    *id,
                    Type::Meta {
                        name: name.clone(),
                        ty: Box::new(ty.try_apply_subst(&renaming)?),
                        refinement: refinement.clone(),
                        value: value.clone(),
                    },
                ))
            })
            .collect::<Result<indexmap::IndexMap<_, _>, Text>>()?;
        for (id, parameter) in &mut renaming.value_bindings {
            *parameter = values[id].clone();
        }
        // The shared Forall substitution freshens type locals if owner ranges
        // would capture them, consistently in both the body and value domains.
        let mut payload = List::from_single(self.body.try_apply_subst(&renaming)?);
        for parameter in &self.parameters {
            if let GenericBinder::Value(Type::Meta {
                value: ValueTerm::Parameter(id),
                ..
            }) = parameter
            {
                let Type::Meta { ty, .. } = &values[id] else {
                    unreachable!()
                };
                payload.push((**ty).clone());
            }
        }
        let Type::Forall { vars, body } = (Type::Forall {
            vars: types,
            body: Box::new(Type::Tuple(payload)),
        })
        .try_apply_subst(&scoped)?
        else {
            return Err("associated assignment substitution lost its binding telescope".into());
        };
        let Type::Tuple(payload) = *body else {
            return Err("associated assignment substitution lost its bound domains".into());
        };
        let mut payload = payload.into_iter();
        let body = payload
            .next()
            .ok_or_else(|| Text::from("associated assignment lost its body"))?;
        let mut types = vars.into_iter();
        let mut parameters = List::new();
        for parameter in &self.parameters {
            parameters.push(match parameter {
                GenericBinder::Type(_) => GenericBinder::Type(
                    types
                        .next()
                        .ok_or_else(|| Text::from("associated assignment lost its type binder"))?,
                ),
                GenericBinder::Value(Type::Meta {
                    value: ValueTerm::Parameter(id),
                    ..
                }) => {
                    let Type::Meta {
                        name,
                        refinement,
                        value,
                        ..
                    } = &values[id]
                    else {
                        unreachable!()
                    };
                    GenericBinder::Value(Type::Meta {
                        name: name.clone(),
                        refinement: refinement.clone(),
                        value: value.clone(),
                        ty: Box::new(payload.next().ok_or_else(|| {
                            Text::from("associated assignment lost its value domain")
                        })?),
                    })
                }
                _ => return Err("associated family requires an owned type or value binder".into()),
            });
        }
        Self::mixed_family(parameters, body)
    }
}

#[cfg(test)]
#[path = "../tests/unit/associated_type_assignment.rs"]
mod tests;
