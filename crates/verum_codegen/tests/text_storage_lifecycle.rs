//! Explicit VBC lifecycle gates for native Text storage, not the complete source
//! call pipeline. Allocation, CloneTyped and DropTyped use production lowering.
#![cfg(target_os = "macos")]

use std::{os::unix::process::ExitStatusExt, path::Path, process::Command, sync::Mutex};
use verum_ast::ItemKind;
use verum_codegen::llvm::{
    FunctionContext, LoweringConfig, VbcToLlvmLowering, instruction::lower_instruction,
};
use verum_common::{List, Text};
use verum_fast_parser::Parser;
use verum_llvm::{
    OptimizationLevel, context::Context, module::Module,
    targets::{CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine},
};
use verum_vbc::{
    Instruction, Reg, TypeId, TypeRef, VbcModule,
    codegen::{CodegenConfig, VbcCodegen}, deserialize::deserialize_module,
    instruction::TextSubOpcode, module::Constant, serialize::serialize_module,
};

static NATIVE: Mutex<()> = Mutex::new(());
const FORBIDDEN: &str = "text-storage-forbidden-side-effect\n";
const IMAGE_ENV: &str = "VERUM_TEST_TEXT_STORAGE_IMAGE";
const CASE_ENV: &str = "VERUM_TEST_TEXT_STORAGE_CASE";
const DESCRIPTOR_BYTES: u64 = verum_common::layout::TEXT_SIZE + verum_common::layout::POINTER_SIZE;
type Allocate = unsafe extern "C" fn(u64, u64) -> *mut u8;
type Adopt = unsafe extern "C" fn(*mut u8, u64, u64) -> u64;
type Release = unsafe extern "C" fn(u64);
type Literal = unsafe extern "C" fn() -> u64;
type Duplicate = unsafe extern "C" fn(u64) -> u64;
type Snapshot = unsafe extern "C" fn(u64) -> u64;

struct Native {
    allocate: Allocate,
    adopt: Adopt,
    adopt_wide: Adopt,
    release: Release,
    release_wide: Release,
    destroy: Release,
    destroy_helper: Release,
    replace: unsafe extern "C" fn(u64, u64),
    literal: Literal,
    duplicate: Duplicate,
    from_cstr: unsafe extern "C" fn(*const u8) -> u64,
    snapshot: Snapshot,
    arm: unsafe extern "C" fn(),
}
impl Native {
    fn load(library: &libloading::Library) -> Self {
        // SAFETY: the exports below are emitted with these exact native ABIs;
        // the caller keeps the library alive throughout every operation.
        unsafe {
            Self {
                allocate: *library.get(b"allocate").unwrap(),
                adopt: *library.get(b"adopt").unwrap(),
                adopt_wide: *library.get(b"adopt_wide").unwrap(),
                release: *library.get(b"release").unwrap(),
                release_wide: *library.get(b"release_wide").unwrap(),
                destroy: *library.get(b"destroy").unwrap(),
                destroy_helper: *library.get(b"destroy_helper").unwrap(),
                replace: *library.get(b"replace").unwrap(),
                literal: *library.get(b"literal").unwrap(),
                duplicate: *library.get(b"duplicate").unwrap(),
                from_cstr: *library.get(b"from_cstr").unwrap(),
                snapshot: *library.get(b"snapshot").unwrap(),
                arm: *library.get(b"arm").unwrap(),
            }
        }
    }
    fn counts(&self) -> [u64; 3] {
        // SAFETY: valid indices address live observer globals in this image.
        unsafe { [(self.snapshot)(0), (self.snapshot)(3), (self.snapshot)(6)] }
    }
    fn last(&self, index: u64) -> u64 {
        assert!(index < 12);
        // SAFETY: the checked index addresses a live observer global.
        unsafe { (self.snapshot)(index) }
    }
}

fn fixture() -> (VbcModule, u32) {
    let mut ast = Parser::new(include_str!("../../../core/text/text.vr"))
        .parse_module().unwrap();
    ast.items.retain(|item| matches!(&item.kind, ItemKind::Module(_))
        || matches!(&item.kind, ItemKind::Type(declaration) if declaration.name.name.as_str() == "Text"));
    assert_eq!(ast.items.len(), 2, "actual canonical Text declaration");
    // The entry only asks ordinary lowering to emit its complete runtime.
    ast.items.extend(Parser::new("fn main() -> Int { 0 }").parse_module().unwrap().items);
    let mut module = VbcCodegen::with_config(CodegenConfig::new("core.text.text"))
        .compile_module(&ast).unwrap();
    assert_eq!(module.get_type(TypeId::TEXT).unwrap().fields.len(), 3);
    let pooled = module.intern_string("pooled");
    let constant = module.add_constant(Constant::String(pooled));
    let mut wire = deserialize_module(&serialize_module(&module).unwrap()).unwrap();
    verum_vbc::validate::validate_module(&wire).unwrap();
    for function in &mut wire.functions {
        let start = function.bytecode_offset as usize;
        let mut body = verum_vbc::bytecode::decode_instructions(
            &wire.bytecode[start..start + function.bytecode_length as usize],
        ).unwrap();
        verum_vbc::bytecode::jump_offsets_to_instr_indices(&mut body);
        function.instructions = Some(body);
    }
    (wire, constant.0)
}

fn publish<'ctx>(context: &'ctx Context, module: &Module<'ctx>, private: &str, name: &str) {
    let selected = module.get_function(private).unwrap_or_else(|| panic!("missing {private}"));
    assert!(selected.count_basic_blocks() > 0, "must execute a real definition");
    let wrapper = module.add_function(name, selected.get_type(), None);
    let builder = context.create_builder();
    builder.position_at_end(context.append_basic_block(wrapper, "entry"));
    let args: List<_> = wrapper.get_params().iter().map(|value| (*value).into()).collect();
    let result = builder.build_call(selected, &args, "").unwrap().try_as_basic_value().basic();
    match result {
        Some(value) => { builder.build_return(Some(&value)).unwrap(); }
        None => { builder.build_return(None).unwrap(); }
    }
}

fn operations<'ctx>(context: &'ctx Context, module: &Module<'ctx>, wire: &VbcModule, constant: u32) {
    let word = context.i64_type();
    for base in [0, 128] {
        for (name, operation, arity) in [
            (if base == 0 { "adopt" } else { "adopt_wide" }, TextSubOpcode::AdoptOwned, 3),
            (if base == 0 { "release" } else { "release_wide" }, TextSubOpcode::ReleaseStorage, 1),
        ] {
            let params = [context.ptr_type(Default::default()).into(), word.into(), word.into()];
            let ty = if arity == 3 { word.fn_type(&params, false) }
                else { context.void_type().fn_type(&[word.into()], false) };
            let function = module.add_function(name, ty, None);
            let mut ctx = FunctionContext::with_vbc_module(context, module, wire, function, name);
            ctx.builder().position_at_end(context.append_basic_block(function, "entry"));
            for (index, parameter) in function.get_params().into_iter().enumerate() {
                ctx.set_register(base + index as u16, parameter);
            }
            let destination = Reg(base + arity);
            let mut operands = List::<u8>::new().into();
            verum_vbc::encoding::encode_reg(destination, &mut operands);
            for index in 0..arity { verum_vbc::encoding::encode_reg(Reg(base + index), &mut operands); }
            lower_instruction(&mut ctx, &Instruction::TextExtended { sub_op: operation as u8, operands }).unwrap();
            if arity == 3 { ctx.builder().build_return(Some(&ctx.get_register(destination.0).unwrap())).unwrap(); }
            else { ctx.builder().build_return(None).unwrap(); }
        }
    }
    for (name, parameters, instruction) in [
        ("literal", 0, Instruction::LoadK { dst: Reg(1), const_id: constant }),
        ("duplicate", 1, Instruction::CloneTyped { dst: Reg(1), src: Reg(0), type_ref: TypeRef::Concrete(TypeId::TEXT) }),
    ] {
        let function = module.add_function(name, word.fn_type(&[word.into()][..parameters], false), None);
        let mut ctx = FunctionContext::with_vbc_module(context, module, wire, function, name);
        ctx.builder().position_at_end(context.append_basic_block(function, "entry"));
        if let Some(parameter) = function.get_first_param() { ctx.set_register(0, parameter); }
        lower_instruction(&mut ctx, &instruction).unwrap();
        let result = ctx.get_register(1).unwrap();
        let result = if result.is_pointer_value() {
            ctx.builder().build_ptr_to_int(result.into_pointer_value(), word, "result_word").unwrap()
        } else { result.into_int_value() };
        ctx.builder().build_return(Some(&result)).unwrap();
    }
    let function = module.add_function("destroy", context.void_type().fn_type(&[word.into()], false), None);
    let mut ctx = FunctionContext::with_vbc_module(context, module, wire, function, "destroy");
    ctx.builder().position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, function.get_first_param().unwrap());
    lower_instruction(&mut ctx, &Instruction::DropTyped {
        src: Reg(0), type_ref: TypeRef::Concrete(TypeId::TEXT),
    }).unwrap();
    lower_instruction(&mut ctx, &Instruction::RetV).unwrap();
    let function = module.add_function("replace", context.void_type().fn_type(&[word.into(), word.into()], false), None);
    let mut ctx = FunctionContext::with_vbc_module(context, module, wire, function, "replace");
    ctx.builder().position_at_end(context.append_basic_block(function, "entry"));
    ctx.set_register(0, function.get_nth_param(0).unwrap());
    ctx.set_register(1, function.get_nth_param(1).unwrap());
    ctx.mark_text_register(1);
    lower_instruction(&mut ctx, &Instruction::DerefMut { ref_reg: Reg(0), value: Reg(1) }).unwrap();
    lower_instruction(&mut ctx, &Instruction::RetV).unwrap();
}

/// Observe, then delegate to unchanged allocator/deallocator definitions. The
/// armed marker proves a refused operation did not publish or release storage.
fn observe<'ctx>(context: &'ctx Context, module: &Module<'ctx>) {
    let word = context.i64_type();
    let cells = module.add_global(word.array_type(12), None, "storage_observations");
    cells.set_initializer(&word.array_type(12).const_zero());
    let armed = module.add_global(context.bool_type(), None, "storage_observer_armed");
    armed.set_initializer(&context.bool_type().const_zero());
    let builder = context.create_builder();
    let address = |index| {
        // SAFETY: every observer index is bounded by the twelve-word array.
        unsafe { builder.build_gep(word, cells.as_pointer_value(), &[index], "observation").unwrap() }
    };
    for (name, index) in [("verum_cbgr_deallocate", 0), ("verum_dealloc", 3), ("verum_os_free", 6), ("verum_alloc", 9)] {
        let function = module.get_function(name).unwrap();
        let production = function.get_first_basic_block().expect("real allocator body");
        let entry = context.append_basic_block(function, "observe");
        entry.move_before(production).unwrap();
        let unexpected = context.append_basic_block(function, "observed_forbidden_operation");
        builder.position_at_end(entry);
        let count = address(word.const_int(index, false));
        let previous = builder.build_load(word, count, "previous").unwrap().into_int_value();
        builder.build_store(count, builder.build_int_add(previous, word.const_int(1, false), "count").unwrap()).unwrap();
        let first = function.get_first_param().unwrap();
        let pointer = if first.is_pointer_value() { builder.build_ptr_to_int(first.into_pointer_value(), word, "pointer").unwrap() }
            else { word.const_zero() };
        let size = if name == "verum_alloc" { first.into_int_value() }
            else { function.get_nth_param(1).map(|value| value.into_int_value()).unwrap_or(word.const_zero()) };
        builder.build_store(address(word.const_int(index + 1, false)), pointer).unwrap();
        builder.build_store(address(word.const_int(index + 2, false)), size).unwrap();
        let enabled = builder.build_load(context.bool_type(), armed.as_pointer_value(), "armed").unwrap().into_int_value();
        builder.build_conditional_branch(enabled, unexpected, production).unwrap();
        builder.position_at_end(unexpected);
        let marker = builder.build_global_string_ptr(FORBIDDEN, "forbidden_marker").unwrap();
        builder.build_call(module.get_function("verum_internal_write").unwrap(), &[
            word.const_int(2, false).into(), marker.as_pointer_value().into(), word.const_int(FORBIDDEN.len() as u64, false).into(),
        ], "").unwrap();
        builder.build_unconditional_branch(production).unwrap();
    }
    let snapshot = module.add_function("snapshot", word.fn_type(&[word.into()], false), None);
    builder.position_at_end(context.append_basic_block(snapshot, "entry"));
    let value = builder.build_load(word, address(snapshot.get_first_param().unwrap().into_int_value()), "value").unwrap();
    builder.build_return(Some(&value)).unwrap();
    let arm = module.add_function("arm", context.void_type().fn_type(&[], false), None);
    builder.position_at_end(context.append_basic_block(arm, "entry"));
    builder.build_store(armed.as_pointer_value(), context.bool_type().const_int(1, false)).unwrap();
    builder.build_return(None).unwrap();
}

fn with_native(run: impl FnOnce(&Native, &Path)) {
    let _exclusive = NATIVE.lock().unwrap();
    let (wire, constant) = fixture();
    let context = Context::create();
    let mut lowering = VbcToLlvmLowering::new(&context, LoweringConfig::debug("text-storage").with_debug_info(false));
    lowering.lower_module(&wire).unwrap();
    let module = lowering.module();
    operations(&context, module, &wire, constant);
    for (private, public) in [("verum_cbgr_allocate_aligned", "allocate"), ("verum_text_free", "destroy_helper"), ("verum_text_from_cstr", "from_cstr")] {
        publish(&context, module, private, public);
    }
    observe(&context, module);
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let triple = TargetMachine::get_default_triple();
    let machine = Target::from_triple(&triple).unwrap().create_target_machine(
        &triple, "generic", "", OptimizationLevel::None, RelocMode::PIC, CodeModel::Default,
    ).unwrap();
    module.set_triple(&triple);
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    module.verify().unwrap();
    let directory = tempfile::Builder::new().prefix("text-storage-")
        .tempdir_in(std::env::var_os("CARGO_TARGET_DIR").expect("private target on development volume")).unwrap();
    let object = directory.path().join("storage.o");
    let image = directory.path().join("storage.dylib");
    machine.write_to_file(module, FileType::Object, &object).unwrap();
    let mut linker = Command::new("cc");
    linker.arg(&object).args(["-dynamiclib", "-nostdlib", "-lSystem", "-Wl,-dead_strip"]);
    for name in ["allocate", "adopt", "adopt_wide", "release", "release_wide", "destroy", "destroy_helper", "replace", "literal", "duplicate", "from_cstr", "snapshot", "arm"] {
        linker.arg(format!("-Wl,-exported_symbol,_{name}"));
    }
    let output = linker.arg("-o").arg(&image).output().unwrap();
    assert!(output.status.success(), "{}", Text::from_utf8_lossy(&output.stderr));
    // SAFETY: production LLVM is linked through the documented Darwin ABI;
    // this library outlives the calls and their storage. No runtime is replaced.
    let library = unsafe { libloading::Library::new(&image) }.unwrap();
    run(&Native::load(&library), &image);
}

#[test]
fn native_owned_text_releases_bytes_once_and_then_its_arena_descriptor() {
    with_native(|native, _| {
        for (adopt, release) in [(native.adopt, native.release), (native.adopt_wide, native.release_wide)] {
            for (len, capacity) in [(1, 17), (0, 0)] {
                // SAFETY: actual CBGR allocation is adopted once, then accessed only
                // before storage release; the descriptor survives until destroy.
                // The owned empty buffer proves cap == 0 does not mean borrowed.
                unsafe {
                    let bytes = (native.allocate)(capacity + 1, 1);
                    assert!(!bytes.is_null());
                    bytes.add(len as usize).write(0x7f);
                    if len != 0 { bytes.write(73); }
                    let text = adopt(bytes, len, capacity);
                    assert_eq!(*bytes.add(len as usize), 0, "adoption initializes the terminator");
                    if len != 0 { assert_eq!(*bytes, 73, "adoption preserves the initialized prefix"); }
                    let before = native.counts();
                    release(text);
                    assert_eq!(native.counts(), [before[0] + 1, before[1] + 1, before[2]]);
                    assert_eq!(native.last(1), bytes as u64);
                    assert_eq!(std::slice::from_raw_parts(text as *const u64, 3), &[0, 0, 0]);
                    release(text);
                    assert_eq!(native.counts(), [before[0] + 1, before[1] + 1, before[2]]);
                    (native.destroy)(text);
                    assert_eq!(native.counts(), [before[0] + 1, before[1] + 2, before[2]]);
                    assert_eq!((native.last(4), native.last(5)), (text, DESCRIPTOR_BYTES));
                }
            }
        }
    });
}

#[test]
fn native_literal_wrappers_preserve_their_pooled_bytes() {
    with_native(|native, _| {
        // SAFETY: literal producers issue distinct live mutable descriptors;
        // only their immutable pooled backing is shared across these calls.
        unsafe {
            let first = (native.literal)();
            let second = (native.literal)();
            assert_ne!(first, second);
            let data = *(first as *const u64);
            assert_eq!(data, *(second as *const u64));
            let before = native.counts();
            (native.release)(first);
            assert_eq!(native.counts(), before);
            (native.destroy)(first);
            assert_eq!(native.counts(), [before[0], before[1] + 1, before[2]]);
            assert_eq!(std::slice::from_raw_parts(data as *const u8, 6), b"pooled");
            assert_eq!(*((second as *const u64).add(1)), 6);
            let third = (native.literal)();
            assert_eq!(*(third as *const u64), data);
            (native.destroy)(second);
            (native.destroy)(third);
            assert_eq!(native.counts(), [before[0], before[1] + 3, before[2]]);
        }
    });
}

#[test]
fn native_typed_text_copy_releases_its_os_descriptor_and_cbgr_bytes() {
    with_native(|native, _| {
        // SAFETY: CloneTyped reads a live producer-issued Text and returns its
        // own descriptor/backing. Each independent owner is destroyed once.
        unsafe {
            let source = (native.literal)();
            let copy = (native.duplicate)(source);
            assert_ne!(copy, source);
            assert_ne!(*(copy as *const u64), *(source as *const u64));
            let data = *(copy as *const u64);
            (native.destroy)(source);
            assert_eq!(std::slice::from_raw_parts(data as *const u8, 6), b"pooled");
            let before = native.counts();
            (native.release)(copy);
            assert_eq!(native.counts(), [before[0] + 1, before[1] + 1, before[2]]);
            (native.destroy)(copy);
            assert_eq!(native.counts(), [before[0] + 1, before[1] + 1, before[2] + 1]);
            assert_eq!((native.last(7), native.last(8)), (copy, DESCRIPTOR_BYTES));
        }
    });
}

#[test]
fn native_foreign_c_string_is_copied_before_its_caller_changes_it() {
    with_native(|native, _| {
        let mut foreign = *b"retained\0";
        // SAFETY: the caller's terminated buffer is live during conversion;
        // the produced Text must own an independent allocation thereafter.
        unsafe {
            let text = (native.from_cstr)(foreign.as_ptr());
            let data = *(text as *const u64);
            assert_ne!(data, foreign.as_ptr() as u64);
            foreign[0] = b'X';
            assert_eq!(std::slice::from_raw_parts(data as *const u8, 8), b"retained");
            let before = native.counts();
            (native.destroy)(text);
            assert_eq!(native.counts(), [before[0] + 1, before[1] + 2, before[2]]);
            assert_eq!(foreign[0], b'X');
        }
    });
}

#[test]
fn native_text_replacement_transfers_bytes_and_preserves_descriptor_domains() {
    with_native(|native, _| {
        // SAFETY: every input comes from an actual producer. Replacement consumes
        // its source; only the surviving destination is accessed afterward.
        unsafe {
            let target = (native.literal)();
            let pooled = *(target as *const u64);
            let source = (native.duplicate)(target);
            let bytes = *(source as *const u64);
            let before = native.counts();
            (native.replace)(target, source);
            assert_eq!(*(target as *const u64), bytes);
            assert_eq!(std::slice::from_raw_parts(pooled as *const u8, 6), b"pooled");
            assert_eq!(native.counts(), [before[0], before[1], before[2] + 1]);
            assert_eq!((native.last(7), native.last(8)), (source, DESCRIPTOR_BYTES));
            (native.destroy)(target);
            assert_eq!(native.counts(), [before[0] + 1, before[1] + 2, before[2] + 1]);

            let literal = (native.literal)();
            let target = (native.duplicate)(literal);
            (native.destroy)(literal);
            let fresh = (native.allocate)(9, 1);
            fresh.write(73);
            let source = (native.adopt)(fresh, 1, 8);
            let before = native.counts();
            (native.replace)(target, source);
            assert_eq!(native.counts(), [before[0] + 1, before[1] + 2, before[2]]);
            assert_eq!(std::slice::from_raw_parts(target as *const u64, 3), &[fresh as u64, 1, 8]);
            (native.replace)(target, target);
            assert_eq!(*fresh, 73);
            assert_eq!(native.counts(), [before[0] + 1, before[1] + 2, before[2]]);
            (native.destroy)(target);
            assert_eq!(native.counts(), [before[0] + 2, before[1] + 3, before[2] + 1]);
        }
    });
}

fn refused_case(native: &Native, case: &str) {
    // SAFETY: even the negative cases use real live allocations. Only the
    // declared dimensions or receipt are corrupted, never the pointer origin.
    unsafe {
        let data = (native.allocate)(9, 1);
        assert!(!data.is_null());
        if case.starts_with("replace") {
            let target = (native.literal)();
            let source = (native.adopt)(data, 1, 8);
            match case {
                "replace_receipt" => (source as *mut u64).add(3).write(u64::MAX),
                "replace_static" => (target as *mut u64).add(3).write(1),
                "replace_shape" => (source as *mut u64).add(1).write(9),
                _ => panic!("unknown replacement refusal {case}"),
            }
            (native.arm)();
            (native.replace)(target, source);
        } else if case.starts_with("receipt") {
            let text = (native.adopt)(data, 1, 8);
            (text as *mut u8).add(verum_common::layout::TEXT_SIZE as usize)
                .cast::<u64>().write(u64::MAX);
            (native.arm)();
            match case {
                "receipt_release" => (native.release)(text),
                "receipt_helper" => (native.destroy_helper)(text),
                _ => (native.destroy)(text),
            }
        } else {
            let (pointer, len, cap) = match case {
                "negative_length" => (data, u64::MAX, 8),
                "negative_capacity" => (data, 1, u64::MAX),
                "length_exceeds_capacity" => (data, 9, 8),
                "null_nonempty" => (std::ptr::null_mut(), 1, 1),
                "null_reserved" => (std::ptr::null_mut(), 0, 8),
                "extent_exceeds_limit" => (data, 1, verum_common::layout::MAX_ALLOCATION_SIZE as u64),
                _ => panic!("unknown refusal case {case}"),
            };
            (native.arm)();
            (native.adopt)(pointer, len, cap);
        }
    }
    panic!("invalid Text storage was accepted: {case}");
}

#[test]
fn native_storage_rejects_invalid_receipts_and_shapes_before_side_effects() {
    if let Some(image) = std::env::var_os(IMAGE_ENV) {
        // SAFETY: this is the same verified image produced by the parent test.
        let library = unsafe { libloading::Library::new(image) }.unwrap();
        refused_case(&Native::load(&library), &std::env::var(CASE_ENV).unwrap());
        return;
    }
    with_native(|_, image| {
        for case in ["replace_receipt", "replace_static", "replace_shape", "receipt_release", "receipt_destroy", "receipt_helper", "negative_length", "negative_capacity", "length_exceeds_capacity", "null_nonempty", "null_reserved", "extent_exceeds_limit"] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "native_storage_rejects_invalid_receipts_and_shapes_before_side_effects", "--nocapture"])
                .env(IMAGE_ENV, image).env(CASE_ENV, case).output().unwrap();
            // Darwin lowers llvm.trap to SIGILL (x86) or SIGTRAP (AArch64).
            assert!(matches!(output.status.signal(), Some(4 | 5)), "{case}: {:?}: {}", output.status, Text::from_utf8_lossy(&output.stderr));
            assert!(!Text::from_utf8_lossy(&output.stderr).contains(FORBIDDEN), "{case}: allocator/free ran before refusal");
        }
    });
}
