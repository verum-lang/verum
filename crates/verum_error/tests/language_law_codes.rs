//! Language-law diagnostics must be explainable through the shared registry.
#[test]
fn ambiguous_constructor_code_has_its_own_registry_entry() {
    let entry = verum_error::registry::lookup("E431").expect("E431 registered");
    assert_eq!(entry.numeric, 431);
    assert!(
        entry
            .description
            .contains("multiple visible owning sum types")
    );
}
