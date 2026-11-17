//! Declared generic pointer operations retain their telescope through source and wire.
#![cfg(feature = "codegen")]

use verum_common::List;
use verum_fast_parser::Parser;
use verum_vbc::{
    Instruction, TypeId, TypeRef, VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    instruction::{LayoutProperty, MemSubOpcode},
    interpreter::Interpreter,
    types::{CbgrTier, TypeParamId},
};

fn compile(source: &str, unit: bool) -> VbcModule {
    let ast = Parser::new(source).parse_module().unwrap();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new("pointer_layout"));
    if unit {
        compiler.collect_unit_declarations(&[&ast]).unwrap();
        compiler.compile_unit_items(&[&ast], ItemFailurePolicy::Strict).unwrap();
        compiler.finalize_module_from_state().unwrap()
    } else {
        compiler.compile_module(&ast).unwrap()
    }
}
fn variants(module: VbcModule) -> [VbcModule; 2] {
    let wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    ).unwrap();
    [module, wire]
}

#[test]
fn generic_raw_pointee_and_integer_offset_survive_both_publication_paths() {
    for unit in [false, true] {
        for module in variants(compile(r#"
            fn advance<T>(pointer: *const T, count: ISize) -> *const T {
                @intrinsic("ptr_offset", pointer, count)
            }
            fn identity<T>(value: T) -> T { value }
            fn ordinary(value: Int) -> Int { value }
        "#, unit)) {
            verum_vbc::validate::validate_module(&module).unwrap();
            let selected = module.find_function_by_name("pointer_layout.advance").unwrap();
            let descriptor = module.get_function(selected).unwrap();
            assert!(descriptor.is_generic, "published telescope must control generic code");
            assert_eq!(descriptor.type_params.len(), 1);
            let parameters = descriptor.semantic_parameter_types().unwrap();
            assert!(matches!(&parameters[0], Some(TypeRef::Reference { inner, tier: CbgrTier::Tier2, .. })
                if **inner == TypeRef::Generic(TypeParamId(0))));
            assert_eq!(parameters[1], Some(TypeRef::Concrete(TypeId::INT)));
            let start = descriptor.bytecode_offset as usize;
            let body = verum_vbc::bytecode::decode_instructions(
                &module.bytecode[start..start + descriptor.bytecode_length as usize],
            ).unwrap();
            assert!(body.iter().any(|instruction| matches!(instruction,
                Instruction::TypeLayout { type_ref: TypeRef::Generic(TypeParamId(0)), property: LayoutProperty::Stride, .. })));
            assert!(body.iter().any(|instruction| matches!(instruction,
                Instruction::MemExtended { sub_op, operands } if *sub_op == MemSubOpcode::PtrAddT as u8 && operands.last() == Some(&1))));
            for (name, generic) in [("pointer_layout.identity", true), ("pointer_layout.ordinary", false)] {
                assert_eq!(module.get_function(module.find_function_by_name(name).unwrap()).unwrap().is_generic, generic);
            }
        }
    }
}

#[test]
fn declared_widths_and_negative_offsets_execute_without_wrapper_name_dispatch() {
    for unit in [false, true] {
        for module in variants(compile(r#"
            fn advance<T>(pointer: *const T, count: ISize) -> *const T {
                @intrinsic("ptr_offset", pointer, count)
            }
            fn retreat<T>(pointer: *const T, count: ISize) -> *const T {
                @intrinsic("ptr_sub", pointer, count)
            }
            fn ptr_offset(pointer: *const Byte, count: Int) -> Int { 73 }
            fn probe() -> Int {
                let a = advance<Byte>(4096 as *const Byte, 3) as Int;
                let b = advance<UInt32>(4096 as *const UInt32, 3) as Int;
                let c = advance<Int128>(4096 as *const Int128, 3) as Int;
                let d = advance<Byte>(4096 as *const Byte, -3) as Int;
                let e = retreat<UInt32>(4096 as *const UInt32, 3) as Int;
                if a == 4099 && b == 4108 && c == 4144 && d == 4093 && e == 4084 {
                    ptr_offset(4096 as *const Byte, 3)
                } else { 0 }
            }
        "#, unit)) {
            verum_vbc::validate::validate_module(&module).unwrap();
            let entry = module.find_function_by_name("pointer_layout.probe").unwrap();
            let value = Interpreter::new(module.into()).execute_function(entry).unwrap();
            assert_eq!(value.try_as_i64(), Some(73), "unit={unit}");
        }
    }
}

#[test]
fn raw_pointer_arithmetic_refuses_missing_pointee_authority() {
    for source in [
        "fn invalid(pointer: Int, count: Int) -> Int { @intrinsic(\"ptr_offset\", pointer, count) }",
        "fn invalid(pointer: *const Unbound, count: Int) -> Int { @intrinsic(\"ptr_offset\", pointer, count) }",
    ] {
        let ast = Parser::new(source).parse_module().unwrap();
        let mut compiler = VbcCodegen::with_config(CodegenConfig::new("pointer_layout"));
        assert!(compiler.compile_module(&ast).is_err());
    }
    // A declared user type remains nominal instead of acquiring a scalar layout
    // from a familiar name. Its pointer arithmetic carries that exact owner.
    let module = compile("type Byte is { first: Int, second: Int }; fn advance(pointer: *const Byte, count: Int) -> *const Byte { @intrinsic(\"ptr_offset\", pointer, count) }", false);
    let body = module.get_function(module.find_function_by_name("pointer_layout.advance").unwrap()).unwrap().instructions.as_ref().unwrap();
    let owners: List<_> = body.iter().filter_map(|instruction| match instruction {
        Instruction::TypeLayout { type_ref, property: LayoutProperty::Stride, .. } => Some(type_ref), _ => None,
    }).collect();
    assert_eq!(owners.len(), 1);
    assert_ne!(owners[0], &TypeRef::Concrete(TypeId::U8));
}
