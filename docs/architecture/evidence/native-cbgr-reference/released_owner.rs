//! Reproducer: actual native reference check after actual OS-page release.
use verum_codegen::llvm::{CbgrLowering, LoweringConfig, VbcToLlvmLowering};
use verum_common::{Text, cbgr::AllocationHeader};
use verum_llvm::{OptimizationLevel, context::Context, memory_buffer::MemoryBuffer,
    targets::{InitializationConfig, Target}, values::AnyValue};
use std::sync::atomic::Ordering;
#[repr(C)]
struct Snapshot { pointer: *mut u8, generation: u32, epoch_caps: u32 }
fn main() {
    Target::initialize_native(&InitializationConfig::default()).unwrap();
    let context = Context::create();
    let ast = verum_fast_parser::Parser::new("fn main() {}").parse_module().unwrap();
    let vbc = verum_vbc::codegen::VbcCodegen::new().compile_module(&ast).unwrap();
    let mut lower = VbcToLlvmLowering::new(&context,
        LoweringConfig::debug("released_header").with_debug_info(false));
    lower.lower_module(&vbc).unwrap();
    let module = lower.module();
    let builder = context.create_builder();
    let pointer = context.ptr_type(Default::default());
    let make = module.add_function("snapshot", context.void_type().fn_type(
        &[pointer.into(), pointer.into()], false), None);
    builder.position_at_end(context.append_basic_block(make, "entry"));
    let mut cbgr = CbgrLowering::new(&context);
    let snapshot = cbgr.create_thin_ref(&builder,
        make.get_nth_param(0).unwrap().into_pointer_value()).unwrap();
    builder.build_store(make.get_nth_param(1).unwrap().into_pointer_value(), snapshot).unwrap();
    builder.build_return(None).unwrap();
    let mut definitions = Text::new();
    for name in ["snapshot", "verum_cbgr_validate_ref", "verum_cbgr_check",
        "verum_os_alloc", "verum_os_free", "mmap", "munmap"] {
        definitions.push_str(&module.get_function(name).unwrap().print_to_string().to_string());
        definitions.push('\n');
    }
    let executable = context.create_module_from_ir(MemoryBuffer::create_from_memory_range_copy(
        definitions.as_bytes(), "actual_native_page_release")).unwrap();
    executable.verify().unwrap();
    let engine = executable.create_jit_execution_engine(OptimizationLevel::None).unwrap();
    // SAFETY: exact ABI for the emitted runtime functions. The last call
    // deliberately presents a released allocation to the safety checker.
    unsafe {
        let allocate = engine.get_function::<unsafe extern "C" fn(u64)->*mut u8>("verum_os_alloc").unwrap();
        let release = engine.get_function::<unsafe extern "C" fn(*mut u8,u64)>("verum_os_free").unwrap();
        let make = engine.get_function::<unsafe extern "C" fn(*mut u8,*mut Snapshot)>("snapshot").unwrap();
        let check = engine.get_function::<unsafe extern "C" fn(*const Snapshot)->bool>("verum_cbgr_check").unwrap();
        let base = allocate.call(4096);
        assert!(!base.is_null());
        base.cast::<AllocationHeader>().write(AllocationHeader::new(4064,32,1,0xff));
        (*base.cast::<AllocationHeader>()).store_epoch(0x1234,Ordering::Release);
        let mut snapshot = Snapshot {pointer:std::ptr::null_mut(),generation:0,epoch_caps:0};
        make.call(base.add(32),&mut snapshot);
        assert!(check.call(&snapshot));
        eprintln!("actual snapshot valid before actual verum_os_free");
        release.call(base,4096);
        eprintln!("actual OS page released; invoking ordinary native check");
        let verdict = check.call(&snapshot);
        eprintln!("released owner validation returned {verdict}");
        assert!(!verdict,"released owner cannot grant access");
    }
}
