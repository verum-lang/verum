//! Selected, unchanged stdlib Text bodies through source, archive wire and native
//! execution. Entry counters reject interception of the selected source bodies.
//! This lifecycle gate is separate from the complete standard-library bake.
#![cfg(target_os = "macos")]

use std::{fs, path::Path, process::Command, sync::Mutex};
use verum_ast::{ItemKind, Span, decl::ImplItemKind};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering, verum_llvm};
use verum_common::{List, Map, Set, Text};
use verum_compiler::{CompilationPipeline, CompilerOptions, CoreConfig, Session};
use verum_fast_parser::Parser;
use verum_llvm::{
    IntPredicate, OptimizationLevel,
    context::Context,
    module::Module,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
    values::{FunctionValue, GlobalValue},
};
use verum_vbc::{VbcModule, archive::read_archive_from_file};

static NATIVE: Mutex<()> = Mutex::new(());

fn source(path: &str) -> Text {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path),
    )
    .unwrap()
    .into()
}

fn declaration(source: &str, mut span: Span, attributes: &[verum_ast::Attribute]) -> Text {
    for attribute in attributes {
        span.start = span.start.min(attribute.span.start);
    }
    // Spans exclude visibility and unsafe modifiers. Preserve the original line.
    let start = source[..span.start as usize]
        .rfind('\n')
        .map_or(0, |line| line + 1);
    source[start..span.end as usize].into()
}

fn select(path: &str, names: &[&str], intrinsic_declarations: bool) -> Text {
    let source = source(path);
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut remaining: Set<&str> = names.iter().copied().collect();
    let mut selected = Text::new();
    for item in &ast.items {
        let name = match &item.kind {
            ItemKind::Function(value) => value.name.name.as_str(),
            ItemKind::Type(value) => value.name.name.as_str(),
            ItemKind::Protocol(value) => value.name.name.as_str(),
            _ => continue,
        };
        if !remaining.remove(&name) {
            continue;
        }
        let mut attributes = item.attributes.clone();
        match &item.kind {
            ItemKind::Function(value) => attributes.extend(value.attributes.iter().cloned()),
            ItemKind::Type(value) => attributes.extend(value.attributes.iter().cloned()),
            _ => {}
        }
        let mut span = item.span;
        if intrinsic_declarations {
            assert!(
                attributes
                    .iter()
                    .any(|attribute| attribute.name.as_str() == "intrinsic"),
                "{path}: {name} must have production intrinsic authority"
            );
            if let ItemKind::Function(function) = &item.kind {
                if let Some(body) = &function.body {
                    span.end = match body {
                        verum_ast::decl::FunctionBody::Block(block) => block.span.start,
                        verum_ast::decl::FunctionBody::Expr(_) => {
                            panic!("expected a block specification for {name}")
                        }
                    };
                }
            }
        }
        let text = declaration(&source, span, &attributes);
        selected.push_str(&text.trim_end());
        if intrinsic_declarations && !text.trim_end().ends_with(";") {
            selected.push(';');
        }
        selected.push('\n');
    }
    assert!(
        remaining.is_empty(),
        "missing production declarations in {path}: {remaining:?}"
    );
    selected
}

fn methods(path: &str, owner: &str, names: &[&str]) -> Text {
    let source = source(path);
    let ast = Parser::new(&source).parse_module().unwrap();
    let mut remaining: Set<&str> = names.iter().copied().collect();
    let mut originals = Map::new();
    let mut selected: Text = format!("implement {owner} {{\n").into();
    for item in &ast.items {
        let ItemKind::Impl(implementation) = &item.kind else {
            continue;
        };
        let verum_ast::decl::ImplKind::Inherent(ty) = &implementation.kind else {
            continue;
        };
        let selected_owner = ty.kind.primitive_name().or_else(|| match &ty.kind {
            verum_ast::ty::TypeKind::Path(path) => path.as_ident().map(|name| name.as_str()),
            _ => None,
        });
        if selected_owner != Some(owner) {
            continue;
        }
        for method in &implementation.items {
            let ImplItemKind::Function(function) = &method.kind else {
                continue;
            };
            if remaining.remove(&function.name.as_str()) {
                let Some(verum_ast::decl::FunctionBody::Block(body)) = &function.body else {
                    panic!(
                        "{owner}.{} must retain its actual source block",
                        function.name
                    );
                };
                originals.insert(
                    function.name.name.clone(),
                    (
                        Text::from(&source[body.span.start as usize..body.span.end as usize]),
                        function.visibility.clone(),
                    ),
                );
                let mut attributes = method.attributes.clone();
                attributes.extend(function.attributes.iter().cloned());
                selected.push_str(&declaration(&source, method.span, &attributes));
                selected.push('\n');
            }
        }
    }
    assert!(
        remaining.is_empty(),
        "missing {owner} methods: {remaining:?}"
    );
    selected.push_str("}\n");
    let retained = Parser::new(&selected).parse_module().unwrap();
    let ItemKind::Impl(implementation) = &retained.items[0].kind else {
        unreachable!()
    };
    assert_eq!(implementation.items.len(), names.len());
    for item in &implementation.items {
        let ImplItemKind::Function(function) = &item.kind else {
            unreachable!()
        };
        let Some(verum_ast::decl::FunctionBody::Block(body)) = &function.body else {
            unreachable!()
        };
        let (original, visibility) = &originals[&function.name.name];
        assert_eq!(
            &selected[body.span.start as usize..body.span.end as usize],
            original.as_str(),
            "selected {owner}.{} changed its source body",
            function.name
        );
        assert_eq!(&function.visibility, visibility);
    }
    selected
}

fn produce(driver: &str, with_builders: bool) -> VbcModule {
    let mut text: Text = if with_builders { "mount core.base.memory.{alloc, is_null};
        mount core.intrinsics.memory.{null_ptr, ptr_offset};
        mount core.intrinsics.runtime.mem_raw.{memcpy_addr, memset_addr};
        mount core.intrinsics.runtime.text.{text_adopt_owned, text_release_storage, text_byte_len, text_as_bytes};\n" }
        else { "mount core.intrinsics.runtime.text.{text_byte_len, text_as_bytes};\n" }.into();
    text.push_str(&select(
        "core/text/text.vr",
        if with_builders {
            &["Text", "is_utf8_char_boundary"]
        } else {
            &["Text"]
        },
        false,
    ));
    text.push_str(&methods(
        "core/text/text.vr",
        "Text",
        if with_builders {
            &[
                "new",
                "with_capacity",
                "from_utf8_unchecked",
                "len",
                "as_bytes",
                "as_ptr",
                "push_byte",
                "grow",
                "truncate",
                "slice",
            ]
        } else {
            &["len", "as_bytes"]
        },
    ));
    let mut memory: Text = "mount core.mem.allocator.{cbgr_alloc, AllocError};
        mount core.base.result.Result; mount core.intrinsics.memory.ptr_is_null;\n"
        .into();
    memory.push_str(&select("core/base/memory.vr", &["alloc", "is_null"], false));
    let mut allocator: Text = "mount core.base.result.Result;\n".into();
    allocator.push_str(&select("core/mem/allocator.vr", &["AllocError"], false));
    allocator.push_str(&methods(
        "core/mem/allocator.vr",
        "AllocError",
        &["message"],
    ));
    // Keep the shipped registry-owned intrinsic signature. Its specification
    // body is not selected by the ordinary intrinsic call dispatch.
    allocator.push_str(&select("core/mem/allocator.vr", &["cbgr_alloc"], true));
    let mut bodies: Map<Text, Text> = [
        ("core.text.text", text),
        ("core.base.memory", memory),
        ("core.mem.allocator", allocator),
        (
            "core.base.result",
            select("core/base/result.vr", &["Result"], false),
        ),
        (
            "core.intrinsics.memory",
            select(
                "core/intrinsics/memory.vr",
                &["null_ptr", "ptr_offset", "ptr_is_null"],
                false,
            ),
        ),
        (
            "core.intrinsics.runtime.mem_raw",
            select(
                "core/intrinsics/runtime/mem_raw.vr",
                &["memcpy_addr", "memset_addr"],
                true,
            ),
        ),
        (
            "core.intrinsics.runtime.text",
            select(
                "core/intrinsics/runtime/text.vr",
                &[
                    "text_adopt_owned",
                    "text_release_storage",
                    "text_byte_len",
                    "text_as_bytes",
                ],
                true,
            ),
        ),
        (
            "core.driver",
            format!("mount core.text.text.Text;\n{driver}").into(),
        ),
    ]
    .into_iter()
    .map(|(name, body)| (name.into(), body))
    .collect();
    if !with_builders {
        // The independent Concat/record case has no allocation-method calls.
        // The growth case above retains its entire original source roster.
        for owner in [
            "core.base.memory",
            "core.mem.allocator",
            "core.base.result",
            "core.intrinsics.memory",
            "core.intrinsics.runtime.mem_raw",
        ] {
            bodies.remove(&Text::from(owner));
        }
    }
    let mut children: Map<Text, Set<Text>> = Map::new();
    for name in bodies.keys() {
        let mut owner = name.as_str();
        while let Some((parent, child)) = owner.rsplit_once('.') {
            children
                .entry(parent.into())
                .or_default()
                .insert(child.into());
            owner = parent;
        }
    }
    for (owner, names) in children {
        let body = bodies.entry(owner).or_default();
        for name in names {
            body.push_str(&format!("public module {name};\n"));
        }
    }
    let directory = if let Some(path) = std::env::var_os("VERUM_TEXT_SOURCE_ARTIFACTS") {
        fs::create_dir_all(&path).unwrap();
        tempfile::Builder::new()
            .prefix("selected-source-")
            .tempdir_in(path)
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    // Retain the exact transformed source and any published archive even when
    // compilation refuses it, so a source failure is independently replayable.
    let directory = directory.keep();
    eprintln!("selected Text source: {}", directory.display());
    let core = directory.join("core");
    for (owner, body) in bodies {
        let path = if owner == "core" {
            core.join("mod.vr")
        } else {
            core.join(owner.strip_prefix("core.").unwrap().replace(".", "/"))
                .with_extension("vr")
        };
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("module {owner};\n{body}")).unwrap();
    }
    let output = directory.join("runtime.vbca");
    let mut session = Session::new(CompilerOptions {
        input: core.join("mod.vr"),
        ..Default::default()
    });
    let result =
        CompilationPipeline::new_core(&mut session, CoreConfig::new(&core).with_output(&output))
            .compile_core()
            .unwrap_or_else(|error| panic!("{error:#}\n{}", session.format_diagnostics()));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let archive = read_archive_from_file(&output).unwrap();
    let app_source = "module app; mount core.driver.probe; fn main() -> Int { probe() }";
    fs::write(directory.join("app.vr"), app_source).unwrap();
    let app = Parser::new(app_source).parse_module().unwrap();
    let mut codegen = verum_vbc::codegen::VbcCodegen::with_config(
        verum_vbc::codegen::CodegenConfig::new("app"),
    );
    // Load the actual archive dependency closure. Loading only the root entry
    // omits separately published modules such as core.text and their bodies.
    verum_compiler::archive_ctx_loader::ArchiveCtxCache::new(&archive)
        .apply_lazy_with_types(&mut codegen, &app, &[]).unwrap();
    codegen.collect_unit_declarations(&[&app]).unwrap();
    codegen.resolve_pending_imports();
    codegen.compile_unit_items(&[&app], verum_vbc::codegen::ItemFailurePolicy::Strict).unwrap();
    let module = codegen.finalize_module_from_state().unwrap();
    let mut wire = verum_vbc::deserialize::deserialize_module(
        &verum_vbc::serialize::serialize_module(&module).unwrap(),
    )
    .unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    let mut graph = verum_vbc::mono::InstantiationGraph::new();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    for function in &wire.functions {
        verum_vbc::mono::discover_call_instantiations(
            &wire,
            function.instructions.as_ref().unwrap(),
            function.func_id_base,
            &mut graph,
        )
        .unwrap();
    }
    verum_vbc::mono::monomorphize_minimal(wire, &graph)
        .unwrap()
        .module
}

fn count_calls<'ctx>(
    context: &'ctx Context,
    native: &Module<'ctx>,
    function: FunctionValue<'ctx>,
    label: &str,
) -> GlobalValue<'ctx> {
    assert!(
        function.count_basic_blocks() > 0,
        "selected {label} lacks a body"
    );
    let counter = native.add_global(context.i64_type(), None, label);
    counter.set_initializer(&context.i64_type().const_zero());
    let builder = context.create_builder();
    builder.position_before(
        &function
            .get_first_basic_block()
            .unwrap()
            .get_first_instruction()
            .unwrap(),
    );
    let value = builder
        .build_load(context.i64_type(), counter.as_pointer_value(), "calls")
        .unwrap()
        .into_int_value();
    let next = builder
        .build_int_add(value, context.i64_type().const_int(1, false), "next")
        .unwrap();
    builder
        .build_store(counter.as_pointer_value(), next)
        .unwrap();
    counter
}

fn count_allocations<'ctx>(context: &'ctx Context, native: &Module<'ctx>) -> GlobalValue<'ctx> {
    let counter = native.add_global(context.i64_type(), None, "buffers_allocated");
    counter.set_initializer(&context.i64_type().const_zero());
    let builder = context.create_builder();
    // These are the two independent production leaves. The one-argument
    // wrapper and failed fallible attempts must not count as allocations.
    for name in [
        "verum_cbgr_allocate_aligned",
        "verum_try_cbgr_allocate_aligned",
    ] {
        let function = native.get_function(name).unwrap();
        assert!(function.count_basic_blocks() > 0);
        let returns: List<_> = function
            .get_basic_blocks()
            .iter()
            .filter_map(|block| block.get_terminator())
            .filter(|instruction| {
                instruction.get_opcode() == verum_llvm::values::InstructionOpcode::Return
            })
            .collect();
        assert!(!returns.is_empty());
        for instruction in returns {
            builder.position_before(&instruction);
            let pointer = instruction
                .get_operand(0)
                .unwrap()
                .unwrap_value()
                .into_pointer_value();
            let live = builder
                .build_is_not_null(pointer, "allocation_succeeded")
                .unwrap();
            let delta = builder
                .build_int_z_extend(live, context.i64_type(), "successful_allocation")
                .unwrap();
            let previous = builder
                .build_load(
                    context.i64_type(),
                    counter.as_pointer_value(),
                    "allocations",
                )
                .unwrap()
                .into_int_value();
            let next = builder
                .build_int_add(previous, delta, "next_allocation")
                .unwrap();
            builder
                .build_store(counter.as_pointer_value(), next)
                .unwrap();
        }
    }
    counter
}

fn native(driver: &str, required_bodies: &[&str], with_builders: bool) {
    // The mutex only serializes images; a previous failed assertion must not
    // prevent the next independent source program from reporting its result.
    let _guard = NATIVE.lock().unwrap_or_else(|poison| poison.into_inner());
    let wire = produce(driver, with_builders);
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple)
        .unwrap()
        .create_target_machine(
            &triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .unwrap();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("text_source")
            .with_debug_info(false)
            .with_target(triple.as_str().to_str().unwrap()),
    );
    lowering.lower_module_with_entry(&wire, Some("app.main")).unwrap();
    let native = lowering.module();
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    let mut counters = List::new();
    for (index, name) in required_bodies.iter().enumerate() {
        let selected = wire
            .find_function_by_name(name)
            .unwrap_or_else(|| panic!("missing actual {name}"));
        let function = lowering.get_function_by_id(selected.0).unwrap();
        counters.push(count_calls(
            &context,
            native,
            function,
            &format!("source_body_{index}"),
        ));
    }
    let allocated = count_allocations(&context, native);
    let released = count_calls(
        &context,
        native,
        native.get_function("verum_cbgr_deallocate").unwrap(),
        "buffers_released",
    );
    let main = native.get_function("main").unwrap();
    let returns: List<_> = main
        .get_basic_blocks()
        .iter()
        .filter_map(|block| block.get_terminator())
        .filter(|instruction| {
            instruction.get_opcode() == verum_llvm::values::InstructionOpcode::Return
        })
        .collect();
    assert_eq!(returns.len(), 1);
    let instruction = returns[0];
    let builder = context.create_builder();
    builder.position_before(&instruction);
    let mut valid = context.bool_type().const_int(1, false);
    for counter in counters {
        let value = builder
            .build_load(
                context.i64_type(),
                counter.as_pointer_value(),
                "observed_body",
            )
            .unwrap()
            .into_int_value();
        let ran = builder
            .build_int_compare(
                IntPredicate::UGT,
                value,
                context.i64_type().const_zero(),
                "body_ran",
            )
            .unwrap();
        valid = builder.build_and(valid, ran, "all_bodies_ran").unwrap();
    }
    let allocations = builder
        .build_load(
            context.i64_type(),
            allocated.as_pointer_value(),
            "allocations",
        )
        .unwrap()
        .into_int_value();
    let releases = builder
        .build_load(context.i64_type(), released.as_pointer_value(), "releases")
        .unwrap()
        .into_int_value();
    let balanced = builder
        .build_int_compare(
            IntPredicate::EQ,
            allocations,
            releases,
            "backing_storage_retired",
        )
        .unwrap();
    let exercised = builder
        .build_int_compare(
            IntPredicate::UGT,
            allocations,
            context.i64_type().const_zero(),
            "backing_storage_was_allocated",
        )
        .unwrap();
    valid = builder
        .build_and(valid, exercised, "allocated_source_storage")
        .unwrap();
    valid = builder.build_and(valid, balanced, "all_checks").unwrap();
    let code = instruction
        .get_operand(0)
        .unwrap()
        .value()
        .unwrap()
        .into_int_value();
    let result = builder
        .build_select(
            valid,
            code,
            context.i32_type().const_int(97, false),
            "verified_source_result",
        )
        .unwrap();
    instruction.set_operand(0, result);
    native.verify().unwrap();
    let directory = if let Some(path) = std::env::var_os("VERUM_TEXT_SOURCE_ARTIFACTS") {
        fs::create_dir_all(&path).unwrap();
        tempfile::tempdir_in(path).unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let object = directory.path().join("text.o");
    let executable = directory.path().join("text");
    native
        .print_to_file(directory.path().join("text.ll"))
        .unwrap();
    machine
        .write_to_file(native, FileType::Object, &object)
        .unwrap();
    let link = Command::new("cc")
        .arg(object)
        .args(["-Wl,-dead_strip", "-nostdlib", "-lSystem", "-o"])
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        link.status.success(),
        "{}",
        Text::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(&executable).output().unwrap();
    let artifact = directory.keep();
    assert_eq!(
        run.status.code(),
        Some(73),
        "native lifecycle {:?}: {}\n{}",
        artifact,
        Text::from_utf8_lossy(&run.stdout),
        Text::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn actual_text_new_grow_truncate_copy_and_automatic_drop() {
    native(
        r#"
        public fn probe() -> Int {
            let mut words = Text.new();
            let mut i = 0;
            while i < 40 { words.push_byte(65 as Byte); i = i + 1; }
            let mut copied = words;
            copied.truncate(3);
            let mut pooled = "abcdef";
            pooled.truncate(2);
            if words.len() == 40 && copied.len() == 3 && pooled.len() == 2
                && words.as_bytes()[39] == 65 as Byte && "abcdef".len() == 6 { 73 } else { 1 }
        }
    "#,
        &[
            "core.text.text.Text.new",
            "core.text.text.Text.grow",
            "core.text.text.Text.truncate",
        ],
        true,
    );
}

#[test]
fn concat_reads_both_operands_and_record_takes_its_field() {
    native(
        r#"
        type Package is { text: Text };
        fn package(number: Int) -> Package { Package { text: f"value={number}" } }
        public fn probe() -> Int {
            let original = f"{37}";
            let doubled = original + original;
            let temporary_pair = f"{123456789}" + f"{987654321}";
            let held = package(19);
            let clone = held;
            if original.len() == 2 && doubled.len() == 4 && held.text.len() == 8
                && clone.text.len() == 8 && held.text.as_bytes()[7] == 57 as Byte
                && temporary_pair.len() == 18 && temporary_pair.as_bytes()[0] == 49 as Byte
                && temporary_pair.as_bytes()[9] == 57 as Byte { 73 } else { 2 }
        }
    "#,
        &["core.text.text.Text.len"],
        false,
    );
}
