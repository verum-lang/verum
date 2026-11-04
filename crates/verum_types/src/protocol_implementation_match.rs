//! One rigid implementation matcher shared by proof obligations and family applications.
use super::{ProtocolChecker, ProtocolImpl};
use crate::{
    ty::{Substitution, Type, TypeVar},
    unify::Unifier,
};
use verum_ast::Span;

impl ProtocolChecker {
    pub(super) fn match_implementation_candidate(
        &self,
        implementation: &ProtocolImpl,
        ty: &Type,
        args: Option<&[Type]>,
        rigid: bool,
    ) -> CandidateMatch {
        if args.is_some_and(|args| implementation.protocol_args.len() != args.len())
            || provably_disjoint(&implementation.for_type, ty)
            || args.is_some_and(|args| {
                implementation
                    .protocol_args
                    .iter()
                    .zip(args)
                    .any(|(a, b)| provably_disjoint(a, b))
            })
        {
            return CandidateMatch::Disjoint;
        }
        match self.try_match_candidate(implementation, ty, args, rigid) {
            Some(substitution) => CandidateMatch::Matched(substitution),
            None => CandidateMatch::Unknown,
        }
    }
    fn try_match_candidate(
        &self,
        implementation: &ProtocolImpl,
        ty: &Type,
        args: Option<&[Type]>,
        rigid: bool,
    ) -> Option<Substitution> {
        let mut variables = implementation.for_type.free_vars();
        for ty in &implementation.protocol_args {
            variables.extend(ty.free_vars());
        }
        for ty in implementation.associated_types.values() {
            variables.extend(ty.free_owner_variables());
        }
        for clause in &implementation.where_clauses {
            variables.extend(clause.ty.free_vars());
            for bound in &clause.bounds {
                for ty in bound.args.iter().chain(bound.associated_types.values()) {
                    variables.extend(ty.free_vars());
                }
            }
        }
        let fresh: Substitution = variables
            .into_iter()
            .map(|variable| (variable, Type::Var(TypeVar::fresh())))
            .collect();
        let mut matcher = Unifier::new();
        if rigid {
            let mut variables = ty.free_vars();
            for arg in args.into_iter().flatten() {
                variables.extend(arg.free_vars());
            }
            matcher.enter_rigid_scope(
                variables
                    .into_iter()
                    .map(|var| (var, format!("parameter{}", var.id()).into(), false, None)),
            );
        }
        let pattern = implementation.for_type.try_apply_subst(&fresh).ok()?;
        matcher.unify(&pattern, ty, Span::default()).ok()?;
        for (pattern, actual) in implementation
            .protocol_args
            .iter()
            .zip(args.into_iter().flatten())
        {
            matcher
                .unify(
                    &pattern.try_apply_subst(&fresh).ok()?,
                    actual,
                    Span::default(),
                )
                .ok()?;
        }
        // Unification also serves inference/coercion. A proof match additionally
        // requires exact equality after substitution, so coercions are not evidence.
        if !self.types_structurally_equal(
            &matcher.try_apply(&pattern, Span::default()).ok()?,
            &matcher.try_apply(ty, Span::default()).ok()?,
        ) {
            return None;
        }
        for (pattern, actual) in implementation
            .protocol_args
            .iter()
            .zip(args.into_iter().flatten())
        {
            if !self.types_structurally_equal(
                &matcher
                    .try_apply(&pattern.try_apply_subst(&fresh).ok()?, Span::default())
                    .ok()?,
                &matcher.try_apply(actual, Span::default()).ok()?,
            ) {
                return None;
            }
        }
        let mut substitution = matcher.get_substitution().clone();
        for (variable, fresh_variable) in &fresh {
            substitution.insert(
                *variable,
                matcher.try_apply(fresh_variable, Span::default()).ok()?,
            );
        }
        Some(substitution)
    }
}

/// Failure to elaborate a match is not evidence that two domains are disjoint.
pub(super) enum CandidateMatch {
    Matched(Substitution),
    Disjoint,
    Unknown,
}

fn provably_disjoint(a: &Type, b: &Type) -> bool {
    match (a, b) {
        (Type::Named { path: a, args: aa }, Type::Named { path: b, args: ba }) => {
            let segments = |path: &verum_ast::Path| {
                path.segments
                    .iter()
                    .map(|segment| match segment {
                        verum_ast::ty::PathSegment::Name(name) => Some(name.name.clone()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            };
            match (segments(a), segments(b)) {
                (Some(a), Some(b)) if a != b => true,
                _ => {
                    aa.len() != ba.len() || aa.iter().zip(ba).any(|(a, b)| provably_disjoint(a, b))
                }
            }
        }
        (Type::Generic { name: a, args: aa }, Type::Generic { name: b, args: ba })
            if !a.starts_with("::") && !b.starts_with("::") =>
        {
            a != b
                || aa.len() != ba.len()
                || aa.iter().zip(ba).any(|(a, b)| provably_disjoint(a, b))
        }
        (Type::Tuple(a), Type::Tuple(b)) => {
            a.len() != b.len() || a.iter().zip(b).any(|(a, b)| provably_disjoint(a, b))
        }
        (
            Type::Reference {
                mutable: a,
                inner: ai,
            },
            Type::Reference {
                mutable: b,
                inner: bi,
            },
        ) => a != b || provably_disjoint(ai, bi),
        (
            Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char | Type::Text,
            Type::Unit | Type::Bool | Type::Int | Type::Float | Type::Char | Type::Text,
        ) => a != b,
        _ => false,
    }
}

impl ProtocolChecker {
    /// Head matching does not prove an implementation's callable conditions.
    /// This judgment uses resolved contracts; it never infers a missing owner.
    pub(super) fn check_implementation_callable_bounds(
        &self,
        implementation: &ProtocolImpl,
        substitution: &Substitution,
    ) -> Result<(), verum_common::Text> {
        let mut permitted = implementation
            .for_type
            .try_apply_subst(substitution)?
            .free_vars();
        for argument in &implementation.protocol_args {
            permitted.extend(argument.try_apply_subst(substitution)?.free_vars());
        }
        for (variable, requirement) in &implementation.type_param_fn_bounds {
            let actual = Type::Var(*variable).try_apply_subst(substitution)?;
            let required = requirement.try_apply_subst(substitution)?;
            if !matches!(actual, Type::Function { .. })
                || !matches!(required, Type::Function { .. })
            {
                return Err(
                    "callable implementation condition lacks a resolved function contract".into(),
                );
            }
            if actual
                .free_vars()
                .iter()
                .chain(required.free_vars().iter())
                .any(|variable| !permitted.contains(variable))
            {
                return Err(
                    "callable implementation condition contains an undetermined owner variable"
                        .into(),
                );
            }
            let (
                Type::Function {
                    params: actual_params,
                    return_type: actual_result,
                    ..
                },
                Type::Function {
                    params: required_params,
                    return_type: required_result,
                    ..
                },
            ) = (&actual, &required)
            else {
                unreachable!()
            };
            // Value coercions (for example taking a temporary reference) cannot
            // establish a callable contract without an explicit callable adapter.
            if actual_params.len() != required_params.len()
                || !actual_params
                    .iter()
                    .zip(required_params)
                    .all(|(actual, required)| self.types_structurally_equal(actual, required))
                || !self.types_structurally_equal(actual_result, required_result)
            {
                return Err(
                    "callable implementation condition requires an unproved value conversion"
                        .into(),
                );
            }
            if !crate::subtype::Subtyping::new().is_subtype(&actual, &required) {
                return Err(
                    "callable implementation condition is not established by its actual contract"
                        .into(),
                );
            }
        }
        Ok(())
    }
}
