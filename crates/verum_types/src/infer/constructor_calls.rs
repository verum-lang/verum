//! Selected data constructors share argument, ownership and callable evidence.
use super::{InferResult, TypeChecker};
use crate::{
    Result, Type, TypeError, computational_properties::PropertyRequirement, context::TypeScheme,
};
use verum_ast::{Expr, Span};
use verum_common::{List, Text};

impl TypeChecker {
    pub(super) fn sum_constructor_scheme(
        owner: &Text,
        parameters: &[Type],
        params: Option<List<Type>>,
        span: Span,
    ) -> Result<TypeScheme> {
        let result = Type::Named {
            path: Self::text_to_path(owner),
            args: parameters.iter().cloned().collect(),
        };
        let ty = match params {
            Some(params) => Type::function_with_requirement(
                params,
                result,
                PropertyRequirement::source_callable(false),
            ),
            None => result,
        };
        let mut scheme = TypeScheme::mono(ty);
        for parameter in parameters {
            match parameter {
                Type::Var(variable) => scheme.vars.push(*variable),
                value @ Type::Meta { .. } => scheme.value_parameters.push(value.clone()),
                _ => {
                    return Err(TypeError::OtherWithCodeSpanned {
                        code: "E407".into(),
                        msg: "sum constructor parameter lacks a declaration-owned identity".into(),
                        span,
                    });
                }
            }
        }
        Ok(scheme)
    }

    /// Value-position constructors retain the same nominal application and
    /// fresh parameter telescope as call-position constructors.
    pub(super) fn selected_constructor_value(
        &mut self,
        owner: &str,
        case: &str,
        span: Span,
    ) -> Result<Type> {
        let owner = self.canonical_alias_target(owner);
        let owner = self.declared_sum_key(&owner).unwrap_or(owner);
        let unavailable = || TypeError::OtherWithCodeSpanned {
            code: "E405".into(),
            msg: format!("constructor `{owner}.{case}` has no declaration-owned carrier").into(),
            span,
        };
        let (parameters, body) = self
            .unifier
            .declared_variant_template(&owner)
            .ok_or_else(unavailable)?;
        let Type::Variant(cases) = body else {
            return Err(unavailable());
        };
        let payload = cases.get(case).ok_or_else(unavailable)?;
        let params = match payload {
            Type::Unit => None,
            Type::Tuple(params) => Some(params.clone()),
            other => Some(List::from_single(other.clone())),
        };
        let constructor = Self::sum_constructor_scheme(&owner, parameters, params, span)?;
        self.instantiate_value_scheme(&constructor, span)
    }

    pub(super) fn check_constructor_call(
        &mut self,
        args: &[Expr],
        params: &[Type],
        result: &Type,
        span: Span,
    ) -> Result<InferResult> {
        if args.len() != params.len() {
            return Err(self.arity_error("constructor".into(), params.len(), args.len(), span));
        }
        self.with_call_argument_context(|checker| {
            for (arg, parameter) in args.iter().zip(params) {
                let parameter = checker.unifier.apply(parameter);
                let parameter = if Self::contains_type_app(&parameter) {
                    checker.normalize_type(&parameter)
                } else {
                    parameter
                };
                checker.check_expr(arg, &parameter)?;
            }
            Ok(())
        })?;
        self.consume_affine_call_args(args, params)?;
        let result = self.unifier.apply(result);
        let result = if Self::contains_type_app(&result) {
            self.normalize_type(&result)
        } else {
            result
        };
        // Constructing a selected data case invokes no user callable. Keep a
        // conservative synchronous bound; arguments retain their own properties
        // in the ordinary expression traversal (including async arguments).
        let target = Type::function_with_requirement(
            params
                .iter()
                .map(|ty| self.unifier.apply(ty))
                .collect::<List<_>>(),
            result.clone(),
            PropertyRequirement::source_callable(false),
        );
        self.type_registry.register_call_target(span, target);
        Ok(InferResult::method(
            result,
            Some(crate::core_metadata::ReceiverKind::None),
        ))
    }
}
