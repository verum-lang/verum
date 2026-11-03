#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, CodegenErrorKind, VbcCodegen},
    interpreter::Interpreter,
};

fn compile(source: &str) -> Result<verum_vbc::VbcModule, verum_vbc::codegen::CodegenError> {
    let ast = Parser::new(source).parse_module().expect("parse fixture");
    VbcCodegen::with_config(CodegenConfig::new("fixture")).compile_module(&ast)
}

#[test]
fn an_unrelated_field_never_supplies_a_known_records_offset() {
    for operation in [
        "cursor.orbital_marker",
        "{ cursor.orbital_marker = 42; cursor.offset }",
        "{ let borrowed = &cursor.orbital_marker; *borrowed }",
    ] {
        for unrelated in [
            "",
            "type Unrelated is { orbital_marker: Int };",
            "type Unrelated is { orbital_marker: Int }; type Another is { orbital_marker: Int };",
            "type Unrelated is { padding: Int, orbital_marker: Int };",
        ] {
            let error = compile(&format!(
                "type Declared is {{ offset: Int }}; {unrelated}
                 fn probe(cursor: &mut Declared) -> Int {{ {operation} }}"
            ))
            .expect_err("a foreign field cannot establish a receiver's layout");
            assert!(
                matches!(error.kind, CodegenErrorKind::MissingRecordField { ref field, .. }
                if field == "orbital_marker"),
                "{error}"
            );
            assert_eq!(error.skip_class(), verum_vbc::codegen::SkipClass::Fatal);
        }
    }
}

#[test]
fn known_generic_records_reject_missing_fields_through_type_aliases() {
    let error = compile(
        "type Cell<T> is { value: T }; type Alias<T> is Cell<T>;
         type Other is { absent: Int };
         fn probe(cell: &Alias<Int>) -> Int { cell.absent }",
    )
    .expect_err("alias has the field set of its declared carrier");
    assert!(
        matches!(error.kind, CodegenErrorKind::MissingRecordField { ref field, .. }
        if field == "absent"),
        "{error}"
    );
}

#[test]
fn empty_records_and_positional_spelling_cannot_borrow_foreign_layouts() {
    for source in [
        "type Empty is {}; type Other is { absent: Int }; fn probe(x: &Empty) -> Int { x.absent }",
        "type Named is { value: Int }; fn probe(x: &Named) -> Int { x._7 }",
    ] {
        let error = compile(source)
            .err()
            .expect("declared record owns even its empty field set");
        assert!(
            matches!(error.kind, CodegenErrorKind::MissingRecordField { .. }),
            "{error}"
        );
    }
}

#[test]
fn generic_alias_layout_keeps_substituted_carriers_and_unbounded_acyclic_chains() {
    let mut source = String::from("type Cell is { value: Int }; type A0<T> is T;");
    for index in 1..12 {
        source.push_str(&format!("type A{index}<T> is A{}<T>;", index - 1));
    }
    source.push_str(
        "fn read(x: &A11<Cell>) -> Int { x.value } fn probe() -> Int { read(&Cell { value: 59 }) }",
    );
    let module = compile(&source).unwrap();
    let id = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("fixture.probe"))
        .unwrap()
        .id;
    assert_eq!(
        Interpreter::new(Arc::new(module))
            .execute_function(id)
            .unwrap()
            .as_i64(),
        59
    );
    let error = compile(&source.replace("x.value", "x.absent"))
        .err()
        .expect("alias carrier remains authoritative");
    assert!(
        matches!(error.kind, CodegenErrorKind::MissingRecordField { .. }),
        "{error}"
    );
}

#[test]
fn archive_declarations_keep_negative_field_information() {
    let producer = compile("public type Declared is { public offset: Int };").unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&producer).unwrap(),
    )
    .unwrap();
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.initialize();
    consumer.import_archive_module_types(&wire);
    let ast = Parser::new(
        "type Unrelated is { orbital_marker: Int };
         fn probe(cursor: &fixture.Declared) -> Int { cursor.orbital_marker }",
    )
    .parse_module()
    .unwrap();
    let result = consumer
        .collect_all_declarations(&ast)
        .and_then(|()| consumer.compile_module_items(&ast))
        .and_then(|()| consumer.finalize_module());
    let error = result.expect_err("imported source declaration keeps the exact field set");
    assert!(
        matches!(error.kind, CodegenErrorKind::MissingRecordField { ref field, .. }
        if field == "orbital_marker"),
        "{error}"
    );
    assert!(
        consumer.finalize_module_from_state().is_err(),
        "bootstrap must preserve the refusal"
    );
}

#[test]
fn declared_fields_execute_at_their_own_offsets_after_serialization() {
    let source = compile(
        "type Declared is { offset: Int, value: Int };
         type Other is { value: Int, offset: Int };
         fn probe() -> Int { let mut cell = Declared { offset: 17, value: 41 };
             cell.value = 42; cell.offset + cell.value }",
    )
    .unwrap();
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&source).unwrap(),
    )
    .unwrap();
    for module in [source, wire] {
        let id = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name.ends_with(".probe"))
            })
            .unwrap()
            .id;
        assert_eq!(
            Interpreter::new(Arc::new(module))
                .execute_function(id)
                .unwrap()
                .as_i64(),
            59
        );
    }
}

#[test]
fn numeric_field_names_in_records_still_use_declaration_order() {
    for source in [
        "type Numbered is { _7: Int, padding: Int }; fn probe() -> Int { let x = Numbered { _7: 59, padding: 2 }; x._7 }",
        "type Distance is (Int); fn probe() -> Int { Distance(59)._0 }",
        "fn probe() -> Int { let pair = (17, 42); pair.0 + pair.1 }",
    ] {
        let source_module = compile(source).unwrap();
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&source_module).unwrap(),
        )
        .unwrap();
        for module in [source_module, wire] {
            let id = module
                .functions
                .iter()
                .find(|f| module.get_string(f.name) == Some("fixture.probe"))
                .unwrap()
                .id;
            assert_eq!(
                Interpreter::new(Arc::new(module))
                    .execute_function(id)
                    .unwrap()
                    .as_i64(),
                59
            );
        }
    }
}

#[test]
fn allocating_wrapper_identity_survives_namespace_qualification() {
    use verum_vbc::{
        module::StringTable,
        types::{TypeDescriptor, TypeId, TypeKind},
    };
    let mut codegen = VbcCodegen::new();
    codegen.initialize();
    let mut strings = StringTable::new();
    for (name, id) in TypeId::SMART_POINTER_TYPE_IDS {
        codegen.register_archive_type_qualified(
            TypeDescriptor {
                id: *id,
                name: strings.intern(name),
                kind: TypeKind::Record,
                ..Default::default()
            },
            (*name).into(),
            Some("core.base.memory"),
            Some(&strings),
        );
        assert!(codegen.is_allocating_wrapper(&format!("core.base.memory.{name}<Int>")));
    }
    codegen.register_archive_type_qualified(
        TypeDescriptor {
            id: TypeId(40000),
            name: strings.intern("Heap"),
            kind: TypeKind::Record,
            ..Default::default()
        },
        "Heap".into(),
        Some("user"),
        Some(&strings),
    );
    assert!(!codegen.is_allocating_wrapper("user.Heap<Int>"));
}
