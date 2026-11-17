//! Arithmetic selects the declared protocol even when another protocol uses the same method name.
#![cfg(feature = "codegen")]

use std::sync::Arc;
use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    instruction::{BinaryGenericOp, Instruction, Reg}, VbcModule,
};

const SOURCE: &str = r#"
module roles;
type Decoy is protocol { fn add(self, rhs: Self) -> Bool; };
type Add<Rhs=Self> is protocol { type Output; fn add(self, rhs:Rhs) -> Self.Output; };
type Sub<Rhs=Self> is protocol { type Output; fn sub(self, rhs:Rhs) -> Self.Output; };
type Mul<Rhs=Self> is protocol { type Output; fn mul(self, rhs:Rhs) -> Self.Output; };
type Div<Rhs=Self> is protocol { type Output; fn div(self, rhs:Rhs) -> Self.Output; };
type Number is { n: Int };
implement Decoy for Number { fn add(self, rhs: Self) -> Bool { false } }
implement Add for Number { type Output=Number; fn add(self, rhs: Self) -> Number { Number { n: self.n + rhs.n } } }
implement Sub for Number { type Output=Number; fn sub(self, rhs: Self) -> Number { Number { n: self.n - rhs.n } } }
implement Mul for Number { type Output=Number; fn mul(self, rhs: Self) -> Number { Number { n: self.n * rhs.n } } }
implement Div for Number { type Output=Number; fn div(self, rhs: Self) -> Number { Number { n: self.n / rhs.n } } }
fn known(x: Number, y: Number) -> Number { x + y }
fn generic<T:Add<Output=T>>(x:T, y:T) -> T { x+y }
fn actual() -> Int {
    let a: Number=known(Number{n:12}, Number{n:3});
    let b: Number=generic(Number{n:12}, Number{n:3});
    a.n * 100 + b.n
}
"#;

fn module() -> VbcModule {
    module_from_source(SOURCE)
}

fn module_from_source(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("roles"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_default_methods_with_policy(ItemFailurePolicy::Strict).unwrap();
    codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn run(module: VbcModule) -> Result<Option<i64>, String> {
    let module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    run_source(module)
}

fn run_source(mut module: VbcModule) -> Result<Option<i64>, String> {
    assert!(module.resolve_external_bands().is_empty());
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("roles.actual").unwrap();
    verum_vbc::interpreter::Interpreter::new(Arc::new(module)).execute_function(entry)
        .map(|value| value.try_as_i64()).map_err(|error| error.to_string())
}

#[test]
fn explicit_generic_method_uses_the_declaring_protocol_on_source_and_wire() {
    let source = SOURCE.replace("{ x+y }", "{ x.add(y) }");
    assert_ne!(source, SOURCE);
    let module = module_from_source(&source);
    assert_eq!(run_source(module.clone()).unwrap(), Some(1515));
    assert_eq!(run(module).unwrap(), Some(1515));
}

#[test]
fn declared_generic_operator_distinguishes_rhs_implementations_between_calls() {
    let module = module_from_source(r#"
        module roles;
        type Add<Rhs = Self> is protocol {
            type Output;
            fn add(self, rhs: Rhs) -> Self.Output;
        };
        type Number is { n: Int };
        type Offset is { n: Int };
        implement Add<Offset> for Number {
            type Output = Number;
            fn add(self, rhs: Offset) -> Number { Number { n: self.n + rhs.n + 100 } }
        }
        implement Add<Number> for Number {
            type Output = Number;
            fn add(self, rhs: Number) -> Number { Number { n: self.n + rhs.n } }
        }
        fn generic<R, T: Add<R, Output = T>>(left: T, right: R) -> T { left + right }
        fn actual() -> Int {
            let a = generic(Number { n: 12 }, Offset { n: 3 });
            let b = generic(Number { n: 12 }, Number { n: 3 });
            a.n * 100 + b.n
        }
    "#);
    assert_eq!(run_source(module.clone()).unwrap(), Some(11515));
    assert_eq!(run(module).unwrap(), Some(11515));
}

#[test]
fn a_qualified_protocol_miss_cannot_fall_back_to_another_declaration() {
    use verum_vbc::bytecode::{
        decode_instructions, encode_instructions_with_fixup, jump_offsets_to_instr_indices,
    };
    for wrong in ["dyn:foreign.Add.add", "dyn:roles.Mul.add"] {
        let mut module = module();
        let entry = module.find_function_by_name("roles.generic").unwrap();
        let function = &module.functions[entry.0 as usize];
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut body = decode_instructions(&module.bytecode[start..end]).unwrap();
        jump_offsets_to_instr_indices(&mut body);
        let replacement = module.strings.intern(wrong);
        let mut changed = 0;
        for instruction in &mut body {
            if let Instruction::CallM { method_id, .. } = instruction
                && module.get_string(verum_vbc::types::StringId(*method_id)) == Some("dyn:roles.Add.add")
            {
                *method_id = replacement.0;
                changed += 1;
            }
        }
        assert_eq!(changed, 1);
        let function = &mut module.functions[entry.0 as usize];
        function.instructions = None;
        function.value_uses = None;
        function.bytecode_offset = module.bytecode.len() as u32;
        encode_instructions_with_fixup(&body, &mut module.bytecode);
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        for result in [run_source(module.clone()), run(module)] {
            let error = result.expect_err("a different protocol cannot authorize the call");
            assert!(error.contains("no implementation of declared protocol member"), "{wrong}: {error}");
        }
    }
}

#[test]
fn direct_and_parametric_operators_use_their_declaring_implementation() {
    assert_eq!(run(module()).unwrap(), Some(1515));
}

#[test]
fn serialized_generic_instructions_consume_and_check_the_protocol_operand() {
    let base = module();
    for (op, protocol, first) in [
        (BinaryGenericOp::Add, "Add", 15),
        (BinaryGenericOp::Sub, "Sub", 9),
        (BinaryGenericOp::Mul, "Mul", 36),
        (BinaryGenericOp::Div, "Div", 4),
        (BinaryGenericOp::Add, "Decoy", 0),
    ] {
        let mut module = base.clone();
        let id = module.types.iter().find(|ty| module.get_string(ty.name) == Some(protocol)).unwrap().id;
        let entry = module.find_function_by_name("roles.known").unwrap();
        let function = &mut module.functions[entry.0 as usize];
        function.instructions = None;
        function.value_uses = None;
        function.bytecode_offset = module.bytecode.len() as u32;
        function.register_count = function.register_count.max(3);
        verum_vbc::bytecode::encode_instruction(&Instruction::BinaryG {
            op, dst: Reg(2), a: Reg(0), b: Reg(1), protocol_id: id.0,
        }, &mut module.bytecode);
        verum_vbc::bytecode::encode_instruction(&Instruction::Ret { value: Reg(2) }, &mut module.bytecode);
        function.bytecode_length = module.bytecode.len() as u32 - function.bytecode_offset;
        let actual = run(module);
        if protocol == "Decoy" {
            assert!(actual.unwrap_err().contains("different protocol"));
        } else {
            assert_eq!(actual.unwrap(), Some(first * 100 + 15), "{protocol}");
        }
    }
}
