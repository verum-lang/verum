//! T1698: native numeric endian producers and consumers share fixed byte storage.
//! The JIT uses a bounded host allocation substrate; this is not an AOT or
//! allocator-lifecycle acceptance test. Production lowering and helpers are real.
use std::cell::RefCell;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Heap, List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    memory_buffer::MemoryBuffer,
    module::Module,
    targets::{InitializationConfig, Target},
    values::AnyValue,
};
use verum_vbc::{
    codegen::VbcCodegen,
    deserialize::deserialize_module,
    instruction::{Instruction, MemSubOpcode},
    module::VbcModule,
    serialize::serialize_module,
};

thread_local! {
    static ALLOCATIONS: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new());
}
extern "C" fn allocate(size: u64) -> *mut u64 {
    assert!(size <= 4096, "bounded endian fixture allocation");
    ALLOCATIONS.with(|allocations| {
        let mut bytes = List::from_elem(0u64, size.max(8).div_ceil(8) as usize).into_boxed_slice();
        let pointer = bytes.as_mut_ptr();
        allocations.borrow_mut().push(bytes);
        pointer
    })
}

fn source(source: &str) -> VbcModule {
    VbcCodegen::new()
        .compile_module(&Parser::new(source).parse_module().expect("grammar"))
        .expect("source VBC")
}

fn decoded_wire(module: &VbcModule) -> VbcModule {
    let mut wire = deserialize_module(&serialize_module(module).expect("wire")).expect("reload");
    // The native API consumes decoded bodies, as does the real archive loader.
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end])
            .expect("decode body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    wire
}

fn reachable_ir(module: &Module, root: &str) -> Text {
    let mut text = Text::new();
    let full = module.print_to_string();
    for line in full.to_str().expect("IR UTF8").lines() {
        if line.starts_with("target ")
            || line.starts_with("attributes #")
            || (line.starts_with('%') && line.contains(" = type "))
        {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str(
        "declare ptr @verum_cbgr_allocate(i64)\ndeclare ptr @verum_checked_malloc(i64)\n",
    );
    let mut pending: List<Text> = [Text::from(root)].into_iter().collect();
    let mut seen: Set<Text> = [
        Text::from("verum_cbgr_allocate"),
        Text::from("verum_checked_malloc"),
    ]
    .into_iter()
    .collect();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let item = if let Some(function) = module.get_function(&name) {
            function.print_to_string()
        } else if let Some(global) = module.get_global(&name) {
            global.print_to_string()
        } else {
            continue;
        };
        let item = item.to_str().expect("IR item");
        text.push_str(item);
        text.push('\n');
        for tail in item.split('@').skip(1) {
            let name = tail
                .split(|ch: char| !ch.is_ascii_alphanumeric() && !"_.$".contains(ch))
                .next()
                .unwrap();
            if module.get_function(name).is_some() || module.get_global(name).is_some() {
                pending.push(Text::from(name));
            }
        }
    }
    text
}

fn native(module: &VbcModule, check: impl Fn(&verum_llvm::execution_engine::ExecutionEngine)) {
    native_with_ir(module, |_| {}, check);
}

fn native_with_ir(
    module: &VbcModule,
    check_ir: impl Fn(&str),
    check: impl Fn(&verum_llvm::execution_engine::ExecutionEngine),
) {
    native_case_with_ir(module, "", check_ir, check);
}

fn native_case_with_ir(
    module: &VbcModule,
    case: &str,
    check_ir: impl Fn(&str),
    check: impl Fn(&verum_llvm::execution_engine::ExecutionEngine),
) {
    let wire = decoded_wire(module);
    for (route, module) in [("source", module), ("wire", &wire)] {
        Target::initialize_native(&InitializationConfig::default()).expect("native target");
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("endian_native").with_debug_info(false),
        );
        lowering.lower_module(module).unwrap_or_else(|error| {
            for function in &module.functions {
                if function.has_source_body {
                    eprintln!(
                        "{route} body {:?}: {:?}",
                        module.get_string(function.name),
                        function.instructions
                    );
                }
            }
            panic!("{route}: native lowering: {error:?}")
        });
        let probe_ir = lowering
            .module()
            .get_function("probe")
            .expect("probe")
            .print_to_string();
        check_ir(probe_ir.to_str().expect("probe IR UTF8"));
        let text = reachable_ir(lowering.module(), "probe");
        if let Ok(directory) = std::env::var("VERUM_T1698_IR_DIR") {
            std::fs::create_dir_all(&directory).expect("IR evidence directory");
            let thread = std::thread::current();
            let suffix: Text = if case.is_empty() {
                "".into()
            } else {
                format!("-{case}").into()
            };
            std::fs::write(
                std::path::Path::new(&directory).join(format!(
                    "{}{suffix}-{route}.ll",
                    thread.name().unwrap_or("endian")
                )),
                text.as_bytes(),
            )
            .expect("IR evidence");
        }
        let executable = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                text.as_bytes(),
                "endian_native",
            ))
            .expect("reachable IR");
        executable.verify().expect("valid IR");
        let engine = executable
            .create_jit_execution_engine(OptimizationLevel::None)
            .expect("JIT");
        for name in ["verum_cbgr_allocate", "verum_checked_malloc"] {
            engine.add_global_mapping(
                &executable.get_function(name).unwrap(),
                allocate as *const () as usize,
            );
        }
        check(&engine);
        ALLOCATIONS.with(|allocations| allocations.borrow_mut().clear());
    }
}

fn packed_result(body: &str, declarations: &str, expected: &[u8]) {
    let module = source(&format!(
        "{declarations} fn probe() -> [Byte; {}] {{ {body} }}",
        expected.len()
    ));
    native(&module, |engine| {
        // SAFETY: the source function accepts no arguments and returns a pointer
        // slot. Only inspect a verified live fixture allocation of enough bytes.
        let pointer = unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> *const u8>("probe")
                .unwrap()
                .call()
        };
        ALLOCATIONS.with(|allocations| {
            let allocations = allocations.borrow();
            let allocation = allocations
                .iter()
                .find(|allocation| allocation.as_ptr() as *const u8 == pointer)
                .unwrap_or_else(|| {
                    panic!(
                        "result {pointer:p} outside live allocations {:?}",
                        allocations
                            .iter()
                            .map(|value| (value.as_ptr(), value.len()))
                            .collect::<List<_>>()
                    )
                });
            assert!(allocation.len() * 8 >= expected.len());
            let bytes = unsafe { std::slice::from_raw_parts(pointer, expected.len()) };
            assert_eq!(bytes, expected, "{body}");
        });
    });
}

const U64_METHOD: &str = "implement UInt64 { fn to_be_bytes(self) -> [Byte; 8] { to_be_bytes_8(self) } fn to_le_bytes(self) -> [Byte; 8] { to_le_bytes_8(self) } }";

#[test]
fn native_dynamic_endian_returns_packed_bytes() {
    for (owner, value, expected) in [
        (
            "UInt64",
            "0x0102030405060708",
            &[1, 2, 3, 4, 5, 6, 7, 8][..],
        ),
        ("Int32", "-2", &[255, 255, 255, 254][..]),
    ] {
        for endian in ["be", "le"] {
            let mut bytes: List<u8> = expected.iter().copied().collect();
            if endian == "le" {
                bytes.reverse();
            }
            packed_result(
                &format!("let value: {owner} = {value}; value.to_{endian}_bytes()"),
                "",
                &bytes,
            );
        }
    }
}

#[test]
fn native_inferred_and_annotated_endian_results_have_the_same_bytes() {
    for binding in ["let bytes =", "let bytes: [Byte; 8] ="] {
        packed_result(
            &format!(
                "let value: UInt64 = 0x0102030405060708; {binding} value.to_be_bytes(); bytes"
            ),
            U64_METHOD,
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
    }
}

#[test]
fn native_inline_endian_widths_are_packed() {
    for width in [2, 4, 8] {
        for endian in ["be", "le"] {
            let mut expected: List<u8> = (9 - width..=8).collect();
            if endian == "le" {
                expected.reverse();
            }
            packed_result(
                &format!("to_{endian}_bytes_{width}(0x0102030405060708)"),
                "",
                &expected,
            );
        }
    }
}

#[test]
fn native_dynamic_from_bytes_refuses_an_unproved_array_parameter() {
    for (owner, width) in [("UInt64", 8), ("Int32", 4)] {
        for endian in ["be", "le"] {
            let module = source(&format!(
                "fn probe(bytes: [Byte; {width}]) -> {owner} {{ {owner}.from_{endian}_bytes(bytes) }}"
            ));
            let context = Context::create();
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("endian_parameter_refusal").with_debug_info(false),
            );
            assert!(matches!(
                lowering.lower_module(&module),
                Err(verum_codegen::llvm::LlvmLoweringError::UnprovenArrayStorage(_))
            ));
        }
    }
}

#[test]
fn native_inferred_and_annotated_indexing_preserve_byte_access() {
    for binding in ["let bytes =", "let bytes: [Byte; 8] ="] {
        let module = source(&format!(
            "{U64_METHOD} fn probe() -> Int {{ let value: UInt64 = 0x0102030405060708; {binding} value.to_be_bytes(); bytes[7] as Int }}"
        ));
        native_with_ir(
            &module,
            |ir| {
                assert!(
                    ir.contains("array_storage_in_bounds") && ir.contains("ba_load_ptr"),
                    "{binding}: selected native producer did not authorize a checked byte load"
                );
                assert!(
                    !ir.contains("geteu"),
                    "{binding}: generic header probe remains"
                );
            },
            |engine| {
                // SAFETY: the source function has no parameters and returns Int.
                let value = unsafe {
                    engine
                        .get_function::<unsafe extern "C" fn() -> i64>("probe")
                        .unwrap()
                        .call()
                };
                assert_eq!(value, 8, "{binding}");
            },
        );
    }
}

#[test]
fn native_fixed_intrinsic_input_preserves_packed_byte_access() {
    let module = source(
        "fn probe() -> Int { let bytes: [Byte;8] = [1,2,3,4,5,6,7,8]; from_be_bytes_8(bytes) }",
    );
    native_with_ir(
        &module,
        |ir| {
            assert!(ir.contains("array_storage_in_bounds") && ir.contains("ba_load_ptr"));
            assert!(
                !ir.contains("geteu"),
                "packed input still uses a generic header probe"
            );
        },
        |engine| {
            // SAFETY: source probe is zero-argument and returns one Int slot.
            let value = unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(value, 0x0102030405060708);
        },
    );
}

#[test]
fn native_list_input_keeps_its_declared_compatibility() {
    // The source library's pointer-sized methods accept List<Byte> and delegate
    // to these intrinsics. Keep that producer distinct from packed fixed bytes.
    let module = source(
        "fn probe() -> Int { let bytes: List<Byte> = [1 as Byte,2 as Byte,3 as Byte,4 as Byte,5 as Byte,6 as Byte,7 as Byte,8 as Byte]; from_be_bytes_8(bytes) }",
    );
    native(&module, |engine| {
        // SAFETY: source probe is zero-argument and returns one Int slot.
        let value = unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("probe")
                .unwrap()
                .call()
        };
        assert_eq!(value, 0x0102030405060708);
    });
}

#[test]
fn native_array_calls_preserve_list_and_packed_bodies_in_both_source_orders() {
    for callee_first in [true, false] {
        for binding in ["let bytes =", "let bytes: [Byte; 2] ="] {
            for (body, packed) in [
                ("let bytes: [Byte; 2] = [7, 9]; bytes", true),
                ("[7 as Byte, 9 as Byte]", false),
            ] {
                let callee = format!("fn selected() -> [Byte; 2] {{ {body} }}");
                let caller =
                    format!("fn probe() -> Int {{ {binding} selected(); bytes[1] as Int }}");
                let text = if callee_first {
                    format!("{callee} {caller}")
                } else {
                    format!("{caller} {callee}")
                };
                native_with_ir(
                    &source(&text),
                    |ir| {
                        assert_eq!(
                            ir.contains("ba_load_ptr"),
                            packed,
                            "{binding}, order {callee_first}"
                        );
                        if packed {
                            assert!(!ir.contains("geteu"));
                        }
                    },
                    |engine| {
                        // SAFETY: this exact source declares zero parameters and Int.
                        let actual = unsafe {
                            engine
                                .get_function::<unsafe extern "C" fn() -> i64>("probe")
                                .unwrap()
                                .call()
                        };
                        assert_eq!(actual, 9);
                    },
                );
            }
        }
    }
}

#[test]
fn native_unproved_array_calls_refuse_before_any_jit_execution() {
    for (declarations, call) in [
        (
            "fn source_array() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes } fn selected() -> [Byte; 2] { source_array() }",
            "selected()",
        ),
        (
            "fn selected(bytes: [Byte; 2]) -> [Byte; 2] { bytes }",
            "selected([7 as Byte, 9 as Byte])",
        ),
        (
            "fn selected(flag: Bool) -> [Byte; 2] { if flag { let bytes: [Byte; 2] = [7, 9]; bytes } else { [7 as Byte, 9 as Byte] } }",
            "selected(true)",
        ),
    ] {
        let module = source(&format!(
            "{declarations} fn probe() -> Int {{ let bytes = {call}; bytes[1] as Int }}"
        ));
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("array_refusal").with_debug_info(false),
        );
        assert!(
            matches!(
                lowering.lower_module(&module),
                Err(verum_codegen::llvm::LlvmLoweringError::UnprovenArrayStorage(_))
            ),
            "{call}"
        );
    }
}

#[test]
fn native_colliding_array_bodies_do_not_gain_a_second_selection_rule() {
    let mut module = source(
        "fn selected() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes } fn probe() -> Int { let bytes = selected(); bytes[1] as Int }",
    );
    let mut duplicate = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("selected"))
        .unwrap()
        .clone();
    duplicate.id =
        verum_vbc::module::FunctionId(module.functions.iter().map(|f| f.id.0).max().unwrap() + 1);
    module.functions.push(duplicate);
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("array_collision").with_debug_info(false),
    );
    assert!(matches!(
        lowering.lower_module(&module),
        Err(verum_codegen::llvm::LlvmLoweringError::UnprovenArrayStorage(_))
    ));
}

#[test]
fn native_loop_over_an_array_call_is_explicitly_pending() {
    let module = source(
        r#"
fn selected() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes }
fn probe() -> Int {
    let bytes = selected();
    let mut index = 0;
    let mut total = 0;
    while index < 2 { total = total + bytes[index] as Int; index = index + 1; }
    total
}
"#,
    );
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("array_loop_refusal").with_debug_info(false),
    );
    assert!(matches!(
        lowering.lower_module(&module),
        Err(verum_codegen::llvm::LlvmLoweringError::UnprovenArrayStorage(_))
    ));
}

#[test]
fn native_selected_packed_arrays_preserve_length_and_indexed_mutation() {
    let module = source(
        r#"
fn selected() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes }
fn probe() -> Int {
    let mut bytes = selected();
    bytes[1] = 11;
    (bytes[1] as Int) + bytes.len()
}
"#,
    );
    native_with_ir(
        &module,
        |ir| {
            assert!(ir.contains("ba_store_ptr") && ir.contains("ba_load_ptr"));
            assert!(!ir.contains("geteu") && !ir.contains("len_hdr_tid"));
        },
        |engine| {
            // SAFETY: the exact source function has no parameters and returns Int.
            let actual = unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(actual, 13);
        },
    );
}

#[test]
fn native_selected_packed_array_access_uses_canonical_wide_registers() {
    let mut text = Text::from(
        "fn selected() -> [Byte; 2] { let bytes: [Byte; 2] = [7, 9]; bytes } fn probe() -> Int {",
    );
    for index in 0..270 {
        text.push_str(&format!("let pressure{index} = {index};\n"));
    }
    text.push_str("let bytes: [Byte; 2] = selected(); bytes[1] as Int }");
    let module = source(&text);
    let body = module
        .functions
        .iter()
        .find(|f| module.get_string(f.name) == Some("probe"))
        .and_then(|f| f.instructions.as_deref())
        .unwrap();
    assert!(body.iter().any(|instruction| matches!(instruction,
        verum_vbc::instruction::Instruction::GetE { arr, .. } if arr.0 >= 256)));
    native_with_ir(
        &module,
        |ir| {
            assert!(ir.contains("ba_load_ptr") && !ir.contains("geteu"));
        },
        |engine| {
            // SAFETY: the exact source function has no parameters and returns Int.
            let actual = unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .unwrap()
                    .call()
            };
            assert_eq!(actual, 9);
        },
    );
}

#[test]
fn native_selected_typed_array_results_keep_integer_and_float_geometry() {
    for (element, packed, listed, replacement, initial, mutated) in [
        (
            "UInt32",
            "let values: [UInt32; 2] = [1, 65539]; values",
            "[1 as UInt32, 65539 as UInt32]",
            "65541",
            65539.0,
            65541.0,
        ),
        (
            "Float",
            "let values: [Float; 2] = [1.25, -0.5]; values",
            "[1.25, -0.5]",
            "-2.75",
            -0.5,
            -2.75,
        ),
    ] {
        for (storage, body) in [("packed", packed), ("list", listed)] {
            for annotated in [false, true] {
                let binding: Text = if annotated {
                    format!("let mut values: [{element}; 2] =").into()
                } else {
                    "let mut values =".into()
                };
                for operation in ["read", "mutate", "length"] {
                    let float_result = element == "Float" && operation != "length";
                    let return_type = if float_result { "Float" } else { "Int" };
                    let value = if operation == "length" {
                        "values.len()"
                    } else if float_result {
                        "values[1]"
                    } else {
                        "values[1] as Int"
                    };
                    let mutation: Text = if operation == "mutate" {
                        format!("values[1] = {replacement};").into()
                    } else {
                        "".into()
                    };
                    let expected = match operation {
                        "read" => initial,
                        "mutate" => mutated,
                        _ => 2.0,
                    };
                    let case: Text =
                        format!("{element}-{storage}-annotated{annotated}-{operation}").into();
                    let module = source(&format!(
                        "fn selected() -> [{element}; 2] {{ {body} }} fn probe() -> {return_type} {{ {binding} selected(); {mutation} {value} }}"
                    ));
                    // The two source fixtures must actually emit different
                    // producers, despite having the same declared array shape.
                    let selected = module
                        .functions
                        .iter()
                        .find(|function| module.get_string(function.name) == Some("selected"))
                        .expect("exact selected producer");
                    let body = selected.instructions.as_deref().expect("producer body");
                    assert!(body.iter().any(|instruction| {
                        if storage == "list" {
                            matches!(instruction, Instruction::NewList { .. })
                        } else {
                            matches!(instruction, Instruction::MemExtended { sub_op, .. }
                                if MemSubOpcode::from_byte(*sub_op) == Some(MemSubOpcode::NewTypedArray))
                        }
                    }), "{case}: fixture lost its physical producer distinction");
                    native_case_with_ir(
                        &module,
                        case.as_str(),
                        |ir| {
                            if storage == "packed" {
                                assert!(
                                    !ir.contains("geteu") && !ir.contains("len_hdr_tid"),
                                    "{case}: generic header probe remains"
                                );
                                if operation != "length" {
                                    assert!(
                                        ir.contains("array_storage_in_bounds"),
                                        "{case}: checked array access absent"
                                    );
                                }
                            }
                        },
                        |engine| {
                            // SAFETY: each source probe is parameterless and its
                            // scalar ABI is fixed by return_type above.
                            if float_result {
                                let actual = unsafe {
                                    engine
                                        .get_function::<unsafe extern "C" fn() -> f64>("probe")
                                        .unwrap()
                                        .call()
                                };
                                assert_eq!(actual, expected, "{case}");
                            } else {
                                let actual = unsafe {
                                    engine
                                        .get_function::<unsafe extern "C" fn() -> i64>("probe")
                                        .unwrap()
                                        .call()
                                };
                                // Integer fixtures and lengths are exactly
                                // representable in this table's scalar oracle.
                                assert_eq!(actual, expected as i64, "{case}");
                            }
                        },
                    );
                }
            }
        }
    }
}

#[test]
fn native_fixed_array_parameters_require_an_actual_argument_contract() {
    for source_text in [
        "fn read(bytes: [Byte; 2]) -> Int { bytes[1] as Int } fn probe() -> Int { let bytes: [Byte; 2] = [7, 9]; read(bytes) }",
        "fn read(bytes: [Byte; 2]) -> Int { bytes[1] as Int } fn probe() -> Int { read([7 as Byte, 9 as Byte]) }",
        "fn read(bytes: &[Byte; 2]) -> Int { bytes[1] as Int } fn probe() -> Int { let bytes: [Byte; 2] = [7, 9]; read(&bytes) }",
        "fn read(bytes: [Byte; 2]) -> Int { bytes[1] as Int } fn forwarded(bytes: [Byte; 2]) -> Int { read(bytes) } fn probe() -> Int { let bytes: [Byte; 2] = [7, 9]; forwarded(bytes) }",
    ] {
        let module = source(source_text);
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("array_parameter_refusal").with_debug_info(false),
        );
        assert!(
            matches!(
                lowering.lower_module(&module),
                Err(verum_codegen::llvm::LlvmLoweringError::UnprovenArrayStorage(_))
            ),
            "{source_text}"
        );
    }
}

#[test]
fn native_array_refusal_is_scoped_to_reachable_program_helpers() {
    assert_ne!(
        std::env::var("VERUM_NO_REACHABILITY_LOWERING").as_deref(),
        Ok("1"),
        "this control requires normal program reachability, not the diagnostic opt-out"
    );
    for (called, entry) in [
        (false, "fn main() -> Int { 41 }"),
        (
            true,
            "fn main() -> Int { array_read([7 as Byte, 9 as Byte]) }",
        ),
    ] {
        let module = source(&format!(
            "fn array_read(bytes: [Byte; 2]) -> Int {{ bytes[1] as Int }} {entry}"
        ));
        let wire = decoded_wire(&module);
        for (route, module) in [("source", &module), ("wire", &wire)] {
            let helper = module.find_function_by_name("array_read").expect("helper");
            let reachability = verum_vbc::reachability::analyze(module);
            assert_eq!(
                reachability.reachable_ids.contains(&helper.0),
                called,
                "{route}: the unchanged helper's call edge must be the causal difference"
            );
            let context = Context::create();
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("reachable_array_refusal").with_debug_info(false),
            );
            let result = lowering.lower_module(module);
            if called {
                assert!(
                    matches!(
                        result,
                        Err(verum_codegen::llvm::LlvmLoweringError::UnprovenArrayStorage(_))
                    ),
                    "{route}: a required unproved array consumer must refuse: {result:?}"
                );
            } else {
                result.unwrap_or_else(|error| {
                    panic!("{route}: an unreachable helper blocked the independent main: {error:?}")
                });
                let entry = lowering.module().get_function("main").expect("native main");
                assert!(entry.count_basic_blocks() > 0, "{route}: main has a body");
            }
        }
    }
}
