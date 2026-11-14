use super::ProtocolChecker;
use crate::{context::TypeScheme, Type, TypeVar};
use crate::computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet};
use verum_common::{ConstValue, List, Map};

fn carrier(argument: Type) -> Type {
    Type::Generic { name: "realm.Carrier".into(), args: List::from_single(argument) }
}
fn scheme(receiver: Type) -> TypeScheme {
    let mut scheme = TypeScheme::mono(Type::function(List::new(), Type::Int));
    scheme.impl_self_type = Some(receiver);
    scheme
}
fn accepts(scheme: &TypeScheme, actual: &Type) -> bool {
    ProtocolChecker::new().inherent_receiver_matches(scheme, actual, Map::new())
}

#[test]
fn equal_display_does_not_match_different_callable_bounds() {
    let unknown = Type::function(List::new(), Type::Int);
    let asynchronous = Type::function_with_requirement(List::new(), Type::Int,
        PropertyRequirement::AtMost(PropertySet::single(ComputationalProperty::Async)));
    assert_ne!(unknown, asynchronous);
    assert_eq!(unknown.to_string(), asynchronous.to_string());
    for (pattern, actual) in [(&unknown, &asynchronous), (&asynchronous, &unknown)] {
        let declared = scheme(carrier(pattern.clone()));
        assert!(accepts(&declared, &carrier(pattern.clone())));
        assert!(!accepts(&declared, &carrier(actual.clone())));
    }
}

#[test]
fn only_a_declared_binder_is_instantiable() {
    let variable = TypeVar::fresh();
    let mut declared = scheme(carrier(Type::Var(variable)));
    assert!(!accepts(&declared, &carrier(Type::Int)), "a free capture is not an implementation binder");
    declared.vars.push(variable);
    assert!(accepts(&declared, &carrier(Type::Int)));
}

#[test]
fn equal_value_payload_does_not_erase_its_scalar_sort() {
    let integer = Type::meta_value(ConstValue::Int(3), Type::Int);
    let character = Type::meta_value(ConstValue::Int(3), Type::Char);
    let declared = scheme(carrier(integer.clone()));
    assert!(accepts(&declared, &carrier(integer)));
    assert!(!accepts(&declared, &carrier(character)));
    assert!(!accepts(&declared, &carrier(Type::Int)), "a runtime scalar is not a value argument");
}

#[test]
fn declared_value_binder_requires_its_domain() {
    let parameter = Type::meta("N".into(), Type::Int, None);
    let mut declared = scheme(carrier(parameter.clone()));
    declared.value_parameters.push(parameter);
    assert!(accepts(&declared, &carrier(Type::meta_value(ConstValue::Int(3), Type::Int))));
    assert!(!accepts(&declared, &carrier(Type::meta_value(ConstValue::Bool(true), Type::Bool))));
    assert!(!accepts(&declared, &carrier(Type::Text)));
}

#[test]
fn an_unknown_type_is_not_a_receiver_wildcard() {
    assert!(!accepts(&scheme(carrier(Type::Unknown)), &carrier(Type::Int)));
    assert!(!accepts(&scheme(carrier(Type::Int)), &carrier(Type::Unknown)));
}

#[test]
fn source_owned_specialization_requires_its_receiver_evidence() {
    let source = "type Shell<T> is { value:T }; implement Shell<&mut Int> { fn selected(&self)->Int { 73 } }";
    let module = verum_fast_parser::Parser::new(source).parse_module().unwrap();
    let mut checker = crate::TypeChecker::new();
    checker.register_primitives();
    let errors = checker.prepare_module_declarations(&module);
    assert!(errors.is_empty(), "{errors:?}");
    let methods = checker.get_inherent_methods();
    let mut selected = methods.read().get(&"cog.Shell".into()).unwrap().get(&"selected".into()).unwrap().clone();
    assert!(selected.callable_source.is_some(), "the scheme must come from a source declaration");
    let receiver = selected.impl_self_type.clone().unwrap();
    let protocols = checker.protocol_checker.read();
    assert!(protocols.inherent_receiver_matches(&selected, &receiver, Map::new()));
    selected.impl_self_type = None;
    assert!(!protocols.inherent_receiver_matches(&selected, &receiver, Map::new()),
        "a callable signature cannot replace its lost implementation receiver");
    assert!(!protocols.inherent_receiver_matches(&selected, &Type::Int, Map::new()),
        "absent receiver evidence must not become a wildcard");
}
