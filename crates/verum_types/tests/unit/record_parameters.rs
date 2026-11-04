use super::*;
use verum_common::ConstValue;

#[test]
fn parameterized_record_requires_its_parameter_authority() {
    let mut checker = TypeChecker::new();
    let fields = IndexMap::from_iter([("value".into(), Type::Int)]);
    let arguments = [Type::meta_value(ConstValue::Int(7), Type::Int)];
    let error = checker
        .instantiate_nominal_record_fields("Missing", &fields, Some(&arguments), Span::default())
        .unwrap_err();
    assert!(format!("{error:?}").contains("parameter roster"));
    let (unchanged, arguments) = checker
        .instantiate_nominal_record_fields("Nongeneric", &fields, None, Span::default())
        .unwrap();
    assert_eq!(unchanged, fields);
    assert!(arguments.is_empty());
}

#[test]
fn instantiation_does_not_quantify_a_foreign_field_value_identity() {
    let mut checker = TypeChecker::new();
    let owned = Type::meta("N".into(), Type::Int, None);
    let foreign = Type::meta("N".into(), Type::Int, None);
    checker.ctx.define_type(
        "__type_params_Owner",
        Type::Record(IndexMap::from_iter([("N".into(), owned.clone())])),
    );
    let fields =
        IndexMap::from_iter([("owned".into(), owned), ("foreign".into(), foreign.clone())]);
    let actual = Type::meta_value(ConstValue::Int(7), Type::Int);
    checker
        .with_value_obligation_scope(true, |checker| {
            let (instantiated, arguments) = checker.instantiate_nominal_record_fields(
                "Owner",
                &fields,
                Some(std::slice::from_ref(&actual)),
                Span::default(),
            )?;
            assert_eq!(instantiated["owned"], actual);
            assert_eq!(instantiated["foreign"], foreign);
            assert_eq!(arguments.as_slice(), std::slice::from_ref(&actual));
            Ok(())
        })
        .unwrap();
}
