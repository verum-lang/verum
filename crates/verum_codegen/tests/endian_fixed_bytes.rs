//! T1698: native numeric endian producers and consumers share fixed byte storage.
//! The JIT uses a bounded host allocation substrate; this is not an AOT or
//! allocator-lifecycle acceptance test. Production lowering and helpers are real.
use std::cell::RefCell;
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::{Heap, List, Set, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel, context::Context, memory_buffer::MemoryBuffer, module::Module,
    targets::{InitializationConfig, Target}, values::AnyValue,
};
use verum_vbc::{
    codegen::VbcCodegen, deserialize::deserialize_module,
    module::VbcModule, serialize::serialize_module,
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
    VbcCodegen::new().compile_module(&Parser::new(source).parse_module().expect("grammar"))
        .expect("source VBC")
}

fn reachable_ir(module: &Module, root: &str) -> Text {
    let mut text = Text::new();
    let full = module.print_to_string();
    for line in full.to_str().expect("IR UTF8").lines() {
        if line.starts_with("target ") || line.starts_with("attributes #")
            || (line.starts_with('%') && line.contains(" = type ")) {
            text.push_str(line); text.push('\n');
        }
    }
    text.push_str("declare ptr @verum_cbgr_allocate(i64)\ndeclare ptr @verum_checked_malloc(i64)\n");
    let mut pending: List<Text> = [Text::from(root)].into_iter().collect();
    let mut seen: Set<Text> = [Text::from("verum_cbgr_allocate"), Text::from("verum_checked_malloc")].into_iter().collect();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) { continue; }
        let item = if let Some(function) = module.get_function(&name) { function.print_to_string() }
            else if let Some(global) = module.get_global(&name) { global.print_to_string() }
            else { continue; };
        let item = item.to_str().expect("IR item");
        text.push_str(item); text.push('\n');
        for tail in item.split('@').skip(1) {
            let name = tail.split(|ch: char| !ch.is_ascii_alphanumeric() && !"_.$".contains(ch)).next().unwrap();
            if module.get_function(name).is_some() || module.get_global(name).is_some() {
                pending.push(Text::from(name));
            }
        }
    }
    text
}

fn native(module: &VbcModule, check: impl Fn(&verum_llvm::execution_engine::ExecutionEngine)) {
    let mut wire = deserialize_module(&serialize_module(module).expect("wire")).expect("reload");
    // The native API consumes decoded bodies, as does the real archive loader.
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end]).expect("decode body");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    for (route, module) in [("source", module), ("wire", &wire)] {
        Target::initialize_native(&InitializationConfig::default()).expect("native target");
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(&context,
            LoweringConfig::debug("endian_native").with_debug_info(false));
        lowering.lower_module(module).expect("native lowering");
        let text = reachable_ir(lowering.module(), "probe");
        if let Ok(directory) = std::env::var("VERUM_T1698_IR_DIR") {
            std::fs::create_dir_all(&directory).expect("IR evidence directory");
            let thread = std::thread::current();
            std::fs::write(std::path::Path::new(&directory).join(format!("{}-{route}.ll", thread.name().unwrap_or("endian"))), text.as_bytes()).expect("IR evidence");
        }
        let executable = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            text.as_bytes(), "endian_native")).expect("reachable IR");
        executable.verify().expect("valid IR");
        let engine = executable.create_jit_execution_engine(OptimizationLevel::None).expect("JIT");
        for name in ["verum_cbgr_allocate", "verum_checked_malloc"] {
            engine.add_global_mapping(&executable.get_function(name).unwrap(), allocate as *const () as usize);
        }
        check(&engine);
        ALLOCATIONS.with(|allocations| allocations.borrow_mut().clear());
    }
}

fn packed_result(body: &str, declarations: &str, expected: &[u8]) {
    let module = source(&format!("{declarations} fn probe() -> [Byte; {}] {{ {body} }}", expected.len()));
    native(&module, |engine| {
        // SAFETY: the source function accepts no arguments and returns a pointer
        // slot. Only inspect a verified live fixture allocation of enough bytes.
        let pointer = unsafe { engine.get_function::<unsafe extern "C" fn() -> *const u8>("probe").unwrap().call() };
        ALLOCATIONS.with(|allocations| {
            let allocations = allocations.borrow();
            let allocation = allocations.iter().find(|allocation| allocation.as_ptr() as *const u8 == pointer)
                .unwrap_or_else(|| panic!("result {pointer:p} outside live allocations {:?}", allocations.iter().map(|value| (value.as_ptr(), value.len())).collect::<List<_>>()));
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
        ("UInt64", "0x0102030405060708", &[1,2,3,4,5,6,7,8][..]),
        ("Int32", "-2", &[255,255,255,254][..]),
    ] {
        for endian in ["be", "le"] {
            let mut bytes: List<u8> = expected.iter().copied().collect();
            if endian == "le" { bytes.reverse(); }
            packed_result(&format!("let value: {owner} = {value}; value.to_{endian}_bytes()"), "", &bytes);
        }
    }
}

#[test]
fn native_inferred_and_annotated_endian_results_have_the_same_bytes() {
    for binding in ["let bytes =", "let bytes: [Byte; 8] ="] {
        packed_result(&format!("let value: UInt64 = 0x0102030405060708; {binding} value.to_be_bytes(); bytes"), U64_METHOD, &[1,2,3,4,5,6,7,8]);
    }
}

#[test]
fn native_inline_endian_widths_are_packed() {
    for width in [2, 4, 8] {
        for endian in ["be", "le"] {
            let mut expected: List<u8> = (9-width..=8).collect();
            if endian == "le" { expected.reverse(); }
            packed_result(&format!("to_{endian}_bytes_{width}(0x0102030405060708)"), "", &expected);
        }
    }
}

#[test]
fn native_dynamic_from_bytes_reads_the_packed_input() {
    for (owner, width) in [("UInt64", 8), ("Int32", 4)] {
        for endian in ["be", "le"] {
            let module = source(&format!("fn probe(bytes: [Byte; {width}]) -> {owner} {{ {owner}.from_{endian}_bytes(bytes) }}"));
            native(&module, |engine| {
                // The first width bytes are the real fixed-array argument.
                // Guard storage makes the old accidental List-field probe safe:
                // it reaches a live zero-valued decoy instead of dereferencing
                // arbitrary bytes. The decoy is outside the declared array.
                let decoy = [0u64; 8];
                let mut guarded = [0u64; 16];
                let raw = guarded.as_mut_ptr() as *mut u8;
                let bytes = if width == 8 { &[1,2,3,4,5,6,7,8][..] } else { &[255,255,255,254][..] };
                let expected = if width == 8 { 0x0102030405060708i64 } else { -2 };
                let mut ordered: List<u8> = bytes.iter().copied().collect();
                if endian == "le" { ordered.reverse(); }
                unsafe { std::ptr::copy_nonoverlapping(ordered.as_ptr(), raw, width); }
                guarded[(verum_codegen::llvm::runtime::LIST_PTR_OFFSET / 8) as usize] = decoy.as_ptr() as u64;
                // SAFETY: the source takes one fixed byte-array pointer slot;
                // both the argument and guard remain live throughout the call.
                let actual = unsafe { engine.get_function::<unsafe extern "C" fn(*const u8) -> i64>("probe").unwrap().call(raw) };
                assert_eq!(actual, expected, "{owner} {endian}");
            });
        }
    }
}

#[test]
fn native_inferred_and_annotated_indexing_preserve_byte_access() {
    for binding in ["let bytes =", "let bytes: [Byte; 8] ="] {
        let module = source(&format!("{U64_METHOD} fn probe() -> Int {{ let value: UInt64 = 0x0102030405060708; {binding} value.to_be_bytes(); bytes[7] as Int }}"));
        let probe = module.functions.iter().find(|function| module.get_string(function.name) == Some("probe")).unwrap();
        // Refuse to run a known unsafe generic container read on a packed
        // allocation. This pins the producer/consumer selection before JIT;
        // once selected correctly, the real native read must return byte8.
        assert!(!probe.instructions.as_ref().unwrap().iter().any(|instruction| matches!(instruction,
            verum_vbc::instruction::Instruction::GetE { .. }
        )), "{binding}: fixed endian producer lost byte access before native lowering");
        native(&module, |engine| {
            // SAFETY: the source function has no parameters and returns Int.
            let value = unsafe { engine.get_function::<unsafe extern "C" fn() -> i64>("probe").unwrap().call() };
            assert_eq!(value, 8, "{binding}");
        });
    }
}
