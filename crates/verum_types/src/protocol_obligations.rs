//! Universal protocol obligations use declared variables and explicit assumptions.
//!
//! Inference may postpone a constraint. This boundary may not: every conditional
//! implementation used here needs evidence, and possible specializations must
//! preserve the same associated equalities under their intersection.
use super::{ProtocolBound, ProtocolChecker, ProtocolConstraints, ProtocolImpl, WhereClause};
use crate::{
    ty::{Substitution, Type, TypeVar},
    unify::Unifier,
};
use verum_ast::Span;
use verum_common::{List, Text, ToText};

impl ProtocolChecker {
    pub fn check_universal_bounds(
        &self,
        ty: &Type,
        bounds: &[ProtocolBound],
        assumptions: &[WhereClause],
    ) -> Result<(), Text> {
        let mut context = Obligations {
            checker: self,
            assumptions,
            active: Vec::new(),
            remaining: 4096,
        };
        for bound in bounds {
            context.check(ty, bound)?;
        }
        Ok(())
    }
}

struct Obligations<'a> {
    checker: &'a ProtocolChecker,
    assumptions: &'a [WhereClause],
    active: Vec<(Type, ProtocolBound)>,
    remaining: usize,
}

impl Obligations<'_> {
    fn same(&self, a: &Type, b: &Type) -> bool {
        self.checker.types_structurally_equal(a, b)
    }
    fn entails(&self, actual: &ProtocolBound, expected: &ProtocolBound) -> bool {
        self.checker
            .paths_equal(&actual.protocol, &expected.protocol)
            && actual.is_negative == expected.is_negative
            && actual.args.len() == expected.args.len()
            && actual
                .args
                .iter()
                .zip(&expected.args)
                .all(|(a, b)| self.same(a, b))
            && expected.associated_types.iter().all(|(name, expected)| {
                actual
                    .associated_types
                    .get(name)
                    .is_some_and(|actual| self.same(actual, expected))
            })
    }
    fn assumption(&mut self, ty: &Type, expected: &ProtocolBound) -> Result<bool, Text> {
        let mut pending: Vec<_> = self
            .assumptions
            .iter()
            .flat_map(|clause| {
                clause
                    .bounds
                    .iter()
                    .map(|bound| (clause.ty.clone(), bound.clone()))
            })
            .collect();
        let mut seen: Vec<(Type, ProtocolBound)> = Vec::new();
        while let Some((subject, actual)) = pending.pop() {
            if self.same(&subject, ty) && self.entails(&actual, expected) {
                return Ok(true);
            }
            if actual.is_negative
                || seen.iter().any(|(prior, bound)| {
                    self.same(prior, &subject)
                        && self.entails(bound, &actual)
                        && self.entails(&actual, bound)
                })
            {
                continue;
            }
            if self.remaining == 0 {
                return Err("protocol assumption budget exhausted".into());
            }
            self.remaining -= 1;
            seen.push((subject.clone(), actual.clone()));
            let key = self.checker.make_protocol_key(&actual.protocol);
            let Some(protocol) = self.checker.get_protocol(&key) else {
                continue;
            };
            let ProtocolConstraints::Declared {
                self_type,
                parameters,
                parameter_bounds,
                ..
            } = &protocol.constraints
            else {
                continue;
            };
            if parameters.len() != actual.args.len() {
                continue;
            }
            let mut substitution: Substitution = parameters
                .iter()
                .zip(&actual.args)
                .map(|(var, ty)| (*var, ty.clone()))
                .collect();
            substitution.insert(*self_type, subject.clone());
            for inherited in &protocol.super_protocols {
                pending.push((subject.clone(), inherited.try_apply_subst(&substitution)?));
            }
            for clause in parameter_bounds {
                let subject = clause.ty.try_apply_subst(&substitution)?;
                for bound in &clause.bounds {
                    pending.push((subject.clone(), bound.try_apply_subst(&substitution)?));
                }
            }
        }
        Ok(false)
    }
    fn check(&mut self, ty: &Type, bound: &ProtocolBound) -> Result<(), Text> {
        if self.assumption(ty, bound)? {
            return Ok(());
        }
        if self.remaining == 0 || self.active.len() >= 64 {
            return Err("protocol obligation budget exhausted".into());
        }
        self.remaining -= 1;
        if self
            .active
            .iter()
            .any(|(prior_ty, prior_bound)| self.same(prior_ty, ty) && prior_bound == bound)
        {
            return Err("recursive protocol obligation has no independent evidence".into());
        }
        if bound.is_negative {
            return Err("negative universal bounds require an explicit negative assumption".into());
        }
        self.active.push((ty.clone(), bound.clone()));
        let result = self.check_implementations(ty, bound);
        self.active.pop();
        result
    }
    fn check_implementations(&mut self, ty: &Type, bound: &ProtocolBound) -> Result<(), Text> {
        let key = self.checker.make_protocol_key(&bound.protocol);
        let candidates = self
            .checker
            .impls_by_protocol
            .get(&key)
            .map(|index| index.candidates(ty))
            .unwrap_or_default();
        let mut covering = false;
        for index in candidates {
            let implementation = &self.checker.impls[index];
            let overlap = match self.match_candidate(implementation, ty, &bound.args, false) {
                CandidateMatch::Matched(substitution) => substitution,
                CandidateMatch::Disjoint => continue,
                CandidateMatch::Unknown => {
                    return Err(
                        "cannot establish disjointness of a competing implementation".into(),
                    );
                }
            };
            // A case that can become applicable after instantiation cannot be
            // discarded merely because its where clause is currently unknown.
            self.check_equalities(implementation, bound, &overlap)?;
            if let CandidateMatch::Matched(substitution) =
                self.match_candidate(implementation, ty, &bound.args, true)
            {
                let mut proved = true;
                for clause in &implementation.where_clauses {
                    let clause_ty = clause.ty.try_apply_subst(&substitution)?;
                    for required in &clause.bounds {
                        let required = required.try_apply_subst(&substitution)?;
                        if self.check(&clause_ty, &required).is_err() {
                            proved = false;
                            break;
                        }
                    }
                    if !proved {
                        break;
                    }
                }
                if proved {
                    covering = true;
                }
            }
        }
        if !covering {
            return Err(format!(
                "no implementation proves {}: {} under its declared assumptions",
                ty.to_text(),
                bound.protocol
            )
            .into());
        }
        let protocol = self
            .checker
            .get_protocol(&key)
            .ok_or_else(|| Text::from("protocol declaration is unavailable"))?;
        let ProtocolConstraints::Declared {
            self_type,
            parameters,
            parameter_bounds,
            ..
        } = &protocol.constraints
        else {
            return Err("protocol constraints lack their declaration-owned telescope".into());
        };
        if parameters.len() != bound.args.len() {
            return Err("protocol obligation has incomplete type arguments".into());
        }
        let mut substitution: Substitution = parameters
            .iter()
            .zip(&bound.args)
            .map(|(var, ty)| (*var, ty.clone()))
            .collect();
        substitution.insert(*self_type, ty.clone());
        for clause in parameter_bounds {
            let subject = clause.ty.try_apply_subst(&substitution)?;
            for required in &clause.bounds {
                self.check(&subject, &required.try_apply_subst(&substitution)?)?;
            }
        }
        for inherited in &protocol.super_protocols {
            self.check(ty, &inherited.try_apply_subst(&substitution)?)?;
        }
        Ok(())
    }
    fn check_equalities(
        &self,
        implementation: &ProtocolImpl,
        bound: &ProtocolBound,
        substitution: &Substitution,
    ) -> Result<(), Text> {
        for (name, expected) in &bound.associated_types {
            let actual = if let Some(actual) = implementation.associated_types.get(name) {
                actual.try_apply_subst(substitution)?
            } else {
                let key = self.checker.make_protocol_key(&bound.protocol);
                let protocol = self
                    .checker
                    .get_protocol(&key)
                    .ok_or_else(|| Text::from("protocol declaration is unavailable"))?;
                let ProtocolConstraints::Declared {
                    self_type,
                    parameters,
                    ..
                } = &protocol.constraints
                else {
                    return Err("associated default lacks its declaration-owned telescope".into());
                };
                let associated = protocol
                    .associated_types
                    .get(name)
                    .ok_or_else(|| Text::from("associated equality names an undeclared type"))?;
                if !associated.type_params.is_empty()
                    || parameters.len() != implementation.protocol_args.len()
                {
                    return Err("associated default requires complete declaration arguments".into());
                }
                let default = associated.default.as_ref().ok_or_else(|| {
                    Text::from(format!(
                        "implementation omits the associated type `{name}` required by an equality"
                    ))
                })?;
                let mut owner = Substitution::new();
                owner.insert(
                    *self_type,
                    implementation.for_type.try_apply_subst(substitution)?,
                );
                for (var, ty) in parameters.iter().zip(&implementation.protocol_args) {
                    owner.insert(*var, ty.try_apply_subst(substitution)?);
                }
                default.try_apply_subst(&owner)?
            };
            let expected = expected.try_apply_subst(substitution)?;
            if !self.same(&actual, &expected) {
                return Err(format!(
                    "potential implementation of {} sets {name} = {}, expected {}",
                    bound.protocol,
                    actual.to_text(),
                    expected.to_text()
                )
                .into());
            }
        }
        Ok(())
    }
    fn match_candidate(
        &self,
        implementation: &ProtocolImpl,
        ty: &Type,
        args: &[Type],
        rigid: bool,
    ) -> CandidateMatch {
        if implementation.protocol_args.len() != args.len()
            || provably_disjoint(&implementation.for_type, ty)
            || implementation
                .protocol_args
                .iter()
                .zip(args)
                .any(|(a, b)| provably_disjoint(a, b))
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
        args: &[Type],
        rigid: bool,
    ) -> Option<Substitution> {
        let mut variables = implementation.for_type.free_vars();
        for ty in &implementation.protocol_args {
            variables.extend(ty.free_vars());
        }
        for ty in implementation.associated_types.values() {
            variables.extend(ty.free_vars());
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
            for arg in args {
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
        for (pattern, actual) in implementation.protocol_args.iter().zip(args) {
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
        if !self.same(
            &matcher.try_apply(&pattern, Span::default()).ok()?,
            &matcher.try_apply(ty, Span::default()).ok()?,
        ) {
            return None;
        }
        for (pattern, actual) in implementation.protocol_args.iter().zip(args) {
            if !self.same(
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
enum CandidateMatch {
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
