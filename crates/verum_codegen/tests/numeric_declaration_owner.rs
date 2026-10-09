//! T1701: exact numeric source declarations survive native method selection.
//! Actual parsed source and separately decoded wire are lowered independently.
//! Bounded host allocation only; no CLI, AOT, stdlib bake or allocator acceptance.
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
    serialize::serialize_module, types::TypeRef,
};

thread_local! {
    static ALLOCATIONS: RefCell<List<Heap<[u64]>>> = RefCell::new(List::new());
}
extern "C" fn allocate(size: u64) -> *mut u64 {
    assert!(size <= 4096, "bounded numeric-owner fixture allocation");
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

fn decoded_wire(module: &VbcModule) -> VbcModule {
    let mut wire = deserialize_module(&serialize_module(module).expect("serialize source"))
        .expect("deserialize source");
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let end = start + function.bytecode_length as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(&wire.bytecode[start..end])
            .expect("independent instruction decode");
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    wire
}

fn native_probe(
    module: &VbcModule,
    route: &str,
    owners: &[&str],
    guard_layout: bool,
) -> Result<i64, Text> {
    Target::initialize_native(&InitializationConfig::default()).expect("native target");
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("numeric_owner_native").with_debug_info(false),
    );
    lowering
        .lower_module(module)
        .map_err(|error| Text::from(format!("native lowering: {error:?}")))?;
    let probe = lowering
        .module()
        .get_function("probe")
        .ok_or_else(|| Text::from("exact native probe is absent"))?;
    let probe_ir = probe.print_to_string();
    let probe_ir = probe_ir.to_str().expect("probe IR UTF8");
    let text = reachable_ir(lowering.module(), "probe");
    if let Ok(directory) = std::env::var("VERUM_T1701_NATIVE_IR_DIR") {
        std::fs::create_dir_all(&directory).expect("IR evidence directory");
        let thread = std::thread::current();
        std::fs::write(
            std::path::Path::new(&directory).join(format!(
                "{}-{}-{route}.ll",
                thread.name().unwrap_or("numeric-owner"),
                owners.join("-")
            )),
            text.as_bytes(),
        )
        .expect("IR evidence");
    }
    if guard_layout {
        let has_array_result = owners.iter().any(|owner| {
            module.functions.iter().any(|function| {
                module.get_string(function.name) == Some(*owner)
                    && matches!(function.return_type, TypeRef::Array { .. })
            })
        });
        // In these fixtures the array body really allocates packed bytes.
        // Before T1704, an explicitly typed caller still emits generic
        // container-header probes on that raw buffer. Body selection alone
        // does not make those reads safe. Each probe has one result owner,
        // so a legitimate List reader cannot trip this conservative guard.
        if has_array_result && (probe_ir.contains("geteu_cv_") || probe_ir.contains("lenlv_cv_")) {
            return Err(format!("selected packed-array body reaches generic container readers; JIT withheld at the T1704 geometry boundary\n{probe_ir}").into());
        }
        // These constant source bodies have known List versus packed-array
        // storage. A primitive replacement could hand a raw buffer to a List
        // reader. Refuse to execute that unsafe negative; retain its actual IR.
        // Debug lowering does not inline this direct declared-body call.
        for owner in owners {
            let call = format!("@{owner}(");
            if !probe_ir
                .lines()
                .any(|line| line.contains("call ") && line.contains(&call))
            {
                return Err(format!("selected body {owner} is absent from probe calls; JIT withheld before an unproved result-layout read\n{probe_ir}").into());
            }
        }
    }
    let executable = context
        .create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
            text.as_bytes(),
            "numeric_owner_native",
        ))
        .map_err(|error| Text::from(format!("reachable IR: {error}")))?;
    executable
        .verify()
        .map_err(|error| Text::from(format!("invalid IR: {error}")))?;
    let engine = executable
        .create_jit_execution_engine(OptimizationLevel::None)
        .map_err(|error| Text::from(format!("JIT: {error}")))?;
    for name in ["verum_cbgr_allocate", "verum_checked_malloc"] {
        engine.add_global_mapping(
            &executable
                .get_function(name)
                .expect("host allocation declaration"),
            allocate as *const () as usize,
        );
    }
    // SAFETY: fixtures declare this exact zero-argument Int function. Source
    // body selection is checked before any container-layout consumer executes.
    let result = unsafe {
        engine
            .get_function::<unsafe extern "C" fn() -> i64>("probe")
            .map_err(|error| Text::from(format!("JIT probe: {error}")))?
            .call()
    };
    ALLOCATIONS.with(|allocations| allocations.borrow_mut().clear());
    Ok(result)
}

fn check_failures(text: &str, expected: i64, owners: &[&str], guard_layout: bool) -> List<Text> {
    let original = source(text);
    let wire = decoded_wire(&original);
    let mut failures = List::<Text>::new();
    for (route, module) in [("source", &original), ("wire", &wire)] {
        for owner in owners {
            let descriptor = module
                .functions
                .iter()
                .find(|function| module.get_string(function.name) == Some(*owner))
                .unwrap_or_else(|| panic!("{route}: exact declaration {owner}"));
            assert!(
                descriptor.has_source_body,
                "{route}: {owner} must retain AST body provenance"
            );
            assert!(
                descriptor
                    .instructions
                    .as_ref()
                    .is_some_and(|body| !body.is_empty()),
                "{route}: {owner} must have an actual decoded body"
            );
        }
        match native_probe(module, route, owners, guard_layout) {
            Ok(actual) if actual == expected => {}
            result => failures.push(format!("{route}: expected {expected}, got {result:?}").into()),
        }
    }
    failures
}

fn check(text: &str, expected: i64, owners: &[&str], guard_layout: bool) {
    let failures = check_failures(text, expected, owners, guard_layout);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const DECLARATIONS: &str = r#"
implement UInt64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [17; 8]; bytes }
}
implement USize {
    public fn to_be_bytes(self) -> List<Byte> { [37, 41] }
}
implement Int64 {
    public fn to_be_bytes(self) -> [Byte; 8] { let bytes: [Byte; 8] = [29; 8]; bytes }
}
implement ISize {
    public fn to_be_bytes(self) -> List<Byte> { [43, 47] }
}
"#;

#[test]
fn static_same_width_owners_keep_declared_array_and_list_contracts() {
    let mut failures = List::<Text>::new();
    // Keep both owners declared in every module, but isolate each consumer
    // so its physical-layout guard can distinguish arrays from real Lists.
    for (owner, body, expected) in [
        (
            "UInt64.to_be_bytes",
            "let bytes: [Byte; 8] = UInt64.to_be_bytes(1); (bytes[0] as Int) * 100 + bytes.len()",
            1708,
        ),
        (
            "Int64.to_be_bytes",
            "let bytes: [Byte; 8] = Int64.to_be_bytes(-1); (bytes[0] as Int) * 100 + bytes.len()",
            2908,
        ),
        (
            "USize.to_be_bytes",
            "let mut bytes = USize.to_be_bytes(1); bytes.push(97); (bytes[0] as Int) * 100 + bytes.len()",
            3703,
        ),
        (
            "ISize.to_be_bytes",
            "let mut bytes = ISize.to_be_bytes(-1); bytes.push(97); (bytes[0] as Int) * 100 + bytes.len()",
            4303,
        ),
    ] {
        failures.extend(check_failures(
            &format!("{DECLARATIONS}\nfn probe() -> Int {{ {body} }}"),
            expected,
            &[owner],
            true,
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn list_owner(declarations: &str, setup: &str, receiver: &str, owner: &str, expected: i64) {
    check(
        &format!(
            r#"{DECLARATIONS}
{declarations}
fn probe() -> Int {{
    {setup}
    let mut bytes = {receiver}.to_be_bytes();
    bytes.push(97);
    (bytes[0] as Int) * 100 + bytes.len()
}}
"#
        ),
        expected,
        &[owner],
        true,
    );
}

#[test]
fn usize_instance_keeps_its_growable_list_body() {
    list_owner(
        "",
        "let value: USize = 1;",
        "value",
        "USize.to_be_bytes",
        3703,
    );
}

#[test]
fn usize_alias_and_cast_keep_the_exact_body() {
    list_owner(
        "type SizeAlias is USize;",
        "",
        "(1 as SizeAlias)",
        "USize.to_be_bytes",
        3703,
    );
}

#[test]
fn isize_instance_keeps_its_growable_list_body() {
    list_owner(
        "",
        "let value: ISize = -1;",
        "value",
        "ISize.to_be_bytes",
        4303,
    );
}

#[test]
fn isize_alias_and_cast_keep_the_exact_body() {
    list_owner(
        "type SizeAlias is ISize;",
        "",
        "((-1) as SizeAlias)",
        "ISize.to_be_bytes",
        4303,
    );
}

#[test]
fn uint64_instance_keeps_its_fixed_array_body() {
    check(
        &format!(
            r#"{DECLARATIONS}
fn probe() -> Int {{ let value: UInt64 = 1;
    let bytes: [Byte; 8] = value.to_be_bytes(); (bytes[0] as Int) * 100 + bytes.len() }}
"#
        ),
        1708,
        &["UInt64.to_be_bytes"],
        true,
    );
}

#[test]
fn int64_instance_keeps_its_fixed_array_body() {
    check(
        &format!(
            r#"{DECLARATIONS}
fn probe() -> Int {{ let value: Int64 = -1;
    let bytes: [Byte; 8] = value.to_be_bytes(); (bytes[0] as Int) * 100 + bytes.len() }}
"#
        ),
        2908,
        &["Int64.to_be_bytes"],
        true,
    );
}

#[test]
fn scalar_numeric_body_wins_over_primitive_method_spelling() {
    check(
        r#"
implement USize { fn count_ones(self) -> Int { 37 } }
fn probe() -> Int { let value: USize = 7; value.count_ones() }
"#,
        37,
        &["USize.count_ones"],
        false,
    );
}
