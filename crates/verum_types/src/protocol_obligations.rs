//! Universal protocol obligations use declared variables and explicit assumptions.
//!
//! Inference may postpone a constraint. This boundary may not: every conditional
//! implementation used here needs evidence, and possible specializations must
//! preserve the same associated equalities under their intersection.
use super::implementation_match::{CandidateMatch, ConstraintEnvironment};
use super::{ProtocolBound, ProtocolChecker, ProtocolConstraints, ProtocolImpl, WhereClause};
use crate::{
    ty::{Substitution, Type, TypeVar},
    unify::Unifier,
};
use verum_common::{List, Text, ToText};

impl ProtocolChecker {
    /// Discover declaration-owned prerequisites for source loading. This does
    /// not prove the goal: the ordinary universal checker still validates all
    /// candidates, equalities, callable bounds and overlapping specializations.
    pub(crate) fn source_bound_requirements(
        &self,
        ty: &Type,
        bound: &ProtocolBound,
        assumptions: &[WhereClause],
        callables: &verum_common::Map<TypeVar, Type>,
    ) -> Result<List<WhereClause>, Text> {
        let key = self.make_protocol_key(&bound.protocol);
        let mut requirements = List::new();
        if let Some(index) = self.impls_by_protocol.get(&key) {
            for index in index.candidates(ty) {
                let implementation = &self.impls[index];
                if let CandidateMatch::Matched(substitution) = self.match_implementation_candidate_in_environment(
                    implementation, ty, Some(&bound.args), true, &ConstraintEnvironment { protocols: assumptions, callables },
                ) {
                    for clause in &implementation.where_clauses {
                        requirements.push(clause.try_apply_subst(&substitution)?);
                    }
                }
            }
        }
        if let Some(protocol) = self.get_protocol(&key)
            && let ProtocolConstraints::Declared {
                self_type, parameters, parameter_bounds, ..
            } = &protocol.constraints
            && parameters.len() == bound.args.len()
        {
            let mut substitution: Substitution = parameters.iter().zip(&bound.args)
                .map(|(parameter, argument)| (*parameter, argument.clone())).collect();
            substitution.insert(*self_type, ty.clone());
            for clause in parameter_bounds {
                requirements.push(clause.try_apply_subst(&substitution)?);
            }
            if !protocol.super_protocols.is_empty() {
                requirements.push(WhereClause {
                    ty: ty.clone(),
                    bounds: protocol.super_protocols.iter()
                        .map(|bound| bound.try_apply_subst(&substitution))
                        .collect::<Result<List<_>, _>>()?,
                });
            }
        }
        Ok(requirements)
    }

    pub fn check_universal_bounds(
        &self,
        ty: &Type,
        bounds: &[ProtocolBound],
        assumptions: &[WhereClause],
    ) -> Result<(), Text> {
        self.check_universal_bounds_with_callables(ty, bounds, assumptions, &Default::default())
    }

    /// Prove in the declaration's complete protocol and callable environment.
    /// Callable premises retain their rigid owner identity and full contract.
    pub fn check_universal_bounds_with_callables(
        &self,
        ty: &Type,
        bounds: &[ProtocolBound],
        assumptions: &[WhereClause],
        callables: &verum_common::Map<TypeVar, Type>,
    ) -> Result<(), Text> {
        let mut context = Obligations {
            checker: self,
            assumptions,
            callables,
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
    callables: &'a verum_common::Map<TypeVar, Type>,
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
            if self.same(&subject, ty)
                && !actual.is_negative && !expected.is_negative
                && self.checker.paths_equal(&actual.protocol, &expected.protocol)
                && actual.args.len() == expected.args.len()
                && actual.args.iter().zip(&expected.args).all(|(a, b)| self.same(a, b))
            {
                let mut proved = true;
                for name in expected.associated_types.keys() {
                    let equations = self.checker.associated_premise_equations(
                        ty, expected, name, self.assumptions,
                    )?;
                    if equations.is_empty() || !equations.iter().all(|(a, b)| self.same(a, b)) {
                        proved = false;
                        break;
                    }
                }
                if proved { return Ok(true); }
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
                let mut inherited = inherited.try_apply_subst(&substitution)?;
                self.checker.inherit_associated_equalities(&subject, &actual, &mut inherited)?;
                pending.push((subject.clone(), inherited));
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
            let overlap = match self.checker.match_implementation_candidate_in_environment(
                implementation,
                ty,
                Some(&bound.args),
                false,
                &ConstraintEnvironment { protocols: self.assumptions, callables: self.callables },
            ) {
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
            if let CandidateMatch::Matched(substitution) = self
                .checker
                .match_implementation_candidate_in_environment(implementation, ty, Some(&bound.args), true, &ConstraintEnvironment { protocols: self.assumptions, callables: self.callables })
            {
                let mut proved = self
                    .checker
                    .check_implementation_callable_bounds_in_environment(implementation, &substitution, &ConstraintEnvironment { protocols: self.assumptions, callables: self.callables })
                    .is_ok();
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
                actual.instantiate(&[], substitution)?
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
}
