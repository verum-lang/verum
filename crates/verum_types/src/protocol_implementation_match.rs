//! One rigid implementation matcher shared by proof obligations and family applications.
use super::{ProtocolChecker, ProtocolImpl, WhereClause};
use crate::{
    ty::{Substitution, Type},
    unify::Unifier,
};
use verum_ast::Span;

/// Checked lexical premises used by both candidate inference and final proof.
/// Candidate-owned variables may be inferred; caller declarations remain rigid.
pub(super) struct ConstraintEnvironment<'a> {
    pub protocols: &'a [WhereClause],
    pub callables: &'a verum_common::Map<crate::ty::TypeVar, Type>,
}

impl ConstraintEnvironment<'_> {
    /// Reachable premise owners carry their whole declared contracts. Follow
    /// protocol subjects as well as callable outputs until neither adds a
    /// variable; no variable is introduced by an implementation requirement.
    fn extend_variables(&self, variables: &mut verum_common::Set<crate::ty::TypeVar>) {
        loop {
            let before = variables.len();
            extend_callable_variables(variables, self.callables);
            for clause in self.protocols {
                if !clause.ty.free_vars().iter().all(|variable| variables.contains(variable)) { continue; }
                for bound in &clause.bounds {
                    if bound.is_negative { continue; }
                    for argument in bound.args.iter().chain(bound.associated_types.values()) {
                        variables.extend(argument.free_vars());
                    }
                }
            }
            if before == variables.len() { break; }
        }
    }
}

impl ProtocolChecker {
    pub(crate) fn instantiate_applicable_implementation(
        &self,
        implementation: &ProtocolImpl,
        ty: &Type,
        args: Option<&[Type]>,
    ) -> Option<Substitution> {
        let CandidateMatch::Matched(substitution) =
            self.match_implementation_candidate(implementation, ty, args, true)
        else {
            return None;
        };
        self.check_implementation_callable_bounds(implementation, &substitution)
            .ok()?;
        for clause in &implementation.where_clauses {
            let subject = clause.ty.try_apply_subst(&substitution).ok()?;
            let bounds = clause
                .bounds
                .iter()
                .map(|bound| bound.try_apply_subst(&substitution))
                .collect::<Result<verum_common::List<_>, _>>()
                .ok()?;
            self.check_bounds(&subject, &bounds).ok()?;
        }
        Some(substitution)
    }

    pub(super) fn match_implementation_candidate(
        &self,
        implementation: &ProtocolImpl,
        ty: &Type,
        args: Option<&[Type]>,
        rigid: bool,
    ) -> CandidateMatch {
        self.match_implementation_candidate_in_environment(implementation, ty, args, rigid, &ConstraintEnvironment { protocols: &[], callables: &Default::default() })
    }

    pub(super) fn match_implementation_candidate_in_environment(
        &self,
        implementation: &ProtocolImpl,
        ty: &Type,
        args: Option<&[Type]>,
        rigid: bool,
        environment: &ConstraintEnvironment<'_>,
    ) -> CandidateMatch {
        // A declaration parameter is rigid until its own telescope instantiates
        // it. An inference hole in a declared head has no such authority. A
        // proof query likewise cannot choose its missing value from a candidate.
        if implementation.for_type.has_value_inference()
            || implementation
                .protocol_args
                .iter()
                .any(Type::has_value_inference)
            || (rigid
                && (ty.has_value_inference()
                    || args.is_some_and(|args| args.iter().any(Type::has_value_inference))))
        {
            return CandidateMatch::Unknown;
        }
        if args.is_some_and(|args| implementation.protocol_args.len() != args.len())
            || provably_disjoint(self, &implementation.for_type, ty)
            || args.is_some_and(|args| {
                implementation
                    .protocol_args
                    .iter()
                    .zip(args)
                    .any(|(a, b)| provably_disjoint(self, a, b))
            })
        {
            return CandidateMatch::Disjoint;
        }
        match self.try_match_candidate(implementation, ty, args, rigid, environment) {
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
        environment: &ConstraintEnvironment<'_>,
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
        for (variable, bound) in &implementation.type_param_fn_bounds {
            variables.insert(*variable);
            variables.extend(bound.free_vars());
        }
        let fresh = crate::context::instantiate_binders(&implementation.parameters, &[]).ok()?;
        // Free variables absent from the declared telescope are rigid captures,
        // never implicit quantifiers. Only owned binders receive fresh holes.
        variables.retain(|variable| !fresh.contains_key(variable));
        if rigid {
            variables.extend(ty.free_vars());
            for argument in args.into_iter().flatten() {
                variables.extend(argument.free_vars());
            }
        }
        if rigid {
            for clause in environment.protocols {
                variables.extend(clause.ty.free_vars());
                for bound in &clause.bounds {
                    for argument in bound.args.iter().chain(bound.associated_types.values()) {
                        variables.extend(argument.free_vars());
                    }
                }
            }
            environment.extend_variables(&mut variables);
        }
        let mut matcher = Unifier::new();
        matcher.enter_rigid_scope(variables.into_iter().map(|variable| {
            (
                variable,
                format!("parameter{}", variable.id()).into(),
                false,
                None,
            )
        }));
        let pattern = implementation.for_type.try_apply_subst(&fresh).ok()?;
        if !self.types_structurally_equal(&pattern, ty) {
            matcher.unify(&pattern, ty, Span::default()).ok()?;
        }
        for (pattern, actual) in implementation
            .protocol_args
            .iter()
            .zip(args.into_iter().flatten())
        {
            let pattern = pattern.try_apply_subst(&fresh).ok()?;
            if !self.types_structurally_equal(&pattern, actual) {
                matcher.unify(&pattern, actual, Span::default()).ok()?;
            }
        }
        // Callable and associated equations share a worklist: either can
        // determine an owner needed by the other. Only actual premises supply
        // equations; the final proof still checks every implementation bound.
        let fresh_type_holes: verum_common::Set<_> =
            fresh.values().flat_map(Type::free_vars).collect();
        let mut pending_callables: Vec<_> = implementation.type_param_fn_bounds.iter().collect();
        let mut pending_equalities = Vec::new();
        for clause in &implementation.where_clauses {
            let subject = clause.ty.try_apply_subst(&fresh).ok()?;
            for bound in &clause.bounds {
                if bound.is_negative { continue; }
                let bound = bound.try_apply_subst(&fresh).ok()?;
                for name in bound.associated_types.keys() {
                    pending_equalities.push((subject.clone(), bound.clone(), name.clone()));
                }
            }
        }
        loop {
            let before = pending_callables.len() + pending_equalities.len();
            let mut blocked = Vec::new();
            for (owner, bound) in pending_callables {
                let actual = matcher.try_apply(
                    &Type::Var(*owner).try_apply_subst(&fresh).ok()?, Span::default(),
                ).ok()?;
                let Some(actual) = callable_contract(&actual, environment.callables) else {
                    blocked.push((owner, bound));
                    continue;
                };
                let required = bound.try_apply_subst(&fresh).ok()?;
                let (
                    Type::Function { params: actual_params, return_type: actual_result, .. },
                    Type::Function { params: required_params, return_type: required_result, .. },
                ) = (actual, &required) else { return None; };
                if actual_params.len() != required_params.len() { return None; }
                for (required, actual) in required_params.iter().zip(actual_params)
                    .chain(std::iter::once((required_result.as_ref(), actual_result.as_ref())))
                {
                    matcher.unify(required, actual, Span::default()).ok()?;
                }
            }
            pending_callables = blocked;
            let mut blocked = Vec::new();
            for (subject, bound, name) in pending_equalities {
                let actual_subject = matcher.try_apply(&subject, Span::default()).ok()?;
                if actual_subject.free_vars().iter().any(|var| fresh_type_holes.contains(var)) {
                    blocked.push((subject, bound, name));
                    continue;
                }
                let required = bound.try_apply_subst(matcher.get_substitution()).ok()?;
                let equations = self.associated_premise_equations(
                    &actual_subject, &required, &name, environment.protocols,
                ).ok()?;
                if equations.is_empty() {
                    blocked.push((subject, bound, name));
                    continue;
                }
                for (required, actual) in equations {
                    matcher.unify(&required, &actual, Span::default()).ok()?;
                    if !self.types_structurally_equal(
                        &matcher.try_apply(&required, Span::default()).ok()?,
                        &matcher.try_apply(&actual, Span::default()).ok()?,
                    ) { return None; }
                }
            }
            pending_equalities = blocked;
            if pending_callables.len() + pending_equalities.len() == before { break; }
        }
        if !pending_callables.is_empty() { return None; }
        // Each value domain belongs to the declaration, not to the inferred
        // argument. Keep it while applying the selected substitution.
        for parameter in &implementation.parameters {
            let crate::context::GenericBinder::Value(parameter) = parameter else {
                continue;
            };
            let mut required = parameter.try_apply_subst(&fresh).ok()?;
            let actual = matcher.try_apply(&required, Span::default()).ok()?;
            let Type::Meta { ty, .. } = &mut required else {
                return None;
            };
            **ty = matcher.try_apply(ty, Span::default()).ok()?;
            if !crate::value_parameter_domain::argument_satisfies_domain(
                &required,
                &actual,
                Type::clone,
            ) {
                return None;
            }
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
            let argument = matcher.try_apply(fresh_variable, Span::default()).ok()?;
            if argument
                .free_vars()
                .iter()
                .any(|variable| fresh_type_holes.contains(variable))
            {
                // The head and declared premise equations did not determine
                // this parameter. It cannot escape as an arbitrary output type.
                return None;
            }
            substitution.insert(*variable, argument);
        }
        for (variable, fresh_variable) in &fresh.value_bindings {
            substitution.value_bindings.insert(
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

fn provably_disjoint(checker: &ProtocolChecker, a: &Type, b: &Type) -> bool {
    if checker.types_structurally_equal(a, b) {
        return false;
    }
    match (a, b) {
        (Type::Named { path: a, args: aa }, Type::Named { path: b, args: ba }) => {
            !checker.paths_equal(a, b)
                || aa.len() != ba.len()
                || aa
                    .iter()
                    .zip(ba)
                    .any(|(a, b)| provably_disjoint(checker, a, b))
        }

        (Type::Generic { name: a, args: aa }, Type::Generic { name: b, args: ba })
            if !a.starts_with("::") && !b.starts_with("::") =>
        {
            a != b
                || aa.len() != ba.len()
                || aa
                    .iter()
                    .zip(ba)
                    .any(|(a, b)| provably_disjoint(checker, a, b))
        }
        (Type::Tuple(a), Type::Tuple(b)) => {
            a.len() != b.len()
                || a.iter()
                    .zip(b)
                    .any(|(a, b)| provably_disjoint(checker, a, b))
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
        ) => a != b || provably_disjoint(checker, ai, bi),
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
        self.check_implementation_callable_bounds_in_environment(implementation, substitution, &ConstraintEnvironment { protocols: &[], callables: &Default::default() })
    }

    pub(super) fn check_implementation_callable_bounds_in_environment(
        &self,
        implementation: &ProtocolImpl,
        substitution: &Substitution,
        environment: &ConstraintEnvironment<'_>,
    ) -> Result<(), verum_common::Text> {
        let mut permitted = implementation
            .for_type
            .try_apply_subst(substitution)?
            .free_vars();
        for argument in &implementation.protocol_args {
            permitted.extend(argument.try_apply_subst(substitution)?.free_vars());
        }
        environment.extend_variables(&mut permitted);
        for (variable, requirement) in &implementation.type_param_fn_bounds {
            let actual = Type::Var(*variable).try_apply_subst(substitution)?;
            let actual = callable_contract(&actual, environment.callables).ok_or_else(||
                verum_common::Text::from("callable implementation condition has no declared function evidence"))?;
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
            ) = (actual, &required)
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
            if !crate::subtype::Subtyping::new().is_subtype(actual, &required) {
                return Err(
                    "callable implementation condition is not established by its actual contract"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

/// A callable premise describes operations on its owner, not equality of the
/// owner's representation with a function value. Resolve only an exact bound
/// variable, without specializing the variable or following spelling aliases.
fn callable_contract<'a>(actual: &'a Type, premises: &'a verum_common::Map<crate::ty::TypeVar, Type>) -> Option<&'a Type> {
    match actual {
        Type::Function { .. } => Some(actual),
        Type::Var(variable) => premises.get(variable).filter(|ty| matches!(ty, Type::Function { .. })),
        _ => None,
    }
}

/// Follow only contracts owned by reachable declaration variables. The visited
/// set bounds cyclic/higher-order premises and preserves every caller variable
/// as rigid evidence, including outputs absent from the nominal receiver head.
fn extend_callable_variables(
    variables: &mut verum_common::Set<crate::ty::TypeVar>,
    premises: &verum_common::Map<crate::ty::TypeVar, Type>,
) {
    let mut pending: Vec<_> = variables.iter().copied().collect();
    while let Some(variable) = pending.pop() {
        if let Some(contract) = premises.get(&variable) {
            for dependency in contract.free_vars() {
                if variables.insert(dependency) { pending.push(dependency); }
            }
        }
    }
}
