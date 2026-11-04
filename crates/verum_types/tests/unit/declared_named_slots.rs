use super::*;
use indexmap::IndexMap;
use verum_ast::{Ident, Path, PathSegment, Span};

fn named(name: &str) -> Type {
    Type::Named { path: Path::single(Ident::new(name, Span::default())), args: List::new() }
}
fn mapping(name: &str, actual: Type) -> IndexMap<Text, Type> {
    IndexMap::from([(name.into(), actual)])
}

#[test]
fn declared_carrier_transport_avoids_rank_two_capture() {
    let checker = TypeChecker::new();
    let local = TypeVar::fresh();
    let carrier = Type::Forall { vars: List::from_iter([local]), body: Box::new(
        Type::Tuple(List::from_iter([named("T"), Type::Var(local)]))) };
    let parameter = Type::meta("N".into(), carrier, None);
    let actual = checker.substitute_declared_named_slots(&parameter,
        &mapping("T", Type::Var(local))).unwrap();
    let Type::Meta { ty, value, .. } = &actual else { panic!("meta") };
    let Type::Forall { vars, body } = ty.as_ref() else { panic!("forall") };
    assert_ne!(vars[0], local, "free actual cannot become the bound local");
    assert_eq!(**body, Type::Tuple(List::from_iter([Type::Var(local), Type::Var(vars[0])])));
    let Type::Meta { value: original_value, .. } = &parameter else { unreachable!() };
    assert_eq!(value, original_value);
    assert!(actual.free_vars().contains(&local));
}

#[test]
fn noncapturing_or_unused_named_substitution_keeps_local_ids() {
    let checker = TypeChecker::new();
    let local = TypeVar::fresh();
    let carrier = Type::Forall { vars: List::from_iter([local]), body: Box::new(Type::Var(local)) };
    assert_eq!(checker.substitute_declared_named_slots(&carrier,
        &mapping("T", Type::Var(local))).unwrap(), carrier);
    let carrier = Type::Forall { vars: List::from_iter([local]), body: Box::new(
        Type::Tuple(List::from_iter([named("T"), Type::Var(local)]))) };
    let actual = checker.substitute_declared_named_slots(&carrier,
        &mapping("T", Type::Bool)).unwrap();
    let Type::Forall { vars, body } = actual else { panic!("forall") };
    assert_eq!(vars.as_slice(), &[local]);
    assert_eq!(*body, Type::Tuple(List::from_iter([Type::Bool, Type::Var(local)])));
}

#[test]
fn source_spelling_cannot_replace_an_unrelated_id_or_qualified_owner() {
    let checker = TypeChecker::new();
    let unrelated = TypeVar::fresh();
    let spelling = format!("T{}", unrelated.id());
    let foreign = Type::Named { path: Path::new(List::from_iter([
        PathSegment::Name(Ident::new("foreign", Span::default())),
        PathSegment::Name(Ident::new(spelling.as_str(), Span::default())),
    ]), Span::default()), args: List::new() };
    let input = Type::Tuple(List::from_iter([named(&spelling), Type::Var(unrelated), foreign.clone()]));
    assert_eq!(checker.substitute_declared_named_slots(&input,
        &mapping(&spelling, Type::Bool)).unwrap(),
        Type::Tuple(List::from_iter([Type::Bool, Type::Var(unrelated), foreign])));
}

#[test]
fn named_function_locals_shadow_the_outer_slot() {
    let checker = TypeChecker::new();
    let mut inner = Type::function(List::from_iter([named("T")]), named("T"));
    let Type::Function { type_params, .. } = &mut inner else { unreachable!() };
    type_params.push(crate::context::TypeParam::new("T", Span::default()));
    let input = Type::Tuple(List::from_iter([named("T"), inner.clone()]));
    assert_eq!(checker.substitute_declared_named_slots(&input, &mapping("T", Type::Bool)).unwrap(),
        Type::Tuple(List::from_iter([Type::Bool, inner])));
}

#[test]
fn replacement_ranges_are_simultaneous_and_value_identity_is_unchanged() {
    let checker = TypeChecker::new();
    let predicate = crate::refinement::RefinementPredicate::trivial(Span::default());
    let parameter = Type::meta("T".into(), Type::Refined {
        base: Box::new(named("T")), predicate: predicate.clone(),
    }, None);
    let input = Type::Tuple(List::from_iter([parameter.clone(), named("U")]));
    let parameters = IndexMap::from([("T".into(), named("U")), ("U".into(), named("T"))]);
    let output = checker.substitute_declared_named_slots(&input, &parameters).unwrap();
    let Type::Meta { value, .. } = parameter else { unreachable!() };
    assert_eq!(output, Type::Tuple(List::from_iter([
        Type::Meta { name: "T".into(), ty: Box::new(Type::Refined {
            base: Box::new(named("U")), predicate,
        }), refinement: None, value }, named("T")
    ])));
}

#[test]
fn deep_dependent_carrier_is_refused_without_a_partial_result() {
    let checker = TypeChecker::new();
    let mut carrier = named("T");
    for _ in 0..20 { carrier = Type::Meta { name: "N".into(), ty: Box::new(carrier),
        refinement: None, value: crate::ty::ValueTerm::Parameter(crate::ty::ValueId::fresh()) }; }
    assert!(checker.substitute_declared_named_slots(&carrier, &mapping("T", Type::Bool)).is_ok());
    for _ in 0..110 { carrier = Type::Refined { base: Box::new(carrier),
        predicate: crate::refinement::RefinementPredicate::trivial(Span::default()) }; }
    let error = checker.substitute_declared_named_slots(&carrier, &mapping("T", Type::Bool)).unwrap_err();
    assert!(error.contains("structural depth budget"), "{error}");
}

#[test]
fn an_unrepresented_carrier_shape_is_refused_without_a_partial_result() {
    let checker = TypeChecker::new();
    let carrier = Type::Generator { yield_ty: Box::new(named("T")), return_ty: Box::new(Type::Unit) };
    let error = checker.substitute_declared_named_slots(&carrier, &mapping("T", Type::Bool)).unwrap_err();
    assert_eq!(error.as_str(), "declared carrier has no supported named-slot traversal");
}

fn function_parameter(function: Type) -> Type { Type::meta("F".into(), function, None) }

#[test]
fn a_dependency_only_in_a_local_default_is_not_silently_retained() {
    let checker = TypeChecker::new();
    let mut carrier = Type::function(List::new(), Type::Unit);
    let Type::Function { type_params, .. } = &mut carrier else { unreachable!() };
    type_params.push(crate::context::TypeParam::new("U", Span::default()).with_default(named("T")));
    let result = checker.substitute_declared_named_slots(&function_parameter(carrier), &mapping("T", Type::Bool));
    assert!(result.is_err(), "a hidden dependent default must be transported or refused: {result:?}");
}

#[test]
fn dependencies_only_in_protocol_bound_arguments_and_equalities_are_refused() {
    let checker = TypeChecker::new();
    let protocol = Path::single(Ident::new("Relation", Span::default()));
    let argument_bound = crate::protocol::ProtocolBound::positive(protocol.clone(), List::from_iter([named("T")]));
    let mut equality_bound = crate::protocol::ProtocolBound::simple(protocol);
    equality_bound.associated_types.insert("Item".into(), named("T"));
    for bound in [argument_bound, equality_bound] {
        let mut carrier = Type::function(List::new(), Type::Unit);
        let Type::Function { type_params, .. } = &mut carrier else { unreachable!() };
        type_params.push(crate::context::TypeParam::new("U", Span::default()).with_bound(bound));
        let result = checker.substitute_declared_named_slots(&function_parameter(carrier), &mapping("T", Type::Bool));
        assert!(result.is_err(), "a hidden dependent bound must be transported or refused: {result:?}");
    }
}

#[test]
fn variable_and_serialized_context_dependencies_require_their_own_transport() {
    use crate::di::requirement::{ContextExpr, ContextRef, ContextRequirement};
    let checker = TypeChecker::new();
    let mut parameterized = ContextRef::new("State".into(), std::any::TypeId::of::<()>());
    parameterized.type_args.push("T".into());
    let mut transformed = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
    transformed.transforms.push(crate::di::requirement::ContextTransformRef {
        name: "with_policy".into(), args: List::from_iter(["T".into()]),
    });
    let mut conditional = ContextRef::new("Logger".into(), std::any::TypeId::of::<()>());
    conditional.condition = Some("T: Relation".into());
    for context in [ContextExpr::Variable(TypeVar::fresh()),
        ContextExpr::Concrete(ContextRequirement::single(parameterized)),
        ContextExpr::Concrete(ContextRequirement::single(conditional)),
        ContextExpr::Concrete(ContextRequirement::single(transformed))] {
        let mut carrier = Type::function(List::new(), Type::Unit);
        let Type::Function { contexts, .. } = &mut carrier else { unreachable!() };
        *contexts = Some(context);
        let result = checker.substitute_declared_named_slots(&function_parameter(carrier), &mapping("T", Type::Bool));
        assert!(result.is_err(), "a hidden context dependency must be transported or refused: {result:?}");
    }
}

#[test]
fn static_context_and_property_requirements_survive_function_carrier_transport() {
    use crate::di::requirement::{ContextExpr, ContextRef, ContextRequirement};
    use crate::computational_properties::{ComputationalProperty, PropertyRequirement, PropertySet};
    let checker = TypeChecker::new();
    let mut carrier = Type::function(List::from_iter([named("T")]), Type::Unit);
    let Type::Function { contexts, properties, property_requirement, .. } = &mut carrier else { unreachable!() };
    // Context names are a separate namespace, even with the same spelling.
    *contexts = Some(ContextExpr::Concrete(ContextRequirement::single(
        ContextRef::new("T".into(), std::any::TypeId::of::<()>()))));
    *properties = Some(PropertySet::single(ComputationalProperty::Pure));
    *property_requirement = PropertyRequirement::Excludes(PropertySet::single(ComputationalProperty::Async));
    let original = function_parameter(carrier);
    let mut expected = original.clone();
    let Type::Meta { ty, .. } = &mut expected else { unreachable!() };
    let Type::Function { params, .. } = ty.as_mut() else { unreachable!() };
    params[0] = Type::Bool;
    assert_eq!(checker.substitute_declared_named_slots(&original, &mapping("T", Type::Bool)).unwrap(), expected);
}
