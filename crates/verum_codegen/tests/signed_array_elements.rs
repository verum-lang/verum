//! T1706: signed element meaning through actual source and decoded-wire LLVM JIT.
//! Bounded fixture allocations only; no CLI, AOT or allocator-lifecycle claim.
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
    codegen::VbcCodegen, deserialize::deserialize_module, module::VbcModule,
    serialize::serialize_module,
};

thread_local! {
    static ALLOCATIONS: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new());
}
extern "C" fn allocate(size: u64) -> *mut u64 {
    assert!(size <= 4096, "bounded signed-array fixture allocation");
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
    let wire = decoded_wire(module);
    let mut failures = List::<Text>::new();
    for (route, module) in [("source", module), ("wire", &wire)] {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Target::initialize_native(&InitializationConfig::default()).expect("native target");
            let context = Context::create();
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("signed_elements_native").with_debug_info(false),
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
            if let Ok(directory) = std::env::var("VERUM_T1706_NATIVE_IR_DIR") {
                std::fs::create_dir_all(&directory).expect("IR evidence directory");
                let thread = std::thread::current();
                std::fs::write(
                    std::path::Path::new(&directory).join(format!(
                        "{}-{route}.ll",
                        thread.name().unwrap_or("signed-elements")
                    )),
                    text.as_bytes(),
                )
                .expect("IR evidence");
            }
            let executable = context
                .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                    text.as_bytes(),
                    "signed_elements_native",
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
        }));
        ALLOCATIONS.with(|allocations| allocations.borrow_mut().clear());
        if let Err(error) = result {
            let message = error
                .downcast_ref::<Text>()
                .map(|value| value.as_str())
                .or_else(|| error.downcast_ref::<&str>().copied())
                .unwrap_or("native control panicked; see route diagnostics");
            failures.push(format!("{route}: {message}").into());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[derive(Clone, Copy, Debug)]
enum Boundary {
    Local,
    ArrayReturn,
    ListReturn,
}

fn elements(
    prelude: &str,
    element: &str,
    producer: &str,
    first: i64,
    second: i64,
    boundary: Boundary,
) {
    let checks = "(values[0] as Int) + (values[1] as Int) + (values[2] as Int)";
    let text = match boundary {
        Boundary::Local => format!("{prelude} fn probe() -> Int {{ {producer} {checks} }}"),
        Boundary::ArrayReturn => format!(
            "{prelude} fn make() -> [{element}; 3] {{ {producer} values }} fn probe() -> Int {{ let values = make(); {checks} }}"
        ),
        Boundary::ListReturn => format!(
            "{prelude} fn make() -> List<{element}> {{ {producer} values }} fn probe() -> Int {{ let values = make(); {checks} }}"
        ),
    };
    let module = source(&text);
    let packed_return = matches!(boundary, Boundary::ArrayReturn)
        && module
            .functions
            .iter()
            .find(|function| module.get_string(function.name) == Some("make"))
            .and_then(|function| function.instructions.as_deref())
            .and_then(verum_vbc::array_storage::straight_line_array_return)
            .is_some_and(|fact| {
                matches!(
                    fact,
                    verum_vbc::array_storage::ArrayResultFact::Packed { .. }
                )
            });
    native_with_ir(
        &module,
        |ir| {
            if packed_return {
                assert!(
                    !ir.contains("geteu_cv_"),
                    "actual packed result reached an unproved container-header read; JIT withheld\n{ir}"
                );
            }
        },
        |engine| {
            // SAFETY: the actual source declares this zero-argument Int ABI.
            let value = unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .expect("native probe")
                    .call()
            };
            assert_eq!(value, first + second + 1, "{element} {boundary:?}: {text}");
        },
    );
}

macro_rules! element_case {
    ($name:ident, $element:literal, $first:expr, $second:expr, $boundary:ident) => {
        #[test]
        fn $name() {
            elements(
                "",
                $element,
                &format!(
                    "let values: [{}; 3] = [{}, {}, 1];",
                    $element, $first, $second
                ),
                $first,
                $second,
                Boundary::$boundary,
            );
        }
    };
}

element_case!(native_signed_byte_local, "Int8", -128, -1, Local);
element_case!(
    native_signed_byte_array_return,
    "Int8",
    -128,
    -1,
    ArrayReturn
);
element_case!(native_signed_byte_list_return, "Int8", -128, -1, ListReturn);
element_case!(native_signed_short_local, "Int16", -32768, -1, Local);
element_case!(
    native_signed_short_array_return,
    "Int16",
    -32768,
    -1,
    ArrayReturn
);
element_case!(
    native_signed_short_list_return,
    "Int16",
    -32768,
    -1,
    ListReturn
);
element_case!(
    native_signed_word_local,
    "Int32",
    -2147483648_i64,
    -1,
    Local
);
element_case!(
    native_signed_word_array_return,
    "Int32",
    -2147483648_i64,
    -1,
    ArrayReturn
);
element_case!(
    native_signed_word_list_return,
    "Int32",
    -2147483648_i64,
    -1,
    ListReturn
);
element_case!(native_unsigned_byte_local, "Byte", 128, 255, Local);
element_case!(
    native_unsigned_byte_array_return,
    "Byte",
    128,
    255,
    ArrayReturn
);
element_case!(
    native_unsigned_byte_list_return,
    "Byte",
    128,
    255,
    ListReturn
);
element_case!(native_unsigned_short_local, "UInt16", 32768, 65535, Local);
element_case!(
    native_unsigned_short_array_return,
    "UInt16",
    32768,
    65535,
    ArrayReturn
);
element_case!(
    native_unsigned_short_list_return,
    "UInt16",
    32768,
    65535,
    ListReturn
);
element_case!(
    native_unsigned_word_local,
    "UInt32",
    2147483648_i64,
    4294967295_i64,
    Local
);
element_case!(
    native_unsigned_word_array_return,
    "UInt32",
    2147483648_i64,
    4294967295_i64,
    ArrayReturn
);
element_case!(
    native_unsigned_word_list_return,
    "UInt32",
    2147483648_i64,
    4294967295_i64,
    ListReturn
);

const PACKED_BYTE: &str =
    "let mut values: [Int8; 3] = [0_i8; 3]; values[0] = -128; values[1] = -1; values[2] = 1;";

#[test]
fn native_actual_packed_byte_local() {
    elements("", "Int8", PACKED_BYTE, -128, -1, Boundary::Local);
}
#[test]
fn native_actual_packed_byte_array_return() {
    elements("", "Int8", PACKED_BYTE, -128, -1, Boundary::ArrayReturn);
}
#[test]
fn native_actual_packed_byte_list_return() {
    elements("", "Int8", PACKED_BYTE, -128, -1, Boundary::ListReturn);
}
#[test]
fn native_packed_signed_alias_array_return() {
    elements(
        "type SignedShort is Int16;",
        "SignedShort",
        "let values: [Int16; 3] = [-32768, -1, 1];",
        -32768,
        -1,
        Boundary::ArrayReturn,
    );
}
#[test]
fn native_packed_signed_alias_list_return() {
    elements(
        "type SignedShort is Int16;",
        "SignedShort",
        "let values: [Int16; 3] = [-32768, -1, 1];",
        -32768,
        -1,
        Boundary::ListReturn,
    );
}
#[test]
fn native_packed_unsigned_alias_array_return() {
    elements(
        "type UnsignedShort is UInt16;",
        "UnsignedShort",
        "let values: [UInt16; 3] = [32768, 65535, 1];",
        32768,
        65535,
        Boundary::ArrayReturn,
    );
}
#[test]
fn native_packed_unsigned_alias_list_return() {
    elements(
        "type UnsignedShort is UInt16;",
        "UnsignedShort",
        "let values: [UInt16; 3] = [32768, 65535, 1];",
        32768,
        65535,
        Boundary::ListReturn,
    );
}

fn list_payload(text: &str, expected: &[i64]) {
    let module = source(text);
    native(&module, |engine| {
        // SAFETY: the actual source declares a zero-argument List-returning
        // function. Inspect only allocations owned by the bounded fixture.
        let pointer = unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> *const u64>("probe")
                .expect("List probe")
                .call()
        };
        ALLOCATIONS.with(|allocations| {
            let allocations = allocations.borrow();
            let header = allocations
                .iter()
                .find(|allocation| allocation.as_ptr() == pointer)
                .expect("List header must be an owned allocation");
            assert!(header.len() * 8 >= verum_common::layout::LIST_OBJECT_SIZE as usize);
            assert_eq!(
                header[0] as u32,
                verum_vbc::types::TypeId::LIST.0,
                "canonical List storage owner"
            );
            let length = header[verum_common::layout::LIST_LEN_OFFSET as usize / 8] as usize;
            assert_eq!(length, expected.len());
            let data = header[verum_common::layout::LIST_PTR_OFFSET as usize / 8] as *const u64;
            let storage = allocations
                .iter()
                .find(|allocation| allocation.as_ptr() == data)
                .expect("List data must be an owned allocation");
            assert!(storage.len() >= length);
            let actual: List<i64> = storage[..length].iter().map(|word| *word as i64).collect();
            assert_eq!(
                actual.as_slice(),
                expected,
                "native payload before any typed index normalization"
            );
        });
    });
}

#[test]
fn native_packed_signed_byte_list_has_signed_payload() {
    list_payload(
        &format!("fn probe() -> List<Int8> {{ {PACKED_BYTE} values }}"),
        &[-128, -1, 1],
    );
}
#[test]
fn native_packed_signed_short_list_has_signed_payload() {
    list_payload(
        "fn probe() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; values }",
        &[-32768, -1, 1],
    );
}
#[test]
fn native_packed_signed_word_list_has_signed_payload() {
    list_payload(
        "fn probe() -> List<Int32> { let values: [Int32; 3] = [-2147483648, -1, 1]; values }",
        &[-2147483648, -1, 1],
    );
}
#[test]
fn native_packed_signed_alias_list_has_signed_payload() {
    list_payload(
        "type SignedShort is Int16; fn probe() -> List<SignedShort> { let values: [Int16; 3] = [-32768, -1, 1]; values }",
        &[-32768, -1, 1],
    );
}
#[test]
fn native_packed_unsigned_alias_list_keeps_high_bit_payload() {
    list_payload(
        "type UnsignedShort is UInt16; fn probe() -> List<UnsignedShort> { let values: [UInt16; 3] = [32768, 65535, 1]; values }",
        &[32768, 65535, 1],
    );
}
#[test]
fn native_signed_read_preserves_packed_proof_for_later_list_conversion() {
    list_payload(
        "fn probe() -> List<Int16> { let values: [Int16; 3] = [-32768, -1, 1]; let first = values[0]; values }",
        &[-32768, -1, 1],
    );
}

#[test]
fn native_existing_sext_uses_width_bytes_after_canonical_wide_registers() {
    use verum_vbc::instruction::{ArithSubOpcode, Instruction, Reg};
    use verum_vbc::module::FunctionDescriptor;
    for (bits, value, expected) in [
        (8, 128, -128),
        (16, 65535, -1),
        (32, 2147483648, -2147483648),
    ] {
        let mut module = VbcModule::new("wide_signed_normalization".into());
        let mut function = FunctionDescriptor::new(module.intern_string("probe"));
        let mut operands = List::new().into();
        verum_vbc::encoding::encode_reg(Reg(270), &mut operands);
        verum_vbc::encoding::encode_reg(Reg(257), &mut operands);
        operands.extend([bits, 64]);
        let body = vec![
            Instruction::LoadI {
                dst: Reg(257),
                value,
            },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::SextI.to_byte(),
                operands,
            },
            Instruction::Ret { value: Reg(270) },
        ];
        for instruction in &body {
            verum_vbc::bytecode::encode_instruction(instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32;
        function.register_count = 271;
        function.return_type = verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::INT);
        function.instructions = Some(body);
        module.add_function(function);
        native(&module, |engine| {
            // SAFETY: explicit descriptor above declares this zero-argument Int ABI.
            let actual = unsafe {
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("probe")
                    .expect("wide-register probe")
                    .call()
            };
            assert_eq!(actual, expected, "from_bits={bits}");
        });
    }
}

#[test]
fn native_sext_refuses_truncated_trailing_and_unsupported_width_operands() {
    use verum_vbc::instruction::{ArithSubOpcode, Instruction, Reg};
    use verum_vbc::module::FunctionDescriptor;
    let mut prefix = List::new().into();
    verum_vbc::encoding::encode_reg(Reg(270), &mut prefix);
    verum_vbc::encoding::encode_reg(Reg(257), &mut prefix);
    let mut cases: List<(Text, List<u8>, &str)> = List::new();
    cases.push((
        "truncated destination".into(),
        List::from_iter([0x81]),
        "read_reg_varlen",
    ));
    for (label, widths) in [
        ("missing widths", &[][..]),
        ("missing target", &[16][..]),
        ("trailing operand", &[16, 64, 0][..]),
    ] {
        let mut operands: List<u8> = prefix.iter().copied().collect();
        operands.extend(widths.iter().copied());
        cases.push((
            label.into(),
            operands,
            "SextI requires source and target widths",
        ));
    }
    for widths in [[0, 64], [128, 64], [16, 0], [64, 16]] {
        let mut operands: List<u8> = prefix.iter().copied().collect();
        operands.extend(widths);
        cases.push((
            format!("unsupported widths {widths:?}").into(),
            operands,
            "SextI has unsupported integer widths",
        ));
    }
    for (label, operands, expected) in cases {
        let mut module = VbcModule::new("malformed_signed_normalization".into());
        let mut function = FunctionDescriptor::new(module.intern_string("probe"));
        let body = vec![
            Instruction::LoadI {
                dst: Reg(257),
                value: 32768,
            },
            Instruction::ArithExtended {
                sub_op: ArithSubOpcode::SextI.to_byte(),
                operands: operands.into(),
            },
            Instruction::Ret { value: Reg(270) },
        ];
        for instruction in &body {
            verum_vbc::bytecode::encode_instruction(instruction, &mut module.bytecode);
        }
        function.bytecode_length = module.bytecode.len() as u32;
        function.register_count = 271;
        function.return_type = verum_vbc::types::TypeRef::Concrete(verum_vbc::types::TypeId::INT);
        function.instructions = Some(body);
        module.add_function(function);
        let wire = decoded_wire(&module);
        for (route, module) in [("source instructions", &module), ("decoded wire", &wire)] {
            let context = Context::create();
            let mut lowering = VbcToLlvmLowering::new(
                &context,
                LoweringConfig::debug("invalid_sext").with_debug_info(false),
            );
            let result = lowering.lower_module(module);
            assert!(
                result.is_err(),
                "{label} {route}: malformed instruction was accepted"
            );
            let error = format!("{:?}", result.unwrap_err());
            assert!(
                error.contains(expected),
                "{label} {route}: unrelated refusal {error}"
            );
        }
    }
}
