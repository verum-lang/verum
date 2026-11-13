//! Execute the standard library's declared comparison bodies and inherited defaults.
#![cfg(target_os = "macos")]

use std::process::Command;
use verum_ast::{ItemKind, decl::ImplKind};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
};
use verum_vbc::{
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    mono::{InstantiationGraph, discover_call_instantiations, monomorphize_minimal},
    serialize::serialize_module,
};

#[test]
fn declared_comparison_and_inherited_inequality_execute_in_a_native_image() {
    let mut module = Parser::new(include_str!("../../../core/base/protocols.vr"))
        .parse_module()
        .expect("protocol source");
    module.items.retain(|item| {
        matches!(&item.kind,
        ItemKind::Type(declaration) if matches!(declaration.name.as_str(), "PartialEq" | "Eq"))
    });
    assert_eq!(module.items.len(), 2);
    let csv_source = include_str!("../../../core/database/sqlite/native/vtabs/csv.vr");
    let csv = Parser::new(csv_source).parse_module().expect("CSV source");
    for item in csv.items {
        let selected = match &item.kind {
            ItemKind::Type(declaration) => declaration.name.as_str() == "CsvError",
            ItemKind::Impl(declaration) => match &declaration.kind {
                ImplKind::Protocol {
                    protocol, for_type, ..
                } => {
                    &csv_source[for_type.span.start as usize..for_type.span.end as usize]
                        == "CsvError"
                        && matches!(protocol.to_string().as_str(), "PartialEq" | "Eq")
                }
                _ => false,
            },
            _ => false,
        };
        if selected {
            module.items.push(item);
        }
    }
    assert_eq!(
        module.items.len(),
        5,
        "actual error and both protocol implementations"
    );
    let probe = Parser::new(r#"
        type Count is { value: Int };
        type Counts is { left: Count, right: Count };
        implement Count {
            fn equal(&self, other: &Count) -> Bool { self.value == other.value }
        }
        fn compare_counts(left: &Count, right: &Count) -> Bool { left.equal(right) }
        fn compare_fields(values: &Counts) -> Bool { values.left.equal(&values.right) }
        fn compare_fields_direct(values: &Counts) -> Bool {
            compare_counts(&values.left, &values.right)
        }
        fn compare<T: PartialEq>(left: &T, right: &T) -> Bool { left.eq(right) }
        fn reflexive<T: Eq>(value: &T) -> Bool { value.eq(value) }
        fn main() -> Int {
            let first = CeUnclosedQuote(3);
            let different_value = CeUnclosedQuote(4);
            let different_variant = CeBadEscape(3);
            if !first.eq(&first) || first.eq(&different_value) || first.eq(&different_variant) { return 1; }
            if first.ne(&first) || !first.ne(&different_value) || !first.ne(&different_variant) { return 2; }
            if !compare(&first, &first) || compare(&first, &different_value) { return 3; }
            if !reflexive(&first) || !reflexive(&CeShortRow(8)) { return 4; }
            let equal = Counts { left: Count { value: 37 }, right: Count { value: 37 } };
            let unequal = Counts { left: Count { value: 37 }, right: Count { value: 91 } };
            if !compare_fields(&equal) || compare_fields(&unequal) { return 5; }
            if !compare_fields_direct(&equal) || compare_fields_direct(&unequal) { return 6; }
            73
        }
    "#).parse_module().expect("probe grammar");
    module.items.extend(probe.items);
    let original = VbcCodegen::new()
        .compile_module(&module)
        .expect("actual source VBC");
    let mut wire =
        deserialize_module(&serialize_module(&original).expect("encode")).expect("decode");
    let mut graph = InstantiationGraph::new();
    for function in &mut wire.functions {
        function.is_generic = !function.type_params.is_empty();
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .expect("loaded body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    for function in &wire.functions {
        discover_call_instantiations(
            &wire,
            function.instructions.as_ref().unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .expect("declared generic call witnesses");
    }
    assert!(!graph.is_empty());
    let specialized = monomorphize_minimal(wire, &graph)
        .expect("specialize")
        .module;
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple)
        .unwrap()
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("stdlib_comparison").with_debug_info(false),
    );
    lowering
        .lower_module(&specialized)
        .expect("native lowering");
    let native = lowering.module();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    native.verify().expect("complete native IR");
    let directory = tempfile::tempdir().unwrap();
    let object = directory.path().join("comparison.o");
    let executable = directory.path().join("comparison");
    machine
        .write_to_file(native, FileType::Object, &object)
        .expect("object");
    let link = Command::new("cc")
        .arg(object)
        .args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"])
        .arg(&executable)
        .output()
        .expect("system linker");
    assert!(
        link.status.success(),
        "{}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(executable).output().expect("native execution");
    assert_eq!(
        run.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}
