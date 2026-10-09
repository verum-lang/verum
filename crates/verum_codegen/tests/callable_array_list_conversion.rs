//! T1700: proved packed callable results materialize their declared List storage.
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
    codegen::VbcCodegen, deserialize::deserialize_module, module::VbcModule,
    serialize::serialize_module,
};

thread_local! {
    static ALLOCATIONS: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new());
}
extern "C" fn allocate(size: u64) -> *mut u64 {
    assert!(size <= 4096, "bounded callable conversion allocation");
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
    for (route, module) in [("source", module), ("wire", &wire)] {
        Target::initialize_native(&InitializationConfig::default()).expect("native target");
        let context = Context::create();
        let mut lowering = VbcToLlvmLowering::new(
            &context,
            LoweringConfig::debug("array_list_native").with_debug_info(false),
        );
        lowering.lower_module(module).expect("native lowering");
        let text = reachable_ir(lowering.module(), "probe");
        if let Ok(directory) = std::env::var("VERUM_T1700_IR_DIR") {
            std::fs::create_dir_all(&directory).expect("IR evidence directory");
            let thread = std::thread::current();
            std::fs::write(
                std::path::Path::new(&directory).join(format!(
                    "{}-{route}.ll",
                    thread.name().unwrap_or("array-list")
                )),
                text.as_bytes(),
            )
            .expect("IR evidence");
        }
        let executable = context
            .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
                text.as_bytes(),
                "array_list_native",
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

fn check(source_text: &str, expected: i64) {
    native(&source(source_text), |engine| {
        // SAFETY: each checked fixture declares a zero-argument Int probe;
        // all allocations remain live until this call returns.
        let value = unsafe {
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("probe")
                .expect("probe")
                .call()
        };
        assert_eq!(value, expected, "{source_text}");
    });
}

#[test]
fn packed_byte_tail_materializes_growable_native_list() {
    check(
        r#"
fn make() -> List<Byte> { let xs: [Byte; 2] = [1, 2]; xs }
fn probe() -> Int { let mut xs = make(); xs.push(3 as Byte);
    xs.len() * 10000 + (xs[0] as Int) * 100 + (xs[2] as Int) }
"#,
        30103,
    );
}

#[test]
fn explicit_packed_return_materializes_native_list() {
    check(
        r#"
fn make() -> List<Byte> { let xs: [Byte; 2] = [4, 5]; return xs; }
fn probe() -> Int { let mut xs = make(); xs.push(6 as Byte);
    xs.len() * 10000 + (xs[0] as Int) * 100 + (xs[2] as Int) }
"#,
        30406,
    );
}

#[test]
fn empty_packed_return_grows_in_native_code() {
    check(
        r#"
fn make() -> List<Byte> { let xs: [Byte; 0] = []; xs }
fn probe() -> Int { let mut xs = make(); xs.push(7 as Byte);
    xs.len() * 100 + (xs[0] as Int) }
"#,
        107,
    );
}

#[test]
fn nonbyte_packed_return_keeps_native_element_width() {
    check(
        r#"
fn make() -> List<UInt32> { let mut xs: [UInt32; 2] = [0; 2];
    xs[0] = 65537; xs[1] = 262147; xs }
fn probe() -> Int { let mut xs = make(); xs.push(7 as UInt32);
    if xs.len() == 3 && xs[0] == 65537 && xs[1] == 262147 && xs[2] == 7 { 1 } else { 0 } }
"#,
        1,
    );
}

#[test]
fn floating_packed_return_keeps_native_values() {
    check(
        r#"
fn make() -> List<Float> { let mut xs: [Float; 2] = [0.0; 2];
    xs[0] = 1.25; xs[1] = -2.5; xs }
fn probe() -> Int { let mut xs = make(); xs.push(3.75);
    if xs.len() == 3 && xs[0] == 1.25 && xs[1] == -2.5 && xs[2] == 3.75 { 1 } else { 0 } }
"#,
        1,
    );
}

#[test]
fn list_backed_array_call_is_not_reinterpreted_natively() {
    check(
        r#"
fn array_value() -> [Byte; 2] { [11 as Byte, 12 as Byte] }
fn make() -> List<Byte> { let xs = array_value(); xs }
fn probe() -> Int { let mut xs = make(); xs.push(13 as Byte);
    xs.len() * 10000 + (xs[0] as Int) * 100 + (xs[2] as Int) }
"#,
        31113,
    );
}
