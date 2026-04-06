//! Metal GPU runtime as LLVM IR — replaces verum_metal.m (563 LOC).
//!
//! All Objective-C calls compile to `objc_msgSend(receiver, selector, ...)`.
//! This is a C function in `/usr/lib/libobjc.dylib` that LLVM IR can call directly.
//!
//! Key ObjC runtime functions:
//!   - `objc_msgSend(ptr, ptr, ...) -> ptr` — message dispatch
//!   - `sel_registerName(ptr) -> ptr` — get selector from C string
//!   - `objc_getClass(ptr) -> ptr` — get class by name
//!   - `objc_retain(ptr) -> ptr` — increment refcount (__bridge_retained)
//!   - `objc_release(ptr)` — decrement refcount (__bridge_transfer)
//!   - `MTLCreateSystemDefaultDevice() -> ptr` — Metal framework entry point
//!
//! ARC bridging:
//!   - `__bridge` = no-op cast (same pointer)
//!   - `__bridge_retained` = objc_retain (increment refcount)
//!   - `__bridge_transfer` = transfer ownership (no increment, will release)
//!
//! MTLSize struct: `{ NSUInteger, NSUInteger, NSUInteger }` = 24 bytes.
//! On arm64, passed in registers (x0-x2 for each triple).

use verum_llvm::context::Context;
use verum_llvm::module::Module;
use verum_llvm::types::FunctionType;
use verum_llvm::values::FunctionValue;
use verum_llvm::{AddressSpace, IntPredicate};
use super::error::{BuildExt, OptionExt, Result};

/// Emit Metal GPU runtime functions as LLVM IR.
pub struct MetalIR<'ctx> {
    context: &'ctx Context,
}

impl<'ctx> MetalIR<'ctx> {
    pub fn new(context: &'ctx Context) -> Self {
        Self { context }
    }

    /// Emit all Metal runtime functions into the module.
    pub fn emit_metal_functions(&self, module: &Module<'ctx>) -> Result<()> {
        // Only emit on macOS — Metal is Apple-only
        if !cfg!(target_os = "macos") {
            self.emit_stubs(module)?;
            return Ok(());
        }

        // Globals
        self.emit_globals(module);

        // ObjC runtime declarations
        self.emit_objc_runtime_decls(module);

        // Initialization
        self.emit_ensure_init(module)?;

        // Device management
        self.emit_get_device(module)?;
        self.emit_device_name(module)?;
        self.emit_max_memory(module)?;
        self.emit_max_threads_per_threadgroup(module)?;
        self.emit_supports_family(module)?;
        self.emit_gpu_core_count(module)?;

        // Buffer management
        self.emit_alloc(module)?;
        self.emit_alloc_with_data(module)?;
        self.emit_buffer_contents(module)?;
        self.emit_buffer_length(module)?;
        self.emit_free(module)?;

        // Shader compilation
        self.emit_compile_shader(module)?;
        self.emit_get_pipeline(module)?;

        // Compute dispatch
        self.emit_dispatch_1d(module)?;
        self.emit_dispatch_2d(module)?;
        self.emit_dispatch_async(module)?;
        self.emit_wait(module)?;
        self.emit_execution_time_ns(module)?;

        // High-level operations
        self.emit_vector_add_f32(module)?;
        self.emit_sgemm(module)?;
        self.emit_benchmark(module)?;
        Ok(())
    }

    // ========================================================================
    // Helpers
    // ========================================================================

    fn get_or_declare(
        &self, module: &Module<'ctx>, name: &str, fn_type: FunctionType<'ctx>,
    ) -> FunctionValue<'ctx> {
        module.get_function(name).unwrap_or_else(|| module.add_function(name, fn_type, None))
    }

    /// Get or declare a function, returning None if it already has a body.
    fn get_or_declare_new(
        &self, module: &Module<'ctx>, name: &str, fn_type: FunctionType<'ctx>,
    ) -> Option<FunctionValue<'ctx>> {
        let func = self.get_or_declare(module, name, fn_type);
        if func.count_basic_blocks() > 0 {
            return None;
        }
        Some(func)
    }

    /// Build an objc_msgSend call: objc_msgSend(receiver, selector, args...)
    /// Returns the result as ptr (i64-castable).
    fn build_objc_msg_send(
        &self,
        module: &Module<'ctx>,
        builder: &verum_llvm::builder::Builder<'ctx>,
        receiver: verum_llvm::values::PointerValue<'ctx>,
        selector_name: &str,
        args: &[verum_llvm::values::BasicValueEnum<'ctx>],
        name: &str,
    ) -> Result<verum_llvm::values::PointerValue<'ctx>> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());

        // Get selector
        let sel_fn = module.get_function("sel_registerName").or_missing_fn("sel_registerName")?;
        let sel_str = self.build_global_str(module, builder, selector_name, &format!("sel_{}", name));
        let sel = builder.build_call(sel_fn, &[sel_str.into()], &format!("{}_sel", name))
            .or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();

        // Build args: [receiver, selector, ...extra_args]
        // Must use into() to convert BasicValueEnum -> BasicMetadataValueEnum
        let mut all_args: Vec<verum_llvm::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
        all_args.push(receiver.into());
        all_args.push(sel.into());
        for arg in args {
            all_args.push((*arg).into());
        }

        // objc_msgSend is variadic: (ptr, ptr, ...)
        let msg_send = module.get_function("objc_msgSend").or_missing_fn("objc_msgSend")?;
        let result = builder.build_call(msg_send, &all_args, name)
            .or_llvm_err()?;

        // objc_msgSend returns ptr (void*) which we treat as ptr
        match result.try_as_basic_value().basic() {
            Some(v) => Ok(v.into_pointer_value()),
            None => {
                // void return — return null ptr
                Ok(ptr_type.const_null())
            }
        }
    }

    /// Build an objc_msgSend call that returns i64 (for NSUInteger results).
    fn build_objc_msg_send_i64(
        &self,
        module: &Module<'ctx>,
        builder: &verum_llvm::builder::Builder<'ctx>,
        receiver: verum_llvm::values::PointerValue<'ctx>,
        selector_name: &str,
        args: &[verum_llvm::values::BasicValueEnum<'ctx>],
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        let result_ptr = self.build_objc_msg_send(module, builder, receiver, selector_name, args, name)?;
        // Cast ptr to i64 (ptrtoint)
        Ok(builder.build_ptr_to_int(result_ptr, i64_type, &format!("{}_i64", name)).or_llvm_err()?)
    }

    /// Build a global constant C string and return a pointer to it.
    fn build_global_str(
        &self,
        module: &Module<'ctx>,
        _builder: &verum_llvm::builder::Builder<'ctx>,
        value: &str,
        name: &str,
    ) -> verum_llvm::values::PointerValue<'ctx> {
        // Use a sanitized name for the global — replace colons, spaces etc.
        let safe_name = format!("__metal_str_{}", name.replace(|c: char| !c.is_alphanumeric(), "_"));
        // Check if we already have this global
        if let Some(gv) = module.get_global(&safe_name) {
            return gv.as_pointer_value();
        }
        let bytes = value.as_bytes();
        let arr_type = self.context.i8_type().array_type((bytes.len() + 1) as u32);
        let global = module.add_global(arr_type, Some(AddressSpace::default()), &safe_name);
        // Build the initializer: array of i8 with null terminator
        let mut vals: Vec<verum_llvm::values::IntValue<'ctx>> = bytes.iter()
            .map(|&b| self.context.i8_type().const_int(b as u64, false))
            .collect();
        vals.push(self.context.i8_type().const_int(0, false)); // null terminator
        let arr_val = self.context.i8_type().const_array(&vals);
        global.set_initializer(&arr_val);
        global.set_constant(true);
        global.set_unnamed_addr(true);
        global.as_pointer_value()
    }

    /// Convert an i64 handle to a ptr (inttoptr).
    fn handle_to_ptr(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        handle: verum_llvm::values::IntValue<'ctx>,
        name: &str,
    ) -> Result<verum_llvm::values::PointerValue<'ctx>> {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        Ok(builder.build_int_to_ptr(handle, ptr_type, name).or_llvm_err()?)
    }

    /// Convert a ptr to i64 handle (ptrtoint).
    fn ptr_to_handle(
        &self,
        builder: &verum_llvm::builder::Builder<'ctx>,
        ptr: verum_llvm::values::PointerValue<'ctx>,
        name: &str,
    ) -> Result<verum_llvm::values::IntValue<'ctx>> {
        let i64_type = self.context.i64_type();
        Ok(builder.build_ptr_to_int(ptr, i64_type, name).or_llvm_err()?)
    }

    // ========================================================================
    // Globals
    // ========================================================================

    fn emit_globals(&self, module: &Module<'ctx>) {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let i64_type = self.context.i64_type();

        // @__verum_metal_device = global ptr null
        if module.get_global("__verum_metal_device").is_none() {
            let g = module.add_global(ptr_type, Some(AddressSpace::default()), "__verum_metal_device");
            g.set_initializer(&ptr_type.const_null());
        }

        // @__verum_metal_queue = global ptr null
        if module.get_global("__verum_metal_queue").is_none() {
            let g = module.add_global(ptr_type, Some(AddressSpace::default()), "__verum_metal_queue");
            g.set_initializer(&ptr_type.const_null());
        }

        // @__verum_metal_initialized = global i64 0
        if module.get_global("__verum_metal_initialized").is_none() {
            let g = module.add_global(i64_type, Some(AddressSpace::default()), "__verum_metal_initialized");
            g.set_initializer(&i64_type.const_int(0, false));
        }
    }

    // ========================================================================
    // ObjC runtime declarations
    // ========================================================================

    fn emit_objc_runtime_decls(&self, module: &Module<'ctx>) {
        let ptr_type = self.context.ptr_type(AddressSpace::default());
        let void_type = self.context.void_type();

        // objc_msgSend(ptr receiver, ptr selector, ...) -> ptr
        // Declared as variadic
        if module.get_function("objc_msgSend").is_none() {
            let ft = ptr_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
            module.add_function("objc_msgSend", ft, None);
        }

        // sel_registerName(ptr name) -> ptr
        if module.get_function("sel_registerName").is_none() {
            let ft = ptr_type.fn_type(&[ptr_type.into()], false);
            module.add_function("sel_registerName", ft, None);
        }

        // objc_getClass(ptr name) -> ptr
        if module.get_function("objc_getClass").is_none() {
            let ft = ptr_type.fn_type(&[ptr_type.into()], false);
            module.add_function("objc_getClass", ft, None);
        }

        // MTLCreateSystemDefaultDevice() -> ptr
        if module.get_function("MTLCreateSystemDefaultDevice").is_none() {
            let ft = ptr_type.fn_type(&[], false);
            module.add_function("MTLCreateSystemDefaultDevice", ft, None);
        }

        // objc_retain(ptr) -> ptr
        if module.get_function("objc_retain").is_none() {
            let ft = ptr_type.fn_type(&[ptr_type.into()], false);
            module.add_function("objc_retain", ft, None);
        }

        // objc_release(ptr)
        if module.get_function("objc_release").is_none() {
            let ft = void_type.fn_type(&[ptr_type.into()], false);
            module.add_function("objc_release", ft, None);
        }
    }

    // ========================================================================
    // Initialization
    // ========================================================================

    /// verum_metal_ensure_init():
    /// Check initialized flag, call MTLCreateSystemDefaultDevice(),
    /// get command queue via objc_msgSend(device, "newCommandQueue").
    /// Uses CAS (cmpxchg) for thread safety.
    fn emit_ensure_init(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let void_type = ctx.void_type();

        let fn_type = void_type.fn_type(&[], false);
        let func = match self.get_or_declare_new(module, "verum_metal_ensure_init", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let do_init = ctx.append_basic_block(func, "do_init");
        let done = ctx.append_basic_block(func, "done");

        // entry: check if already initialized
        builder.position_at_end(entry);
        let init_global = module.get_global("__verum_metal_initialized").or_internal("missing global")?;
        let init_val = builder.build_load(i64_type, init_global.as_pointer_value(), "init_flag").or_llvm_err()?.into_int_value();
        let is_init = builder.build_int_compare(IntPredicate::NE, init_val, i64_type.const_int(0, false), "is_init").or_llvm_err()?;
        builder.build_conditional_branch(is_init, done, do_init).or_llvm_err()?;

        // do_init: CAS 0 -> 1, if we win, initialize
        builder.position_at_end(do_init);

        // CAS: cmpxchg i64* @__verum_metal_initialized, 0, 1
        let cas_result = builder.build_cmpxchg(
            init_global.as_pointer_value(),
            i64_type.const_int(0, false),
            i64_type.const_int(1, false),
            verum_llvm::AtomicOrdering::AcquireRelease,
            verum_llvm::AtomicOrdering::Monotonic,
        ).or_llvm_err()?;
        let cas_success = builder.build_extract_value(cas_result, 1, "cas_ok").or_llvm_err()?.into_int_value();

        let init_body = ctx.append_basic_block(func, "init_body");
        builder.build_conditional_branch(cas_success, init_body, done).or_llvm_err()?;

        // init_body: call MTLCreateSystemDefaultDevice(), store device, create queue
        builder.position_at_end(init_body);

        let create_device_fn = module.get_function("MTLCreateSystemDefaultDevice").or_missing_fn("MTLCreateSystemDefaultDevice")?;
        let device = builder.build_call(create_device_fn, &[], "device").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();

        // Check device != null
        let device_null = builder.build_is_null(device, "dev_null").or_llvm_err()?;
        let store_dev = ctx.append_basic_block(func, "store_dev");
        let init_fail = ctx.append_basic_block(func, "init_fail");
        builder.build_conditional_branch(device_null, init_fail, store_dev).or_llvm_err()?;

        // init_fail: reset flag to 0, return
        builder.position_at_end(init_fail);
        builder.build_store(init_global.as_pointer_value(), i64_type.const_int(0, false)).or_llvm_err()?;
        builder.build_unconditional_branch(done).or_llvm_err()?;

        // store_dev: store device, create command queue
        builder.position_at_end(store_dev);
        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        builder.build_store(dev_global.as_pointer_value(), device).or_llvm_err()?;

        // queue = objc_msgSend(device, "newCommandQueue")
        let queue = self.build_objc_msg_send(module, &builder, device, "newCommandQueue", &[], "queue")?;
        let queue_global = module.get_global("__verum_metal_queue").or_internal("missing global")?;
        builder.build_store(queue_global.as_pointer_value(), queue).or_llvm_err()?;

        builder.build_unconditional_branch(done).or_llvm_err()?;

        // done: return
        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // Device management
    // ========================================================================

    /// verum_metal_get_device() -> i64: ensure_init, return device as i64
    fn emit_get_device(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[], false);
        let func = match self.get_or_declare_new(module, "verum_metal_get_device", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        builder.position_at_end(entry);

        // ensure_init()
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        // return device as i64
        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let result = self.ptr_to_handle(&builder, dev, "dev_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_device_name() -> i64:
    /// objc_msgSend(device, "name") -> NSString
    /// objc_msgSend(nsstring, "UTF8String") -> ptr
    /// return ptr as i64
    fn emit_device_name(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[], false);
        let func = match self.get_or_declare_new(module, "verum_metal_device_name", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let have_dev = ctx.append_basic_block(func, "have_dev");
        let no_dev = ctx.append_basic_block(func, "no_dev");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let is_null = builder.build_is_null(dev, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, no_dev, have_dev).or_llvm_err()?;

        builder.position_at_end(no_dev);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(have_dev);
        // nsname = objc_msgSend(dev, "name")
        let nsname = self.build_objc_msg_send(module, &builder, dev, "name", &[], "nsname")?;
        // cstr = objc_msgSend(nsname, "UTF8String")
        let cstr = self.build_objc_msg_send(module, &builder, nsname, "UTF8String", &[], "cstr")?;
        let result = self.ptr_to_handle(&builder, cstr, "name_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_max_memory() -> i64:
    /// objc_msgSend(device, "recommendedMaxWorkingSetSize") returns NSUInteger
    fn emit_max_memory(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[], false);
        let func = match self.get_or_declare_new(module, "verum_metal_max_memory", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let have_dev = ctx.append_basic_block(func, "have_dev");
        let no_dev = ctx.append_basic_block(func, "no_dev");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let is_null = builder.build_is_null(dev, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, no_dev, have_dev).or_llvm_err()?;

        builder.position_at_end(no_dev);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(have_dev);
        let result = self.build_objc_msg_send_i64(module, &builder, dev, "recommendedMaxWorkingSetSize", &[], "max_mem")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_max_threads_per_threadgroup() -> i64:
    /// MTLSize is { NSUInteger width, height, depth }
    /// objc_msgSend returns this struct — on arm64, width is in x0.
    /// We use a simpler approach: call and treat return as i64 (first field = width).
    fn emit_max_threads_per_threadgroup(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[], false);
        let func = match self.get_or_declare_new(module, "verum_metal_max_threads_per_threadgroup", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let have_dev = ctx.append_basic_block(func, "have_dev");
        let no_dev = ctx.append_basic_block(func, "no_dev");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let is_null = builder.build_is_null(dev, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, no_dev, have_dev).or_llvm_err()?;

        builder.position_at_end(no_dev);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        // maxThreadsPerThreadgroup returns MTLSize (24 bytes).
        // On arm64, a 24-byte struct returned via registers: x0=width, x1=height, x2=depth.
        // objc_msgSend return convention: x0 gets the first field (width).
        // We treat the ptr return as i64 which gives us width.
        builder.position_at_end(have_dev);
        let result = self.build_objc_msg_send_i64(module, &builder, dev, "maxThreadsPerThreadgroup", &[], "max_thr")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_supports_family(family: i64) -> i64
    fn emit_supports_family(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_supports_family", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let have_dev = ctx.append_basic_block(func, "have_dev");
        let no_dev = ctx.append_basic_block(func, "no_dev");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let family = func.get_nth_param(0).or_internal("missing param")?.into_int_value();

        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let is_null = builder.build_is_null(dev, "is_null").or_llvm_err()?;
        builder.build_conditional_branch(is_null, no_dev, have_dev).or_llvm_err()?;

        builder.position_at_end(no_dev);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(have_dev);
        // supportsFamily: takes NSInteger (i64 on arm64)
        let family_ptr = builder.build_int_to_ptr(family, ptr_type, "fam_ptr").or_llvm_err()?;
        let result_ptr = self.build_objc_msg_send(module, &builder, dev, "supportsFamily:", &[family_ptr.into()], "supports")?;
        let result_i64 = self.ptr_to_handle(&builder, result_ptr, "sup_i64")?;
        // Convert BOOL (0 or 1) — mask to 1 bit
        let one = i64_type.const_int(1, false);
        let masked = builder.build_and(result_i64, one, "masked").or_llvm_err()?;
        builder.build_return(Some(&masked)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_gpu_core_count() -> i64: return 0 (not directly queryable)
    fn emit_gpu_core_count(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();

        let fn_type = i64_type.fn_type(&[], false);
        let func = match self.get_or_declare_new(module, "verum_metal_gpu_core_count", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        builder.position_at_end(entry);

        // Apple doesn't expose core count via public API. Return 0.
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // Buffer management
    // ========================================================================

    /// verum_metal_alloc(size: i64) -> i64:
    /// sel = sel_registerName("newBufferWithLength:options:")
    /// buf = objc_msgSend(device, sel, size, 0 /*MTLStorageModeShared*/)
    /// objc_retain(buf)
    /// return buf as i64
    fn emit_alloc(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_alloc", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let size = func.get_nth_param(0).or_internal("missing param")?.into_int_value();

        // Check size > 0
        let size_ok = builder.build_int_compare(IntPredicate::SGT, size, i64_type.const_int(0, false), "sz_ok").or_llvm_err()?;
        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let dev_null = builder.build_is_null(dev, "dev_null").or_llvm_err()?;
        let dev_ok = builder.build_not(dev_null, "dev_ok").or_llvm_err()?;
        let both_ok = builder.build_and(size_ok, dev_ok, "both_ok").or_llvm_err()?;
        builder.build_conditional_branch(both_ok, valid, invalid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);
        // newBufferWithLength:options: — size as NSUInteger (i64), options=0 (MTLStorageModeShared)
        let size_ptr = builder.build_int_to_ptr(size, ptr_type, "sz_ptr").or_llvm_err()?;
        let zero_ptr = ptr_type.const_null();
        let buf = self.build_objc_msg_send(module, &builder, dev,
            "newBufferWithLength:options:", &[size_ptr.into(), zero_ptr.into()], "buf")?;

        // Check buf != null
        let buf_null = builder.build_is_null(buf, "buf_null").or_llvm_err()?;
        let retain_bb = ctx.append_basic_block(func, "retain");
        let null_ret = ctx.append_basic_block(func, "null_ret");
        builder.build_conditional_branch(buf_null, null_ret, retain_bb).or_llvm_err()?;

        builder.position_at_end(null_ret);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        // objc_retain for __bridge_retained semantics
        builder.position_at_end(retain_bb);
        let retain_fn = module.get_function("objc_retain").or_missing_fn("objc_retain")?;
        let retained = builder.build_call(retain_fn, &[buf.into()], "retained").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let result = self.ptr_to_handle(&builder, retained, "buf_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_alloc_with_data(data: i64, size: i64) -> i64
    fn emit_alloc_with_data(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into(), i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_alloc_with_data", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let data = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let size = func.get_nth_param(1).or_internal("missing param")?.into_int_value();

        let size_ok = builder.build_int_compare(IntPredicate::SGT, size, i64_type.const_int(0, false), "sz_ok").or_llvm_err()?;
        let data_ok = builder.build_int_compare(IntPredicate::NE, data, i64_type.const_int(0, false), "data_ok").or_llvm_err()?;
        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let dev_null = builder.build_is_null(dev, "dev_null").or_llvm_err()?;
        let dev_ok = builder.build_not(dev_null, "dev_ok").or_llvm_err()?;
        let ok1 = builder.build_and(size_ok, data_ok, "ok1").or_llvm_err()?;
        let all_ok = builder.build_and(ok1, dev_ok, "all_ok").or_llvm_err()?;
        builder.build_conditional_branch(all_ok, valid, invalid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);
        let data_ptr = self.handle_to_ptr(&builder, data, "data_ptr")?;
        let size_ptr = builder.build_int_to_ptr(size, ptr_type, "sz_ptr").or_llvm_err()?;
        let zero_ptr = ptr_type.const_null();
        // newBufferWithBytes:length:options:
        let buf = self.build_objc_msg_send(module, &builder, dev,
            "newBufferWithBytes:length:options:",
            &[data_ptr.into(), size_ptr.into(), zero_ptr.into()], "buf")?;

        let buf_null = builder.build_is_null(buf, "buf_null").or_llvm_err()?;
        let retain_bb = ctx.append_basic_block(func, "retain");
        let null_ret = ctx.append_basic_block(func, "null_ret");
        builder.build_conditional_branch(buf_null, null_ret, retain_bb).or_llvm_err()?;

        builder.position_at_end(null_ret);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(retain_bb);
        let retain_fn = module.get_function("objc_retain").or_missing_fn("objc_retain")?;
        let retained = builder.build_call(retain_fn, &[buf.into()], "retained").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let result = self.ptr_to_handle(&builder, retained, "buf_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_buffer_contents(handle: i64) -> i64
    fn emit_buffer_contents(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_buffer_contents", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let is_zero = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_int(0, false), "is_zero").or_llvm_err()?;
        builder.build_conditional_branch(is_zero, invalid, valid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);
        let buf_ptr = self.handle_to_ptr(&builder, handle, "buf_ptr")?;
        // __bridge cast (no-op) — just send "contents" message
        let contents = self.build_objc_msg_send(module, &builder, buf_ptr, "contents", &[], "contents")?;
        let result = self.ptr_to_handle(&builder, contents, "contents_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_buffer_length(handle: i64) -> i64
    fn emit_buffer_length(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_buffer_length", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let is_zero = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_int(0, false), "is_zero").or_llvm_err()?;
        builder.build_conditional_branch(is_zero, invalid, valid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);
        let buf_ptr = self.handle_to_ptr(&builder, handle, "buf_ptr")?;
        let len = self.build_objc_msg_send_i64(module, &builder, buf_ptr, "length", &[], "len")?;
        builder.build_return(Some(&len)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_free(handle: i64): objc_release(handle_as_ptr)
    fn emit_free(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();

        let fn_type = void_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_free", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let done = ctx.append_basic_block(func, "done");

        builder.position_at_end(entry);
        let handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let is_zero = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_int(0, false), "is_zero").or_llvm_err()?;
        builder.build_conditional_branch(is_zero, done, valid).or_llvm_err()?;

        builder.position_at_end(valid);
        let ptr = self.handle_to_ptr(&builder, handle, "buf_ptr")?;
        let release_fn = module.get_function("objc_release").or_missing_fn("objc_release")?;
        builder.build_call(release_fn, &[ptr.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(done).or_llvm_err()?;

        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // Shader compilation
    // ========================================================================

    /// verum_metal_compile_shader(source: ptr, len: i64) -> i64
    fn emit_compile_shader(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_compile_shader", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let source = func.get_nth_param(0).or_internal("missing param")?.into_pointer_value();
        let len = func.get_nth_param(1).or_internal("missing param")?.into_int_value();

        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let dev_null = builder.build_is_null(dev, "dev_null").or_llvm_err()?;
        let src_null = builder.build_is_null(source, "src_null").or_llvm_err()?;
        let either_null = builder.build_or(dev_null, src_null, "either_null").or_llvm_err()?;
        builder.build_conditional_branch(either_null, invalid, valid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);

        // Create NSString from source bytes:
        // NSString_class = objc_getClass("NSString")
        let get_class_fn = module.get_function("objc_getClass").or_missing_fn("objc_getClass")?;
        let nsstring_name = self.build_global_str(module, &builder, "NSString", "nsstring_cls");
        let nsstring_class = builder.build_call(get_class_fn, &[nsstring_name.into()], "nsstring_class").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();

        // nsstr_alloc = objc_msgSend(NSString_class, "alloc")
        let nsstr_alloc = self.build_objc_msg_send(module, &builder, nsstring_class, "alloc", &[], "nsstr_alloc")?;

        // nsstr = objc_msgSend(nsstr_alloc, "initWithBytes:length:encoding:", source, len, 4 /*NSUTF8StringEncoding*/)
        let len_ptr = builder.build_int_to_ptr(len, ptr_type, "len_ptr").or_llvm_err()?;
        let encoding = builder.build_int_to_ptr(i64_type.const_int(4, false), ptr_type, "enc_ptr").or_llvm_err()?; // NSUTF8StringEncoding = 4
        let nsstr = self.build_objc_msg_send(module, &builder, nsstr_alloc,
            "initWithBytes:length:encoding:", &[source.into(), len_ptr.into(), encoding.into()], "nsstr")?;

        // MTLCompileOptions *options = [MTLCompileOptions new]
        let mtl_opts_name = self.build_global_str(module, &builder, "MTLCompileOptions", "mtl_opts_cls");
        let mtl_opts_class = builder.build_call(get_class_fn, &[mtl_opts_name.into()], "opts_class").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let options = self.build_objc_msg_send(module, &builder, mtl_opts_class, "new", &[], "options")?;

        // lib = objc_msgSend(device, "newLibraryWithSource:options:error:", nsstr, options, null)
        let null_ptr = ptr_type.const_null();
        let lib = self.build_objc_msg_send(module, &builder, dev,
            "newLibraryWithSource:options:error:", &[nsstr.into(), options.into(), null_ptr.into()], "lib")?;

        let lib_null = builder.build_is_null(lib, "lib_null").or_llvm_err()?;
        let retain_bb = ctx.append_basic_block(func, "retain");
        let fail_bb = ctx.append_basic_block(func, "fail");
        builder.build_conditional_branch(lib_null, fail_bb, retain_bb).or_llvm_err()?;

        builder.position_at_end(fail_bb);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(retain_bb);
        let retain_fn = module.get_function("objc_retain").or_missing_fn("objc_retain")?;
        let retained = builder.build_call(retain_fn, &[lib.into()], "retained").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let result = self.ptr_to_handle(&builder, retained, "lib_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_get_pipeline(lib: i64, name: ptr) -> i64
    fn emit_get_pipeline(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into(), ptr_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_get_pipeline", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let lib_handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let kernel_name = func.get_nth_param(1).or_internal("missing param")?.into_pointer_value();

        let dev_global = module.get_global("__verum_metal_device").or_internal("missing global")?;
        let dev = builder.build_load(ptr_type, dev_global.as_pointer_value(), "dev").or_llvm_err()?.into_pointer_value();
        let dev_null = builder.build_is_null(dev, "dev_null").or_llvm_err()?;
        let lib_zero = builder.build_int_compare(IntPredicate::EQ, lib_handle, i64_type.const_int(0, false), "lib_zero").or_llvm_err()?;
        let name_null = builder.build_is_null(kernel_name, "name_null").or_llvm_err()?;
        let bad1 = builder.build_or(dev_null, lib_zero, "bad1").or_llvm_err()?;
        let bad = builder.build_or(bad1, name_null, "bad").or_llvm_err()?;
        builder.build_conditional_branch(bad, invalid, valid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);

        // Create NSString from C string for kernel name
        let get_class_fn = module.get_function("objc_getClass").or_missing_fn("objc_getClass")?;
        let nsstring_name = self.build_global_str(module, &builder, "NSString", "nsstring_cls2");
        let nsstring_class = builder.build_call(get_class_fn, &[nsstring_name.into()], "nsstring_cls").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let nsname = self.build_objc_msg_send(module, &builder, nsstring_class,
            "stringWithUTF8String:", &[kernel_name.into()], "nsname")?;

        // Get function from library: func = objc_msgSend(lib, "newFunctionWithName:", nsname)
        let lib_ptr = self.handle_to_ptr(&builder, lib_handle, "lib_ptr")?;
        let mtl_func = self.build_objc_msg_send(module, &builder, lib_ptr,
            "newFunctionWithName:", &[nsname.into()], "mtl_func")?;

        let func_null = builder.build_is_null(mtl_func, "func_null").or_llvm_err()?;
        let make_pipeline = ctx.append_basic_block(func, "make_pipeline");
        let no_func = ctx.append_basic_block(func, "no_func");
        builder.build_conditional_branch(func_null, no_func, make_pipeline).or_llvm_err()?;

        builder.position_at_end(no_func);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        // pipeline = objc_msgSend(device, "newComputePipelineStateWithFunction:error:", mtl_func, null)
        builder.position_at_end(make_pipeline);
        let null_ptr = ptr_type.const_null();
        let pipeline = self.build_objc_msg_send(module, &builder, dev,
            "newComputePipelineStateWithFunction:error:", &[mtl_func.into(), null_ptr.into()], "pipeline")?;

        let pipe_null = builder.build_is_null(pipeline, "pipe_null").or_llvm_err()?;
        let retain_bb = ctx.append_basic_block(func, "retain");
        let fail_bb = ctx.append_basic_block(func, "fail");
        builder.build_conditional_branch(pipe_null, fail_bb, retain_bb).or_llvm_err()?;

        builder.position_at_end(fail_bb);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(retain_bb);
        let retain_fn = module.get_function("objc_retain").or_missing_fn("objc_retain")?;
        let retained = builder.build_call(retain_fn, &[pipeline.into()], "retained").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let result = self.ptr_to_handle(&builder, retained, "pipe_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // Compute dispatch
    // ========================================================================

    /// Helper: build the common dispatch body — encode pipeline, bind buffers, dispatch, end encoding.
    /// Returns (command_buffer_ptr, encoder_ptr) — caller decides whether to commit+wait or return handle.
    fn build_dispatch_common(
        &self,
        module: &Module<'ctx>,
        builder: &verum_llvm::builder::Builder<'ctx>,
        func: FunctionValue<'ctx>,
        pipeline_handle: verum_llvm::values::IntValue<'ctx>,
        buffers_ptr_i64: verum_llvm::values::IntValue<'ctx>,
        buffer_count: verum_llvm::values::IntValue<'ctx>,
        grid_dims: &[verum_llvm::values::IntValue<'ctx>],    // [grid_x] or [grid_x, grid_y]
        group_dims: &[verum_llvm::values::IntValue<'ctx>],    // [group_x] or [group_x, group_y]
    ) -> Result<(verum_llvm::values::PointerValue<'ctx>, verum_llvm::values::PointerValue<'ctx>)> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());
        let i8_type = ctx.i8_type();

        let pipeline_ptr = self.handle_to_ptr(builder, pipeline_handle, "pipeline_ptr")?;

        // cmdBuf = objc_msgSend(queue, "commandBuffer")
        let queue_global = module.get_global("__verum_metal_queue").or_internal("missing global")?;
        let queue = builder.build_load(ptr_type, queue_global.as_pointer_value(), "queue").or_llvm_err()?.into_pointer_value();
        let cmd_buf = self.build_objc_msg_send(module, builder, queue, "commandBuffer", &[], "cmd_buf")?;

        // encoder = objc_msgSend(cmdBuf, "computeCommandEncoder")
        let encoder = self.build_objc_msg_send(module, builder, cmd_buf, "computeCommandEncoder", &[], "encoder")?;

        // objc_msgSend(encoder, "setComputePipelineState:", pipeline)
        self.build_objc_msg_send(module, builder, encoder, "setComputePipelineState:", &[pipeline_ptr.into()], "set_pso")?;

        // Bind buffers in a loop
        let buffers_ptr = self.handle_to_ptr(builder, buffers_ptr_i64, "bufs_ptr")?;

        let loop_bb = ctx.append_basic_block(func, "bind_loop");
        let body_bb = ctx.append_basic_block(func, "bind_body");
        let after_bb = ctx.append_basic_block(func, "after_bind");

        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(loop_bb);
        let phi_i = builder.build_phi(i64_type, "i").or_llvm_err()?;
        phi_i.add_incoming(&[(&i64_type.const_int(0, false), builder.get_insert_block().or_internal("no insert block")?)]);
        // Fix: we need the predecessor block — but we already branched. Use the block before loop_bb.
        // Actually phi_i already got its incoming from the branch above. Let's re-do:
        // The phi needs incoming from entry (=0) and from body (=i+1).
        let i_val = phi_i.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, i_val, buffer_count, "cmp_i").or_llvm_err()?;
        builder.build_conditional_branch(cmp, body_bb, after_bb).or_llvm_err()?;

        builder.position_at_end(body_bb);
        // Load buffer handle from array: buffer_array[i]
        let elem_ptr = unsafe {
            builder.build_in_bounds_gep(i64_type, buffers_ptr, &[i_val], "elem_ptr").or_llvm_err()?
        };
        let buf_handle = builder.build_load(i64_type, elem_ptr, "buf_handle").or_llvm_err()?.into_int_value();
        let buf_ptr = self.handle_to_ptr(builder, buf_handle, "buf_ptr")?;

        // objc_msgSend(encoder, "setBuffer:offset:atIndex:", buf, 0, i)
        let zero_ptr = ptr_type.const_null();
        let idx_ptr = builder.build_int_to_ptr(i_val, ptr_type, "idx_ptr").or_llvm_err()?;
        self.build_objc_msg_send(module, builder, encoder,
            "setBuffer:offset:atIndex:", &[buf_ptr.into(), zero_ptr.into(), idx_ptr.into()], "set_buf")?;

        let next_i = builder.build_int_add(i_val, i64_type.const_int(1, false), "next_i").or_llvm_err()?;
        phi_i.add_incoming(&[(&next_i, body_bb)]);
        builder.build_unconditional_branch(loop_bb).or_llvm_err()?;

        builder.position_at_end(after_bb);

        // dispatchThreads:threadsPerThreadgroup:
        // MTLSize is {NSUInteger, NSUInteger, NSUInteger} = passed as 3 i64 args each on arm64.
        // For objc_msgSend with struct args by value on arm64, the struct fields are passed
        // in registers. Since MTLSize is 3 x i64, we pass 6 extra args (3 for grid, 3 for group).
        let one = i64_type.const_int(1, false);
        let grid_x_ptr = builder.build_int_to_ptr(grid_dims[0], ptr_type, "gx_ptr").or_llvm_err()?;
        let grid_y_ptr = if grid_dims.len() > 1 {
            builder.build_int_to_ptr(grid_dims[1], ptr_type, "gy_ptr").or_llvm_err()?
        } else {
            builder.build_int_to_ptr(one, ptr_type, "gy_ptr").or_llvm_err()?
        };
        let grid_z_ptr = builder.build_int_to_ptr(one, ptr_type, "gz_ptr").or_llvm_err()?;

        let group_x_ptr = builder.build_int_to_ptr(group_dims[0], ptr_type, "tgx_ptr").or_llvm_err()?;
        let group_y_ptr = if group_dims.len() > 1 {
            builder.build_int_to_ptr(group_dims[1], ptr_type, "tgy_ptr").or_llvm_err()?
        } else {
            builder.build_int_to_ptr(one, ptr_type, "tgy_ptr").or_llvm_err()?
        };
        let group_z_ptr = builder.build_int_to_ptr(one, ptr_type, "tgz_ptr").or_llvm_err()?;

        // On arm64, MTLSize by-value args go into registers as 3 consecutive i64 values.
        // objc_msgSend(encoder, sel, grid.width, grid.height, grid.depth, group.width, group.height, group.depth)
        self.build_objc_msg_send(module, builder, encoder,
            "dispatchThreads:threadsPerThreadgroup:",
            &[grid_x_ptr.into(), grid_y_ptr.into(), grid_z_ptr.into(),
              group_x_ptr.into(), group_y_ptr.into(), group_z_ptr.into()], "dispatch")?;

        // endEncoding
        self.build_objc_msg_send(module, builder, encoder, "endEncoding", &[], "end_enc")?;

        Ok((cmd_buf, encoder))
    }

    /// verum_metal_dispatch_1d(pipeline, buffers_ptr, buffer_count, grid_size, threadgroup_size)
    fn emit_dispatch_1d(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();

        let fn_type = void_type.fn_type(&[
            i64_type.into(), i64_type.into(), i64_type.into(),
            i64_type.into(), i64_type.into(),
        ], false);
        let func = match self.get_or_declare_new(module, "verum_metal_dispatch_1d", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let done = ctx.append_basic_block(func, "done");
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let pipeline_handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let buffers_ptr = func.get_nth_param(1).or_internal("missing param")?.into_int_value();
        let buffer_count = func.get_nth_param(2).or_internal("missing param")?.into_int_value();
        let grid_size = func.get_nth_param(3).or_internal("missing param")?.into_int_value();
        let tg_size = func.get_nth_param(4).or_internal("missing param")?.into_int_value();

        // Validate
        let queue_global = module.get_global("__verum_metal_queue").or_internal("missing global")?;
        let queue = builder.build_load(ptr_type, queue_global.as_pointer_value(), "queue").or_llvm_err()?.into_pointer_value();
        let queue_null = builder.build_is_null(queue, "q_null").or_llvm_err()?;
        let pipe_zero = builder.build_int_compare(IntPredicate::EQ, pipeline_handle, i64_type.const_int(0, false), "pipe_zero").or_llvm_err()?;
        let bad = builder.build_or(queue_null, pipe_zero, "bad").or_llvm_err()?;
        builder.build_conditional_branch(bad, done, valid).or_llvm_err()?;

        builder.position_at_end(valid);

        // Auto-select threadgroup size: if tg_size <= 0, use 256
        let tg_ok = builder.build_int_compare(IntPredicate::SGT, tg_size, i64_type.const_int(0, false), "tg_ok").or_llvm_err()?;
        let default_tg = i64_type.const_int(256, false);
        let actual_tg = builder.build_select(tg_ok, tg_size, default_tg, "actual_tg").or_llvm_err()?.into_int_value();

        let (cmd_buf, _encoder) = self.build_dispatch_common(
            module, &builder, func,
            pipeline_handle, buffers_ptr, buffer_count,
            &[grid_size], &[actual_tg],
        )?;

        // commit + waitUntilCompleted
        self.build_objc_msg_send(module, &builder, cmd_buf, "commit", &[], "commit")?;
        self.build_objc_msg_send(module, &builder, cmd_buf, "waitUntilCompleted", &[], "wait")?;

        builder.build_unconditional_branch(done).or_llvm_err()?;

        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_dispatch_2d(pipeline, buffers_ptr, buffer_count, grid_x, grid_y, group_x, group_y)
    fn emit_dispatch_2d(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = void_type.fn_type(&[
            i64_type.into(), i64_type.into(), i64_type.into(),
            i64_type.into(), i64_type.into(),
            i64_type.into(), i64_type.into(),
        ], false);
        let func = match self.get_or_declare_new(module, "verum_metal_dispatch_2d", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let done = ctx.append_basic_block(func, "done");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let pipeline_handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let buffers_ptr = func.get_nth_param(1).or_internal("missing param")?.into_int_value();
        let buffer_count = func.get_nth_param(2).or_internal("missing param")?.into_int_value();
        let grid_x = func.get_nth_param(3).or_internal("missing param")?.into_int_value();
        let grid_y = func.get_nth_param(4).or_internal("missing param")?.into_int_value();
        let group_x = func.get_nth_param(5).or_internal("missing param")?.into_int_value();
        let group_y = func.get_nth_param(6).or_internal("missing param")?.into_int_value();

        let queue_global = module.get_global("__verum_metal_queue").or_internal("missing global")?;
        let queue = builder.build_load(ptr_type, queue_global.as_pointer_value(), "queue").or_llvm_err()?.into_pointer_value();
        let queue_null = builder.build_is_null(queue, "q_null").or_llvm_err()?;
        let pipe_zero = builder.build_int_compare(IntPredicate::EQ, pipeline_handle, i64_type.const_int(0, false), "pipe_zero").or_llvm_err()?;
        let bad = builder.build_or(queue_null, pipe_zero, "bad").or_llvm_err()?;
        builder.build_conditional_branch(bad, done, valid).or_llvm_err()?;

        builder.position_at_end(valid);

        // Default group sizes if <= 0
        let default_16 = i64_type.const_int(16, false);
        let gx_ok = builder.build_int_compare(IntPredicate::SGT, group_x, i64_type.const_int(0, false), "gx_ok").or_llvm_err()?;
        let actual_gx = builder.build_select(gx_ok, group_x, default_16, "actual_gx").or_llvm_err()?.into_int_value();
        let gy_ok = builder.build_int_compare(IntPredicate::SGT, group_y, i64_type.const_int(0, false), "gy_ok").or_llvm_err()?;
        let actual_gy = builder.build_select(gy_ok, group_y, default_16, "actual_gy").or_llvm_err()?.into_int_value();

        let (cmd_buf, _encoder) = self.build_dispatch_common(
            module, &builder, func,
            pipeline_handle, buffers_ptr, buffer_count,
            &[grid_x, grid_y], &[actual_gx, actual_gy],
        )?;

        self.build_objc_msg_send(module, &builder, cmd_buf, "commit", &[], "commit")?;
        self.build_objc_msg_send(module, &builder, cmd_buf, "waitUntilCompleted", &[], "wait")?;

        builder.build_unconditional_branch(done).or_llvm_err()?;

        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_dispatch_async(pipeline, buffers_ptr, buffer_count, grid_size, tg_size) -> i64
    /// Returns command buffer handle for later wait.
    fn emit_dispatch_async(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[
            i64_type.into(), i64_type.into(), i64_type.into(),
            i64_type.into(), i64_type.into(),
        ], false);
        let func = match self.get_or_declare_new(module, "verum_metal_dispatch_async", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let pipeline_handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let buffers_ptr = func.get_nth_param(1).or_internal("missing param")?.into_int_value();
        let buffer_count = func.get_nth_param(2).or_internal("missing param")?.into_int_value();
        let grid_size = func.get_nth_param(3).or_internal("missing param")?.into_int_value();
        let tg_size = func.get_nth_param(4).or_internal("missing param")?.into_int_value();

        let queue_global = module.get_global("__verum_metal_queue").or_internal("missing global")?;
        let queue = builder.build_load(ptr_type, queue_global.as_pointer_value(), "queue").or_llvm_err()?.into_pointer_value();
        let queue_null = builder.build_is_null(queue, "q_null").or_llvm_err()?;
        let pipe_zero = builder.build_int_compare(IntPredicate::EQ, pipeline_handle, i64_type.const_int(0, false), "pipe_zero").or_llvm_err()?;
        let bad = builder.build_or(queue_null, pipe_zero, "bad").or_llvm_err()?;
        builder.build_conditional_branch(bad, invalid, valid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);

        let tg_ok = builder.build_int_compare(IntPredicate::SGT, tg_size, i64_type.const_int(0, false), "tg_ok").or_llvm_err()?;
        let default_tg = i64_type.const_int(256, false);
        let actual_tg = builder.build_select(tg_ok, tg_size, default_tg, "actual_tg").or_llvm_err()?.into_int_value();

        let (cmd_buf, _encoder) = self.build_dispatch_common(
            module, &builder, func,
            pipeline_handle, buffers_ptr, buffer_count,
            &[grid_size], &[actual_tg],
        )?;

        // commit (but do NOT wait)
        self.build_objc_msg_send(module, &builder, cmd_buf, "commit", &[], "commit")?;

        // __bridge_retained: objc_retain to hand ownership to caller
        let retain_fn = module.get_function("objc_retain").or_missing_fn("objc_retain")?;
        let retained = builder.build_call(retain_fn, &[cmd_buf.into()], "retained").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let result = self.ptr_to_handle(&builder, retained, "cb_i64")?;
        builder.build_return(Some(&result)).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_wait(command_buffer_handle: i64)
    fn emit_wait(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();

        let fn_type = void_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_wait", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let done = ctx.append_basic_block(func, "done");

        builder.position_at_end(entry);
        let handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let is_zero = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_int(0, false), "is_zero").or_llvm_err()?;
        builder.build_conditional_branch(is_zero, done, valid).or_llvm_err()?;

        builder.position_at_end(valid);
        let cb_ptr = self.handle_to_ptr(&builder, handle, "cb_ptr")?;
        // waitUntilCompleted
        self.build_objc_msg_send(module, &builder, cb_ptr, "waitUntilCompleted", &[], "wait")?;
        // __bridge_transfer: release the retained reference
        let release_fn = module.get_function("objc_release").or_missing_fn("objc_release")?;
        builder.build_call(release_fn, &[cb_ptr.into()], "").or_llvm_err()?;
        builder.build_unconditional_branch(done).or_llvm_err()?;

        builder.position_at_end(done);
        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_execution_time_ns(command_buffer_handle: i64) -> i64
    fn emit_execution_time_ns(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let f64_type = ctx.f64_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = i64_type.fn_type(&[i64_type.into()], false);
        let func = match self.get_or_declare_new(module, "verum_metal_execution_time_ns", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let valid = ctx.append_basic_block(func, "valid");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let handle = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let is_zero = builder.build_int_compare(IntPredicate::EQ, handle, i64_type.const_int(0, false), "is_zero").or_llvm_err()?;
        builder.build_conditional_branch(is_zero, invalid, valid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        builder.position_at_end(valid);
        let cb_ptr = self.handle_to_ptr(&builder, handle, "cb_ptr")?;

        // GPUEndTime and GPUStartTime are CFTimeInterval (double) properties.
        // On arm64, objc_msgSend returns double in d0 for floating-point properties.
        // We need a special variant: declare objc_msgSend_fpret or cast.
        // Actually on arm64, objc_msgSend handles floating-point returns correctly
        // (unlike i386 which needed objc_msgSend_fpret).
        // However, our objc_msgSend is declared as returning ptr.
        // We need to bitcast the return. The ptr (i64) bits will contain the double bits
        // on arm64 since d0 and x0 are separate register files.
        //
        // For correctness, we should declare a separate variant that returns double.
        // Let's declare objc_msgSend_double: (ptr, ptr, ...) -> double
        let objc_msg_send_fp = {
            let ft = f64_type.fn_type(&[ptr_type.into(), ptr_type.into()], true);
            self.get_or_declare(module, "objc_msgSend_fp64", ft)
        };

        // Get selectors for GPUEndTime and GPUStartTime
        let sel_fn = module.get_function("sel_registerName").or_missing_fn("sel_registerName")?;
        let end_sel_str = self.build_global_str(module, &builder, "GPUEndTime", "sel_gpu_end");
        let end_sel = builder.build_call(sel_fn, &[end_sel_str.into()], "end_sel").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();
        let start_sel_str = self.build_global_str(module, &builder, "GPUStartTime", "sel_gpu_start");
        let start_sel = builder.build_call(sel_fn, &[start_sel_str.into()], "start_sel").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_pointer_value();

        // Call objc_msgSend_fp64(cb, endTimeSel) -> double
        let end_time = builder.build_call(objc_msg_send_fp, &[cb_ptr.into(), end_sel.into()], "end_time").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();
        let start_time = builder.build_call(objc_msg_send_fp, &[cb_ptr.into(), start_sel.into()], "start_time").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_float_value();

        // gpu_time = end_time - start_time (seconds as double)
        let gpu_time = builder.build_float_sub(end_time, start_time, "gpu_time").or_llvm_err()?;
        // Convert to nanoseconds: * 1e9
        let ns_factor = f64_type.const_float(1e9);
        let ns_float = builder.build_float_mul(gpu_time, ns_factor, "ns_float").or_llvm_err()?;
        let ns_i64 = builder.build_float_to_signed_int(ns_float, i64_type, "ns_i64").or_llvm_err()?;
        builder.build_return(Some(&ns_i64)).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // High-level operations
    // ========================================================================

    /// verum_metal_vector_add_f32(a: i64, b: i64, result: i64, n: i64)
    fn emit_vector_add_f32(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = void_type.fn_type(&[
            i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(),
        ], false);
        let func = match self.get_or_declare_new(module, "verum_metal_vector_add_f32", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        builder.position_at_end(entry);

        let a = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let b = func.get_nth_param(1).or_internal("missing param")?.into_int_value();
        let result = func.get_nth_param(2).or_internal("missing param")?.into_int_value();
        let n = func.get_nth_param(3).or_internal("missing param")?.into_int_value();

        // Embed the vector_add shader source as a global string
        let shader_src = concat!(
            "#include <metal_stdlib>\n",
            "using namespace metal;\n",
            "kernel void vector_add(\n",
            "    device const float* a [[buffer(0)]],\n",
            "    device const float* b [[buffer(1)]],\n",
            "    device float* result  [[buffer(2)]],\n",
            "    uint id              [[thread_position_in_grid]])\n",
            "{\n",
            "    result[id] = a[id] + b[id];\n",
            "}\n",
        );

        let src_ptr = self.build_global_str(module, &builder, shader_src, "vec_add_src");
        let src_len = i64_type.const_int(shader_src.len() as u64, false);

        // compile_shader(src, len)
        let compile_fn = self.get_or_declare(module, "verum_metal_compile_shader",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false));
        let lib = builder.build_call(compile_fn, &[src_ptr.into(), src_len.into()], "lib").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // get_pipeline(lib, "vector_add")
        let kernel_name = self.build_global_str(module, &builder, "vector_add", "vec_add_name");
        let get_pipe_fn = self.get_or_declare(module, "verum_metal_get_pipeline",
            i64_type.fn_type(&[i64_type.into(), ptr_type.into()], false));
        let pipeline = builder.build_call(get_pipe_fn, &[lib.into(), kernel_name.into()], "pipeline").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // Build buffer array on stack: [a, b, result]
        let arr_alloca = builder.build_array_alloca(i64_type, i64_type.const_int(3, false), "bufs").or_llvm_err()?;
        let idx0 = unsafe { builder.build_in_bounds_gep(i64_type, arr_alloca, &[i64_type.const_int(0, false)], "p0").or_llvm_err()? };
        builder.build_store(idx0, a).or_llvm_err()?;
        let idx1 = unsafe { builder.build_in_bounds_gep(i64_type, arr_alloca, &[i64_type.const_int(1, false)], "p1").or_llvm_err()? };
        builder.build_store(idx1, b).or_llvm_err()?;
        let idx2 = unsafe { builder.build_in_bounds_gep(i64_type, arr_alloca, &[i64_type.const_int(2, false)], "p2").or_llvm_err()? };
        builder.build_store(idx2, result).or_llvm_err()?;

        let bufs_i64 = self.ptr_to_handle(&builder, arr_alloca, "bufs_i64")?;

        // dispatch_1d(pipeline, bufs, 3, n, 0)
        let dispatch_fn = self.get_or_declare(module, "verum_metal_dispatch_1d",
            ctx.void_type().fn_type(&[
                i64_type.into(), i64_type.into(), i64_type.into(),
                i64_type.into(), i64_type.into(),
            ], false));
        builder.build_call(dispatch_fn, &[
            pipeline.into(), bufs_i64.into(), i64_type.const_int(3, false).into(),
            n.into(), i64_type.const_int(0, false).into(),
        ], "").or_llvm_err()?;

        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_sgemm(A: i64, B: i64, C: i64, M: i64, N: i64, K: i64)
    fn emit_sgemm(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        let fn_type = void_type.fn_type(&[
            i64_type.into(), i64_type.into(), i64_type.into(),
            i64_type.into(), i64_type.into(), i64_type.into(),
        ], false);
        let func = match self.get_or_declare_new(module, "verum_metal_sgemm", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        builder.position_at_end(entry);

        let a_buf = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let b_buf = func.get_nth_param(1).or_internal("missing param")?.into_int_value();
        let c_buf = func.get_nth_param(2).or_internal("missing param")?.into_int_value();
        let m_val = func.get_nth_param(3).or_internal("missing param")?.into_int_value();
        let n_val = func.get_nth_param(4).or_internal("missing param")?.into_int_value();
        let k_val = func.get_nth_param(5).or_internal("missing param")?.into_int_value();

        let shader_src = concat!(
            "#include <metal_stdlib>\n",
            "using namespace metal;\n",
            "kernel void sgemm(\n",
            "    device const float* A [[buffer(0)]],\n",
            "    device const float* B [[buffer(1)]],\n",
            "    device float* C       [[buffer(2)]],\n",
            "    constant uint& M      [[buffer(3)]],\n",
            "    constant uint& N      [[buffer(4)]],\n",
            "    constant uint& K      [[buffer(5)]],\n",
            "    uint2 gid             [[thread_position_in_grid]])\n",
            "{\n",
            "    uint row = gid.y;\n",
            "    uint col = gid.x;\n",
            "    if (row >= M || col >= N) return;\n",
            "    float sum = 0.0f;\n",
            "    for (uint k = 0; k < K; k++) {\n",
            "        sum += A[row * K + k] * B[k * N + col];\n",
            "    }\n",
            "    C[row * N + col] = sum;\n",
            "}\n",
        );

        let src_ptr = self.build_global_str(module, &builder, shader_src, "sgemm_src");
        let src_len = i64_type.const_int(shader_src.len() as u64, false);

        let compile_fn = self.get_or_declare(module, "verum_metal_compile_shader",
            i64_type.fn_type(&[ptr_type.into(), i64_type.into()], false));
        let lib = builder.build_call(compile_fn, &[src_ptr.into(), src_len.into()], "lib").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        let kernel_name = self.build_global_str(module, &builder, "sgemm", "sgemm_name");
        let get_pipe_fn = self.get_or_declare(module, "verum_metal_get_pipeline",
            i64_type.fn_type(&[i64_type.into(), ptr_type.into()], false));
        let pipeline = builder.build_call(get_pipe_fn, &[lib.into(), kernel_name.into()], "pipeline").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // Create constant buffers for M, N, K (4 bytes each, uint32)
        let i32_type = ctx.i32_type();
        let alloc_fn = self.get_or_declare(module, "verum_metal_alloc_with_data",
            i64_type.fn_type(&[i64_type.into(), i64_type.into()], false));

        // Allocate stack space for uint32 values
        let m_alloca = builder.build_alloca(i32_type, "m_val").or_llvm_err()?;
        let m_trunc = builder.build_int_truncate(m_val, i32_type, "m32").or_llvm_err()?;
        builder.build_store(m_alloca, m_trunc).or_llvm_err()?;
        let m_ptr_i64 = self.ptr_to_handle(&builder, m_alloca, "m_ptr_i64")?;
        let four = i64_type.const_int(4, false);
        let m_buf_h = builder.build_call(alloc_fn, &[m_ptr_i64.into(), four.into()], "m_buf").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        let n_alloca = builder.build_alloca(i32_type, "n_val_alloc").or_llvm_err()?;
        let n_trunc = builder.build_int_truncate(n_val, i32_type, "n32").or_llvm_err()?;
        builder.build_store(n_alloca, n_trunc).or_llvm_err()?;
        let n_ptr_i64 = self.ptr_to_handle(&builder, n_alloca, "n_ptr_i64")?;
        let n_buf_h = builder.build_call(alloc_fn, &[n_ptr_i64.into(), four.into()], "n_buf").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        let k_alloca = builder.build_alloca(i32_type, "k_val_alloc").or_llvm_err()?;
        let k_trunc = builder.build_int_truncate(k_val, i32_type, "k32").or_llvm_err()?;
        builder.build_store(k_alloca, k_trunc).or_llvm_err()?;
        let k_ptr_i64 = self.ptr_to_handle(&builder, k_alloca, "k_ptr_i64")?;
        let k_buf_h = builder.build_call(alloc_fn, &[k_ptr_i64.into(), four.into()], "k_buf").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // Build buffer array: [A, B, C, m_buf, n_buf, k_buf]
        let arr = builder.build_array_alloca(i64_type, i64_type.const_int(6, false), "bufs").or_llvm_err()?;
        for (idx, val) in [a_buf, b_buf, c_buf, m_buf_h, n_buf_h, k_buf_h].iter().enumerate() {
            let p = unsafe { builder.build_in_bounds_gep(i64_type, arr, &[i64_type.const_int(idx as u64, false)], &format!("p{}", idx)).or_llvm_err()? };
            builder.build_store(p, *val).or_llvm_err()?;
        }
        let bufs_i64 = self.ptr_to_handle(&builder, arr, "bufs_i64")?;

        // dispatch_2d(pipeline, bufs, 6, N, M, 16, 16)
        let dispatch_fn = self.get_or_declare(module, "verum_metal_dispatch_2d",
            ctx.void_type().fn_type(&[
                i64_type.into(), i64_type.into(), i64_type.into(),
                i64_type.into(), i64_type.into(),
                i64_type.into(), i64_type.into(),
            ], false));
        builder.build_call(dispatch_fn, &[
            pipeline.into(), bufs_i64.into(), i64_type.const_int(6, false).into(),
            n_val.into(), m_val.into(),
            i64_type.const_int(16, false).into(), i64_type.const_int(16, false).into(),
        ], "").or_llvm_err()?;

        builder.build_return(None).or_llvm_err()?;
        Ok(())
    }

    /// verum_metal_benchmark(pipeline, buffers_ptr, buffer_count, grid_size, tg_size, iterations) -> i64
    fn emit_benchmark(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();

        let fn_type = i64_type.fn_type(&[
            i64_type.into(), i64_type.into(), i64_type.into(),
            i64_type.into(), i64_type.into(), i64_type.into(),
        ], false);
        let func = match self.get_or_declare_new(module, "verum_metal_benchmark", fn_type) {
            Some(f) => f,
            None => return Ok(()),
        };

        let builder = ctx.create_builder();
        let entry = ctx.append_basic_block(func, "entry");
        let warmup = ctx.append_basic_block(func, "warmup");
        let loop_head = ctx.append_basic_block(func, "loop_head");
        let loop_body = ctx.append_basic_block(func, "loop_body");
        let loop_done = ctx.append_basic_block(func, "loop_done");
        let invalid = ctx.append_basic_block(func, "invalid");

        builder.position_at_end(entry);
        let init_fn = self.get_or_declare(module, "verum_metal_ensure_init",
            ctx.void_type().fn_type(&[], false));
        builder.build_call(init_fn, &[], "").or_llvm_err()?;

        let pipeline = func.get_nth_param(0).or_internal("missing param")?.into_int_value();
        let buffers_ptr = func.get_nth_param(1).or_internal("missing param")?.into_int_value();
        let buffer_count = func.get_nth_param(2).or_internal("missing param")?.into_int_value();
        let grid_size = func.get_nth_param(3).or_internal("missing param")?.into_int_value();
        let tg_size = func.get_nth_param(4).or_internal("missing param")?.into_int_value();
        let iterations = func.get_nth_param(5).or_internal("missing param")?.into_int_value();

        let iters_ok = builder.build_int_compare(IntPredicate::SGT, iterations, i64_type.const_int(0, false), "iters_ok").or_llvm_err()?;
        let pipe_ok = builder.build_int_compare(IntPredicate::NE, pipeline, i64_type.const_int(0, false), "pipe_ok").or_llvm_err()?;
        let ok = builder.build_and(iters_ok, pipe_ok, "ok").or_llvm_err()?;
        builder.build_conditional_branch(ok, warmup, invalid).or_llvm_err()?;

        builder.position_at_end(invalid);
        builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;

        // Warmup: one dispatch
        builder.position_at_end(warmup);
        let dispatch_fn = self.get_or_declare(module, "verum_metal_dispatch_1d",
            ctx.void_type().fn_type(&[
                i64_type.into(), i64_type.into(), i64_type.into(),
                i64_type.into(), i64_type.into(),
            ], false));
        builder.build_call(dispatch_fn, &[
            pipeline.into(), buffers_ptr.into(), buffer_count.into(),
            grid_size.into(), tg_size.into(),
        ], "").or_llvm_err()?;
        builder.build_unconditional_branch(loop_head).or_llvm_err()?;

        // Loop: dispatch_async + wait + accumulate time
        builder.position_at_end(loop_head);
        let phi_iter = builder.build_phi(i64_type, "iter").or_llvm_err()?;
        phi_iter.add_incoming(&[(&i64_type.const_int(0, false), warmup)]);
        let phi_total = builder.build_phi(i64_type, "total_ns").or_llvm_err()?;
        phi_total.add_incoming(&[(&i64_type.const_int(0, false), warmup)]);
        let iter_val = phi_iter.as_basic_value().into_int_value();
        let total_val = phi_total.as_basic_value().into_int_value();
        let cmp = builder.build_int_compare(IntPredicate::ULT, iter_val, iterations, "cmp").or_llvm_err()?;
        builder.build_conditional_branch(cmp, loop_body, loop_done).or_llvm_err()?;

        builder.position_at_end(loop_body);
        // dispatch_async
        let async_fn = self.get_or_declare(module, "verum_metal_dispatch_async",
            i64_type.fn_type(&[
                i64_type.into(), i64_type.into(), i64_type.into(),
                i64_type.into(), i64_type.into(),
            ], false));
        let cb_handle = builder.build_call(async_fn, &[
            pipeline.into(), buffers_ptr.into(), buffer_count.into(),
            grid_size.into(), tg_size.into(),
        ], "cb").or_llvm_err()?.try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();

        // wait
        let wait_fn = self.get_or_declare(module, "verum_metal_wait",
            ctx.void_type().fn_type(&[i64_type.into()], false));
        builder.build_call(wait_fn, &[cb_handle.into()], "").or_llvm_err()?;

        // execution_time_ns
        let time_fn = self.get_or_declare(module, "verum_metal_execution_time_ns",
            i64_type.fn_type(&[i64_type.into()], false));
        let ns = builder.build_call(time_fn, &[cb_handle.into()], "ns").or_llvm_err()?
            .try_as_basic_value().basic().or_internal("expected basic value")?.into_int_value();
        let new_total = builder.build_int_add(total_val, ns, "new_total").or_llvm_err()?;
        let next_iter = builder.build_int_add(iter_val, i64_type.const_int(1, false), "next_iter").or_llvm_err()?;
        phi_iter.add_incoming(&[(&next_iter, loop_body)]);
        phi_total.add_incoming(&[(&new_total, loop_body)]);
        builder.build_unconditional_branch(loop_head).or_llvm_err()?;

        // Return average
        builder.position_at_end(loop_done);
        let avg = builder.build_int_signed_div(total_val, iterations, "avg").or_llvm_err()?;
        builder.build_return(Some(&avg)).or_llvm_err()?;
        Ok(())
    }

    // ========================================================================
    // Stubs for non-macOS platforms
    // ========================================================================

    /// On non-macOS, emit stub functions that return 0 / do nothing.
    fn emit_stubs(&self, module: &Module<'ctx>) -> Result<()> {
        let ctx = self.context;
        let i64_type = ctx.i64_type();
        let void_type = ctx.void_type();
        let ptr_type = ctx.ptr_type(AddressSpace::default());

        // Functions returning i64 (return 0)
        let i64_ret_stubs: &[(&str, &[verum_llvm::types::BasicMetadataTypeEnum<'ctx>])] = &[
            ("verum_metal_get_device", &[]),
            ("verum_metal_device_name", &[]),
            ("verum_metal_max_memory", &[]),
            ("verum_metal_max_threads_per_threadgroup", &[]),
            ("verum_metal_supports_family", &[i64_type.into()]),
            ("verum_metal_gpu_core_count", &[]),
            ("verum_metal_alloc", &[i64_type.into()]),
            ("verum_metal_alloc_with_data", &[i64_type.into(), i64_type.into()]),
            ("verum_metal_buffer_contents", &[i64_type.into()]),
            ("verum_metal_buffer_length", &[i64_type.into()]),
            ("verum_metal_compile_shader", &[ptr_type.into(), i64_type.into()]),
            ("verum_metal_get_pipeline", &[i64_type.into(), ptr_type.into()]),
            ("verum_metal_dispatch_async", &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()]),
            ("verum_metal_execution_time_ns", &[i64_type.into()]),
            ("verum_metal_benchmark", &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()]),
        ];

        for (name, params) in i64_ret_stubs {
            let fn_type = i64_type.fn_type(params, false);
            if let Some(func) = self.get_or_declare_new(module, name, fn_type) {
                let builder = ctx.create_builder();
                let entry = ctx.append_basic_block(func, "entry");
                builder.position_at_end(entry);
                builder.build_return(Some(&i64_type.const_int(0, false))).or_llvm_err()?;
            }
        }

        // Functions returning void (no-op)
        let void_ret_stubs: &[(&str, &[verum_llvm::types::BasicMetadataTypeEnum<'ctx>])] = &[
            ("verum_metal_free", &[i64_type.into()]),
            ("verum_metal_dispatch_1d", &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()]),
            ("verum_metal_dispatch_2d", &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()]),
            ("verum_metal_wait", &[i64_type.into()]),
            ("verum_metal_vector_add_f32", &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()]),
            ("verum_metal_sgemm", &[i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into(), i64_type.into()]),
        ];

        for (name, params) in void_ret_stubs {
            let fn_type = void_type.fn_type(params, false);
            if let Some(func) = self.get_or_declare_new(module, name, fn_type) {
                let builder = ctx.create_builder();
                let entry = ctx.append_basic_block(func, "entry");
                builder.position_at_end(entry);
                builder.build_return(None).or_llvm_err()?;
            }
        }

        // Also emit ensure_init as a no-op
        let fn_type = void_type.fn_type(&[], false);
        if let Some(func) = self.get_or_declare_new(module, "verum_metal_ensure_init", fn_type) {
            let builder = ctx.create_builder();
            let entry = ctx.append_basic_block(func, "entry");
            builder.position_at_end(entry);
            builder.build_return(None).or_llvm_err()?;
        }
        Ok(())
    }
}
