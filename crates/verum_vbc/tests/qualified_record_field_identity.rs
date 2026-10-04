#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::types::StringId;

fn producer(module: &str, field_type: &str) -> VbcCodegen {
    let source = format!(
        "module {module}; type {field_type} is (Int); type Envelope is {{ value: {field_type} }};"
    );
    let ast = Parser::new(&source).parse_module().expect("parse producer");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new(module));
    codegen.compile_module(&ast).expect("compile producer");
    codegen
}

fn import(consumer: &mut VbcCodegen, producer: &VbcCodegen) {
    consumer.import_type_layouts(&producer.export_type_layouts());
    consumer.import_type_field_names(&producer.export_type_field_names());
}

fn body_targets(mut consumer: VbcCodegen, source: &str) -> Vec<String> {
    let ast = Parser::new(source).parse_module().expect("parse consumer");
    let module = consumer
        .compile_additional_module(&ast)
        .expect("compile consumer");
    decode_instructions(&module.bytecode)
        .expect("decode")
        .into_iter()
        .filter_map(|instruction| match instruction {
            Instruction::CallM { method_id, .. } => Some(
                module
                    .get_string(StringId(method_id))
                    .expect("method name")
                    .to_owned(),
            ),
            _ => None,
        })
        .collect()
}

#[test]
fn fresh_module_codegen_retains_imported_record_field_receiver() {
    let producer = producer("alpha", "Version");
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    import(&mut consumer, &producer);
    assert_eq!(
        body_targets(
            consumer,
            "fn probe(e: &Envelope) -> Int { e.value.identity() }"
        ),
        ["alpha.Version.identity"]
    );
}

#[test]
fn local_record_declaration_shadows_imported_field_types() {
    let producer = producer("alpha", "Version");
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    import(&mut consumer, &producer);
    assert_eq!(
        body_targets(
            consumer,
            r#"
module consumer;
type Envelope is { value: LocalVersion };
fn probe(e: &Envelope) -> Int { e.value.identity() }
"#
        ),
        ["LocalVersion.identity"]
    );
}

#[test]
fn same_leaf_module_metadata_keeps_qualified_fields_and_first_bare_owner() {
    let alpha = producer("alpha", "Version");
    let beta = producer("beta", "Version");
    let expression = Parser::new("e.value.identity()")
        .parse_expr()
        .expect("parse expression");
    for reverse in [false, true] {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        let mut producers = [&alpha, &beta];
        if reverse {
            producers.reverse();
        }
        for producer in producers {
            import(&mut consumer, producer);
        }
        let mut targets = Vec::new();
        // Feed carried receiver identities directly: this tests the field
        // metadata boundary independently of source path rendering.
        for owner in ["alpha.Envelope", "beta.Envelope", "Envelope"] {
            consumer
                .ctx_mut()
                .begin_function("probe", &[("e".to_owned(), false)], None);
            consumer
                .ctx_mut()
                .variable_type_names
                .insert("e".to_owned(), owner.to_owned());
            consumer
                .compile_expr(&expression)
                .expect("compile field method");
            let ctx = consumer.ctx_mut();
            targets.extend(
                ctx.instructions
                    .iter()
                    .filter_map(|instruction| match instruction {
                        Instruction::CallM { method_id, .. } => {
                            Some(ctx.strings[*method_id as usize].clone())
                        }
                        _ => None,
                    }),
            );
        }
        assert_eq!(
            targets,
            [
                "alpha.Version.identity",
                "beta.Version.identity",
                if reverse {
                    "beta.Version.identity"
                } else {
                    "alpha.Version.identity"
                }
            ]
        );
    }
}

#[test]
fn scoped_field_renderer_preserves_explicit_paths_mounts_and_declared_generics() {
    let source = r#"
module alpha;
mount beta.{Version as RemoteVersion};
type T is (Int);
type F is (Int);
type Version is (Int);
type Envelope<T, F<_>> is {
    local: Version,
    remote: RemoteVersion,
    explicit: gamma.Version,
    parameter: T,
    nested: Maybe<T>,
    higher: F<T>,
    pointer: *mut Version,
};
"#;
    let ast = Parser::new(source).parse_module().expect("parse scope");
    let codegen = VbcCodegen::new();
    let fields = codegen.declared_field_type_names(&ast, "alpha");
    for (field, expected) in [
        ("local", "alpha.Version"),
        ("remote", "beta.Version"),
        ("explicit", "gamma.Version"),
        ("parameter", "T"),
        ("nested", "Maybe<T>"),
        ("higher", "F<T>"),
        ("pointer", "*mut alpha.Version"),
    ] {
        assert_eq!(
            fields
                .get(&("alpha.Envelope".into(), field.into()))
                .map(String::as_str),
            Some(expected)
        );
    }
}

#[test]
fn explicit_mount_wins_over_an_unrelated_bare_record_owner() {
    let alpha = producer("alpha", "Version");
    let beta = producer("beta", "Version");
    for reverse in [false, true] {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        let mut producers = [&alpha, &beta];
        if reverse {
            producers.reverse();
        }
        for producer in producers {
            import(&mut consumer, producer);
        }
        assert_eq!(
            body_targets(
                consumer,
                r#"
module consumer;
mount beta.Envelope;
fn probe(e: &Envelope) -> Int { e.value.identity() }
"#
            ),
            ["beta.Version.identity"]
        );
    }
}

#[test]
fn http_response_fields_keep_the_declaring_nominal_owner() {
    let ast = Parser::new(include_str!("../../../core/net/http.vr"))
        .parse_module()
        .expect("parse HTTP declarations");
    let fields = VbcCodegen::new().declared_field_type_names(&ast, "core.net");
    for (field, expected) in [
        ("version", "core.net.http.Version"),
        ("status", "core.net.http.StatusCode"),
    ] {
        assert_eq!(
            fields
                .get(&("core.net.http.Response".into(), field.into()))
                .map(String::as_str),
            Some(expected)
        );
    }
}

#[test]
fn missing_mounted_child_does_not_borrow_an_ancestor_field_identity() {
    let ancestor = producer("alpha", "Version");
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    import(&mut consumer, &ancestor);
    assert_eq!(
        body_targets(
            consumer,
            r#"
module consumer;
mount alpha.child.Envelope;
fn probe(e: &Envelope) -> Int { e.value.identity() }
"#
        ),
        ["identity"]
    );
}

#[test]
fn rank_two_field_binder_is_not_rewritten_as_a_local_nominal() {
    let ast = Parser::new(
        r#"
module alpha;
type R is (Int);
type Holder is { call: fn<R>(R) -> R };
"#,
    )
    .parse_module()
    .expect("parse rank-2 field");
    let fields = VbcCodegen::new().declared_field_type_names(&ast, "alpha");
    assert_eq!(
        fields
            .get(&("alpha.Holder".into(), "call".into()))
            .map(String::as_str),
        Some("fn<R>(R) -> R")
    );
}

#[test]
fn bundled_archive_owner_requires_the_carried_source_origin() {
    use verum_vbc::module::StringTable;
    use verum_vbc::types::{FieldDescriptor, TypeDescriptor, TypeId, TypeKind, TypeRef};
    let mut strings = StringTable::new();
    let descriptor = TypeDescriptor {
        id: TypeId(1300),
        name: strings.intern("Envelope"),
        kind: TypeKind::Record,
        origin_module: Some(strings.intern("alpha.child")),
        fields: [FieldDescriptor {
            name: strings.intern("value"),
            type_name: strings.intern("alpha.child.Version"),
            type_ref: TypeRef::Concrete(TypeId::PTR),
            ..Default::default()
        }]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_type_with_protocol_remap_qualified(
        &descriptor,
        &strings,
        &Default::default(),
        Some("alpha"),
    );
    assert_eq!(
        body_targets(
            consumer,
            r#"
module consumer;
mount alpha.child.Envelope;
fn probe(e: &Envelope) -> Int { e.value.identity() }
"#
        ),
        ["alpha.child.Version.identity"]
    );
}
