//! Native atomic transitions share the RetainCopy ownership contract.
use std::sync::atomic::{AtomicU32, Ordering};
use verum_codegen::llvm::retained_counter;
use verum_llvm::{OptimizationLevel, context::Context};

type Transition = unsafe extern "C" fn(*mut u32) -> bool;

#[test]
fn native_retain_and_local_rollback_are_nonwrapping_and_composable() {
    let context = Context::create();
    let module = context.create_module("retained-counter");
    let cache = retained_counter::RetainedCounters::new(&module);
    let (retain, release) = cache.get().unwrap();
    assert_eq!(cache.get().unwrap(), (retain, release));
    // A host ABI wrapper makes the actual internal IR callable by this test;
    // it is not a Verum implementation interlayer.
    for (name, callee) in [("retain", retain), ("rollback", release)] {
        let f = module.add_function(name, callee.get_type(), None);
        let b = context.create_builder();
        b.position_at_end(context.append_basic_block(f, "entry"));
        let result = b
            .build_call(callee, &[f.get_nth_param(0).unwrap().into()], "result")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap();
        b.build_return(Some(&result)).unwrap();
    }
    module.verify().unwrap();
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Default)
        .unwrap();
    // SAFETY: the wrappers expose the exact pointer/i1 ABI declared above.
    let acquire = unsafe { engine.get_function::<Transition>("retain").unwrap() };
    let rollback = unsafe { engine.get_function::<Transition>("rollback").unwrap() };
    for initial in [0, u32::MAX] {
        let count = AtomicU32::new(initial);
        // SAFETY: live aligned UInt32 storage, no non-atomic counter access.
        assert!(!unsafe { acquire.call(count.as_ptr()) });
        assert_eq!(count.load(Ordering::Relaxed), initial);
    }
    for initial in [0, 1] {
        let count = AtomicU32::new(initial);
        // SAFETY: live atomic counter; invalid rollback must be refused.
        assert!(!unsafe { rollback.call(count.as_ptr()) });
        assert_eq!(count.load(Ordering::Relaxed), initial);
    }
    let count = AtomicU32::new(u32::MAX - 1);
    // SAFETY: the original borrowed ownership remains alive for this sequence.
    unsafe {
        assert!(acquire.call(count.as_ptr()));
        assert!(!acquire.call(count.as_ptr()));
        assert!(rollback.call(count.as_ptr()));
    }
    assert_eq!(count.load(Ordering::Relaxed), u32::MAX - 1);
    let count = AtomicU32::new(1);
    // SAFETY: two retained owners, then only the first transaction rolls back.
    unsafe {
        assert!(acquire.call(count.as_ptr()));
        assert!(acquire.call(count.as_ptr()));
        assert!(rollback.call(count.as_ptr()));
    }
    assert_eq!(count.load(Ordering::Relaxed), 2);
}

#[test]
fn foreign_declarations_and_complete_internal_homonyms_are_refused() {
    for name in ["verum_retain_u32_checked", "verum_release_local_retain_u32"] {
        for internal_body in [false, true] {
            let context = Context::create();
            let module = context.create_module("foreign-retained-counter");
            let pointer = context.ptr_type(Default::default());
            let signature = context.bool_type().fn_type(&[pointer.into()], false);
            let function = module.add_function(name, signature, None);
            if internal_body {
                function.set_linkage(verum_llvm::module::Linkage::Internal);
                let builder = context.create_builder();
                builder.position_at_end(context.append_basic_block(function, "foreign"));
                builder
                    .build_return(Some(&context.bool_type().const_int(1, false)))
                    .unwrap();
            }
            let cache = retained_counter::RetainedCounters::new(&module);
            let error = cache.get().unwrap_err();
            assert!(error.to_string().contains("foreign declaration"), "{error}");
        }
    }
}

#[test]
fn independent_modules_have_independently_owned_counter_bodies() {
    let context = Context::create();
    let first = context.create_module("first");
    let second = context.create_module("second");
    let first_cache = retained_counter::RetainedCounters::new(&first);
    let second_cache = retained_counter::RetainedCounters::new(&second);
    let own = first_cache.get().unwrap();
    assert_eq!(first_cache.get().unwrap(), own);
    let other = second_cache.get().unwrap();
    assert_ne!(own, other);
    assert_eq!(second_cache.get().unwrap(), other);
    first.verify().unwrap();
    second.verify().unwrap();
}
