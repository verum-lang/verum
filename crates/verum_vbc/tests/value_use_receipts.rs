//! T1599: producer facts are distinct from runtime ownership permissions.
#![cfg(feature = "codegen")]
use verum_common::value_use::{
    DuplicationOperation, ValueUseOperation as Op, ValueUseSite as Site,
};
use verum_common::{List, Map, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    module::VbcModule,
};

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("grammar-valid source");
    VbcCodegen::with_config(CodegenConfig::new("receipts"))
        .compile_module(&ast)
        .expect("compile")
}
fn fid(module: &VbcModule, leaf: &str) -> verum_vbc::FunctionId {
    module
        .find_function_by_name(&format!("receipts.{leaf}"))
        .or_else(|| module.find_function_by_name(leaf))
        .expect(leaf)
}
fn uses<'a>(module: &'a VbcModule, leaf: &str) -> &'a [verum_vbc::value_use::ValueUseReceipt] {
    module
        .value_use_receipts(fid(module, leaf))
        .expect("valid production receipts")
}
const RECORD: &str = "type Cell is { value: Int };";

#[test]
fn actual_record_clone_is_published_and_current_interpreter_behavior_is_unchanged() {
    let module = compile(&format!(
        "{RECORD} fn probe() -> Int {{ let mut a: Cell = Cell {{ value: 7 }}; let b = a; a.value = 9; b.value }}"
    ));
    assert!(
        uses(&module, "probe")
            .iter()
            .any(|u| u.event.operation == Op::Copy(DuplicationOperation::ValueCopy))
    );
    let id = fid(&module, "probe");
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(id)
        .expect("execute");
    assert_eq!(value.as_i64(), 7);
}
#[test]
fn affine_observations_do_not_grant_transfer_or_cleanup_permission() {
    let module = compile(
        "type affine Cell is { value: Int }; fn pass(a: Cell) -> Cell { let b = a; return b; }",
    );
    let receipts = uses(&module, "pass");
    assert!(receipts.iter().any(|u| u.event.site == Site::Local));
    assert!(receipts.iter().all(|u| u.event.operation == Op::Unknown));
    assert!(receipts.iter().any(|u| u.event.site == Site::Return));
}
#[test]
fn references_preserve_borrow_identity_through_local_and_return() {
    let module = compile(&format!(
        "{RECORD} fn pass(a: &Cell) -> &Cell {{ let b = a; b }}"
    ));
    let receipts = uses(&module, "pass");
    assert!(receipts.iter().any(|u| u.event.operation == Op::Borrow));
    assert!(
        receipts
            .iter()
            .any(|u| u.event.site == Site::Return && u.event.operation == Op::Borrow)
    );
    assert!(
        receipts
            .iter()
            .all(|u| !matches!(u.event.operation, Op::Transfer | Op::Copy(_)))
    );
}
#[test]
fn direct_arguments_and_returns_have_sites_but_do_not_grant_transfer() {
    let module = compile(
        "fn receive(a: Int) -> Int { a } fn pass(a: Int) -> Int { let b = receive(a); return a; }",
    );
    let receipts = uses(&module, "pass");
    assert!(
        receipts.iter().any(|u| u.event.site == Site::Argument(0)),
        "{receipts:?}"
    );
    assert!(receipts.iter().any(|u| u.event.site == Site::Return));
    assert!(receipts.iter().all(|u| u.event.operation != Op::Transfer));
}
#[test]
fn shadowed_names_keep_distinct_binding_identities() {
    let module = compile(&format!(
        "{RECORD} fn pass(a: Cell) -> Cell {{ {{ let a: Cell = Cell {{ value: 1 }}; let b = a; }} {{ let a: Cell = Cell {{ value: 2 }}; let b = a; }} let b = a; return b; }}"
    ));
    let copies: List<_> = uses(&module, "pass")
        .iter()
        .filter(|u| matches!(u.event.operation, Op::Copy(_)))
        .collect();
    assert_eq!(copies.len(), 3, "{copies:?}");
    assert_ne!(copies[0].event.binding, copies[1].event.binding);
    assert_ne!(copies[0].event.binding, copies[2].event.binding);
}
#[test]
fn user_and_bootstrap_production_publish_the_same_receipts() {
    let source: Text =
        format!("{RECORD} fn pass(a: Cell) -> Cell {{ let b = a; return b; }}").into();
    let direct = compile(&source);
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut producer = VbcCodegen::with_config(CodegenConfig::new("receipts"));
    producer.collect_unit_declarations(&[&ast]).unwrap();
    producer
        .compile_unit_items(&[&ast], ItemFailurePolicy::StubAndContinue)
        .unwrap();
    let baked = producer.finalize_module_from_state().unwrap();
    assert_eq!(uses(&direct, "pass"), uses(&baked, "pass"));
}
#[test]
fn archive_roundtrip_and_exact_type_remap_keep_receipts_current() {
    let original = compile(&format!(
        "{RECORD} fn pass(a: Cell) -> Cell {{ let b = a; return b; }}"
    ));
    let loaded = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&original).unwrap(),
    )
    .unwrap();
    assert_eq!(uses(&original, "pass"), uses(&loaded, "pass"));
    let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    consumer.import_archive_module_types(&loaded);
    let remap: Map<_, _> = loaded
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.0, verum_vbc::FunctionId(5000 + i as u32)))
        .collect();
    consumer.merge_archive_function_bodies(&loaded, &remap.into());
    let imported = consumer.finalize_module_from_state().unwrap();
    let receipts = uses(&imported, "pass");
    assert!(
        receipts
            .iter()
            .any(|u| matches!(u.event.operation, Op::Copy(_)))
    );
    for receipt in receipts {
        if let Some(verum_vbc::types::TypeRef::Concrete(id)) = &receipt.declaration_type {
            assert_eq!(
                imported.get_string(imported.get_type(*id).expect("imported exact type").name),
                Some("Cell")
            );
        }
    }
}
#[test]
fn any_unmapped_body_rewrite_invalidates_facts_and_cfg() {
    let mut module = compile("fn pass(a: Int) -> Int { return a; }");
    let id = fid(&module, "pass");
    assert!(module.value_use_cfg(id).is_some());
    let offset = module.get_function(id).unwrap().bytecode_offset as usize;
    module.bytecode[offset] ^= 1;
    assert!(module.value_use_receipts(id).is_none());
    assert!(module.value_use_cfg(id).is_none());
}
#[test]
fn shared_cfg_places_branch_uses_in_distinct_existing_blocks() {
    let module = compile(
        "fn pass(a: Int, flag: Bool) -> Int { if flag { return a; } else { let b = a; return b; } }",
    );
    let cfg = module
        .value_use_cfg(fid(&module, "pass"))
        .expect("bounded ordinary CFG");
    assert!(cfg.blocks.values().any(|block| block.successors.len() == 2));
    assert!(cfg.value_uses.len() >= 2);
    assert!(
        cfg.value_uses
            .keys()
            .all(|block| cfg.blocks.contains_key(block))
    );
    assert!(
        cfg.value_uses
            .values()
            .flatten()
            .all(|event| event.operation != Op::Transfer)
    );
}

#[test]
fn unresolved_generic_never_becomes_copy_permission() {
    let module = compile("fn pass<T>(a: T) -> T { let b = a; return b; }");
    let receipts = uses(&module, "pass");
    assert!(
        receipts
            .iter()
            .all(|u| u.declaration_type.is_none() && u.event.operation == Op::Unknown)
    );
}

#[test]
fn malformed_site_or_permission_is_rejected_even_with_unchanged_body_hash() {
    let original = compile("fn pass(a: Int) -> Int { return a; }");
    let id = fid(&original, "pass");
    for corruption in 0..4 {
        let mut module = original.clone();
        let receipt = &mut module
            .functions
            .iter_mut()
            .find(|f| f.id == id)
            .unwrap()
            .value_uses
            .as_mut()
            .unwrap()
            .uses[0];
        match corruption {
            0 => receipt.event.operation = Op::Transfer,
            1 => receipt.operand = verum_vbc::instruction::Reg(65_535),
            2 => receipt.event.site = Site::Argument(0),
            _ => receipt.event.id.0 += 1,
        }
        // Model a producer that re-sealed a structurally invalid receipt: the
        // opcode/site/operand validator must work independently of hash checks.
        let descriptor = module.get_function(id).unwrap();
        let seal = verum_vbc::value_use::ValueUsePlan::signature_hash(
            descriptor,
            &descriptor.value_uses.as_ref().unwrap().uses,
        )
        .unwrap();
        module
            .functions
            .iter_mut()
            .find(|f| f.id == id)
            .unwrap()
            .value_uses
            .as_mut()
            .unwrap()
            .signature_hash = seal;
        assert!(
            module.value_use_receipts(id).is_none(),
            "corruption {corruption}"
        );
        assert!(
            module.value_use_cfg(id).is_none(),
            "corruption {corruption}"
        );
    }
}

#[test]
fn changed_signature_or_resource_descriptor_invalidates_semantic_permission() {
    let mut reference = compile("fn pass(a: &Int) -> &Int { a }");
    let id = fid(&reference, "pass");
    assert!(
        uses(&reference, "pass")
            .iter()
            .any(|u| u.event.operation == Op::Borrow)
    );
    reference
        .functions
        .iter_mut()
        .find(|f| f.id == id)
        .unwrap()
        .params[0]
        .type_ref = verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::INT);
    assert!(reference.value_use_receipts(id).is_none());

    let mut record = compile(&format!(
        "{RECORD} fn pass(a: Cell) -> Cell {{ let b = a; b }}"
    ));
    let id = fid(&record, "pass");
    let type_id = record
        .types
        .iter()
        .find(|t| record.get_string(t.name) == Some("Cell"))
        .unwrap()
        .id;
    record
        .types
        .iter_mut()
        .find(|t| t.id == type_id)
        .unwrap()
        .resource_discipline = verum_common::ResourceDiscipline::Affine;
    assert!(record.value_use_receipts(id).is_none());
}

#[test]
fn nested_function_snapshot_does_not_rollback_outer_use_or_binding_ids() {
    let module = compile(&format!(
        "{RECORD} fn pass(a: Cell) -> Cell {{ let before = a; fn nested(n: Int) -> Int {{ n }} let after = a; after }}"
    ));
    let outer = uses(&module, "pass");
    let copies: List<_> = outer
        .iter()
        .filter(|u| matches!(u.event.operation, Op::Copy(_)))
        .collect();
    assert_eq!(copies.len(), 2);
    assert_ne!(copies[0].event.id, copies[1].event.id);
    assert_eq!(copies[0].event.binding, copies[1].event.binding);
    assert_eq!(
        outer.iter().map(|u| u.event.id.0).collect::<List<_>>(),
        (0..outer.len() as u32).collect::<List<_>>()
    );
}

#[test]
fn specialization_or_linker_never_reseals_an_unmapped_plan() {
    let module = compile(
        "fn identity<T>(value: T) -> T { return value; } fn call(value: Int) -> Int { identity<Int>(value) }",
    );
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    let generic = fid(&module, "identity");
    assert!(!module.get_function(generic).unwrap().type_params.is_empty());
    graph.record_instantiation(
        generic,
        [verum_vbc::types::TypeRef::Concrete(
            verum_vbc::types::TypeId::INT,
        )]
        .into_iter()
        .collect::<List<_>>()
        .into(),
        verum_vbc::mono::SourceLocation::default(),
    );
    assert!(!graph.is_empty());
    let mono = verum_vbc::mono::monomorphize_minimal(module.clone(), &graph)
        .unwrap()
        .module;
    for function in &mono.functions {
        if !module.functions.iter().any(|old| old.id == function.id) {
            assert!(
                mono.value_use_receipts(function.id).is_none(),
                "specialization must publish its own evidence"
            );
        }
    }
    let mut linker = verum_vbc::linker::VbcLinker::new("aarch64-apple-darwin");
    linker.add_user_module(module).unwrap();
    let linked = linker.finalize();
    assert!(
        linked
            .functions
            .iter()
            .all(|f| linked.value_use_receipts(f.id).is_none())
    );
}

#[test]
fn pre_receipt_function_wire_reads_as_unknown() {
    let mut original = compile("fn fixed() -> Int { 7 }");
    let fixed = fid(&original, "fixed");
    // Synthetic intrinsic wrappers are unrelated to this legacy function fixture.
    original.functions.retain(|f| f.id == fixed);
    assert_eq!(original.functions.len(), 1);
    assert!(original.functions[0].value_uses.is_none());
    let mut bytes = verum_vbc::serialize::serialize_module(&original).unwrap();
    let read = |bytes: &[u8], at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let tail = read(&bytes, 32) as usize - 1;
    assert_eq!(bytes[tail], 0, "empty receipt tail");
    bytes.remove(tail);
    bytes[6..8].copy_from_slice(&19u16.to_le_bytes());
    for at in [16, 24, 32, 40, 48, 56, 64, 88] {
        let offset = read(&bytes, at);
        if offset > tail as u32 {
            bytes[at..at + 4].copy_from_slice(&(offset - 1).to_le_bytes());
        }
    }
    let hash = blake3::hash(&bytes[verum_vbc::format::HEADER_SIZE..]);
    bytes[72..80].copy_from_slice(&hash.as_bytes()[..8]);
    let legacy = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    assert!(legacy.functions[0].value_uses.is_none());
    assert!(legacy.value_use_cfg(legacy.functions[0].id).is_none());
}

#[test]
fn generic_parameter_shadowing_a_nominal_never_publishes_the_nominal() {
    for source in [
        "type T is { value: Int }; fn concrete(a: T) -> T { a } fn generic<T>(a: T) -> T { let b = a; b }",
        "fn generic<T>(a: T) -> T { let b = a; b } type T is { value: Int }; fn concrete(a: T) -> T { a }",
    ] {
        let module = compile(source);
        let receipts = uses(&module, "generic");
        assert!(
            receipts
                .iter()
                .all(|u| u.declaration_type.is_none() && u.event.operation == Op::Unknown),
            "{receipts:?}"
        );
    }
}

#[test]
fn changed_receipt_type_cannot_promote_an_unchanged_body_to_borrow() {
    let reference = compile("fn pass(a: &Int) -> &Int { a }");
    let borrowed_type = uses(&reference, "pass")[0].declaration_type.clone();
    let mut value = compile("fn pass(a: Int) -> Int { a }");
    let id = fid(&value, "pass");
    let receipt = &mut value
        .functions
        .iter_mut()
        .find(|f| f.id == id)
        .unwrap()
        .value_uses
        .as_mut()
        .unwrap()
        .uses[0];
    receipt.declaration_type = borrowed_type;
    receipt.event.operation = Op::Borrow;
    assert!(value.value_use_receipts(id).is_none());
}

#[test]
fn archive_equal_numeric_type_ids_keep_exact_owner_and_discipline_in_both_orders() {
    let modules: List<_> = [
        ("alpha", "module alpha; public type affine Cell is { value: Int }; public fn pass(a: Cell) -> Cell { let b = a; b }"),
        ("beta", "module beta; public type Cell is { value: Int }; public fn pass(a: Cell) -> Cell { let b = a; b }"),
    ].into_iter().map(|(name, source)| {
        let ast = Parser::new(source).parse_module().unwrap();
        let module = VbcCodegen::with_config(CodegenConfig::new(name)).compile_module(&ast).unwrap();
        verum_vbc::deserialize::deserialize_module(&verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap()
    }).collect();
    let cell = |module: &VbcModule| {
        module
            .types
            .iter()
            .find(|t| module.get_string(t.name) == Some("Cell"))
            .unwrap()
            .id
    };
    assert_eq!(
        cell(&modules[0]),
        cell(&modules[1]),
        "control must exercise archive-local ID collision"
    );
    for order in [[0, 1], [1, 0]] {
        let mut consumer = VbcCodegen::with_config(CodegenConfig::new("consumer"));
        for index in order {
            let module = &modules[index];
            consumer.import_archive_module_types(module);
            let remap: Map<_, _> = module
                .functions
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    (
                        f.id.0,
                        verum_vbc::FunctionId(5000 + index as u32 * 100 + i as u32),
                    )
                })
                .collect();
            consumer.merge_archive_function_bodies(module, &remap.into());
        }
        let module = consumer.finalize_module_from_state().unwrap();
        for (name, copies) in [("alpha.pass", false), ("beta.pass", true)] {
            let id = module.find_function_by_name(name).unwrap();
            let receipts = module.value_use_receipts(id).expect(name);
            assert_eq!(
                receipts
                    .iter()
                    .any(|u| matches!(u.event.operation, Op::Copy(_))),
                copies,
                "{name}"
            );
        }
    }
}
