//! Static initializers are ENTRY-sequenced, never dyld-sequenced (T0837).
//!
//! A VBC `global_ctors` entry used to be registered in
//! `llvm.global_ctors`, which dyld runs at image load — before
//! `verum_runtime_init` and the CBGR/TLS bootstrap. An initializer that
//! allocates (`Shared.new` in a `@thread_local static` init) then
//! crashed inside `dyld4::Loader::findAndRunAllInitializers` with
//! EXC_BAD_ACCESS (the darwin L0 autopsy that named this defect).
//!
//! The contract pinned here is the fix: the emitted `main` calls
//! `__verum_static_init` AFTER `verum_runtime_init` and
//! `__verum_static_fini` BEFORE `verum_runtime_cleanup`, and the module
//! carries NO `llvm.global_ctors` / `llvm.global_dtors` arrays at all —
//! matching the interpreter twin, whose `run_main` runs `global_ctors`
//! as its first act with the runtime fully constructed.

use verum_codegen::llvm::{LoweringConfig, VbcToLlvmLowering};
use verum_llvm::context::Context;
use verum_vbc::instruction::{Instruction, Reg};
use verum_vbc::module::{FunctionDescriptor, VbcModule};

/// Build a module shaped like a program with one static initializer and
/// one finalizer: `main` returns 0; `static_seed` / `static_drop` are
/// trivial zero-arg bodies registered in `global_ctors` / `global_dtors`.
fn module_with_static_lifecycle() -> VbcModule {
    let mut module = VbcModule::new("static_init_pin".to_string());

    let main_id = module.intern_string("main");
    let mut main_desc = FunctionDescriptor::new(main_id);
    main_desc.register_count = 2;
    main_desc.return_type = verum_vbc::types::TypeRef::concrete(verum_vbc::types::TypeId::INT);
    main_desc.instructions = Some(vec![
        Instruction::LoadI {
            dst: Reg(0),
            value: 0,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    module.add_function(main_desc);

    let ctor_name = module.intern_string("static_seed");
    let mut ctor_desc = FunctionDescriptor::new(ctor_name);
    ctor_desc.register_count = 2;
    ctor_desc.instructions = Some(vec![Instruction::RetV]);
    let ctor_id = module.add_function(ctor_desc);
    module.global_ctors.push((ctor_id, 65535));

    let dtor_name = module.intern_string("static_drop");
    let mut dtor_desc = FunctionDescriptor::new(dtor_name);
    dtor_desc.register_count = 2;
    dtor_desc.instructions = Some(vec![Instruction::RetV]);
    let dtor_id = module.add_function(dtor_desc);
    module.global_dtors.push((dtor_id, 65535));

    module
}

fn lower(module: &VbcModule) -> String {
    let context = Context::create();
    let config = LoweringConfig::debug("static_init_pin");
    let mut lowering = VbcToLlvmLowering::new(&context, config);
    lowering
        .lower_module(module)
        .expect("a module with static ctors/dtors must lower");
    lowering.get_ir().to_string()
}

/// The emitted `main`'s body, `define`-line through closing brace.
fn body_of_main(ir: &str) -> &str {
    let start = ir
        .find("define i32 @main(")
        .expect("platform entry `main` must be emitted");
    let rest = &ir[start..];
    let end = rest.find("\n}").expect("main body must close");
    &rest[..end]
}

#[test]
fn static_lifecycle_rides_the_entry_not_the_image_loader() {
    let ir = lower(&module_with_static_lifecycle());

    // The dyld-time arrays must not exist AT ALL — their presence is the
    // defect regardless of what they contain.
    assert!(
        !ir.contains("llvm.global_ctors") && !ir.contains("llvm.global_dtors"),
        "static init/fini must not ride llvm.global_ctors/dtors: dyld runs \
         those before verum_runtime_init, and an allocating initializer \
         crashes in findAndRunAllInitializers (T0837)"
    );

    // The wrappers exist and main drives them in runtime order:
    // runtime_init → static_init → verum_main → static_fini → cleanup.
    let main_body = body_of_main(&ir);
    let idx = |needle: &str| {
        main_body.find(needle).unwrap_or_else(|| {
            panic!("main must call {needle}; body:\n{main_body}")
        })
    };
    let init = idx("call void @verum_runtime_init");
    let static_init = idx("call void @__verum_static_init");
    let user_main = idx("@verum_main");
    let static_fini = idx("call void @__verum_static_fini");
    let cleanup = idx("call void @verum_runtime_cleanup");
    assert!(
        init < static_init && static_init < user_main,
        "static initializers must run AFTER the runtime is up and BEFORE \
         user main (init={init}, static_init={static_init}, main={user_main})"
    );
    assert!(
        user_main < static_fini && static_fini < cleanup,
        "static finalizers must run after user main and BEFORE runtime \
         teardown (main={user_main}, fini={static_fini}, cleanup={cleanup})"
    );
}

#[test]
fn a_module_without_statics_carries_no_lifecycle_wrappers() {
    let mut module = VbcModule::new("static_init_pin".to_string());
    let main_id = module.intern_string("main");
    let mut main_desc = FunctionDescriptor::new(main_id);
    main_desc.register_count = 2;
    main_desc.return_type = verum_vbc::types::TypeRef::concrete(verum_vbc::types::TypeId::INT);
    main_desc.instructions = Some(vec![
        Instruction::LoadI {
            dst: Reg(0),
            value: 0,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    module.add_function(main_desc);

    let ir = lower(&module);
    assert!(
        !ir.contains("__verum_static_init") && !ir.contains("__verum_static_fini"),
        "a program with no static ctors/dtors must not carry the wrappers — \
         the entry calls are conditional on their existence"
    );
}

#[test]
fn monomorphized_lifecycle_keeps_its_mapped_native_entry_calls() {
    use verum_vbc::module::FunctionId;
    use verum_vbc::mono::{ModuleMerger, MonomorphizationResolver};

    let mut source = module_with_static_lifecycle();
    // Table position is not the input FunctionId. A raw roster copy could
    // otherwise pass while calling a different function after compaction.
    for (function, id) in source.functions.iter_mut().zip([91, 7, 900]) {
        function.id = FunctionId(id);
    }
    source.global_ctors[0].0 = FunctionId(7);
    source.global_dtors[0].0 = FunctionId(900);
    let (module, _) = ModuleMerger::new(source, None, vec![], MonomorphizationResolver::new())
        .merge()
        .expect("lifecycle identities must survive mono");
    assert_eq!(module.global_ctors, vec![(FunctionId(1), 65535)]);
    assert_eq!(module.global_dtors, vec![(FunctionId(2), 65535)]);

    let ir = lower(&module);
    let main = body_of_main(&ir);
    let init = main.find("call void @__verum_static_init").unwrap();
    let entry = main.find("@verum_main").unwrap();
    let fini = main.find("call void @__verum_static_fini").unwrap();
    assert!(
        init < entry && entry < fini,
        "post-mono entry order: {main}"
    );
    // Linkage is internalized during lowering; select a definition by
    // its exact symbol rather than requiring a particular linkage spelling.
    let body = |symbol: &str| {
        let header = ir.lines().find(|line| {
            line.starts_with("define ") && line.contains(&format!("@{symbol}("))
        }).unwrap_or_else(|| panic!("missing lifecycle definition: {symbol}"));
        let start = ir.find(header).unwrap();
        ir[start..].split("\n}").next().unwrap()
    };
    let init_body = body("__verum_static_init");
    let fini_body = body("__verum_static_fini");
    assert!(
        init_body.contains("@static_seed("),
        "ctor callee: {init_body}"
    );
    assert!(
        fini_body.contains("@static_drop("),
        "dtor callee: {fini_body}"
    );
}
