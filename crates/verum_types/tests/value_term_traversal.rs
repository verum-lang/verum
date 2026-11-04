//! Value identities survive nested semantic structure and diagnostic renaming.
use verum_common::{ConstValue, Heap};
use verum_types::ty::{Type, ValueId, ValueTerm};
use verum_types::type_level_computation::equality::types_equal;

#[test]
fn nested_value_id_queries_include_domains_and_inference() {
    let rigid = ValueId::fresh();
    let inference = ValueId::fresh();
    let nested = Type::Meta {
        name: "outer".into(),
        ty: Heap::new(Type::Generic {
            name: "Domain".into(),
            args: [Type::Meta {
                name: "inner".into(),
                ty: Heap::new(Type::Int),
                refinement: None,
                value: ValueTerm::Inference(inference),
            }]
            .into_iter()
            .collect(),
        }),
        refinement: None,
        value: ValueTerm::Parameter(rigid),
    };
    let identities = nested.value_ids();
    assert_eq!(identities.len(), 2);
    assert!(identities.contains(&rigid));
    assert!(identities.contains(&inference));
    assert!(nested.has_value_inference());
    let constant = Type::meta_value(ConstValue::Int(0), Type::Int);
    assert!(constant.value_ids().is_empty());
    assert!(!constant.has_value_inference());
}

#[test]
fn value_equality_uses_identity_instead_of_diagnostic_name() {
    let original = Type::meta("N".into(), Type::Int, None);
    let mut renamed = original.clone();
    if let Type::Meta { name, .. } = &mut renamed {
        *name = "M".into();
    }
    assert!(types_equal(&original, &renamed));
    assert!(!types_equal(
        &original,
        &Type::meta("N".into(), Type::Int, None)
    ));
    let constant = Type::meta_value(ConstValue::Int(3), Type::Int);
    let mut renamed_constant = constant.clone();
    if let Type::Meta { name, .. } = &mut renamed_constant {
        *name = "diagnostic label".into();
    }
    assert!(types_equal(&constant, &renamed_constant));
}
