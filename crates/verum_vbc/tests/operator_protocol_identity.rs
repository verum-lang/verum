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
    let ast = Parser::new(SOURCE).parse_module().unwrap();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("roles"));
    codegen.collect_unit_declarations(&[&ast]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_default_methods_with_policy(ItemFailurePolicy::Strict).unwrap();
    codegen.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
    codegen.finalize_module_from_state().unwrap()
}

fn run(module: VbcModule) -> Result<Option<i64>, String> {
    let mut module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap()).unwrap();
    assert!(module.resolve_external_bands().is_empty());
    verum_vbc::validate::validate_module(&module).unwrap();
    let entry = module.find_function_by_name("roles.actual").unwrap();
    verum_vbc::interpreter::Interpreter::new(Arc::new(module)).execute_function(entry)
        .map(|value| value.try_as_i64()).map_err(|error| error.to_string())
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
