//! Shared admission judgment for declaration-owned value arguments.
//!
//! Normalization authority belongs to the caller. An empty domain still requires
//! the exact normalized carrier sort; unknown inference values are not evidence.
use crate::{refinement::RefinementPredicate, ty::{Type, ValueTerm}};

pub(crate) fn argument_satisfies_domain(
    parameter: &Type,
    argument: &Type,
    mut normalize: impl FnMut(&Type) -> Type,
) -> bool {
    let (Type::Meta { ty: expected, refinement: required, .. },
         Type::Meta { ty: actual, refinement: known, value, .. }) = (parameter, argument)
    else { return false; };
    if matches!(value, ValueTerm::Inference(_)) { return false; }
    let mut domain = |ty: &Type, predicate: &Option<RefinementPredicate>| {
        let ty = normalize(ty);
        match predicate {
            Some(predicate) => Type::refined(ty, predicate.clone()),
            None => ty,
        }
    };
    let expected = domain(expected, required);
    let actual = domain(actual, known);
    if expected.base() != actual.base() { return false; }
    if let ValueTerm::Constant(value) = value {
        let mut current = &expected;
        while let Type::Refined { base, predicate } = current {
            let mut evaluator = crate::const_eval::ConstEvaluator::new();
            evaluator.bind(predicate.bound_variable(), value.clone());
            if predicate.binds_subject("self") { evaluator.bind("self", value.clone()); }
            if !matches!(evaluator.eval(&predicate.predicate),
                Ok(verum_common::ConstValue::Bool(true))) { return false; }
            current = base;
        }
        true
    } else {
        let mut required_domain = &expected;
        while let Type::Refined { base, predicate } = required_domain {
            let mut known_domain = &actual;
            let mut established = false;
            while let Type::Refined { base, predicate: premise } = known_domain {
                established |= crate::refinement_entailment::entails(
                    actual.base(), premise, expected.base(), predicate,
                );
                known_domain = base;
            }
            let unconstrained = RefinementPredicate::trivial(predicate.span);
            if !established && !crate::refinement_entailment::entails(
                actual.base(), &unconstrained, expected.base(), predicate,
            ) { return false; }
            required_domain = base;
        }
        true
    }
}

#[cfg(test)]
#[path = "../tests/unit/value_parameter_domain.rs"]
mod tests;
