#![cfg(feature = "codegen")]
//! Context ABI descriptors must retain their declaring source owner.
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, VbcCodegen},
};

fn ast(source: &str) -> verum_ast::Module {
    Parser::new(source).parse_module().unwrap()
}
fn named<'a>(
    module: &'a VbcModule,
    owner: &str,
    name: &str,
) -> Vec<&'a verum_vbc::types::TypeDescriptor> {
    module
        .types
        .iter()
        .filter(|ty| {
            module.get_string(ty.name).is_some_and(|declared| {
                verum_vbc::module::qualify_module_name(owner, declared) == format!("{owner}.{name}")
            }) && ty.origin_module.and_then(|id| module.get_string(id)) == Some(owner)
        })
        .collect()
}
fn check_owner_pair(module: &VbcModule, semantic: bool) {
    let left = named(module, "fixture.left", "MessagePortFixture");
    let right = named(module, "fixture.right", "MessagePortFixture");
    assert_eq!(left.len(), 1, "left descriptor must have its exact owner");
    assert_eq!(right.len(), 1, "right descriptor must have its exact owner");
    assert_ne!(
        left[0].id, right[0].id,
        "distinct owners must not share a descriptor"
    );
    assert!(
        left[0].source_declaration.is_none(),
        "context ABI is not a fabricated type AST"
    );
    assert_eq!(right[0].source_declaration.is_some(), semantic);
    assert_eq!(module.get_string(left[0].variants[0].name), Some("send"));
    assert_eq!(
        module.get_string(right[0].variants[0].name),
        Some("receive")
    );
}
fn pair(left: &verum_ast::Module, right: &verum_ast::Module, semantic: bool) {
    for reverse in [false, true] {
        let mut sources = vec![("fixture.left", left), ("fixture.right", right)];
        if reverse {
            sources.reverse();
        }
        let interface = VbcCodegen::with_config(CodegenConfig::new("bundle"))
            .collect_owned_declaration_interface(&sources)
            .unwrap();
        check_owner_pair(&interface, semantic);
        let bytes = verum_vbc::serialize::serialize_module(&interface).unwrap();
        let replay = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        check_owner_pair(&replay, semantic);
    }
}
#[test]
fn context_and_protocol_same_leaf_keep_distinct_owners_in_either_order() {
    pair(
        &ast("public context MessagePortFixture { fn send(value: Int); }"),
        &ast("public type MessagePortFixture is protocol { fn receive(&self) -> Bool; };"),
        true,
    );
}
#[test]
fn sibling_contexts_same_leaf_keep_distinct_owners_in_either_order() {
    pair(
        &ast("public context MessagePortFixture { fn send(value: Int); }"),
        &ast("public context MessagePortFixture { fn receive() -> Bool; }"),
        false,
    );
}
#[test]
fn explicit_context_header_overrides_supplied_parent() {
    let context =
        ast("module fixture.left; public context MessagePortFixture { fn send(value: Int); }");
    let protocol = ast(
        "module fixture.right; public type MessagePortFixture is protocol { fn receive(&self) -> Bool; };",
    );
    let module = VbcCodegen::with_config(CodegenConfig::new("bundle"))
        .collect_owned_declaration_interface(&[
            ("wrong.left", &context),
            ("wrong.right", &protocol),
        ])
        .unwrap();
    check_owner_pair(&module, true);
}
#[test]
fn context_cannot_impersonate_a_protocol_in_the_same_owner() {
    let context = ast("public context MessagePortFixture { fn send(value: Int); }");
    let protocol =
        ast("public type MessagePortFixture is protocol { fn receive(&self) -> Bool; };");
    for reverse in [false, true] {
        let mut sources = vec![("fixture.same", &context), ("fixture.same", &protocol)];
        if reverse {
            sources.reverse();
        }
        let error = VbcCodegen::with_config(CodegenConfig::new("bundle"))
            .collect_owned_declaration_interface(&sources)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("conflicting semantic source declarations"),
            "{error}"
        );
    }
}

#[test]
fn archive_import_retains_distinct_context_and_protocol_owners() {
    let context = ast("public context MessagePortFixture { fn send(value: Int); }");
    let protocol =
        ast("public type MessagePortFixture is protocol { fn receive(&self) -> Bool; };");
    let interface = VbcCodegen::with_config(CodegenConfig::new("bundle"))
        .collect_owned_declaration_interface(&[
            ("fixture.left", &context),
            ("fixture.right", &protocol),
        ])
        .unwrap();
    let bytes = verum_vbc::serialize::serialize_module(&interface).unwrap();
    let imported = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_module_types(&imported);
    let output = consumer.finalize_module_from_state().unwrap();
    let left = named(&output, "fixture.left", "MessagePortFixture");
    let right = named(&output, "fixture.right", "MessagePortFixture");
    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_ne!(left[0].id, right[0].id);
    assert!(left[0].source_declaration.is_none());
    assert!(right[0].source_declaration.is_some());
}

#[test]
fn context_name_matching_a_preseeded_type_still_gets_its_own_identity() {
    let context = ast("public context Channel { fn transmit(value: Int); }");
    let module = VbcCodegen::with_config(CodegenConfig::new("bundle"))
        .collect_owned_declaration_interface(&[("fixture.context", &context)])
        .unwrap();
    let own = named(&module, "fixture.context", "Channel");
    assert_eq!(own.len(), 1);
    assert_ne!(own[0].id, verum_vbc::types::TypeId::CHANNEL);
}

#[test]
fn different_exact_owner_context_declarations_are_refused() {
    for (left, right) in [
        (
            "public context SoundGateFixture { fn send(x: Int); }",
            "public context SoundGateFixture { fn receive() -> Bool; }",
        ),
        (
            "public context SoundGateFixture { fn send(x: Int); }",
            "public context SoundGateFixture { fn send(x: Bool); }",
        ),
        (
            "public context SoundGateFixture { fn get() -> Int; }",
            "public context SoundGateFixture { fn get() -> Bool; }",
        ),
    ] {
        let left = ast(left);
        let right = ast(right);
        for reverse in [false, true] {
            let mut sources = vec![("fixture.same", &left), ("fixture.same", &right)];
            if reverse {
                sources.reverse();
            }
            let error = VbcCodegen::with_config(CodegenConfig::new("bundle"))
                .collect_owned_declaration_interface(&sources)
                .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("conflicting exact source declarations"),
                "{error}"
            );
        }
    }
}

#[test]
fn context_method_bodies_are_refused_at_source_admission() {
    for value in [1, 2] {
        let source = format!("public context SoundGateFixture {{ fn get() -> Int {{ {value} }} }}");
        let errors = Parser::new(&source).parse_module().unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.to_string().contains("context method signature")),
            "{errors:?}"
        );
    }
}

#[test]
fn identical_context_source_is_idempotent_across_passes() {
    let source = ast("public context SoundGateFixture { fn get() -> Int; }");
    let clone = source.clone();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    codegen
        .collect_owned_declaration_interface(&[("fixture.same", &source)])
        .unwrap();
    let second = codegen
        .collect_owned_declaration_interface(&[("fixture.same", &clone)])
        .unwrap();
    let descriptors = named(&second, "fixture.same", "SoundGateFixture");
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].variants.len(), 1);
}

#[test]
fn context_source_evidence_survives_interface_publication() {
    let first = ast("public context SoundGateFixture { fn get() -> Int; }");
    let changed = ast("public context SoundGateFixture { fn get() -> Bool; }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("bundle"));
    codegen
        .collect_owned_declaration_interface(&[("fixture.same", &first)])
        .unwrap();
    let error = codegen
        .collect_owned_declaration_interface(&[("fixture.same", &changed)])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("conflicting exact source declarations"),
        "{error}"
    );
}

#[test]
fn local_context_does_not_reinterpret_imported_semantic_protocol() {
    let protocol = ast("public type SoundGateFixture is protocol { fn get(&self) -> Bool; };");
    let imported = VbcCodegen::with_config(CodegenConfig::new("bundle"))
        .collect_owned_declaration_interface(&[("fixture.same", &protocol)])
        .unwrap();
    let context = ast("public context SoundGateFixture { fn get() -> Int; }");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    codegen.import_archive_module_types(&imported);
    let error = codegen
        .collect_owned_declaration_interface(&[("fixture.same", &context)])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("conflicting semantic source declarations"),
        "{error}"
    );
}
