//! Concrete conditional counts retain exact layout witnesses through the wire.
#![cfg(feature = "codegen")]
use verum_common::{Heap, Shared, Text};
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::VbcCodegen,
    instruction::Instruction,
    interpreter::Interpreter,
    types::{TypeId, TypeRef},
};

#[test]
fn conditional_counts_execute_exact_witnesses_after_wire_roundtrip() {
    for (count, expected) in [
        ("if 2 > 1 {3} else {4}", 3),
        ("if false {1 / 0} else {4}", 4),
        ("if false && 1 / 0 > 0 {1} else {4}", 4),
        ("if true || 1 / 0 > 0 {3} else {4}", 3),
        ("{let n = 2; if n == 2 {n + 1} else {4}}", 3),
        ("if (true) {if false {9} else {3}} else {4}", 3),
        ("{let n = 2; {let n = 9; n}; n + 1}", 3),
        ("if false {9} else {0}", 0),
    ] {
        let source =
            format!("fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}");
        let ast = Parser::new(&source).parse_module().expect("source grammar");
        let module = VbcCodegen::new().compile_module(&ast).expect(count);
        let bytes = verum_vbc::serialize::serialize_module(&module).expect("wire writer");
        let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("wire reader");
        let id = module.find_function_by_name("probe").expect("probe");
        let function = module.functions.iter().find(|f| f.id == id).unwrap();
        let instructions = verum_vbc::bytecode::decode_instructions(
            &module.bytecode[function.bytecode_offset as usize
                ..(function.bytecode_offset + function.bytecode_length) as usize],
        )
        .unwrap();
        let witness = TypeRef::Array {
            element: Heap::new(TypeRef::Concrete(TypeId::U8)),
            length: expected as u64,
        };
        assert!(instructions.iter().any(|i| matches!(i, Instruction::CallG {type_args, ..} if type_args == &[witness.clone()])), "{count}: {instructions:?}");
        assert_eq!(
            Interpreter::new(Shared::new(module).into_arc())
                .execute_function(id)
                .expect("layout execution")
                .as_i64(),
            expected,
            "{count}"
        );
    }
}

#[test]
fn invalid_selected_counts_and_unknown_values_are_refused() {
    for count in [
        "if true {1 / 0} else {4}",
        "if true {-1} else {4}",
        "if true {9223372036854775807 + 1 - 1} else {4}",
        "if true {1 << 63 >> 63} else {4}",
        "if 1 {3} else {4}",
        "if false {3}",
        "{let mut n = 2; n = 3; n}",
        "if missing {3} else {4}",
    ] {
        let source =
            format!("fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}");
        let ast = Parser::new(&source).parse_module().expect("source grammar");
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "invalid count accepted: {count}"
        );
    }
}

#[test]
fn owner_scoped_conditional_constants_survive_wire() {
    use verum_vbc::codegen::{CodegenConfig, ItemFailurePolicy};
    for order in [false, true] {
        let origin = Parser::new("module origin; const BASE: Int = 3; public const SIZE: Int = if BASE > 0 {{let extra = 1; BASE + extra}} else {8};").parse_module().unwrap();
        let caller = Parser::new("module caller; const BASE: Int = 7; fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; origin.SIZE]>()}").parse_module().unwrap();
        let units = if order {
            [&origin, &caller]
        } else {
            [&caller, &origin]
        };
        let mut codegen = VbcCodegen::with_config(CodegenConfig::new("conditional_owners"));
        codegen.collect_unit_declarations(&units).unwrap();
        codegen
            .compile_unit_items(&units, ItemFailurePolicy::Strict)
            .unwrap();
        let module = codegen.finalize_module().unwrap();
        let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
        let module = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
        let id = module
            .functions
            .iter()
            .find(|function| {
                module
                    .get_string(function.name)
                    .is_some_and(|name| name == "probe" || name.ends_with(".probe"))
            })
            .unwrap()
            .id;
        assert_eq!(
            Interpreter::new(Shared::new(module).into_arc())
                .execute_function(id)
                .unwrap()
                .as_i64(),
            4
        );
    }
}

#[test]
fn named_conditionals_preserve_checked_errors_and_lazy_branches() {
    for initializer in [
        "if true {3} else {1 / 0}",
        "if false && 1 / 0 > 0 {8} else {3}",
        "if FLAG {3} else {1 / 0}",
    ] {
        let source = format!(
            "const FLAG: Bool = true; const N: Int = {initializer}; fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; N]>()}}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        let module = VbcCodegen::new().compile_module(&ast).expect(initializer);
        let id = module.find_function_by_name("probe").unwrap();
        assert_eq!(
            Interpreter::new(Shared::new(module).into_arc())
                .execute_function(id)
                .unwrap()
                .as_i64(),
            3
        );
    }
    for initializer in [
        "if true {9223372036854775807 + 1 - 1} else {4}",
        "if true {1 << 63 >> 63} else {4}",
        "if true {N} else {3}",
    ] {
        let source = format!(
            "const N: Int = {initializer}; fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; N]>()}}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "{initializer}"
        );
    }
}

#[test]
fn unknown_owners_and_arbitrary_calls_do_not_supply_counts() {
    for count in ["missing.N", "if true {ordinary(3)} else {4}", "N"] {
        let source = format!(
            "fn ordinary(value: Int)->Int {{value}} const N: Int = ordinary(3); fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        assert!(VbcCodegen::new().compile_module(&ast).is_err(), "{count}");
    }
}

#[test]
fn block_local_projections_cannot_use_an_unrelated_global_type() {
    for count in [
        "{let Foo = 3; Foo.size}",
        "{let Foo = 3; offset_of(Foo, value)}",
        "{let Foo = 3; ([Foo; 2]).size}",
    ] {
        let source = format!(
            "type Foo is {{value: Int}}; fn size<T>()->Int {{T.size}} fn probe()->Int {{size<[Byte; {count}]>()}}"
        );
        let ast = Parser::new(&source).parse_module().unwrap();
        assert!(
            VbcCodegen::new().compile_module(&ast).is_err(),
            "local projection accepted: {count}"
        );
    }
}

#[test]
fn repeated_boolean_dependencies_are_memoized_and_cycles_refused() {
    let mut source = Text::from("const FLAG0: Bool = true;");
    for index in 1..30 {
        source.push_str(&format!(
            "const FLAG{index}: Bool = FLAG{} && FLAG{};",
            index - 1,
            index - 1
        ));
    }
    source.push_str(
        "fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; if FLAG29 {3} else {4}]>()}",
    );
    let ast = Parser::new(&source).parse_module().unwrap();
    let module = VbcCodegen::new()
        .compile_module(&ast)
        .expect("bounded Bool dependency DAG");
    let id = module.find_function_by_name("probe").unwrap();
    assert_eq!(
        Interpreter::new(Shared::new(module).into_arc())
            .execute_function(id)
            .unwrap()
            .as_i64(),
        3
    );
    let cyclic = "const FIRST: Bool = SECOND; const SECOND: Bool = FIRST; fn size<T>()->Int {T.size} fn probe()->Int {size<[Byte; if FIRST {3} else {4}]>()}";
    assert!(
        VbcCodegen::new()
            .compile_module(&Parser::new(cyclic).parse_module().unwrap())
            .is_err()
    );
}
