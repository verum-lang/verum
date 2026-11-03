use verum_common::qualified_name::qualify_module_name;

#[test]
fn anonymous_root_preserves_declaration_identity() {
    for name in ["", "alpha", "alpha.Token", "alpha.beta.receive"] {
        assert_eq!(qualify_module_name("", name).as_str(), name);
    }
}

#[test]
fn named_owners_preserve_longest_declared_overlap() {
    for (owner, name, expected) in [
        ("alpha", "Token", "alpha.Token"),
        ("alpha", "alpha.Token", "alpha.Token"),
        ("alpha.beta", "beta.Token", "alpha.beta.Token"),
        ("alpha.beta", "gamma.Token", "alpha.beta.gamma.Token"),
    ] {
        assert_eq!(qualify_module_name(owner, name).as_str(), expected);
    }
}
