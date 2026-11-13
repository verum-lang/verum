//! Whole family applications retain their domains during protocol admission.
use verum_fast_parser::Parser;
use verum_types::TypeChecker;
fn errors(source: &str) -> Vec<String> {
    errors_with_policy(source, true)
}
fn errors_with_policy(source: &str, generic_associated_types: bool) -> Vec<String> {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut checker = TypeChecker::new();
    checker.register_primitives();
    checker.set_generic_associated_types_enabled(generic_associated_types);
    checker.set_current_module_path("families");
    let mut errors: Vec<_> = checker.prepare_module_declarations(&ast).iter().map(|e|format!("{e:?}")).collect();
    errors.extend(checker.take_deferred_errors().iter().map(|e| format!("{e:?}")));
    errors
}
fn accepts(source: &str) { let e=errors(source); assert!(e.is_empty(), "{e:#?}"); }
fn refuses_domain(source: &str) { let e=errors(source); assert!(e.iter().any(|e|e.contains("E405")), "{e:#?}"); }
#[test]
fn complete_family_application_keeps_its_supplied_parameter() {
    accepts("type F is protocol { type C<T>; fn pass<U>(x:Self.C<U>)->Self.C<U>; };");
}
#[test]
fn disabling_generic_associated_types_refuses_the_family_declaration() {
    let errors = errors_with_policy("type F is protocol { type C<T>; };", false);
    assert!(errors.iter().any(|error| error.contains("generic_associated_types = false")
        && error.contains("C<...>")), "{errors:#?}");
}
#[test]
fn a_bare_required_family_still_refuses_missing_arguments() {
    let e=errors("type F is protocol { type C<T>; fn pass(x:Self.C)->Self.C; };");
    assert!(e.iter().any(|e|e.contains("missing required associated family argument")), "{e:#?}");
}
#[test]
fn family_domain_requires_the_actual_method_bound() {
    refuses_domain("type Mark is protocol {}; type F is protocol { type C<T:Mark>; fn pass<U>(x:Self.C<U>)->Self.C<U>; };");
}
#[test]
fn family_domain_accepts_the_actual_method_bound() {
    accepts("type Mark is protocol {}; type F is protocol { type C<T:Mark>; fn pass<U:Mark>(x:Self.C<U>)->Self.C<U>; };");
}
#[test]
fn nested_family_inputs_are_still_checked() {
    refuses_domain("type Mark is protocol {}; type F is protocol { type Inner<T:Mark>; type Outer<T>; fn pass<U>(x:Self.Outer<Self.Inner<U>>)->Self.Outer<Self.Inner<U>>; };");
}
#[test]
fn nested_family_inputs_preserve_valid_bounds() {
    accepts("type Mark is protocol {}; type F is protocol { type Inner<T:Mark>; type Outer<T>; fn pass<U:Mark>(x:Self.Outer<Self.Inner<U>>)->Self.Outer<Self.Inner<U>>; };");
}
