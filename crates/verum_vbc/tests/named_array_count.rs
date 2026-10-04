#![cfg(feature = "codegen")]

use verum_fast_parser::Parser;
use verum_vbc::bytecode::decode_instructions;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{Instruction, MemSubOpcode};

fn assert_packed(declaration: &str, prefix: &str, local: &str) {
    let source = format!(
        r#"
{prefix}
fn probe() -> Int {{
    {local}
    {declaration}
    byte_buf[0] = 65 as Byte;
    byte_buf[1] = 66 as Byte;
    let first = &byte_buf[0] as *const Byte as Int;
    let second = &byte_buf[1] as *const Byte as Int;
    second - first
}}
"#
    );
    let ast = Parser::new(&source).parse_module().expect("parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("named_array_count"));
    let module = codegen.compile_module(&ast).expect("compile");
    let function = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe");
    let start = function.bytecode_offset as usize;
    let instructions =
        decode_instructions(&module.bytecode[start..start + function.bytecode_length as usize])
            .expect("decode");
    let packed = instructions.iter().filter(|i| matches!(i,
        Instruction::MemExtended { sub_op, .. } if *sub_op == MemSubOpcode::NewByteArray.to_byte()
    )).count();
    assert_eq!(packed, 1, "{declaration}: {instructions:#?}");
    let entry = function.id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert_eq!(value.as_i64(), 1, "byte addresses must be contiguous");
}

#[test]
fn literal_array_count_packs_bytes() {
    assert_packed("let mut byte_buf: [Byte; 65536] = [0; 65536];", "", "");
}

#[test]
fn module_constant_array_count_packs_bytes() {
    assert_packed(
        "let mut byte_buf: [Byte; STREAM_CHUNK_SIZE] = [0; STREAM_CHUNK_SIZE];",
        "const STREAM_CHUNK_SIZE: Int = 65536;",
        "",
    );
}

#[test]
fn local_constant_array_count_packs_bytes() {
    assert_packed(
        "let mut byte_buf: [Byte; STREAM_CHUNK_SIZE] = [0; STREAM_CHUNK_SIZE];",
        "",
        "const STREAM_CHUNK_SIZE: Int = 65536;",
    );
}

#[test]
fn suffixed_byte_repeat_constant_count_packs_bytes() {
    assert_packed(
        "let mut byte_buf = [0_u8; STREAM_CHUNK_SIZE];",
        "const STREAM_CHUNK_SIZE: Int = 65536;",
        "",
    );
}

#[test]
fn arithmetic_and_parenthesized_constant_count_packs_bytes() {
    assert_packed(
        "let mut byte_buf: [Byte; (CHUNK + 2)] = [0; CHUNK + 2];",
        "const CHUNK: Int = 6;",
        "",
    );
}

#[test]
fn invalid_static_counts_are_errors_instead_of_unpacked_fallbacks() {
    for count in [
        "-1",
        "9223372036854775807 + 1",
        "18446744073709551616",
        "8 / 0",
    ] {
        let source = format!("fn probe() {{ let bytes: [Byte; {count}] = [0; {count}]; }}");
        let ast = Parser::new(&source).parse_module().expect("parse");
        let error = VbcCodegen::with_config(CodegenConfig::new("invalid_count"))
            .compile_module(&ast)
            .expect_err(count);
        assert!(
            error.to_string().contains("constant integer")
                || error.to_string().contains("array count"),
            "{count}: {error}"
        );
    }
}

#[test]
fn overflowing_named_constant_must_not_wrap_to_zero() {
    let ast = Parser::new("const SIZE: Int = 18446744073709551616; fn probe() { let bytes: [Byte; SIZE] = [0; SIZE]; }")
        .parse_module().expect("parse");
    let error = VbcCodegen::with_config(CodegenConfig::new("overflow_count"))
        .compile_module(&ast)
        .expect_err("overflowing constant must not become size zero");
    assert!(error.to_string().contains("constant integer"), "{error}");
}

#[test]
fn runtime_binding_shadows_a_module_constant() {
    let ast = Parser::new(
        "const count: Int = 4; fn probe(count: Int) { let bytes: [Byte; count] = [0; count]; }",
    )
    .parse_module()
    .expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("shadow_count"))
        .compile_module(&ast)
        .expect("compile");
    let probe = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe");
    let start = probe.bytecode_offset as usize;
    let instructions =
        decode_instructions(&module.bytecode[start..start + probe.bytecode_length as usize])
            .expect("decode");
    assert!(!instructions.iter().any(|i| matches!(i,
        Instruction::MemExtended { sub_op, .. } if *sub_op == MemSubOpcode::NewByteArray.to_byte()
    )), "a runtime parameter cannot be replaced with the module constant: {instructions:?}");
}

#[test]
fn same_named_constants_in_two_modules_keep_their_own_counts() {
    let first = Parser::new("module first; const SIZE: Int = 3; fn probe_first() -> Int { let bytes: [Byte; SIZE] = [0; SIZE]; bytes.len() }")
        .parse_module().expect("parse first");
    let second = Parser::new("module second; const SIZE: Int = 5; fn probe_second() -> Int { let bytes: [Byte; SIZE] = [0; SIZE]; bytes.len() }")
        .parse_module().expect("parse second");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("multiple_counts"));
    codegen
        .collect_unit_declarations(&[&first, &second])
        .expect("collect");
    codegen
        .compile_unit_items(
            &[&first, &second],
            verum_vbc::codegen::ItemFailurePolicy::Strict,
        )
        .expect("compile");
    let module = std::sync::Arc::new(codegen.finalize_module().expect("finalize"));
    for (name, expected) in [("probe_first", 3), ("probe_second", 5)] {
        let probe = module
            .functions
            .iter()
            .find(|f| {
                module
                    .get_string(f.name)
                    .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
            })
            .expect(name);
        let start = probe.bytecode_offset as usize;
        let instructions =
            decode_instructions(&module.bytecode[start..start + probe.bytecode_length as usize])
                .expect("decode");
        assert!(instructions.iter().any(|i| matches!(i,
            Instruction::MemExtended { sub_op, .. } if *sub_op == MemSubOpcode::NewByteArray.to_byte()
        )), "{name} must allocate packed bytes");
        let value = verum_vbc::interpreter::Interpreter::new(module.clone())
            .execute_function(probe.id)
            .expect("execute");
        assert_eq!(value.as_i64(), expected, "{name}");
    }
}

#[test]
fn wider_primitive_array_count_uses_the_same_resolver() {
    let ast = Parser::new("const SIZE: Int = 4; fn probe() -> Int { let bytes: [UInt32; SIZE] = [7; SIZE]; bytes[3] as Int }")
        .parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("typed_count"))
        .compile_module(&ast)
        .expect("compile");
    let probe = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe");
    let start = probe.bytecode_offset as usize;
    let instructions =
        decode_instructions(&module.bytecode[start..start + probe.bytecode_length as usize])
            .expect("decode");
    assert!(instructions.iter().any(|i| matches!(i,
        Instruction::MemExtended { sub_op, .. } if *sub_op == MemSubOpcode::NewTypedArray.to_byte()
    )), "{instructions:?}");
    let entry = probe.id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert_eq!(value.as_i64(), 7);
}

#[test]
fn zero_is_a_valid_known_count() {
    let ast = Parser::new("const SIZE: Int = 0; fn probe() -> Int { let bytes: [Byte; SIZE] = [0; SIZE]; bytes.len() }")
        .parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("zero_count"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert_eq!(value.as_i64(), 0);
}

#[test]
fn typed_array_byte_extent_cannot_overflow() {
    let ast = Parser::new(
        "fn probe() { let words: [UInt32; 4611686018427387904] = [0; 4611686018427387904]; }",
    )
    .parse_module()
    .expect("parse");
    let error = VbcCodegen::with_config(CodegenConfig::new("extent_overflow"))
        .compile_module(&ast)
        .expect_err("count times element size must fit");
    assert!(error.to_string().contains("array count"), "{error}");
}

#[test]
fn named_alias_and_division_initializers_pack_bytes() {
    for prefix in [
        "const BASE: Int = 4; const SIZE: Int = BASE;",
        "const SIZE: Int = 8 / 2;",
        "const SIZE: Int = BASE + 2; const BASE: Int = 2;",
    ] {
        assert_packed("let mut byte_buf: [Byte; SIZE] = [0; SIZE];", prefix, "");
    }
}

#[test]
fn named_constant_cycles_are_rejected() {
    let ast = Parser::new("const FIRST: Int = SECOND; const SECOND: Int = FIRST; fn probe() { let bytes: [Byte; FIRST] = [0; FIRST]; }")
        .parse_module().expect("parse");
    let error = VbcCodegen::with_config(CodegenConfig::new("cyclic_count"))
        .compile_module(&ast)
        .expect_err("constant cycle cannot give a count");
    assert!(error.to_string().contains("cyclic constant"), "{error}");
}

#[test]
fn qualified_alias_uses_its_declaration_scope() {
    let origin = Parser::new("module origin; const BASE: Int = 3; public const SIZE: Int = BASE;")
        .parse_module()
        .expect("parse origin");
    let caller = Parser::new("module caller; const BASE: Int = 7; fn probe() -> Int { let bytes: [Byte; origin.SIZE] = [0; origin.SIZE]; bytes.len() }")
        .parse_module().expect("parse caller");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("alias_scopes"));
    codegen
        .collect_unit_declarations(&[&origin, &caller])
        .expect("collect");
    codegen
        .compile_unit_items(
            &[&origin, &caller],
            verum_vbc::codegen::ItemFailurePolicy::Strict,
        )
        .expect("compile");
    let module = codegen.finalize_module().expect("finalize");
    let probe = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe");
    let start = probe.bytecode_offset as usize;
    let instructions =
        decode_instructions(&module.bytecode[start..start + probe.bytecode_length as usize])
            .expect("decode");
    assert!(instructions.iter().any(|i| matches!(i,
        Instruction::MemExtended { sub_op, .. } if *sub_op == MemSubOpcode::NewByteArray.to_byte()
    )), "qualified alias must allocate packed bytes: {instructions:?}");
    let entry = probe.id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert_eq!(value.as_i64(), 3);
}

#[test]
fn folded_alias_value_survives_serialization_without_source_ast() {
    let ast = Parser::new("const BASE: Int = 8; public const SIZE: Int = BASE / 2;")
        .parse_module()
        .expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("archived_count"))
        .compile_module(&ast)
        .expect("compile");
    let bytes = verum_vbc::serialize::serialize_module(&module).expect("serialize");
    let module = verum_vbc::deserialize::deserialize_module(&bytes).expect("deserialize");
    let constant = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "SIZE" || n.ends_with(".SIZE"))
        })
        .expect("SIZE");
    assert!(constant.is_const);
    assert_eq!(
        constant.intrinsic_name.and_then(|id| module.get_string(id)),
        Some("__const_val_4")
    );
    let entry = constant.id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute constant");
    assert_eq!(value.as_i64(), 4);
}

#[test]
fn repeated_constant_dependencies_are_memoized() {
    let mut source = String::from("const N0: Int = 1;\n");
    for index in 1..=24 {
        source.push_str(&format!(
            "const N{index}: Int = N{} + N{};\n",
            index - 1,
            index - 1
        ));
    }
    source.push_str("fn probe() { let bytes: [Byte; N24] = [0; N24]; }");
    let ast = Parser::new(&source).parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("constant_dag"))
        .compile_module(&ast)
        .expect("compile");
    let constant = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "N24" || n.ends_with(".N24"))
        })
        .expect("N24");
    assert_eq!(
        constant.intrinsic_name.and_then(|id| module.get_string(id)),
        Some("__const_val_16777216")
    );
}

#[test]
fn local_constant_shadow_does_not_change_a_module_alias() {
    let ast = Parser::new("module counts; const BASE: Int = 3; const SIZE: Int = BASE; fn probe() -> Int { const BASE: Int = 7; let bytes: [Byte; SIZE] = [0; SIZE]; bytes.len() }")
        .parse_module().expect("parse");
    let module = VbcCodegen::with_config(CodegenConfig::new("counts"))
        .compile_module(&ast)
        .expect("compile");
    let entry = module
        .functions
        .iter()
        .find(|f| {
            module
                .get_string(f.name)
                .is_some_and(|n| n == "probe" || n.ends_with(".probe"))
        })
        .expect("probe")
        .id;
    let value = verum_vbc::interpreter::Interpreter::new(std::sync::Arc::new(module))
        .execute_function(entry)
        .expect("execute");
    assert_eq!(value.as_i64(), 3);
}

#[test]
fn reusing_codegen_does_not_reuse_previous_constant_bindings() {
    let first = Parser::new("const BASE: Int = 9; fn seed() {}").parse_module().expect("first parse");
    let second = Parser::new("const SIZE: Int = BASE; fn probe() { let bytes: [Byte; SIZE] = [0; SIZE]; }")
        .parse_module().expect("second parse");
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("reused_counts"));
    codegen.compile_module(&first).expect("first compile");
    let error = codegen.compile_module(&second)
        .expect_err("BASE from the previous module must not survive reset or FunctionId reuse");
    assert!(error.to_string().contains("constant integer"), "{error}");
}
