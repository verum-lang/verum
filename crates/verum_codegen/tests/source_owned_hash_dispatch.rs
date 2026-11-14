//! Direct calls retain the selected source declaration, including receiver methods.
#[cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;
#[path = "support/canonical_hash_source.rs"]
mod canonical_hash_source;
use verum_codegen::llvm::context::FunctionContext;
use verum_codegen::llvm::instruction::lower_instruction;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Map, Text};
use verum_fast_parser::Parser;
use verum_llvm::OptimizationLevel;
use verum_llvm::context::Context;
use verum_llvm::memory_buffer::MemoryBuffer;
use verum_llvm::targets::{InitializationConfig, Target};
use verum_llvm::values::AnyValue;
use verum_vbc::codegen::{CodegenConfig, VbcCodegen};
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::module::{FunctionId, VbcModule};
use verum_vbc::types::StringId;

fn compile(source: &str) -> VbcModule {
    let ast = Parser::new(source).parse_module().expect("source parses");
    VbcCodegen::new().compile_module(&ast).expect("source VBC")
}

fn roundtrip(source: &VbcModule) -> VbcModule {
    let mut module = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(source).expect("serialize"),
    )
    .expect("deserialize");
    // The LLVM API consumes decoded bodies, just as the archive loader does.
    for function in &mut module.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions =
            verum_vbc::bytecode::decode_instructions(&module.bytecode[start..end])
                .expect("decode loaded body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    module
}

fn assert_source_call(source: &str, probe: &str, expected: u64, target: &str) {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    // The loaded descriptor must preserve the same negative authority as
    // the local source descriptor, rather than depend on an AST side table.
    let original = compile(source);
    for vbc in [original.clone(), roundtrip(&original)] {
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("user_hash").with_debug_info(false),
        );
        lowering.lower_module(&vbc).expect("source LLVM");
        let function = lowering.module().get_function(probe).expect("caller");
        let llvm = function.print_to_string();
        let body = llvm.to_str().expect("LLVM text");
        assert!(
            body.contains(&format!("@{target}(")),
            "selected declaration: {body}"
        );
        assert!(
            !body.contains("@verum_generic_hash("),
            "ordinary body replaced: {body}"
        );
        // Execute only the emitted source functions. This keeps the focused
        // JIT control independent of process/runtime initialization.
        let mut definitions = Text::new();
        for descriptor in &vbc.functions {
            let name = vbc.get_string(descriptor.name).expect("name");
            let lowered = lowering.module().get_function(name).expect(name);
            assert!(lowered.verify(true));
            definitions.push_str(lowered.print_to_string().to_str().expect("UTF-8 IR"));
            definitions.push('\n');
        }
        let module = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                definitions.as_bytes(),
                "source_hash",
            ))
            .expect("source IR");
        module.verify().expect("valid source module");
        let engine = module
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        // SAFETY: the source probe has no parameters and returns one Int slot.
        let actual = unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> u64>(probe)
                .expect("probe")
                .call()
        };
        assert_eq!(actual, expected, "{probe}");
    }
}

#[test]
fn native_user_hash_value_calls_its_declared_body() {
    assert_source_call(
        "fn hash_value(value: Int) -> Int { value + 30 } fn probe() -> Int { hash_value(7) }",
        "probe",
        37,
        "hash_value",
    );
}

#[test]
fn qualified_same_leaf_functions_keep_their_selected_bodies() {
    let source = r#"
        module alpha { public fn hash_value(value: Int) -> Int { value + 30 } }
        module beta { public fn hash_value(value: Int) -> Int { value + 70 } }
        fn left_probe() -> Int { alpha.hash_value(7) }
        fn right_probe() -> Int { beta.hash_value(7) }
    "#;
    assert_source_call(source, "left_probe", 37, "alpha.hash_value");
    assert_source_call(source, "right_probe", 77, "beta.hash_value");
}

#[test]
fn mounted_alias_retains_the_original_regular_parameter_declaration() {
    assert_source_call(
        r#"
        module upstream { public fn hash_value(value: Int) -> Int { value + 30 } }
        mount upstream.{hash_value as renamed};
        fn probe() -> Int { renamed(7) }
    "#,
        "probe",
        37,
        "upstream.hash_value",
    );
}

#[test]
fn zero_argument_free_function_never_reads_a_phantom_hash_receiver() {
    assert_source_call(
        "fn hash_value() -> Int { 73 } fn probe() -> Int { hash_value() }",
        "probe",
        73,
        "hash_value",
    );
}

#[test]
fn two_argument_free_function_keeps_its_original_arity() {
    assert_source_call(
        "fn hash_value(a: Int, b: Int) -> Int { a + b } fn probe() -> Int { hash_value(30, 7) }",
        "probe",
        37,
        "hash_value",
    );
}

fn direct_call_ir(vbc: &VbcModule, name: &str) -> Text {
    direct_call_ir_with_target(vbc, name, true).expect("selected direct lowering")
}

fn direct_call_ir_with_target(
    vbc: &VbcModule, name: &str, publish_target: bool,
) -> verum_codegen::llvm::error::Result<Text> {
    let id = vbc.find_function_by_name(name).expect("selected callee");
    let descriptor = vbc.get_function(id).expect("descriptor");
    let context = Context::create();
    let module = context.create_module("direct_hash");
    let i64_type = context.i64_type();
    let target = module.add_function(name, i64_type.fn_type(&[i64_type.into()], false), None);
    let function = module.add_function("probe", i64_type.fn_type(&[], false), None);
    let mut ctx = FunctionContext::with_vbc_module(&context, &module, vbc, function, "probe");
    if publish_target {
        ctx.set_func_id_map(std::sync::Arc::new([(descriptor.id.0, target)].into_iter().collect()));
    }
    ctx.builder()
        .position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, i64_type.const_int(7, false).into());
    lower_instruction(
        &mut ctx,
        &Instruction::Call {
            dst: Reg(1),
            func_id: descriptor.id.0,
            args: RegRange {
                start: Reg(0),
                count: 1,
            },
        },
    )?;
    ctx.builder()
        .build_return(Some(&ctx.get_register(1).expect("result")))
        .expect("return");
    module.verify().expect("valid direct IR");
    Ok(Text::from(function.print_to_string().to_str().expect("UTF-8 IR")))
}

use canonical_hash_source::canonical_hash_source;

fn canonical_hash_declarations() -> VbcModule { compile(&canonical_hash_source()) }

fn assert_self_declaration(vbc: &VbcModule, descriptor: &verum_vbc::module::FunctionDescriptor) {
    let source = descriptor.source_signature.as_ref().expect("retained source signature");
    assert!(matches!(source.params[0].kind, verum_ast::FunctionParamKind::SelfRef));
    assert_eq!(vbc.get_string(descriptor.params[0].type_name), Some("&Self"));
}

#[test]
fn real_inherited_defaults_keep_their_selected_owner_and_body() {
    let source = canonical_hash_declarations();
    for vbc in [source.clone(), roundtrip(&source)] {
        for name in ["Int.hash_value", "Unit.hash_value"] {
            let descriptor = vbc
                .get_function(vbc.find_function_by_name(name).unwrap())
                .unwrap();
            assert_eq!(descriptor.params.len(), 1);
            assert_self_declaration(&vbc, descriptor);
            let owner = if name == "Unit.hash_value" {
                verum_vbc::types::TypeId::UNIT
            } else {
                verum_vbc::types::TypeId::INT
            };
            assert_eq!(descriptor.parent_type, Some(owner));
            assert_eq!(descriptor.source_receiver,
                Some(verum_vbc::types::TypeRef::Concrete(owner)));
            let ir = direct_call_ir(&vbc, name);
            assert!(ir.contains(&format!("@{name}(")), "{ir}");
            assert!(!ir.contains("@verum_generic_hash("), "{ir}");
        }
    }
}

#[test]
fn absent_optional_spelling_does_not_replace_the_selected_regular_body() {
    let mut vbc = compile("fn hash_value(value: Int) -> Int { value + 30 }");
    let id = vbc.find_function_by_name("hash_value").unwrap();
    assert_ne!(
        vbc.get_function(id).unwrap().params[0].type_name,
        StringId::EMPTY
    );
    vbc.functions
        .iter_mut()
        .find(|f| f.id == id)
        .unwrap()
        .params[0]
        .type_name = StringId::EMPTY;
    // Optional display spelling is not authority for replacing the selected
    // callable. The actual declaration remains a Regular source parameter.
    assert!(
        vbc.get_string(StringId::EMPTY)
            .is_some_and(|s| !s.is_empty())
    );
    let ir = direct_call_ir(&vbc, "hash_value");
    assert!(ir.contains("@hash_value("));
    assert!(!ir.contains("@verum_generic_hash("));
}

fn import_bodies(source: &VbcModule) -> VbcModule {
    let source = roundtrip(source);
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("consumer"));
    for i in 0..20 {
        codegen
            .ctx_mut()
            .intern_string_raw(&format!("unrelated_{i}"));
    }
    codegen.import_archive_module_types(&source);
    let remap: Map<_, _> = source
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.0, FunctionId(7000 + i as u32)))
        .collect();
    codegen.merge_archive_function_bodies(&source, &remap.into());
    codegen
        .finalize_module_from_state()
        .expect("imported source descriptors")
}

#[test]
fn actual_archive_body_merge_keeps_self_and_regular_selected_bodies() {
    let defaults = canonical_hash_declarations();
    assert!(
        defaults
            .get_string(StringId::EMPTY)
            .is_some_and(|text| !text.is_empty())
    );
    let imported = import_bodies(&defaults);
    for name in ["Int.hash_value", "Unit.hash_value"] {
        let descriptor = imported
            .get_function(imported.find_function_by_name(name).unwrap())
            .unwrap();
        assert_self_declaration(&imported, descriptor);
        let ir = direct_call_ir(&imported, name);
        assert!(ir.contains(&format!("@{name}(")), "{ir}");
        assert!(!ir.contains("@verum_generic_hash("), "{ir}");
    }
    let regular = import_bodies(&compile("fn hash_value(value: Int) -> Int { value + 30 }"));
    let ir = direct_call_ir(&regular, "hash_value");
    assert!(ir.contains("@hash_value("));
    assert!(!ir.contains("@verum_generic_hash("));
}

#[test]
fn unresolved_or_empty_optional_spelling_cannot_replace_a_selected_body() {
    let source = compile("fn hash_value(value: Int) -> Int { value + 30 }");
    for invalid in [false, true] {
        let mut module = source.clone();
        let marker = if invalid {
            StringId(u32::MAX)
        } else {
            module.strings.intern("")
        };
        assert_ne!(marker, StringId::EMPTY);
        let id = module.find_function_by_name("hash_value").unwrap();
        module
            .functions
            .iter_mut()
            .find(|f| f.id == id)
            .unwrap()
            .params[0]
            .type_name = marker;
        let ir = direct_call_ir(&module, "hash_value");
        assert!(ir.contains("@hash_value("));
        assert!(!ir.contains("@verum_generic_hash("));
    }
}

#[test]
fn explicitly_typed_self_and_ordinary_uppercase_name_are_regular_parameters() {
    for name in ["self", "Receiver"] {
        let source = format!(
            "fn hash_value({name}: Int) -> Int {{ {name} + 30 }} fn probe() -> Int {{ hash_value(7) }}"
        );
        let vbc = compile(&source);
        let descriptor = vbc.get_function(vbc.find_function_by_name("hash_value").unwrap()).unwrap();
        assert!(matches!(descriptor.source_signature.as_ref().unwrap().params[0].kind,
            verum_ast::FunctionParamKind::Regular { .. }));
        assert_source_call(&source, "probe", 37, "hash_value");
    }
}

#[test]
fn self_spelling_or_absent_source_header_cannot_replace_a_selected_body() {
    for remove_header in [false, true] {
        let mut vbc = compile("fn hash_value(value: Int) -> Int { value + 30 }");
        let spelling = vbc.strings.intern("&Self");
        let id = vbc.find_function_by_name("hash_value").unwrap();
        let descriptor = vbc.functions.iter_mut().find(|function| function.id == id).unwrap();
        descriptor.params[0].type_name = spelling;
        if remove_header { descriptor.source_signature = None; }
        if remove_header {
            let error = verum_vbc::serialize::serialize_module(&vbc).unwrap_err();
            assert!(matches!(error, verum_vbc::error::VbcError::Serialization(_)));
            assert!(error.to_string().contains("callable declaration identity has no source signature"), "{error}");
        } else {
            for module in [vbc.clone(), roundtrip(&vbc)] {
                let ir = direct_call_ir(&module, "hash_value");
                assert!(ir.contains("@hash_value("), "{ir}");
                assert!(!ir.contains("@verum_generic_hash("), "{ir}");
            }
        }
    }
}

#[test]
fn missing_selected_native_target_refuses_even_when_a_same_named_symbol_exists() {
    let vbc = compile("fn hash_value(value: Int) -> Int { value + 30 }");
    let error = direct_call_ir_with_target(&vbc, "hash_value", false).unwrap_err();
    assert!(matches!(error, verum_codegen::llvm::error::LlvmLoweringError::MissingFunction(_)));
    assert!(error.to_string().contains("has no native target"), "{error}");
}

#[cfg(target_os = "macos")]
#[test]
fn direct_user_self_method_executes_its_actual_body() {
    let source = r#"
        type Key is { value:Int };
        implement Key { fn hash_value(&self)->Int { 73 } }
        fn probe(value:&Key)->Int { Key.hash_value(value) }
        fn main()->Int { let key=Key{value:7}; probe(&key) }
    "#;
    let vbc = compile(source);
    let target = vbc.find_function_by_name("Key.hash_value").unwrap();
    let probe = vbc.get_function(vbc.find_function_by_name("probe").unwrap()).unwrap();
    assert!(probe.instructions.as_ref().unwrap().iter().any(|instruction|
        matches!(instruction, Instruction::Call { func_id, .. } if *func_id == target.0)
    ), "control must use the selected direct Call");
    let wire = roundtrip(&vbc);
    verum_vbc::validate::validate_module(&wire).unwrap();
    let entry = wire.find_function_by_name("main").unwrap();
    let mut vm = verum_vbc::interpreter::Interpreter::try_new_validated(wire.into()).unwrap();
    assert_eq!(vm.execute_function(entry).unwrap().try_as_i64(), Some(73));
    source_image::run(source);
}

#[cfg(target_os = "macos")]
#[test]
fn actual_default_body_executes_without_direct_name_substitution() {
    let mut source = canonical_hash_source();
    source.push_str(r#"
        fn main()->Int {
            let value:Int=7;
            let mut hasher=DefaultHasher.new();
            DefaultHasher.write_int(&mut hasher, value);
            let expected=DefaultHasher.finish(&hasher);
            let actual=Int.hash_value(&value);
            if actual == expected && actual == -1747035056885442531 {73} else {0}
        }
    "#);
    // No runtime body substitution is performed by this fixture. If the actual
    // source dependency graph cannot execute, retain its exact refusal.
    source_image::run(&source);
}
