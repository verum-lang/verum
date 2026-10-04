#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::Instruction;
use verum_vbc::module::StringTable;
use verum_vbc::types::{FieldDescriptor, StringId, TypeDescriptor, TypeId, TypeKind, TypeRef};

fn codegen() -> VbcCodegen {
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("field_consumer"));
    // Archive ids must be read in their own pool, even when they also name
    // valid (but unrelated) strings in the consumer.
    for i in 0..256 {
        codegen
            .ctx_mut()
            .intern_string_raw(&format!("consumer_string_{i}"));
    }
    codegen
}

fn register(
    codegen: &mut VbcCodegen,
    owner: &str,
    id: u32,
    carried: Option<&str>,
    type_ref: TypeRef,
) {
    let mut strings = StringTable::new();
    for i in 0..32 {
        strings.intern(&format!("archive_string_{i}"));
    }
    let descriptor = TypeDescriptor {
        id: TypeId(id),
        name: strings.intern("Envelope"),
        kind: TypeKind::Record,
        fields: [FieldDescriptor {
            name: strings.intern("value"),
            type_name: carried
                .map(|name| strings.intern(name))
                .unwrap_or(StringId::EMPTY),
            type_ref,
            ..Default::default()
        }]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    codegen.register_archive_type_qualified(
        descriptor,
        "Envelope".to_owned(),
        Some(owner),
        Some(&strings),
    );
}

fn method_targets(mut codegen: VbcCodegen, owners: &[&str]) -> Vec<String> {
    let expression = Parser::new("e.value.identity()")
        .parse_expr()
        .expect("parse expression");
    let mut targets = Vec::new();
    for owner in owners {
        codegen
            .ctx_mut()
            .begin_function("probe", &[("e".to_owned(), false)], None);
        codegen
            .ctx_mut()
            .variable_type_names
            .insert("e".to_owned(), (*owner).to_owned());
        codegen
            .compile_expr(&expression)
            .expect("compile field method");
        let ctx = codegen.ctx_mut();
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
    targets
}

#[test]
fn archive_field_carried_name_survives_an_opaque_type_ref() {
    let mut codegen = codegen();
    register(
        &mut codegen,
        "alpha",
        1300,
        Some("alpha.Version"),
        TypeRef::Concrete(TypeId::PTR),
    );
    assert_eq!(
        method_targets(codegen, &["Envelope"]),
        ["alpha.Version.identity"],
    );
}

#[test]
fn archive_field_carried_name_overrides_a_conflicting_numeric_type() {
    let mut codegen = codegen();
    register(
        &mut codegen,
        "alpha",
        1300,
        Some("alpha.Version"),
        TypeRef::Concrete(TypeId::TEXT),
    );
    assert_eq!(
        method_targets(codegen, &["Envelope"]),
        ["alpha.Version.identity"],
    );
}

#[test]
fn archive_field_primitive_fallback_remains_available() {
    let mut codegen = codegen();
    register(
        &mut codegen,
        "alpha",
        1300,
        None,
        TypeRef::Concrete(TypeId::INT),
    );
    assert_eq!(method_targets(codegen, &["Envelope"]), ["Int.identity"],);
}

#[test]
fn archive_field_same_leaf_owners_keep_their_qualified_identity() {
    for reverse in [false, true] {
        let mut codegen = codegen();
        let mut siblings = [
            ("alpha", 1300, "alpha.Version"),
            ("beta", 1301, "beta.Version"),
        ];
        if reverse {
            siblings.reverse();
        }
        for (owner, id, carried) in siblings {
            register(
                &mut codegen,
                owner,
                id,
                Some(carried),
                TypeRef::Concrete(TypeId::PTR),
            );
        }
        let targets = method_targets(codegen, &["alpha.Envelope", "beta.Envelope", "Envelope"]);
        assert_eq!(
            targets,
            [
                "alpha.Version.identity",
                "beta.Version.identity",
                if reverse {
                    "beta.Version.identity"
                } else {
                    "alpha.Version.identity"
                },
            ]
        );
    }
}

#[test]
fn archive_field_same_owner_reregistration_updates_forward_metadata() {
    let mut codegen = codegen();
    register(
        &mut codegen,
        "alpha",
        1300,
        None,
        TypeRef::Concrete(TypeId::PTR),
    );
    register(
        &mut codegen,
        "alpha",
        1300,
        Some("alpha.Version"),
        TypeRef::Concrete(TypeId::PTR),
    );
    assert_eq!(
        method_targets(codegen, &["alpha.Envelope", "Envelope"]),
        ["alpha.Version.identity", "alpha.Version.identity"]
    );
}
