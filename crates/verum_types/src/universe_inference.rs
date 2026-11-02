//! Simultaneous declaration instantiation and checked inference substitutions.
//! Rigid parameters can be renamed only by an explicit instantiation map;
//! unification writes exclusively to call-local metavariables.

use crate::universe_level::{MAX_LEVEL_NODES, UniverseLevel as Level, UniverseMeta};
use verum_common::{List, Map, Set, Text};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UniverseInference {
    parameters: Map<u32, Level>,
    metas: Map<UniverseMeta, Level>,
}

impl UniverseInference {
    pub fn is_empty(&self) -> bool {
        self.parameters.is_empty() && self.metas.is_empty()
    }
    pub fn parameter_bindings(&self) -> &Map<u32, Level> {
        &self.parameters
    }
    pub fn meta_bindings(&self) -> &Map<UniverseMeta, Level> {
        &self.metas
    }
    pub fn instantiate(parameters: &[u32]) -> Result<Self, Text> {
        if parameters.len() > MAX_LEVEL_NODES {
            return Err("universe binder limit exceeded".into());
        }
        let mut result = Self::default();
        for parameter in parameters {
            if result
                .parameters
                .insert(*parameter, Level::Meta(UniverseMeta::fresh()))
                .is_some()
            {
                return Err("duplicate quantified universe parameter".into());
            }
        }
        Ok(result)
    }

    pub fn apply(&self, source: &Level) -> Result<Level, Text> {
        enum Work<'a> {
            Enter(&'a Level, bool),
            Leave(UniverseMeta),
            Succ,
            Max,
        }
        let mut work = List::from_single(Work::Enter(source, true));
        let mut values = List::new();
        let mut active = Set::new();
        let mut remaining = MAX_LEVEL_NODES;
        let invalid = || Text::from("universe substitution exceeds its structural budget");
        while let Some(step) = work.pop() {
            match step {
                Work::Enter(level, rename) => {
                    remaining = remaining.checked_sub(1).ok_or_else(invalid)?;
                    match level {
                        Level::Variable(id) if rename && self.parameters.contains_key(id) => {
                            // Simultaneous substitution: never capture a parameter
                            // appearing in a replacement by applying the map again.
                            let replacement = &self.parameters[id];
                            if let Level::Meta(meta) = replacement
                                && !self.metas.contains_key(meta)
                            {
                                // Fresh instantiation replaces one leaf with one
                                // leaf; it cannot enlarge the source expression.
                                values.push(replacement.clone());
                            } else {
                                work.push(Work::Enter(replacement, false));
                            }
                        }
                        Level::Meta(id) if self.metas.contains_key(id) => {
                            if !active.insert(*id) {
                                return Err("cyclic universe inference substitution".into());
                            }
                            work.push(Work::Leave(*id));
                            work.push(Work::Enter(&self.metas[id], false));
                        }
                        Level::Concrete(_) | Level::Variable(_) | Level::Meta(_) => {
                            values.push(level.clone())
                        }
                        Level::Succ(inner) => {
                            work.push(Work::Succ);
                            work.push(Work::Enter(inner, rename));
                        }
                        Level::Max(left, right) => {
                            work.push(Work::Max);
                            work.push(Work::Enter(right, rename));
                            work.push(Work::Enter(left, rename));
                        }
                    }
                }
                Work::Leave(id) => {
                    active.remove(&id);
                }
                Work::Succ => {
                    let value = values.pop().ok_or_else(invalid)?;
                    values.push(Level::Succ(Box::new(value)));
                }
                Work::Max => {
                    let right = values.pop().ok_or_else(invalid)?;
                    let left = values.pop().ok_or_else(invalid)?;
                    values.push(Level::Max(Box::new(left), Box::new(right)));
                }
            }
        }
        values.pop().ok_or_else(invalid)
    }

    /// Solve structural equalities transactionally. Equations which require
    /// additional max-arithmetic search are refused, not guessed satisfied.
    pub fn unify(&mut self, left: &Level, right: &Level) -> Result<(), Text> {
        let mut candidate = self.clone();
        let mut work = List::from_single((left.clone(), right.clone()));
        let mut remaining = MAX_LEVEL_NODES;
        while let Some((left, right)) = work.pop() {
            remaining = remaining
                .checked_sub(1)
                .ok_or_else(|| Text::from("universe equation budget exceeded"))?;
            let left = candidate.apply(&left)?;
            let right = candidate.apply(&right)?;
            if left == right || left.equivalent(&right) {
                continue;
            }
            match (left, right) {
                (Level::Meta(id), level) | (level, Level::Meta(id)) => {
                    candidate.metas.insert(id, level);
                    candidate.apply(&Level::Meta(id))?; // occurs check, including indirect cycles
                }
                (Level::Succ(left), Level::Succ(right)) => work.push((*left, *right)),
                _ => return Err("universe equality is not established".into()),
            }
        }
        *self = candidate;
        Ok(())
    }

    /// Preserve both scopes when ordinary type substitutions are composed.
    pub(crate) fn compose(&self, next: &Self) -> Result<Self, Text> {
        let mut result = self.clone();
        for level in result.parameters.values_mut() {
            *level = next.apply(level)?;
        }
        for level in result.metas.values_mut() {
            *level = next.apply(level)?;
        }
        for (id, level) in &next.parameters {
            result
                .parameters
                .entry(*id)
                .or_insert_with(|| level.clone());
        }
        let mut overlaps = List::new();
        for (id, level) in &next.metas {
            if let Some(previous) = result.metas.get(id) {
                overlaps.push((previous.clone(), level.clone()));
            } else {
                result.metas.insert(*id, level.clone());
            }
        }
        // Acyclic substitutions can introduce a cycle when combined. Validate
        // the merged graph before exposing it to ordinary type substitution.
        for id in result.metas.keys() {
            result.apply(&Level::Meta(*id))?;
        }
        for (left, right) in overlaps {
            result.unify(&left, &right)?;
        }
        for id in result.parameters.keys() {
            result.apply(&Level::Variable(*id))?;
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_refuses_cross_substitution_cycles_transactionally() {
        let a = Level::Meta(UniverseMeta::fresh());
        let b = Level::Meta(UniverseMeta::fresh());
        let mut first = UniverseInference::default();
        first.unify(&a, &b).unwrap();
        let mut second = UniverseInference::default();
        second.unify(&b, &a.succ()).unwrap();
        let original = first.clone();
        assert!(first.compose(&second).is_err());
        assert_eq!(first, original);
        assert_eq!(first.apply(&a).unwrap(), b);
    }

    #[test]
    fn composition_refuses_conflicting_meta_equations() {
        let a = Level::Meta(UniverseMeta::fresh());
        let mut first = UniverseInference::default();
        first.unify(&a, &Level::Concrete(0)).unwrap();
        let mut second = UniverseInference::default();
        second.unify(&a, &Level::Concrete(1)).unwrap();
        assert!(first.compose(&second).is_err());
    }

    #[test]
    fn composition_preserves_resolved_meta_chains() {
        let a = Level::Meta(UniverseMeta::fresh());
        let b = Level::Meta(UniverseMeta::fresh());
        let mut first = UniverseInference::default();
        first.unify(&a, &b.succ()).unwrap();
        let mut second = UniverseInference::default();
        second.unify(&b, &Level::Concrete(2)).unwrap();
        let composed = first.compose(&second).unwrap();
        assert!(composed.apply(&a).unwrap().equivalent(&Level::Concrete(3)));
        assert_eq!(composed.apply(&b).unwrap(), Level::Concrete(2));
    }

    #[test]
    fn source_substitution_reports_universe_budget_failure() {
        use crate::ty::{Substitution, Type};
        use crate::unify::Unifier;
        use verum_ast::span::Span;

        let mut level = Level::Concrete(0);
        for _ in 0..MAX_LEVEL_NODES {
            level = Level::Succ(Box::new(level));
        }
        let ty = Type::Tuple(List::from_single(Type::Universe { level }));
        assert!(ty.try_apply_subst(&Substitution::new()).is_err());
        assert!(matches!(
            Unifier::new().unify(&ty, &ty, Span::default()),
            Err(crate::TypeError::UniverseBinding { .. })
        ));
    }

    #[test]
    fn checked_type_substitution_refuses_partial_depth_result() {
        use crate::ty::{Substitution, Type};

        let mut ty = Type::Universe {
            level: Level::Variable(7),
        };
        for _ in 0..32 {
            ty = Type::Tuple(List::from_single(ty));
        }
        let mut substitution = Substitution::new();
        substitution.universes = UniverseInference::instantiate(&[7]).unwrap();
        assert!(ty.try_apply_subst(&substitution).is_err());
    }
}
