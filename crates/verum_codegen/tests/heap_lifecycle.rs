//! A linked Heap must share its constructor, payload representation and destructor.
#![cfg(target_os = "macos")]
#[path = "support/source_image.rs"]
mod source_image;

use verum_ast::{FileId, ItemKind, decl::ImplItemKind};
use verum_common::List;
use verum_fast_parser::FastParser;
use verum_vbc::{
    codegen::{CodegenConfig, ItemFailurePolicy, VbcCodegen},
    interpreter::Interpreter,
};

fn heap_program() -> verum_vbc::VbcModule {
    let parser = FastParser::new();
    let mut inputs = List::new();
    // Mounts and complete selected function bodies retain their source ownership.
    let files: &[(&str, &[&str])] = &[
        (
            include_str!("../../../core/base/memory.vr"),
            &[
                "Heap",
                "new",
                "try_new",
                "validate_cbgr",
                "as_ref",
                "as_mut",
                "drop",
                "from_raw",
                "into_raw",
                "into_inner",
                "leak",
                "header_generation",
                "current_epoch",
                "is_valid",
                "GEN_OFFSET",
                "EPOCH_CAPS_OFFSET",
            ],
        ),
        (include_str!("../../../core/base/protocols.vr"), &["Drop"]),
        (include_str!("../../../core/base/result.vr"), &["Result"]),
        (
            include_str!("../../../core/mem/allocator.vr"),
            &["AllocError", "message", "cbgr_alloc", "cbgr_dealloc"],
        ),
        (
            include_str!("../../../core/mem/header.vr"),
            &["HEADER_SIZE"],
        ),
        (
            include_str!("../../../core/intrinsics/atomic.vr"),
            &["atomic_load_u32", "ORDERING_ACQUIRE"],
        ),
        (
            include_str!("../../../core/intrinsics/runtime/cbgr.vr"),
            &["cbgr_deallocate"],
        ),
        (
            include_str!("../../../core/intrinsics/memory.vr"),
            &["forget"],
        ),
    ];
    for (source, names) in files {
        let mut unit = parser
            .parse_module_str(source, FileId::new(inputs.len() as u32))
            .unwrap();
        unit.items.retain_mut(|item| match &mut item.kind {
            ItemKind::Module(owner) => owner.items.is_none(),
            ItemKind::Mount(_) => true,
            ItemKind::Const(item) => names.contains(&item.name.as_str()),
            ItemKind::Type(item) => names.contains(&item.name.as_str()),
            ItemKind::Function(item) => names.contains(&item.name.as_str()),
            ItemKind::Impl(implementation) => {
                use verum_ast::{TypeKind, decl::ImplKind};
                let target = match &implementation.kind {
                    ImplKind::Inherent(target)
                    | ImplKind::Protocol {
                        for_type: target, ..
                    } => target,
                };
                let target = if let TypeKind::Generic { base, .. } = &target.kind {
                    base.as_ref()
                } else {
                    target
                };
                let TypeKind::Path(path) = &target.kind else {
                    return false;
                };
                if !path
                    .as_ident()
                    .is_some_and(|name| names.contains(&name.as_str()))
                {
                    return false;
                }
                implementation.items.retain(|item| {
                    matches!(&item.kind,
                    ImplItemKind::Function(method) if names.contains(&method.name.as_str()))
                });
                !implementation.items.is_empty()
            }
            _ => false,
        });
        inputs.push(unit);
    }
    inputs.push(
        parser
            .parse_module_str(
                r#"
        module owning_heap;
        mount core.base.memory.Heap;
        mount core.base.protocols.Drop;
        mount core.base.result.Result;
        static mut RELEASED: Int = 0;
        type affine Ticket is { count: Int };
        implement Drop for Ticket {
            fn drop(&mut self) { unsafe { RELEASED = RELEASED + self.count; } }
        }
        type Record is { number: Int };
        fn wide_is(value: &Int128, expected: Int128) -> Bool { *value == expected }
        fn main() -> Int {
            let wide: Int128 = (1_i128 << 90_i128) + 13_i128;
            let big: Heap<Int128> = Heap.new(wide);
            if !wide_is(big.as_ref(), wide) { return 1; }
            let mut small: Heap<Byte> = Heap.new(23_u8);
            *small.as_mut() = 240_u8;
            if *small != 240_u8 { return 2; }
            let floating = Heap.new(-13.75);
            if *floating != -13.75 { return 3; }
            let record = match Heap.try_new(Record { number: 19 }) {
                Result.Ok(owner) => owner,
                _ => return 4,
            };
            let ptr = record.into_raw();
            let restored: Heap<Record> = unsafe { Heap.from_raw(ptr) };
            if restored.number != 19 { return 5; }
            let original = Heap.new(Record { number: 41 });
            let borrow = original.leak();
            if borrow.number != 41 { return 7; }
            borrow.number = 61;
            if borrow.number != 61 { return 8; }
            {
                let ticket = Heap.new(Ticket { count: 73 });
                if ticket.as_ref().count != 73 || unsafe { RELEASED } != 0 { return 6; }
            }
            unsafe { RELEASED }
        }
    "#,
                FileId::new(inputs.len() as u32),
            )
            .unwrap(),
    );
    let units = inputs.iter().collect::<List<_>>();
    let mut compiler = VbcCodegen::with_config(CodegenConfig::new(""));
    compiler.initialize();
    compiler.collect_unit_declarations(&units).unwrap();
    compiler.resolve_pending_imports();
    compiler
        .compile_unit_items(&units, ItemFailurePolicy::Strict)
        .unwrap();
    let mut module = compiler.finalize_module().unwrap();
    module.resolve_protocol_dispatch();
    module
}

#[test]
fn actual_heap_moves_and_borrows_its_declared_payload_in_both_tiers() {
    let source = heap_program();
    let bytes = verum_vbc::serialize::serialize_module(&source).unwrap();
    let wire = verum_vbc::deserialize::deserialize_module_validated(&bytes).unwrap();
    for image in [source, wire.clone()] {
        let main = image.find_function_by_name("owning_heap.main").unwrap();
        let mut vm = Interpreter::try_new_validated(std::sync::Arc::new(image)).unwrap();
        vm.run_global_ctors().unwrap();
        assert_eq!(vm.execute_function(main).unwrap().try_as_i64(), Some(73));
    }
    source_image::run_module(wire, |_| {});
}

#[test]
fn a_foreign_heap_constructor_retains_its_source_declaration() {
    source_image::run(
        r#"
        module separate;
        type Heap<T> is { tag: Int, payload: T };
        implement<T> Heap<T> {
            fn new(payload: T) -> Heap<T> { Heap { tag: 36, payload: payload } }
        }
        fn main() -> Int {
            let foreign = Heap.new(37);
            foreign.tag + foreign.payload
        }
    "#,
    );
}

#[test]
fn owned_slots_require_the_declared_raw_place_and_an_exact_encoding() {
    use verum_vbc::{
        Instruction,
        bytecode::{decode_instructions, encode_instructions},
        instruction::{HeapSlotAction, Reg},
        types::{TypeId, TypeRef},
    };
    for (body, reason) in [
        (r#"fn read(p: &unsafe mut Int) -> Int { @intrinsic("heap_slot_take", p) }"#, "requires unsafe"),
        (r#"fn read(p: Int) -> Int { unsafe { @intrinsic("heap_slot_take", p) } }"#, "raw pointee"),
        (r#"fn read(p: &unsafe Int) -> Int { unsafe { @intrinsic("heap_slot_take", p) } }"#, "raw pointee"),
        (r#"fn write(p: &unsafe mut Int) { unsafe { @intrinsic("heap_slot_init", p, true); } }"#, "initializer must have"),
    ] {
        let ast = verum_fast_parser::Parser::new(body).parse_module().unwrap();
        let error = VbcCodegen::new().compile_module(&ast).unwrap_err();
        assert!(error.to_string().contains(reason), "{body}: {error}");
    }
    for action in [
        HeapSlotAction::Take, HeapSlotAction::Init(Reg(513)),
        HeapSlotAction::Borrow { mutable: false }, HeapSlotAction::Borrow { mutable: true },
    ] {
        let instruction = Instruction::HeapSlot {
            dst: Reg(128),
            address: Reg(256),
            action,
            type_ref: TypeRef::Concrete(TypeId::U128),
        };
        let mut bytes = Vec::new();
        encode_instructions(std::slice::from_ref(&instruction), &mut bytes);
        assert_eq!(decode_instructions(&bytes).unwrap(), vec![instruction]);
        let mut trailing = bytes.clone();
        trailing[2] += 1;
        trailing.push(0);
        assert!(decode_instructions(&trailing).is_err());
        bytes.pop();
        assert!(decode_instructions(&bytes).is_err());
    }
}
