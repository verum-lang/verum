use super::*;
    use verum_common::ConstValue;

    fn variable(id: ValueId) -> Type {
        Type::Meta { name: "N".into(), ty: Box::new(Type::Int), refinement: None,
            value: ValueTerm::Inference(id) }
    }

    #[test]
    fn one_inference_slot_cannot_acquire_conflicting_values() {
        let id = ValueId::fresh();
        let mut left = Substitution::new();
        left.value_bindings.insert(id, Type::meta_value(ConstValue::Int(1), Type::Int));
        let mut right = Substitution::new();
        right.value_bindings.insert(id, Type::meta_value(ConstValue::Int(2), Type::Int));
        assert!(left.compose(&right).is_err());
    }

    #[test]
    fn composition_transports_an_inference_chain_to_its_value() {
        let first = ValueId::fresh();
        let second = ValueId::fresh();
        let mut left = Substitution::new();
        left.value_bindings.insert(first, variable(second));
        let mut right = Substitution::new();
        let value = Type::meta_value(ConstValue::Int(7), Type::Int);
        right.value_bindings.insert(second, value.clone());
        let composed = left.compose(&right).unwrap();
        assert_eq!(variable(first).try_apply_subst(&composed).unwrap(), value);
        assert_eq!(variable(first).try_apply_subst(&left).unwrap().try_apply_subst(&right).unwrap(), value);
    }

    #[test]
    fn composition_refuses_value_identity_cycles() {
        let first = ValueId::fresh();
        let second = ValueId::fresh();
        let mut left = Substitution::new();
        left.value_bindings.insert(first, variable(second));
        let mut right = Substitution::new();
        right.value_bindings.insert(second, variable(first));
        assert!(left.compose(&right).is_err());
    }

    #[test]
    fn value_replacement_cannot_capture_a_quantified_type_in_its_sort() {
        let value = ValueId::fresh();
        let ty = TypeVar::fresh();
        let replacement = Type::Meta { name: "M".into(), ty: Box::new(Type::Var(ty)),
            refinement: None, value: ValueTerm::Parameter(ValueId::fresh()) };
        let body = Type::Forall { vars: List::from_iter([ty]), body: Box::new(variable(value)) };
        let mut substitution = Substitution::new();
        substitution.value_bindings.insert(value, replacement);
        let Type::Forall { vars, body } = body.try_apply_subst(&substitution).unwrap() else { panic!("forall") };
        assert_ne!(vars[0], ty);
        assert!(body.free_vars().contains(&ty));
    }

#[test]
fn deep_acyclic_value_graph_is_not_rejected_by_a_recursion_budget() {
    let ids: List<_> = (0..4096).map(|_| ValueId::fresh()).collect();
    let mut substitution = Substitution::new();
    for pair in ids.as_slice().windows(2) {
        substitution.value_bindings.insert(pair[0], variable(pair[1]));
    }
    substitution.value_bindings.insert(ids[4095], Type::meta_value(ConstValue::Int(7), Type::Int));
    assert!(substitution.check_value_cycles().is_ok());
    substitution.value_bindings.insert(ids[4095], variable(ids[0]));
    assert!(substitution.check_value_cycles().is_err());
}
