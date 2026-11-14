//! The explicit typed-copy operation owns preparation, commit and rollback.
//! The source copy emitter is deliberately not involved in these engine gates.
#![cfg(target_os = "macos")]
use std::process::Command;
use verum_ast::{ItemKind, decl::ImplKind};
use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_common::List;
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel,
    context::Context,
    module::Module,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
};
use verum_vbc::{
    Instruction, Reg, TypeId, TypeRef, VbcModule,
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    copy_plan::CopyFailure,
    deserialize::deserialize_module,
    serialize::serialize_module,
};

type CheckedCopy = unsafe extern "C" fn(u64, *mut u64, u64, u64) -> u32;
type RawAlloc = unsafe extern "C" fn(u64, u64) -> *mut u8;
type RawFree = unsafe extern "C" fn(*mut u8);
type ObjectFree = unsafe extern "C" fn(*mut u8, u64);

fn fixture(selected: &str, additional: &str) -> VbcModule {
    let mut protocols = Parser::new(include_str!("../../../core/base/protocols.vr"))
        .parse_module()
        .unwrap();
    protocols.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || item.kind.type_or_protocol_name().is_some_and(|n| {
                [
                    "Clone",
                    "PartialEq",
                    "Eq",
                    "Hash",
                    "Hasher",
                    "DefaultHasher",
                ]
                .contains(&n.as_str())
            })
    });
    let mut memory = Parser::new(include_str!("../../../core/base/memory.vr"))
        .parse_module()
        .unwrap();
    memory.items.retain(|item| match &item.kind {
        ItemKind::Module(_) => true,
        ItemKind::Type(d) => matches!(d.name.name.as_str(),"Shared"|"SharedInner"),
        ItemKind::Impl(d) => matches!(&d.kind,ImplKind::Protocol{protocol,for_type,..} if protocol.to_string()=="Clone" && verum_ast::pretty::format_type(for_type)=="Shared<T>"),
        _=>false,
    });
    assert_eq!(
        memory.items.len(),
        4,
        "actual declarations and complete selected Clone"
    );
    let mut list = Parser::new(include_str!("../../../core/collections/list.vr"))
        .parse_module()
        .unwrap();
    list.items.retain(|item| {
        matches!(&item.kind, ItemKind::Module(_))
            || item
                .kind
                .type_or_protocol_name()
                .is_some_and(|n| n.as_str() == "List")
    });
    let selected_types = |source: &str, names: &[&str]| {
        let mut ast = Parser::new(source).parse_module().unwrap();
        ast.items.retain(|item| {
            matches!(&item.kind, ItemKind::Module(_))
                || item
                    .kind
                    .type_or_protocol_name()
                    .is_some_and(|n| names.contains(&n.as_str()))
        });
        ast
    };
    let map = selected_types(
        include_str!("../../../core/collections/map.vr"),
        &["Map", "Slot"],
    );
    let deque = selected_types(
        include_str!("../../../core/collections/deque.vr"),
        &["Deque"],
    );
    let app = Parser::new(&format!(
        r#"
        module app; mount core.base.memory.Shared; mount core.collections.list.List;
        mount core.collections.map.Map; mount core.collections.deque.Deque;
        type Pair is {{ left:Shared<Int>, right:Shared<Int> }};
        type Leaf is {{ value:Int }}; type Branch is {{ left:Leaf, right:Leaf }};
        type Borrowed is {{ value:&Int }};
        type ArrayOwner is {{ values:List<Leaf> }};
        type Node is {{ children:List<Node> }};
        type MapOwner is {{ values:Map<Int,Leaf> }};
        type DequeOwner is {{ values:Deque<Leaf> }};
        type SharedAndList is {{ first:Shared<Int>, later:List<Int> }};
        type Bytes is {{ values:List<Byte> }};
        type Empty is ();
        type Choice is Packed(Leaf) | Vacant;
        fn duplicate(value:{selected})->{selected} {{ value }}
        {additional}

    "#
    ))
    .parse_module()
    .unwrap();
    let units: List<_> = [&protocols, &memory, &list, &map, &deque, &app]
        .into_iter()
        .collect();
    let mut codegen = VbcCodegen::with_config(CodegenConfig::new("app"));
    codegen.collect_unit_declarations(&units).unwrap();
    codegen.resolve_pending_imports();
    codegen
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = codegen.finalize_module_from_state().unwrap();
    let id = module.find_function_by_name("app.duplicate").unwrap();
    let ty = TypeRef::Concrete(owner(&module, selected));
    let instructions = [
        Instruction::CloneTyped {
            dst: Reg(1),
            src: Reg(0),
            type_ref: ty,
        },
        Instruction::Ret { value: Reg(1) },
    ];
    let mut bytes = Default::default();
    for instruction in &instructions {
        verum_vbc::bytecode::encode_instruction(instruction, &mut bytes);
    }
    let offset = module.append_bytecode(&bytes);
    let body = module.functions.iter_mut().find(|f| f.id == id).unwrap();
    body.bytecode_offset = offset;
    body.bytecode_length = bytes.len() as u32;
    body.register_count = 2;
    body.locals_count = 1;
    body.instructions = None;
    body.value_uses = None;
    let mut wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut instructions = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        )
        .unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut instructions);
        function.instructions = Some(instructions);
    }
    wire
}
fn owner(module: &VbcModule, name: &str) -> TypeId {
    module
        .types
        .iter()
        .find(|d| {
            module
                .get_string(d.name)
                .is_some_and(|n| n == name || n.ends_with(&format!(".{name}")))
        })
        .unwrap()
        .id
}
fn object(owner: TypeId, fields: &[u64]) -> List<u64> {
    let mut words: List<u64> = [
        u64::from(owner.0),
        ((24 + fields.len() * 8) as u64) << 32,
        0,
    ]
    .into_iter()
    .collect();
    words.extend(fields.iter().copied());
    words
}
fn publish_wrapper<'ctx>(context: &'ctx Context, module: &Module<'ctx>, private: &str, name: &str) {
    let selected = module
        .get_function(private)
        .unwrap_or_else(|| panic!("missing checked helper {private}"));
    let wrapper = module.add_function(name, selected.get_type(), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(wrapper, "entry"));
    let args: List<_> = wrapper.get_params().iter().map(|v| (*v).into()).collect();
    let result = builder
        .build_call(selected, &args, "result")
        .unwrap()
        .try_as_basic_value()
        .basic();
    match result {
        Some(value) => {
            builder.build_return(Some(&value)).unwrap();
        }
        None => {
            builder.build_return(None).unwrap();
        }
    }
}
fn with_native(
    selected: &str,
    run: impl FnOnce(&VbcModule, CheckedCopy, RawAlloc, RawFree, ObjectFree),
) {
    with_native_options(selected, None, "", |wire, copy, alloc, free, release, _| {
        run(wire, copy, alloc, free, release)
    });
}
fn with_native_options(
    selected: &str,
    initial_generation: Option<u64>,
    additional: &str,
    run: impl FnOnce(&VbcModule, CheckedCopy, RawAlloc, RawFree, ObjectFree, &libloading::Library),
) {
    let wire = fixture(selected, additional);
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(
        &context,
        LoweringConfig::debug("typed-copy-journal").with_debug_info(false),
    );
    lowering.lower_module(&wire).unwrap();
    let native = lowering.module();
    for (private, name) in [
        ("verum_typed_value_copy", "checked_copy"),
        ("verum_cbgr_allocate_aligned", "allocate"),
        ("verum_cbgr_deallocate", "deallocate"),
        ("verum_os_free", "release_object"),
    ] {
        publish_wrapper(&context, native, private, name);
    }
    if let Some(initial) = initial_generation {
        let counter = native.get_global("verum_ir_generation_counter").unwrap();
        counter.set_initializer(&context.i64_type().const_int(initial, false));
    }
    if native.get_function("app.specimen").is_some() {
        publish_wrapper(&context, native, "app.specimen", "specimen");
    }
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
    native.set_triple(&triple);
    native.set_data_layout(&machine.get_target_data().get_data_layout());
    native.verify().unwrap();
    let directory = tempfile::Builder::new()
        .prefix("typed-copy-native-")
        .tempdir_in(std::env::var_os("CARGO_TARGET_DIR").expect("private target"))
        .unwrap();
    let object = directory.path().join("copy.o");
    let image = directory.path().join("copy.dylib");
    machine
        .write_to_file(native, FileType::Object, &object)
        .unwrap();
    let mut linker = Command::new("cc");
    linker.arg(&object).args([
        "-dynamiclib",
        "-nostdlib",
        "-lSystem",
        "-Wl,-dead_strip",
        "-Wl,-exported_symbol,_checked_copy",
        "-Wl,-exported_symbol,_allocate",
        "-Wl,-exported_symbol,_deallocate",
        "-Wl,-exported_symbol,_release_object",
    ]);
    if native.get_function("specimen").is_some() {
        linker.arg("-Wl,-exported_symbol,_specimen");
    }
    let linked = linker.arg("-o").arg(&image).output().unwrap();
    assert!(
        linked.status.success(),
        "{}",
        verum_common::Text::from_utf8_lossy(&linked.stderr)
    );
    // SAFETY: the exact generated helpers are linked through the supported
    // macOS system ABI. The loaded image outlives all calls and owned results;
    // no allocator or runtime implementation is replaced by the harness.
    unsafe {
        let library = libloading::Library::new(&image).unwrap();
        run(
            &wire,
            *library.get::<CheckedCopy>(b"checked_copy").unwrap(),
            *library.get::<RawAlloc>(b"allocate").unwrap(),
            *library.get::<RawFree>(b"deallocate").unwrap(),
            *library.get::<ObjectFree>(b"release_object").unwrap(),
            &library,
        );
    }
}
#[test]
fn native_checked_copy_preserves_distinct_owned_subtrees() {
    with_native("Branch", |wire, copy, _, _, release| {
        let mut leaf = object(owner(wire, "Leaf"), &[73]);
        let branch = object(
            owner(wire, "Branch"),
            &[leaf.as_ptr() as u64, leaf.as_ptr() as u64],
        );
        let mut output = 0;
        // SAFETY: exact live declared native records and writable result slot.
        unsafe {
            assert_eq!(copy(branch.as_ptr() as u64, &mut output, 1000, 1000), 0);
            let result = output as *mut u64;
            let left = *result.add(3) as *mut u64;
            let right = *result.add(4) as *mut u64;
            assert_ne!(left, right);
            assert_ne!(left, leaf.as_mut_ptr());
            leaf[3] = 0;
            assert_eq!(*left.add(3), 73);
            assert_eq!(*right.add(3), 73);
            release(left.cast(), 32);
            release(right.cast(), 32);
            release(result.cast(), 40);
        }
    });
}
#[test]
fn native_late_shared_overflow_rolls_back_before_publication() {
    with_native("Pair", |wire, copy, allocate, deallocate, _| {
        // SAFETY: real CBGR backing holds the actual SharedInner<Int> fields;
        // the original wrapper remains live until the checked operation ends.
        unsafe {
            let inner = allocate(24, 8).cast::<u64>();
            assert!(!inner.is_null());
            inner.write(u64::from(u32::MAX - 1));
            inner.add(1).write(0);
            inner.add(2).write(73);
            let shared = object(owner(wire, "Shared"), &[inner as u64, 1, 0]);
            let pair = object(
                owner(wire, "Pair"),
                &[shared.as_ptr() as u64, shared.as_ptr() as u64],
            );
            let mut result = 919;
            for _ in 0..8 {
                assert_eq!(
                    copy(pair.as_ptr() as u64, &mut result, 1000, 1000),
                    CopyFailure::RetainRefused as u32
                );
                assert_eq!(result, 919);
                assert_eq!(*inner, u64::from(u32::MAX - 1));
                assert_eq!(*inner.add(2), 73);
            }
            deallocate(inner.cast());
        }
    });
}
#[test]
fn native_resource_refusal_keeps_source_and_output_unchanged() {
    // SAFETY: the allocated backing and exact declared wrappers remain live
    // throughout both failed operations; no uncommitted result is accessed.
    with_native("Pair", |wire, copy, allocate, deallocate, _| unsafe {
        let inner = allocate(24, 8).cast::<u64>();
        assert!(!inner.is_null());
        inner.write(2);
        inner.add(1).write(0);
        inner.add(2).write(73);
        let shared = object(owner(wire, "Shared"), &[inner as u64, 1, 0]);
        let pair = object(
            owner(wire, "Pair"),
            &[shared.as_ptr() as u64, shared.as_ptr() as u64],
        );
        let mut result = 919;
        // The first wrapper is prepared, then the second exceeds the payload budget.
        assert_eq!(
            copy(pair.as_ptr() as u64, &mut result, 1000, 40),
            CopyFailure::AllocationLimit as u32
        );
        assert_eq!(result, 919);
        assert_eq!(*inner, 2);
        assert_eq!(*inner.add(2), 73);
        assert_eq!(
            copy(pair.as_ptr() as u64, &mut result, 0, 1000),
            CopyFailure::WorkLimit as u32
        );
        assert_eq!(*inner, 2);
        deallocate(inner.cast());
    });
}
#[test]
fn native_borrowed_field_preserves_the_original_reference() {
    with_native("Borrowed", |wire, copy, _, _, release| {
        let value = 73u64;
        let source = object(owner(wire, "Borrowed"), &[(&value as *const u64) as u64]);
        let mut output = 0;
        // SAFETY: the source reference remains live across copy and observation.
        unsafe {
            assert_eq!(copy(source.as_ptr() as u64, &mut output, 1000, 1000), 0);
            let result = output as *mut u64;
            assert_eq!(*result.add(3), (&value as *const u64) as u64);
            assert_eq!(*(*result.add(3) as *const u64), 73);
            release(result.cast(), 32);
        }
    });
}

#[test]
fn native_repeated_shared_handles_retain_independently() {
    with_native("Pair", |wire, copy, allocate, deallocate, release| {
        // SAFETY: all exact source and copied wrappers stay live through observation;
        // this engine control explicitly retires its backing after all owners.
        unsafe {
            let inner = allocate(24, 8).cast::<u64>();
            assert!(!inner.is_null());
            inner.write(2);
            inner.add(1).write(0);
            inner.add(2).write(73);
            let shared = object(owner(wire, "Shared"), &[inner as u64, 1, 0]);
            let pair = object(
                owner(wire, "Pair"),
                &[shared.as_ptr() as u64, shared.as_ptr() as u64],
            );
            let mut output = 0;
            assert_eq!(copy(pair.as_ptr() as u64, &mut output, 1000, 1000), 0);
            let result = output as *mut u64;
            let first = *result.add(3) as *mut u64;
            let second = *result.add(4) as *mut u64;
            assert_ne!(first, second);
            assert_ne!(first, shared.as_ptr().cast_mut());
            assert_eq!(*inner, 4);
            assert_eq!(*first.add(3), inner as u64);
            assert_eq!(*second.add(3), inner as u64);
            assert_eq!(*inner.add(2), 73);
            release(first.cast(), 48);
            release(second.cast(), 48);
            release(result.cast(), 40);
            deallocate(inner.cast());
        }
    });
}
#[test]
fn native_list_copy_owns_only_initialized_values() {
    with_native("ArrayOwner", |wire, copy, allocate, deallocate, release| {
        let mut leaf = object(owner(wire, "Leaf"), &[73]);
        // SAFETY: the real CBGR buffer owns three slots, of which two are initialized.
        unsafe {
            let storage = allocate(24, 8).cast::<u64>();
            assert!(!storage.is_null());
            storage.write(leaf.as_ptr() as u64);
            storage.add(1).write(leaf.as_ptr() as u64);
            storage.add(2).write(u64::MAX);
            let list = object(TypeId::LIST, &[2, 3, storage as u64]);
            let source = object(owner(wire, "ArrayOwner"), &[list.as_ptr() as u64]);
            let mut output = 0;
            assert_eq!(copy(source.as_ptr() as u64, &mut output, 1000, 1000), 0);
            let outer = output as *mut u64;
            let list_copy = *outer.add(3) as *mut u64;
            let copied = *list_copy.add(5) as *mut u64;
            let first = *copied as *mut u64;
            let second = *copied.add(1) as *mut u64;
            assert_eq!(*list_copy.add(3), 2);
            assert_eq!(*list_copy.add(4), 3);
            assert_ne!(first, second);
            assert_ne!(first, leaf.as_mut_ptr());
            leaf[3] = 0;
            assert_eq!(*first.add(3), 73);
            assert_eq!(*second.add(3), 73);
            release(first.cast(), 32);
            release(second.cast(), 32);
            deallocate(copied.cast());
            release(list_copy.cast(), 48);
            release(outer.cast(), 32);
            deallocate(storage.cast());
        }
    });
}
#[test]
fn native_actual_owning_cycle_refuses_without_publishing() {
    with_native("Node", |wire, copy, allocate, deallocate, _| {
        // SAFETY: a deliberately cyclic but fully live owning graph tests bounded refusal.
        unsafe {
            let storage = allocate(8, 8).cast::<u64>();
            assert!(!storage.is_null());
            let list = object(TypeId::LIST, &[1, 1, storage as u64]);
            let node = object(owner(wire, "Node"), &[list.as_ptr() as u64]);
            storage.write(node.as_ptr() as u64);
            let mut output = 919;
            assert_eq!(
                copy(node.as_ptr() as u64, &mut output, 1000, 1000),
                CopyFailure::OwnedCycle as u32
            );
            assert_eq!(output, 919);
            assert_eq!(*storage, node.as_ptr() as u64);
            deallocate(storage.cast());
        }
    });
}
#[test]
fn native_map_copy_preserves_occupied_values_and_refuses_wrong_count() {
    with_native("MapOwner", |wire, copy, allocate, deallocate, release| {
        let mut leaf = object(owner(wire, "Leaf"), &[73]);
        // SAFETY: the live raw table has two complete declared Slot records.
        unsafe {
            let storage = allocate(64, 8).cast::<u64>();
            assert!(!storage.is_null());
            for (index, word) in [7, leaf.as_ptr() as u64, 1, 0, u64::MAX, u64::MAX, 0, 0]
                .into_iter()
                .enumerate()
            {
                storage.add(index).write(word);
            }
            let mut map = object(owner(wire, "Map"), &[1, 2, storage as u64, 0]);
            let source = object(owner(wire, "MapOwner"), &[map.as_ptr() as u64]);
            let mut output = 0;
            assert_eq!(copy(source.as_ptr() as u64, &mut output, 1000, 1000), 0);
            let outer = output as *mut u64;
            let map_copy = *outer.add(3) as *mut u64;
            let copied = *map_copy.add(5) as *mut u64;
            let value = *copied.add(1) as *mut u64;
            leaf[3] = 0;
            assert_eq!(*value.add(3), 73);
            assert_eq!(*copied, 7);
            assert_eq!(*copied.add(2), 1);
            release(value.cast(), 32);
            deallocate(copied.cast());
            release(map_copy.cast(), 56);
            release(outer.cast(), 32);
            map[3] = 2;
            output = 919;
            assert_eq!(
                copy(source.as_ptr() as u64, &mut output, 1000, 1000),
                CopyFailure::InvalidStorage as u32
            );
            assert_eq!(output, 919);
            deallocate(storage.cast());
        }
    });
}
#[test]
fn native_deque_copy_preserves_only_its_initialized_ring() {
    with_native("DequeOwner", |wire, copy, allocate, deallocate, release| {
        let mut leaf = object(owner(wire, "Leaf"), &[73]);
        // SAFETY: the exact capacity-four ring owns slots 3 and 0; spare slots are not values.
        unsafe {
            let storage = allocate(32, 8).cast::<u64>();
            assert!(!storage.is_null());
            for (index, word) in [
                leaf.as_ptr() as u64,
                u64::MAX,
                u64::MAX,
                leaf.as_ptr() as u64,
            ]
            .into_iter()
            .enumerate()
            {
                storage.add(index).write(word);
            }
            let deque = object(owner(wire, "Deque"), &[storage as u64, 3, 2, 4]);
            let source = object(owner(wire, "DequeOwner"), &[deque.as_ptr() as u64]);
            let mut output = 0;
            assert_eq!(copy(source.as_ptr() as u64, &mut output, 1000, 1000), 0);
            let outer = output as *mut u64;
            let deque_copy = *outer.add(3) as *mut u64;
            let copied = *deque_copy.add(3) as *mut u64;
            let first = *copied as *mut u64;
            let second = *copied.add(3) as *mut u64;
            assert_ne!(first, second);
            leaf[3] = 0;
            assert_eq!(*first.add(3), 73);
            assert_eq!(*second.add(3), 73);
            release(first.cast(), 32);
            release(second.cast(), 32);
            deallocate(copied.cast());
            release(deque_copy.cast(), 56);
            release(outer.cast(), 32);
            deallocate(storage.cast());
        }
    });
}

#[test]
fn native_actual_backing_refusal_after_prepared_shared_rolls_back() {
    with_native_options(
        "SharedAndList",
        Some(u64::from(verum_common::cbgr::GEN_MAX) - 2),
        "",
        |wire, copy, allocate, deallocate, _, _| {
            // SAFETY: the two real allocations consume the final issued identities;
            // the copy's actual fallible backing allocator must then return null.
            unsafe {
                let inner = allocate(24, 8).cast::<u64>();
                assert!(!inner.is_null());
                inner.write(2);
                inner.add(1).write(0);
                inner.add(2).write(73);
                let storage = allocate(8, 8).cast::<u64>();
                assert!(!storage.is_null());
                storage.write(73);
                let shared = object(owner(wire, "Shared"), &[inner as u64, 1, 0]);
                let list = object(TypeId::LIST, &[1, 1, storage as u64]);
                let source = object(
                    owner(wire, "SharedAndList"),
                    &[shared.as_ptr() as u64, list.as_ptr() as u64],
                );
                let mut output = 919;
                assert_eq!(
                    copy(source.as_ptr() as u64, &mut output, 1000, 1000),
                    CopyFailure::AllocationFailure as u32
                );
                assert_eq!(output, 919);
                assert_eq!(*inner, 2);
                assert_eq!(*inner.add(2), 73);
                assert_eq!(*storage, 73);
                deallocate(storage.cast());
                deallocate(inner.cast());
            }
        },
    );
}
#[test]
fn native_scalar_backing_is_bulk_copied_with_bounded_work() {
    with_native("Bytes", |wire, copy, allocate, deallocate, release| {
        // SAFETY: the canonical packed-byte allocation has one initialized prefix.
        unsafe {
            let cap = 1 << 20;
            let storage = allocate(cap, 8);
            assert!(!storage.is_null());
            storage.write(73);
            storage.add(1).write(74);
            storage.add(2).write(75);
            let list = object(TypeId::BYTE_LIST, &[3, cap, storage as u64]);
            let source = object(owner(wire, "Bytes"), &[list.as_ptr() as u64]);
            let mut output = 0;
            assert_eq!(copy(source.as_ptr() as u64, &mut output, 3, cap + 32), 0);
            let outer = output as *mut u64;
            let copied_list = *outer.add(3) as *mut u64;
            let copied = *copied_list.add(5) as *mut u8;
            storage.write(0);
            assert_eq!(*copied, 73);
            assert_eq!(*copied.add(1), 74);
            assert_eq!(*copied.add(2), 75);
            assert_eq!(*copied_list as u32, TypeId::BYTE_LIST.0);
            deallocate(copied);
            release(copied_list.cast(), 48);
            release(outer.cast(), 32);
            deallocate(storage);
        }
    });
}
#[test]
fn native_empty_record_has_no_logical_payload_charge() {
    with_native("Empty", |wire, copy, _, _, release| {
        let source = object(owner(wire, "Empty"), &[]);
        let mut output = 0;
        // SAFETY: the live exact empty declaration has a header but no value payload.
        unsafe {
            assert_eq!(copy(source.as_ptr() as u64, &mut output, 1, 0), 0);
            assert_ne!(output, source.as_ptr() as u64);
            release((output as *mut u8).cast(), 24);
        }
    });
}
#[test]
fn native_sum_copy_uses_the_actual_variant_producer_extent() {
    with_native_options(
        "Choice",
        None,
        "fn specimen()->Choice { Choice.Packed(Leaf { value:73 }) }",
        |_, copy, _, _, release, library| {
            // SAFETY: the selected source constructor returns its actual native owner;
            // all copied fields remain live until explicitly released after observation.
            unsafe {
                let make = library
                    .get::<unsafe extern "C" fn() -> u64>(b"specimen")
                    .unwrap();
                let source = make();
                let mut output = 0;
                assert_eq!(copy(source, &mut output, 1000, 1000), 0);
                let source = source as *mut u64;
                let result = output as *mut u64;
                let original = *source.add(4) as *mut u64;
                let child = *result.add(4) as *mut u64;
                assert_ne!(child, original);
                *original.add(3) = 0;
                assert_eq!(*child.add(3), 73);
                release(child.cast(), 32);
                release(result.cast(), 40);
                release(original.cast(), 32);
                release(source.cast(), 40);
            }
        },
    );
}

#[test]
fn native_long_active_path_and_repeated_subtree_use_ordinary_stack() {
    with_native("Node", |wire, copy, allocate, deallocate, release| {
        // SAFETY: all source records and declared one/two-element backing buffers
        // remain live; copied records are retired iteratively after observation.
        unsafe {
            let mut records: List<List<u64>> = List::new();
            let mut buffers = List::new();
            let empty = object(TypeId::LIST, &[0, 0, 0]);
            let node = object(owner(wire, "Node"), &[empty.as_ptr() as u64]);
            let mut current = node.as_ptr() as u64;
            records.push(empty);
            records.push(node);
            for _ in 1..1500 {
                let storage = allocate(8, 8).cast::<u64>();
                assert!(!storage.is_null());
                storage.write(current);
                buffers.push(storage);
                let list = object(TypeId::LIST, &[1, 1, storage as u64]);
                let node = object(owner(wire, "Node"), &[list.as_ptr() as u64]);
                current = node.as_ptr() as u64;
                records.push(list);
                records.push(node);
            }
            let storage = allocate(16, 8).cast::<u64>();
            assert!(!storage.is_null());
            storage.write(current);
            storage.add(1).write(current);
            buffers.push(storage);
            let list = object(TypeId::LIST, &[2, 2, storage as u64]);
            let source = object(owner(wire, "Node"), &[list.as_ptr() as u64]);
            let mut output = 919;
            let limits = verum_vbc::copy_plan::DEFAULT_COPY_LIMITS;
            assert_eq!(
                copy(
                    source.as_ptr() as u64,
                    &mut output,
                    limits.max_work as u64,
                    limits.max_allocation_bytes as u64
                ),
                0
            );
            let result = output as *mut u64;
            let list_copy = *result.add(3) as *mut u64;
            let children = *list_copy.add(5) as *mut u64;
            let first = *children;
            let second = *children.add(1);
            assert_ne!(first, current);
            assert_ne!(first, second);
            for mut cursor in [first, second] {
                let mut count = 0;
                loop {
                    count += 1;
                    let node = cursor as *mut u64;
                    let list = *node.add(3) as *mut u64;
                    let length = *list.add(3);
                    let backing = *list.add(5) as *mut u64;
                    let next = if length == 0 {
                        0
                    } else {
                        assert_eq!(length, 1);
                        *backing
                    };
                    release(node.cast(), 32);
                    release(list.cast(), 48);
                    if length == 0 {
                        break;
                    }
                    deallocate(backing.cast());
                    cursor = next;
                }
                assert_eq!(count, 1500);
            }
            assert_eq!(*storage, current);
            assert_eq!(*storage.add(1), current);
            deallocate(children.cast());
            release(list_copy.cast(), 48);
            release(result.cast(), 32);
            for buffer in buffers {
                deallocate(buffer.cast());
            }
        }
    });
}
