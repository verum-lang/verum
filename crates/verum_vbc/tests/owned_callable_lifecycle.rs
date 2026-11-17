//! Formation owns a snapshot; copying a callable shares that owned environment.
#![cfg(feature = "codegen")]
#[path = "fixtures/canonical_drop.rs"]
mod canonical_drop;

use verum_fast_parser::Parser;
use verum_vbc::{Instruction, VbcModule, codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    deserialize::deserialize_module_validated, interpreter::Interpreter, serialize::serialize_module};

const DECLARATIONS: &str = r#"
mount core.base.protocols.Drop;
type Count is { value: Int };
type Payload is { count: &mut Count, value: Int };
implement Drop for Payload {
    fn drop(&mut self) { self.count.value = self.count.value + 1; }
}
fn make(count: &mut Count) -> fn() -> Int {
    let payload = Payload { count, value: 73 };
    || payload.value
}
fn invoke(callback: fn() -> Int) -> Int { callback() }
fn wrap(callback: fn() -> Int) -> fn() -> Int { || callback() }
"#;

fn compile(source: &str, unit: bool) -> VbcModule {
    let source = format!("{DECLARATIONS}\n{source}");
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("owned_callables"));
    canonical_drop::initialize(&mut codegen);
    if unit {
        codegen.collect_unit_declarations(&[&ast]).unwrap();
        codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
        codegen.finalize_module_from_state().unwrap()
    } else { codegen.compile_additional_module(&ast).unwrap() }
}

fn execute(module: VbcModule) -> Result<i64, String> {
    let entry = module.find_function_by_name("owned_callables.probe").unwrap();
    Interpreter::try_new_validated(module.into()).unwrap().execute_function(entry)
        .map(|value| value.try_as_i64().expect("Int result")).map_err(|error| error.to_string())
}

fn check(source: &str) {
    for unit in [false, true] {
        let module = compile(source, unit);
        let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
        for (route, module) in [("source", module), ("wire", wire)] {
            assert_eq!(execute(module).unwrap(), 73, "unit={unit}, {route}");
        }
    }
}

#[test]
fn escaped_capture_survives_its_source_owner_and_shared_handles_drop_once() {
    check(r#"
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let callback = make(&mut count);
                if count.value != 1 { return 10 + count.value; }
                {
                    let copied = callback;
                    if invoke(copied) != 73 { return 20; }
                }
                if count.value != 1 { return 30 + count.value; }
                if callback() != 73 { return 40; }
            }
            if count.value == 2 { 73 } else { 50 + count.value }
        }
    "#);
}

#[test]
fn captured_callable_keeps_its_owned_environment_alive() {
    check(r#"
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let callback = wrap(make(&mut count));
                if count.value != 1 { return 10 + count.value; }
                if invoke(callback) != 73 { return 20; }
                if callback() != 73 { return 30; }
            }
            if count.value == 2 { 73 } else { 40 + count.value }
        }
    "#);
}

#[test]
fn immutable_capture_formation_copies_nested_owned_values() {
    check(r#"
        type Outer is { payload: Payload };
        fn nested(count: &mut Count) -> fn() -> Int {
            let outer = Outer { payload: Payload { count, value: 73 } };
            || outer.payload.value
        }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            { let callback = nested(&mut count); if callback() != 73 { return 10; } }
            if count.value == 2 { 73 } else { 20 + count.value }
        }
    "#);
}

#[test]
fn callable_field_binding_retains_the_environment_owned_by_the_record() {
    check(r#"
        type Factory is { create: fn() -> Int };
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let factory = Factory { create: make(&mut count) };
                { let callback = factory.create; if callback() != 73 { return 10; } }
                if count.value != 1 { return 20 + count.value; }
                if (factory.create)() != 73 { return 30; }
            }
            if count.value == 2 { 73 } else { 40 + count.value }
        }
    "#);
}

#[test]
fn an_earlier_field_destructor_observes_the_not_yet_retired_sibling() {
    check(r#"
        type Watch is { sibling: &Int, count: &mut Count };
        type Slot is Vacant | Full(Watch);
        type Pair is { first: Slot, second: Int };
        implement Drop for Watch {
            fn drop(&mut self) { self.count.value = *self.sibling; }
        }
        fn probe() -> Int {
            let mut count = Count { value: 0 };
            {
                let mut pair = Pair { first: Vacant, second: 73 };
                pair.first = Full(Watch { sibling: &pair.second, count: &mut count });
            }
            count.value
        }
    "#);
}

#[test]
fn owned_capture_instructions_require_their_declared_wire_version() {
    for unit in [false, true] {
        let module = compile("fn probe()->Int {let value=73; let callback=||value; invoke(callback)}", unit);
        let current = serialize_module(&module).unwrap();
        let floor = verum_vbc::format::OWNED_CLOSURE_ENVIRONMENT_VERSION;
        for minor in [floor - 1, floor] {
            let mut direct = module.clone();
            direct.header.version_minor = minor;
            let direct_result = verum_vbc::validate::validate_module(&direct);
            let mut wire = current.clone();
            wire[6..8].copy_from_slice(&minor.to_le_bytes());
            let wire_result = deserialize_module_validated(&wire);
            if minor < floor {
                for error in [direct_result.unwrap_err(), wire_result.unwrap_err()] {
                    assert!(error.to_string().contains("owned closure environments require VBC minor 63"), "{error}");
                }
            } else {
                direct_result.unwrap();
                assert_eq!(execute(wire_result.unwrap()).unwrap(), 73);
            }
        }
    }
}

#[test]
fn a_legacy_capture_flag_cannot_authorize_shared_ownership() {
    for unit in [false, true] {
        let mut module = compile("fn probe()->Int {let value=73; let callback=||value; invoke(callback)}", unit);
        let original = std::mem::take(&mut module.bytecode);
        let mut changed = 0;
        for function in &mut module.functions {
            let start = function.bytecode_offset as usize;
            let mut body = verum_vbc::bytecode::decode_instructions(
                &original[start..start + function.bytecode_length as usize]).unwrap();
            verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
            for instruction in &mut body {
                if let Instruction::NewClosure { captures, owns_captures, .. } = instruction
                    && !captures.is_empty() {
                    *owns_captures = false;
                    changed += 1;
                }
            }
            verum_vbc::bytecode::fixup_jump_offsets(&mut body);
            function.bytecode_offset = module.bytecode.len() as u32;
            for instruction in body { verum_vbc::bytecode::encode_instruction(&instruction, &mut module.bytecode); }
            function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
            function.instructions = None;
        }
        assert!(changed > 0);
        let wire = deserialize_module_validated(&serialize_module(&module).unwrap()).unwrap();
        for module in [module, wire] {
            let error = execute(module).unwrap_err();
            assert!(error.contains("captured environment requires an owned lifecycle"), "{error}");
        }
    }
}
