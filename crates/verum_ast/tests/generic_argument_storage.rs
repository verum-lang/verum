use verum_ast::{Expr, GenericArg, Literal, Span};

#[test]
fn constant_arguments_keep_the_existing_serialized_shape() {
    let expression = Expr::literal(Literal::int(42, Span::default()));
    let legacy = serde_json::json!({"Const": expression});
    let argument: GenericArg = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&argument).unwrap(), legacy);
    assert_eq!(
        argument.value_for_declared_slot().unwrap().as_ref(),
        &expression
    );
}

#[test]
fn type_argument_storage_does_not_embed_a_constant_expression() {
    assert!(
        std::mem::size_of::<GenericArg>() <= 384,
        "generic argument storage grew to {} bytes",
        std::mem::size_of::<GenericArg>()
    );
}
