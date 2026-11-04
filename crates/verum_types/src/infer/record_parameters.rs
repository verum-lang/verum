//! Instantiate nominal record parameters from the selected declaration's roster.
use super::TypeChecker;
use crate::{
    Result, TypeError,
    context::TypeScheme,
    ty::{Type, TypeVar, ValueTerm},
};
use indexmap::IndexMap;
use verum_ast::Span;
use verum_common::{Heap, List, Text};

impl TypeChecker {
    /// Freshen only declared parameters, then constrain that one instance by
    /// the expected arguments. Fields and returned nominal arguments share it.
    pub(super) fn instantiate_nominal_record_fields(
        &mut self,
        owner: &str,
        fields: &IndexMap<Text, Type>,
        supplied: Option<&[Type]>,
        span: Span,
    ) -> Result<(IndexMap<Text, Type>, List<Type>)> {
        let key = format!("__type_params_{owner}");
        let Some(Type::Record(parameters)) = self.ctx.lookup_type(&key).cloned() else {
            if supplied.is_some_and(|arguments| !arguments.is_empty()) {
                return Err(TypeError::OtherWithCodeSpanned {
                    code: "E407".into(),
                    msg: "parameterized record has no declaration-owned parameter roster".into(),
                    span,
                });
            }
            return Ok((fields.clone(), List::new()));
        };
        if let Some(arguments) = supplied
            && arguments.len() != parameters.len()
        {
            return Err(TypeError::OtherWithCodeSpanned {
                code: "E407".into(),
                msg: "nominal record arguments do not match their declaration's parameter roster"
                    .into(),
                span,
            });
        }
        let mut scheme = TypeScheme::mono(Type::Unit);
        let mut arguments = List::new();
        let mut named_types = IndexMap::new();
        for (index, (name, parameter)) in parameters.iter().enumerate() {
            let argument = match parameter {
                Type::Meta {
                    value: ValueTerm::Parameter(_),
                    ..
                } => parameter.clone(),
                Type::Var(var) => {
                    scheme.vars.push(*var);
                    parameter.clone()
                }
                // Legacy ordinary record parameters have declared named slots.
                // They never supply a value identity or quantify foreign fields.
                Type::Named { args, .. } | Type::Generic { args, .. } if args.is_empty() => {
                    supplied
                        .map(|values| values[index].clone())
                        .unwrap_or_else(|| Type::Var(TypeVar::fresh()))
                }
                _ => {
                    return Err(TypeError::OtherWithCodeSpanned {
                        code: "E407".into(),
                        msg: "record parameter has no supported declaration-owned binder".into(),
                        span,
                    });
                }
            };
            if !matches!(parameter, Type::Meta { .. }) {
                named_types.insert(name.clone(), argument.clone());
            }
            arguments.push(argument);
        }
        for parameter in parameters.values() {
            if let Type::Meta {
                name,
                ty,
                refinement,
                value: ValueTerm::Parameter(id),
            } = parameter
            {
                scheme.value_parameters.push(Type::Meta {
                    name: name.clone(),
                    ty: Heap::new(self.substitute_type_params(ty, &named_types)),
                    refinement: refinement.clone(),
                    value: ValueTerm::Parameter(*id),
                });
            }
        }
        let fields = fields
            .iter()
            .map(|(name, ty)| (name.clone(), self.substitute_type_params(ty, &named_types)))
            .collect();
        scheme.ty = Type::Tuple(List::from_iter([
            Type::Record(fields),
            Type::Tuple(arguments),
        ]));
        let instance = scheme
            .try_instantiate()
            .map_err(|message| TypeError::UniverseBinding { message, span })?;
        self.retain_call_value_domains(instance.value_parameters, span)?;
        let Type::Tuple(parts) = instance.ty else {
            unreachable!("record instance tuple")
        };
        let Type::Tuple(arguments) = &parts[1] else {
            unreachable!("record argument tuple")
        };
        if let Some(supplied) = supplied {
            for (parameter, argument) in arguments.iter().zip(supplied) {
                self.unifier.unify(parameter, argument, span)?;
            }
        }
        let Type::Record(fields) = self.unifier.apply(&parts[0]) else {
            unreachable!("record fields")
        };
        let arguments = arguments.iter().map(|ty| self.unifier.apply(ty)).collect();
        Ok((fields, arguments))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/record_parameters.rs"]
mod tests;
