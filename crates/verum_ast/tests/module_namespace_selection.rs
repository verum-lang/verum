use verum_ast::checked_const::ModuleOwners;

#[test]
fn namespace_operands_select_a_single_head_without_a_member() {
    let mut owners = ModuleOwners::default();
    owners.declare("app.provider");
    owners.declare("app.consumer.provider");
    assert_eq!(
        owners
            .namespace("provider", "app.consumer", "app")
            .as_deref(),
        Some("app.consumer.provider")
    );
    assert_eq!(
        owners.namespace("provider", "app.other", "app").as_deref(),
        Some("app.provider")
    );
    assert_eq!(owners.namespace("missing", "app.consumer", "app"), None);
    assert_eq!(owners.member("provider", "app.consumer", "app"), None);
    assert_eq!(
        owners
            .member("provider.Token", "app.consumer", "app")
            .as_deref(),
        Some("app.consumer.provider.Token")
    );
}
