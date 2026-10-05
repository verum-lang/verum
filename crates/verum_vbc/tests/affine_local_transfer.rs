//! T1602: consuming local initialization transfers the existing cleanup obligation.
#![cfg(feature = "codegen")]
use std::sync::Arc;
use verum_common::Text;
use verum_fast_parser::Parser;
use verum_vbc::{
    VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

const DECLARATIONS: &str = r#"
type Count is { value: Int };
type affine Watch is { counter: &mut Count, digit: Int };
implement Drop for Watch {
    fn drop(&mut self) { self.counter.value = self.counter.value * 10 + self.digit; }
}
"#;

fn compile(source: &str, bootstrap: bool) -> VbcModule {
    let ast = Parser::new(source)
        .parse_module()
        .expect("grammar-valid source");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("transfer"));
    if bootstrap {
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen
            .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
            .unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else {
        codegen.compile_module(&ast).expect("compile")
    }
}
fn run(source: &str, bootstrap: bool) -> i64 {
    let module = compile(source, bootstrap);
    let id = module.find_function_by_name("transfer.probe").unwrap();
    Interpreter::new(Arc::new(module))
        .execute_function(id)
        .expect("execute")
        .as_i64()
}
fn watch(source: &str) -> Text {
    format!("{DECLARATIONS}\n{source}").into()
}

#[test]
fn local_transfer_drops_only_the_destination_in_both_source_routes() {
    let source = watch(
        r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let original: Watch = Watch { counter: &mut counter, digit: 1 };
        { let moved = original; if counter.value != 0 { return 99; } }
        if counter.value != 1 { return 98; }
    }
    counter.value
}
"#,
    );
    for bootstrap in [false, true] {
        assert_eq!(run(&source, bootstrap), 1, "bootstrap={bootstrap}");
    }
}

#[test]
fn each_exclusive_branch_transfers_and_join_does_not_drop_the_source_again() {
    for flag in ["true", "false"] {
        let source = watch(&format!(
            r#"
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{
        let original: Watch = Watch {{ counter: &mut counter, digit: 2 }};
        if {flag} {{ let moved = original; }} else {{ let other = original; }}
        if counter.value != 2 {{ return 99; }}
    }}
    counter.value
}}
"#
        ));
        assert_eq!(run(&source, false), 2, "flag={flag}");
    }
}

#[test]
fn transfer_in_one_branch_leaves_cleanup_active_on_the_other_path() {
    for flag in ["true", "false"] {
        let source = watch(&format!(
            r#"
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{
        let original: Watch = Watch {{ counter: &mut counter, digit: 3 }};
        if {flag} {{ let moved = original; }}
    }}
    counter.value
}}
"#
        ));
        assert_eq!(run(&source, false), 3, "flag={flag}");
    }
}

#[test]
fn same_name_shadow_and_reused_scope_registers_keep_distinct_obligations() {
    let source = watch(
        r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let original: Watch = Watch { counter: &mut counter, digit: 1 }; { let original = original; } }
    { let original: Watch = Watch { counter: &mut counter, digit: 2 }; let moved = original; }
    counter.value
}
"#,
    );
    assert_eq!(run(&source, false), 12);
}

#[test]
fn ordinary_copy_and_reference_borrow_do_not_consume_the_source() {
    let ordinary = Text::from(DECLARATIONS.replace("type affine Watch", "type Watch"));
    let source: Text = format!(r#"{ordinary}
fn probe() -> Int {{
    let mut counter = Count {{ value: 0 }};
    {{ let original: Watch = Watch {{ counter: &mut counter, digit: 1 }}; {{ let copied = original; }} }}
    counter.value
}}
"#).into();
    assert_eq!(run(&source, false), 11);
    let source = watch(
        r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    {
        let original: Watch = Watch { counter: &mut counter, digit: 2 };
        { let borrowed: &Watch = &original; let copied_borrow = borrowed; }
        if counter.value != 0 { return 99; }
    }
    counter.value
}
"#,
    );
    assert_eq!(run(&source, false), 2);
}

#[test]
fn explicit_destination_drop_does_not_revive_the_source_cleanup() {
    let source = watch(
        r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let original: Watch = Watch { counter: &mut counter, digit: 4 }; let moved = original; drop(moved); }
    counter.value
}
"#,
    );
    assert_eq!(run(&source, false), 4);
}

#[test]
fn repeated_loop_allocations_get_distinct_runtime_obligations() {
    let source = watch(
        r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    let mut digit = 1;
    while digit <= 3 {
        let original: Watch = Watch { counter: &mut counter, digit };
        let moved = original;
        digit += 1;
    }
    counter.value
}
"#,
    );
    assert_eq!(run(&source, false), 123);
}

#[test]
fn unresolved_shared_generic_and_reference_inputs_do_not_get_source_clears() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, instruction::Reg};
    // Shared here has no imported descriptor: this is an unknown-owner negative,
    // not proof of the actual standard-library retain/release implementation.
    for source in [
        "fn probe(original: Shared<Int>) -> Shared<Int> { let copied = original; original }",
        "fn probe<T>(original: T) -> T { let copied = original; original }",
        "type affine Token is { value: Int }; fn probe(original: &Token) -> &Token { let copied = original; original }",
        "type affine Token is { value: Int }; fn probe(original: *const Token) -> *const Token { let copied = original; original }",
    ] {
        let module = compile(source, false);
        let id = module.find_function_by_name("transfer.probe").unwrap();
        let f = module.get_function(id).unwrap();
        let ops = decode_instructions(
            &module.bytecode
                [f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize],
        )
        .unwrap();
        assert!(
            !ops.iter()
                .any(|i| matches!(i, Instruction::LoadUnit { dst: Reg(0) })),
            "{source}: {ops:?}"
        );
    }
}

#[test]
fn emitted_handoff_survives_wire_without_observation_receipts() {
    let source = watch(
        r#"
fn probe() -> Int {
    let mut counter = Count { value: 0 };
    { let original: Watch = Watch { counter: &mut counter, digit: 5 }; let moved = original; }
    counter.value
}
"#,
    );
    let mut module = compile(&source, false);
    for f in &mut module.functions {
        f.value_uses = None;
    }
    let bytes = verum_vbc::serialize::serialize_module(&module).unwrap();
    let loaded = verum_vbc::deserialize::deserialize_module(&bytes).unwrap();
    let id = loaded.find_function_by_name("transfer.probe").unwrap();
    assert_eq!(
        Interpreter::new(Arc::new(loaded))
            .execute_function(id)
            .unwrap()
            .as_i64(),
        5
    );
}

#[test]
fn declaration_owner_controls_transfer_in_both_registration_orders() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, instruction::Reg};
    let affine = "module alpha { public type affine Token is { value: Int }; }";
    let ordinary = "module beta { public type Token is { value: Int }; }";
    for reverse in [false, true] {
        let source: Text = format!("{} {} fn consuming(original: alpha.Token) {{ let moved = original; }} fn ordinary(original: beta.Token) {{ let copied = original; }}", if reverse {ordinary} else {affine}, if reverse {affine} else {ordinary}).into();
        let module = compile(&source, false);
        for (name, consumes) in [("consuming", true), ("ordinary", false)] {
            let id = module
                .find_function_by_name(&format!("transfer.{name}"))
                .unwrap();
            let f = module.get_function(id).unwrap();
            let ops = decode_instructions(
                &module.bytecode
                    [f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize],
            )
            .unwrap();
            assert_eq!(
                ops.iter()
                    .any(|i| matches!(i, Instruction::LoadUnit { dst: Reg(0) })),
                consumes,
                "reverse={reverse}, {name}: {ops:?}"
            );
        }
    }
}

#[test]
fn borrowing_and_at_patterns_are_not_simple_consuming_bindings() {
    use verum_vbc::{Instruction, bytecode::decode_instructions, instruction::Reg};
    for pattern in ["ref borrowed", "whole @ Token { value }"] {
        let source: Text = format!("type affine Token is {{ value: Int }}; fn probe(original: Token) {{ let {pattern} = original; }}").into();
        let module = compile(&source, false);
        let id = module.find_function_by_name("transfer.probe").unwrap();
        let f = module.get_function(id).unwrap();
        let ops = decode_instructions(
            &module.bytecode
                [f.bytecode_offset as usize..(f.bytecode_offset + f.bytecode_length) as usize],
        )
        .unwrap();
        assert!(
            !ops.iter()
                .any(|i| matches!(i, Instruction::LoadUnit { dst: Reg(0) })),
            "{pattern}: {ops:?}"
        );
    }
}
