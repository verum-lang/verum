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
        if !constant_in_carrier(value, actual.base()) { return false; }
        for domain in [&actual, &expected] {
            let mut current = domain;
            while let Type::Refined { base, predicate } = current {
                let mut evaluator = crate::const_eval::ConstEvaluator::new();
                evaluator.bind(predicate.bound_variable(), value.clone());
                if predicate.binds_subject("self") { evaluator.bind("self", value.clone()); }
                if !matches!(evaluator.eval(&predicate.predicate),
                    Ok(verum_common::ConstValue::Bool(true))) { return false; }
                current = base;
            }
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

/// A claimed carrier is not evidence about the payload. Integer ranges and
/// primitive aliases come from the language's common numeric authority.
fn constant_in_carrier(value: &verum_common::ConstValue, carrier: &Type) -> bool {
    use verum_common::{ConstValue, well_known_types::type_names};
    match (value, carrier) {
        (ConstValue::Unit, Type::Unit)
        | (ConstValue::Bool(_), Type::Bool)
        | (ConstValue::Char(_), Type::Char)
        | (ConstValue::Text(_), Type::Text)
        | (ConstValue::Float(_), Type::Float) => return true,
        _ => {},
    }
    let name = match carrier {
        Type::Int => type_names::INT,
        Type::Named { path, args } if args.is_empty() => {
            let Some(name) = path.as_ident() else { return false; };
            name.as_str()
        }
        _ => return false,
    };
    match value {
        ConstValue::Int(value) if type_names::is_signed_integer_type(name) => {
            match (type_names::integer_limit(name, "MIN"), type_names::integer_limit(name, "MAX")) {
                (Some(min), Some(max)) => (min..=max).contains(value),
                _ => false,
            }
        }
        ConstValue::UInt(value) if type_names::is_unsigned_integer_type(name) => {
            match type_names::numeric_bit_width(name) {
                Some(128) => true,
                Some(bits) if bits < 128 => *value < (1_u128 << bits),
                _ => false,
            }
        }
        ConstValue::Float(value) if type_names::is_float_type(name) => {
            match type_names::numeric_bit_width(name) {
                Some(64) => true,
                Some(32) => (f64::from(*value as f32)).to_bits() == value.to_bits(),
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "../tests/unit/value_parameter_domain.rs"]
mod tests;
