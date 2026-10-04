//! T1533: explicit method arguments determine the result's declared layout.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    bytecode::decode_instruction,
    codegen::{CodegenConfig, VbcCodegen},
    instruction::Instruction,
    module::VbcModule,
};

const SOURCE: &str = r#"
type Left is { value: Int, padding: Int };
type Right is { padding: Int, value: Int };
implement Left {
    fn from_value(value: Int) -> Left { Left { value, padding: 99 } }
    fn answer(&self) -> Int { self.value }
}
implement Right {
    fn from_value(value: Int) -> Right { Right { padding: 99, value } }
    fn answer(&self) -> Int { self.value }
}
type Factory<Item> is { value: Item };
implement<Item> Factory<Item> {
    fn build<Output>(self) -> Output { Output.from_value(self.value) }
    fn shadow<Item>(self) -> Item { Item.from_value(self.value) }
}
"#;

fn compile(body: &str) -> VbcModule {
    let source = format!("{SOURCE}\nfn probe() -> Int {{ {body} }}");
    let ast = Parser::new(&source).parse_module().expect("parse");
    VbcCodegen::with_config(CodegenConfig::new("explicit_result"))
        .compile_module(&ast)
        .expect("compile")
}

fn assert_layout(body: &str, index: u32) {
    let module = compile(body);
    let function = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|name| name.ends_with(".probe"))
        })
        .expect("qualified probe");
    let mut pc = function.bytecode_offset as usize;
    let end = pc + function.bytecode_length as usize;
    let mut indices = Vec::new();
    while pc < end {
        if let Instruction::GetF { field_idx, .. } =
            decode_instruction(&module.bytecode, &mut pc).unwrap()
        {
            indices.push(field_idx);
        }
    }
    assert_eq!(indices, vec![index], "wrong field owner in {body}");
    let id = function.id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(id)
        .expect("execute");
    assert_eq!(value.as_i64(), 7);
}

#[test]
fn explicit_result_selects_each_declared_field_layout() {
    for (name, index) in [("Left", 0), ("Right", 1)] {
        assert_layout(
            &format!("let f: Factory<Int> = Factory {{ value: 7 }}; f.build<{name}>().value"),
            index,
        );
    }
}

#[test]
fn shadowed_method_parameter_selects_each_declared_field_layout() {
    for (name, index) in [("Left", 0), ("Right", 1)] {
        assert_layout(
            &format!("let f: Factory<Int> = Factory {{ value: 7 }}; f.shadow<{name}>().value"),
            index,
        );
    }
}

#[test]
fn explicit_result_identity_survives_an_unannotated_binding() {
    for (name, index) in [("Left", 0), ("Right", 1)] {
        assert_layout(
            &format!(
                "let f: Factory<Int> = Factory {{ value: 7 }}; let result = f.build<{name}>(); result.value"
            ),
            index,
        );
    }
}

#[test]
fn explicit_result_selects_the_following_method_owner() {
    for method in ["build", "shadow"] {
        for name in ["Left", "Right"] {
            let module = compile(&format!(
                "let f: Factory<Int> = Factory {{ value: 7 }}; f.{method}<{name}>().answer()"
            ));
            let function = module
                .functions
                .iter()
                .find(|f| {
                    module
                        .get_string(f.name)
                        .is_some_and(|name| name.ends_with(".probe"))
                })
                .expect("probe");
            let mut pc = function.bytecode_offset as usize;
            let end = pc + function.bytecode_length as usize;
            let mut called = Vec::new();
            while pc < end {
                let name = match decode_instruction(&module.bytecode, &mut pc).unwrap() {
                    Instruction::Call { func_id, .. } | Instruction::CallG { func_id, .. } => {
                        module.get_string(module.functions[func_id as usize].name)
                    }
                    Instruction::CallM { method_id, .. } => {
                        module.get_string(verum_vbc::types::StringId(method_id))
                    }
                    _ => None,
                };
                if let Some(name) = name {
                    called.push(name.to_owned());
                }
            }
            assert!(
                called
                    .iter()
                    .any(|callee| callee.ends_with(&format!("{name}.answer"))),
                "{method}<{name}>: {called:?}"
            );
        }
    }
}
