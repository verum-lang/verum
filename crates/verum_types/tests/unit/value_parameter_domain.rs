use crate::refinement::{PredicateProvenance, RefinementBinding, RefinementPredicate};
use crate::ty::{Type, ValueId, ValueTerm};
use verum_ast::{ItemKind, ty::GenericParamKind};
use verum_common::ConstValue;
fn parameter(domain: &str) -> Type {
    let ast = verum_fast_parser::Parser::new(&format!(
        "fn bounded<N: meta Int{domain}>(x:Int)->Int {{ x }}"
    ))
    .parse_module()
    .unwrap();
    let ItemKind::Function(f) = &ast.items[0].kind else {
        panic!("function")
    };
    let GenericParamKind::Meta {
        name,
        ty,
        refinement,
    } = &f.generics[0].kind
    else {
        panic!("meta")
    };
    let mut checker = crate::TypeChecker::new();
    checker.register_primitives();
    let carrier = checker.ast_to_type(ty).expect("actual declared carrier");
    Type::meta(
        name.name.clone(),
        carrier,
        refinement.as_ref().map(|p| RefinementPredicate {
            predicate: (**p).clone().into(),
            binding: RefinementBinding::Sigma(name.name.clone()),
            span: ty.span,
            provenance: PredicateProvenance::Declared,
        }),
    )
}
fn admits(p: &Type, a: &Type) -> bool {
    super::argument_satisfies_domain(p, a, Clone::clone)
}
#[test]
fn empty_domain_still_requires_exact_carrier_sort() {
    let p = parameter("");
    assert!(admits(&p, &Type::meta_value(ConstValue::Int(7), Type::Int)));
    assert!(!admits(
        &p,
        &Type::meta_value(ConstValue::Bool(true), Type::Bool)
    ));
    assert!(!admits(&p, &Type::meta("B".into(), Type::Bool, None)));
}
#[test]
fn unresolved_inference_is_not_a_domain_witness() {
    let p = parameter("");
    let a = Type::Meta {
        name: "N".into(),
        ty: Box::new(Type::Int),
        refinement: None,
        value: ValueTerm::Inference(ValueId::fresh()),
    };
    assert!(!admits(&p, &a));
    assert!(admits(&p, &parameter("")));
}
#[test]
fn constants_discharge_actual_predicate() {
    let p = parameter("{>=0}");
    assert!(admits(&p, &Type::meta_value(ConstValue::Int(7), Type::Int)));
    assert!(!admits(
        &p,
        &Type::meta_value(ConstValue::Int(-1), Type::Int)
    ));
}
#[test]
fn symbolic_domain_uses_entailment_not_matching_names() {
    let p = parameter("{>=0}");
    assert!(admits(&p, &parameter("{>=7}")));
    assert!(!admits(&p, &parameter("")));
    assert!(!admits(&parameter("{>=7}"), &p));
}
#[test]
fn identical_unsupported_predicate_is_not_proof() {
    let p = parameter("{it>=M}");
    assert!(!admits(&p, &p));
    assert!(!admits(
        &p,
        &Type::meta_value(ConstValue::Int(7), Type::Int)
    ));
}
#[test]
fn normalization_authority_is_explicit() {
    let p = parameter("");
    let a = Type::meta_value(
        ConstValue::Int(7),
        Type::Generic {
            name: "CarrierAlias".into(),
            args: Default::default(),
        },
    );
    assert!(!admits(&p, &a));
    assert!(super::argument_satisfies_domain(&p, &a, |ty| match ty {
        Type::Generic { name, args } if name.as_str() == "CarrierAlias" && args.is_empty() =>
            Type::Int,
        _ => ty.clone(),
    }));
}

#[test]
fn a_claimed_scalar_carrier_cannot_replace_payload_evidence() {
    let cases = [
        (ConstValue::Unit, Type::Unit),
        (ConstValue::Bool(true), Type::Bool),
        (ConstValue::Int(7), Type::Int),
        (ConstValue::UInt(7), Type::u64_refined(0)),
        (ConstValue::Float(1.5), Type::Float),
        (ConstValue::Char('λ'), Type::Char),
        (ConstValue::Text("exact".into()), Type::Text),
    ];
    for (index, (payload, _)) in cases.iter().enumerate() {
        for (carrier_index, (_, carrier)) in cases.iter().enumerate() {
            let declared = Type::meta("P".into(), carrier.clone(), None);
            let actual = Type::meta_value(payload.clone(), carrier.clone());
            assert_eq!(
                admits(&declared, &actual),
                index == carrier_index,
                "{payload:?} as {carrier:?}"
            );
        }
    }
}

#[test]
fn integer_payloads_must_fit_the_exact_carrier() {
    for (carrier, min, max) in [
        (Type::Int, i128::from(i64::MIN), i128::from(i64::MAX)),
        (
            Type::i8_refined(0),
            i128::from(i8::MIN),
            i128::from(i8::MAX),
        ),
        (
            Type::i32_refined(0),
            i128::from(i32::MIN),
            i128::from(i32::MAX),
        ),
        (Type::i128_refined(0), i128::MIN, i128::MAX),
    ] {
        let declared = Type::meta("N".into(), carrier.clone(), None);
        for value in [min, 0, max] {
            assert!(admits(
                &declared,
                &Type::meta_value(ConstValue::Int(value), carrier.clone())
            ));
        }
        for outside in [min.checked_sub(1), max.checked_add(1)]
            .into_iter()
            .flatten()
        {
            assert!(!admits(
                &declared,
                &Type::meta_value(ConstValue::Int(outside), carrier.clone())
            ));
        }
    }
    for (carrier, max) in [
        (Type::u8_refined(0), u128::from(u8::MAX)),
        (Type::u64_refined(0), u128::from(u64::MAX)),
        (Type::u128_refined(0), u128::MAX),
    ] {
        let declared = Type::meta("N".into(), carrier.clone(), None);
        for value in [0, max] {
            assert!(admits(
                &declared,
                &Type::meta_value(ConstValue::UInt(value), carrier.clone())
            ));
        }
        if let Some(outside) = max.checked_add(1) {
            assert!(!admits(
                &declared,
                &Type::meta_value(ConstValue::UInt(outside), carrier.clone())
            ));
        }
        assert!(!admits(
            &declared,
            &Type::meta_value(ConstValue::Int(-1), carrier)
        ));
    }
}

#[test]
fn a_constant_must_also_satisfy_its_own_claimed_refinement() {
    let mut forged = parameter("{>=10}");
    let Type::Meta { value, .. } = &mut forged else {
        unreachable!()
    };
    *value = ValueTerm::Constant(ConstValue::Int(7));
    assert!(!admits(&parameter(""), &forged));
    let Type::Meta { value, .. } = &mut forged else {
        unreachable!()
    };
    *value = ValueTerm::Constant(ConstValue::Int(10));
    assert!(admits(&parameter(""), &forged));
}

#[test]
fn unsupported_aggregate_payloads_do_not_gain_a_scalar_carrier() {
    let p = parameter("");
    for payload in [
        ConstValue::Array(Default::default()),
        ConstValue::Bytes(vec![1]),
        ConstValue::Tuple(Default::default()),
        ConstValue::Maybe(Default::default()),
        ConstValue::Map(Default::default()),
        ConstValue::Set(Default::default()),
    ] {
        assert!(!admits(&p, &Type::meta_value(payload, Type::Int)));
    }
}

#[test]
fn a_narrow_float_carrier_requires_an_already_representable_payload() {
    let carrier = Type::Named {
        path: verum_ast::ty::Path::single(verum_ast::ty::Ident::new(
            "Float32",
            verum_ast::Span::default(),
        )),
        args: Default::default(),
    };
    let declared = Type::meta("F".into(), carrier.clone(), None);
    for value in [
        0.0,
        -0.0,
        1.5,
        f64::from(1.2_f32),
        f64::from(f32::MAX),
        f64::INFINITY,
    ] {
        assert!(admits(
            &declared,
            &Type::meta_value(ConstValue::Float(value), carrier.clone())
        ));
    }
    for value in [1.2_f64, f64::MAX, f64::MIN_POSITIVE] {
        assert!(!admits(
            &declared,
            &Type::meta_value(ConstValue::Float(value), carrier.clone())
        ));
    }
}
