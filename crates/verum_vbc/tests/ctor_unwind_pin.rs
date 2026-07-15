//! CTOR-UNWIND / entry-depth pins (SYNC-SYSCALL-CTORS-REGRESSION-1, task #20).
//!
//! Root defect pinned here: a `__tls_init_*` global ctor whose body hits an
//! unresolved XMOD-band `Call` (`FunctionNotFound(0x2000_00xx)`) is lenient-
//! skipped by `run_global_ctors`, but pre-fix the skip left the crashed
//! ctor's frames on the call stack.  Every subsequent top-level execution on
//! the same interpreter (the next ctor, `main`, every `verum test` body)
//! then `Ret`-popped INTO the dead frame, resumed it at the failing
//! instruction, and re-raised the ctor's `FunctionNotFound` as its own
//! error — 9 `core-tests/runtime/{sync,syscall,supervisor}` tests red with
//! an id their bodies never called.
//!
//! Two independent defenses are pinned:
//!  1. `run_global_ctors` unwinds the crashed ctor's frames + registers
//!     back to the pre-ctor snapshot (state hygiene at the failure site).
//!  2. `execute_table_with_args` runs each top-level execution with its own
//!     entry depth, so even if stale frames exist it terminates at ITS
//!     entry frame instead of falling through into them.

use std::sync::Arc;
use verum_vbc::bytecode;
use verum_vbc::instruction::{Instruction, Reg, RegRange};
use verum_vbc::interpreter::Interpreter;
use verum_vbc::module::{
    FunctionDescriptor, FunctionId, VbcModule, XMOD_CALL_ID_BAND_BASE,
};
use verum_vbc::value::Value;

fn encode(instructions: &[Instruction]) -> Vec<u8> {
    let mut bc = Vec::new();
    for instr in instructions {
        bytecode::encode_instruction(instr, &mut bc);
    }
    bc
}

/// Builds a module with:
///  * fn 0 `main` — `LoadSmallI 7; Ret` (the healthy user body),
///  * fn 1 `__tls_init_PINNED` — `Call <unresolved XMOD-band id>; Ret`
///    (the crashing ctor), registered in `global_ctors`.
fn build_module() -> Arc<VbcModule> {
    let mut module = VbcModule::new("ctor_unwind_pin".to_string());

    let main_body = encode(&[
        Instruction::LoadSmallI {
            dst: Reg(0),
            value: 7,
        },
        Instruction::Ret { value: Reg(0) },
    ]);
    // The frozen band id: no descriptor exists for it in this module, so
    // the interpreter raises FunctionNotFound — the exact runtime shape a
    // remap-missed XMOD callee produces.
    let ctor_body = encode(&[
        Instruction::Call {
            dst: Reg(0),
            func_id: XMOD_CALL_ID_BAND_BASE + 6,
            args: RegRange {
                start: Reg(0),
                count: 0,
            },
        },
        Instruction::Ret { value: Reg(0) },
    ]);

    let main_name = module.intern_string("main");
    let mut main_desc = FunctionDescriptor::new(main_name);
    main_desc.id = FunctionId(0);
    main_desc.bytecode_offset = 0;
    main_desc.bytecode_length = main_body.len() as u32;
    main_desc.register_count = 8;
    module.functions.push(main_desc);

    // `run_global_ctors` only executes ctors whose descriptor name starts
    // with `__tls_init_` — mirror the synthetic naming.
    let ctor_name = module.intern_string("__tls_init_PINNED");
    let mut ctor_desc = FunctionDescriptor::new(ctor_name);
    ctor_desc.id = FunctionId(1);
    ctor_desc.bytecode_offset = main_body.len() as u32;
    ctor_desc.bytecode_length = ctor_body.len() as u32;
    ctor_desc.register_count = 8;
    module.functions.push(ctor_desc);

    let mut bc = main_body;
    bc.extend_from_slice(&ctor_body);
    module.bytecode = bc;

    module.global_ctors.push((FunctionId(1), 65535));
    Arc::new(module)
}

#[test]
fn crashed_xmod_ctor_is_lenient_skipped_and_unwound() {
    let mut interp = Interpreter::new(build_module());

    // The XMOD-band FunctionNotFound inside a `__tls_init_*` ctor is the
    // lenient class: run_global_ctors WARNs and continues instead of
    // propagating.
    interp
        .run_global_ctors()
        .expect("XMOD-band ctor crash must be lenient-skipped");

    // CTOR-UNWIND pin: the crashed ctor's frames are gone — the stack is
    // back at its pre-ctor depth.
    assert_eq!(
        interp.state.call_stack.depth(),
        0,
        "lenient skip must unwind the crashed ctor's dead frames"
    );

    // The regression's user-visible shape: the NEXT execution used to
    // Ret-pop into the dead ctor frame and re-raise its FunctionNotFound.
    // Post-fix it completes cleanly with its own value.
    let v: Value = interp
        .execute_function(FunctionId(0))
        .expect("execution after a lenient-skipped ctor must not inherit the ctor's error");
    assert!(v.is_int());
    assert_eq!(v.as_i64(), 7);
}

#[test]
fn repeated_executions_after_crashed_ctor_stay_clean() {
    // The suite runner executes many test fns on interpreters that ran
    // ctors first; every one of them must stay isolated from the crash.
    let mut interp = Interpreter::new(build_module());
    interp.run_global_ctors().expect("lenient skip");
    for _ in 0..3 {
        let v = interp
            .execute_function(FunctionId(0))
            .expect("each top-level execution terminates at its own entry frame");
        assert_eq!(v.as_i64(), 7);
        assert_eq!(interp.state.call_stack.depth(), 0);
    }
}
