//! Primitive operation results retain their type at ordinary value boundaries.
#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

#[test]
fn slice_length_keeps_its_type_when_passed_through_a_named_binding() {
    run(
        r#"
        fn accept(count: Int) -> Int { count }
        fn count(bytes: &[Int]) -> Int {
            let length = bytes.len();
            accept(length)
        }
        fn probe() -> Int {
            let bytes = [7, 3, 1];
            count(&bytes)
        }
    "#,
        3,
    );
}

#[test]
fn formatted_text_keeps_its_owner_across_repeated_value_arguments() {
    run(
        r#"
        fn accept(value: Text) -> Int { value.len() }
        fn probe() -> Int {
            let value = f"value={73}";
            let first = accept(value);
            let second = accept(value);
            first + second
        }
    "#,
        16,
    );
}

#[test]
fn a_homonymous_user_method_keeps_its_declared_record_result() {
    run(
        r#"
        type Counter is { value: Int };
        implement Counter {
            fn len(&self) -> Counter { Counter { value: self.value } }
        }
        fn probe() -> Int {
            let source = Counter { value: 73 };
            let mut obtained = source.len();
            let copy = obtained;
            obtained.value = 0;
            copy.value
        }
    "#,
        73,
    );
}

#[test]
fn extended_result_authority_requires_complete_operands() {
    use verum_common::List;
    use verum_vbc::{
        Instruction, TypeId, TypeRef,
        instruction::{CbgrSubOpcode, Reg},
    };
    let mut operands = List::<u8>::new().into();
    verum_vbc::encoding::encode_reg(Reg(512), &mut operands);
    verum_vbc::encoding::encode_reg(Reg(257), &mut operands);
    let operation = |operands| Instruction::CbgrExtended {
        sub_op: CbgrSubOpcode::SliceLen as u8,
        operands,
    };
    assert_eq!(
        operation(operands.clone()).intrinsic_result_type(),
        Some((Reg(512), TypeRef::Concrete(TypeId::I64)))
    );
    for end in 0..operands.len() {
        assert!(
            operation(operands[..end].to_vec())
                .intrinsic_result_type()
                .is_none()
        );
    }
    operands.push(0);
    assert!(operation(operands).intrinsic_result_type().is_none());
}

fn run(source: &str, expected: i64) {
    let ast = Parser::new(source).parse_module().unwrap();
    for unit in [false, true] {
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("primitive_result"));
        let module = if unit {
            compiler.collect_unit_declarations(&[&ast]).unwrap();
            compiler
                .compile_unit_items(&[&ast], ItemFailurePolicy::Strict)
                .unwrap();
            compiler.finalize_module_from_state().unwrap()
        } else {
            compiler.compile_module(&ast).unwrap()
        };
        let wire = verum_vbc::deserialize::deserialize_module(
            &verum_vbc::serialize::serialize_module(&module).unwrap(),
        )
        .unwrap();
        for module in [module, wire] {
            verum_vbc::validate::validate_module(&module).unwrap();
            let probe = module
                .find_function_by_name("primitive_result.probe")
                .unwrap();
            let result = Interpreter::new(module.into())
                .execute_function(probe)
                .unwrap();
            assert_eq!(result.try_as_i64(), Some(expected), "unit={unit}");
        }
    }
}
